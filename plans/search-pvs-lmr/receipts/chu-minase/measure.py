"""Measure PVS, LMR and Minase model effects on Haitaka main 8630a75 (macOS mba copy)."""
from concurrent.futures import ThreadPoolExecutor
from collections import Counter
from contextlib import ExitStack
from datetime import datetime
import argparse
import fcntl
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import queue
import statistics
import subprocess
import time

out = Path(__file__).resolve().parent
inputs = out.parent/'inputs'
models = {
    'v3': (inputs/'reference.nnue', '415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9', 'minase-layer-stack'),
    'v2.6': (inputs/'nnue.bin', 'faa5fbe262745363caa55f7516b38bffdf183182ac9d1b9c66f2645194f3a985', 'minase-mnue-v5'),
}
comparisons = {
    'v3-pvs-vs-baseline': (('pvs', 'v3'), ('baseline', 'v3')),
    'v3-lmr-vs-pvs': (('lmr', 'v3'), ('pvs', 'v3')),
    'v3-vs-v26': (('baseline', 'v3'), ('baseline', 'v2.6')),
}
counters = ('pvsProbes', 'pvsResearches', 'lmrReductions', 'lmrResearches')
cpus = (0, 1, 2, 3)
depth = 64
movetime = 1000
cap = 4096
commit = '8630a75c6c3e9aaf700311fa4dd92480c6ea025c'
spec = importlib.util.spec_from_file_location('match', inputs/'handcrafted_json_match.py')
match = importlib.util.module_from_spec(spec)
spec.loader.exec_module(match)


class Worker(match.Worker):
    def __init__(self, binary, log, cpu, model=None):
        self.log = log.open('wb')
        args = [str(out/binary)]
        if model is not None:
            args.append(str(models[model][0]))
        self.process = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log, bufsize=0)
        self.buffer = b''
        self.sequence = 0
        self.watchdog = 45
        os.set_blocking(self.process.stdin.fileno(), False)
        try:
            assert self.call('info') == {'modelFormat': models[model][2] if model else None,
                                         'pvs': binary != 'baseline', 'lmr': binary == 'lmr'}
        except BaseException:
            self.close()
            raise


def log(*message):
    print(datetime.now().astimezone().isoformat(timespec='seconds'), *message, flush=True)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_setup():
    for path, expected, _ in models.values():
        assert digest(path) == expected, path
    source = inputs/'setup.json'
    openings = json.loads(source.read_text())['openings']
    assert len(openings) == len({json.dumps(o['position'], sort_keys=True) for o in openings}) == 16
    cfg = dict(host='mba (Apple M4, 4 performance cores, no CPU pinning)', commit=commit,
               experimentPatchSha256=digest(out/'experiment.patch'), moveMs=movetime, depth=depth, maxPlies=cap, capIncludesOpening=True,
               cpus=cpus, gamesPerComparison=32, comparisons=comparisons, openings=openings,
               openingSource=str(source), openingSourceSha256=digest(source),
               modelSha256={k:v[1] for k,v in models.items()},
               binarySha256={name:digest(out/name) for name in ('baseline','pvs','lmr')})
    manifest = out/'manifest.json'
    if manifest.exists():
        assert json.loads(manifest.read_text()) == json.loads(json.dumps(cfg))
    else:
        match.atomic_json(manifest, cfg)
    return openings


def check_search(result, build):
    assert result['schemaVersion'] == 2
    stats = result['searchStats']
    assert set(stats) == set(counters)
    assert stats['pvsResearches'] <= stats['pvsProbes']
    assert stats['lmrResearches'] <= stats['lmrReductions']
    assert build == 'lmr' or stats['lmrReductions'] == 0
    assert build != 'baseline' or stats['pvsProbes'] == 0


def bench(openings):
    fixed_positions = [openings[0], *openings[1:4]]
    saved = inputs/'openings.json'
    fixed_positions.extend(row['request'] for row in json.loads(saved.read_text())[::3])
    linux = json.loads((inputs/'bench-linux.json').read_text())
    assert len(linux) == 40
    records = []
    with ExitStack() as stack:
        workers = {}
        for build in ('baseline','pvs','lmr'):
            for model in models:
                worker = Worker(build, out/f'bench-{build}-{model}.log', cpus[0], model)
                stack.callback(worker.close)
                workers[build,model] = worker
        for i, position in enumerate(fixed_positions):
            for dep in (3,4):
                for model in models:
                    rows = {}
                    for build in ('baseline','pvs','lmr'):
                        result = workers[build,model].call('search', position=position, depth=dep, timeoutMs=30000, nodes=0)
                        check_search(result, build)
                        assert result['completedDepth'] == dep or (result['bestMove'] and abs(result['score']) >= 29_000), (i, dep, build, result)
                        rows[build] = result
                    reference = linux[len(records)]
                    assert (reference['position'], reference['depth'], reference['model']) == (i, dep, model)
                    for build in rows:
                        for key in ('score','bestMove','states','completedDepth','outcome','searchStats'):
                            assert rows[build][key] == reference['builds'][build][key], (i, dep, model, build, key)
                    assert rows['baseline']['score'] == rows['pvs']['score'], (i,dep,model,rows)
                    records.append(dict(position=i, depth=dep, model=model, builds=rows))
                match.atomic_json(out/'bench.json', records)
                log('fixed-depth verified against linux', i, dep)
    assert any(r['builds']['lmr']['searchStats']['lmrReductions'] for r in records)


def trace_entries(path):
    if not path.exists():
        return []
    data = path.read_bytes()
    if data and not data.endswith(b'\n'):
        length = data.rfind(b'\n')+1
        with path.open('r+b') as file:
            file.truncate(length)
        data = data[:length]
    return [json.loads(line) for line in data.splitlines()]


def verify(ref, game, opening, engines):
    assert game['opening'] == opening
    position = game['final']
    history = opening['history']['moves']
    assert position['history']['moves'] == history + [r['move'] for r in game['trace']]
    assert ref.call('validate', position=position) == position
    assert game['totalPlies'] == len(position['history']['moves']) <= cap
    assert game['moves'] == len(game['trace'])
    observer = ref.call('inspect', position=position)
    assert observer['outcome'] == game['outcome'] and observer['contextHash'] == game['contextHash']
    side = opening['position']['sideToMove']
    for ply, entry in enumerate(game['trace'], start=1):
        seat = 0 if side == game['aColor'] else 1
        assert entry['ply'] == ply and entry['seat'] == seat and entry['side'] == side
        search = entry['search']
        assert search['bestMove'] == entry['move'] and search['outcome']['kind'] == 'ongoing'
        check_search(search, engines[seat][0])
        side ^= 1
    outcome = game['outcome']
    assert game['aScore'] == (None if outcome['kind']=='ongoing' else
                              (.5 if outcome['kind']=='draw' else float(outcome['winner']==game['aColor'])))
    assert game['aScore'] is not None or game['totalPlies'] == cap


def paths(label, pair, color):
    basename = f'{out/label}/pair-{pair:02}-{color}'
    return Path(basename+'.json'), Path(basename+'.jsonl')


def play(label, pair, color, opening, ref, workers):
    engines = comparisons[label]
    final_path, trace_path = paths(label, pair, color)
    started = time.monotonic()
    position = opening
    previous = trace_entries(trace_path)
    assert ref.call('validate', position=opening) == opening
    for entry in previous:
        position = ref.call('apply', position=position, move=entry['move'])
    log('start', label, pair, color, 'ply', len(position['history']['moves']))
    trace = list(previous)
    with trace_path.open('a', buffering=1) as file:
        while True:
            status = ref.call('inspect', position=position)
            outcome = match.check_outcome(status['outcome'])
            total = len(position['history']['moves'])
            if outcome['kind'] != 'ongoing' or total == cap:
                break
            side = position['position']['sideToMove']
            seat = 0 if side == color else 1
            t0 = time.monotonic()
            search = workers[engines[seat]].call('search', position=position, depth=depth, timeoutMs=movetime, nodes=0)
            rpc_ms = 1000 * (time.monotonic()-t0)
            check_search(search, engines[seat][0])
            move = search['bestMove']
            assert search['outcome'] == outcome and move is not None and not search['nodeLimitReached']
            position = ref.call('apply', position=position, move=move)
            assert side ^ 1 == position['position']['sideToMove']
            assert position['history']['moves'][-1] == move
            entry = dict(ply=len(trace)+1, side=side, seat=seat, move=move, search=search, rpcMs=rpc_ms)
            file.write(json.dumps(entry, ensure_ascii=False, separators=(',',':'))+'\n')
            file.flush()
            os.fsync(file.fileno())
            trace.append(entry)
            if (total+1) % 100 == 0:
                log('progress', label, pair, color, 'ply', total+1)
    score = None if outcome['kind']=='ongoing' else (.5 if outcome['kind']=='draw' else float(outcome['winner']==color))
    game = dict(pair=pair, aColor=color, opening=opening, final=position, totalPlies=total,
                outcome=outcome, aScore=score, contextHash=status['contextHash'], trace=trace,
                moves=len(trace), wallSeconds=time.monotonic()-started)
    verify(ref, game, opening, engines)
    match.atomic_json(final_path, game)
    log('done', label, pair, color, 'ply', total, 'score', score)


def work(cpu, jobs):
    with ExitStack() as stack:
        ref = Worker('baseline', out/f'referee-{cpu}.log', cpu)
        stack.callback(ref.close)
        workers = {}
        while True:
            try:
                label, pair, color, opening = jobs.get_nowait()
            except queue.Empty:
                return
            for build, model in comparisons[label]:
                if (build, model) not in workers:
                    worker = Worker(build, out/f'{build}-{model}-{cpu}.log', cpu, model)
                    stack.callback(worker.close)
                    workers[build,model] = worker
            play(label, pair, color, opening, ref, workers)


def pair_stats(games, threshold):
    observed = {(g['pair'],g['aColor']): (g['aScore'] if g['totalPlies']<=threshold else None) for g in games}
    wdl = Counter(observed.values())
    pair_scores = []
    excluded = 0
    for pair in range(16):
        if any((pair,color) not in observed for color in (0,1)):
            continue
        scores = [observed[pair,color] for color in (0,1)]
        if None in scores:
            excluded += 1
        else:
            pair_scores.append(statistics.mean(scores))
    score = statistics.mean(pair_scores) if pair_scores else None
    elo = None
    ci = None
    if score is not None and 0 < score < 1:
        elo = 400*math.log10(score/(1-score))
        if len(set(pair_scores)) >= 2:
            half = 1.96*400/math.log(10)/((1-score)*score)*statistics.stdev(pair_scores)/math.sqrt(len(pair_scores))
            ci = [elo-half,elo+half]
    return dict(games=len(games), wins=wdl[1], losses=wdl[0], draws=wdl[.5], capped=wdl[None],
                usablePairs=len(pair_scores), excludedPairs=excluded, scores=pair_scores,
                score=score, elo=elo, elo95=ci)


def per_side(games, label):
    sides = {}
    for seat,(build,model) in enumerate(comparisons[label]):
        trace = [r for g in games for r in g['trace'] if r['seat']==seat]
        rows = [r['search'] for r in trace]
        sides[f'{model}-{build}'] = dict(
            searches=len(rows), meanDepth=statistics.mean(r['completedDepth'] for r in rows) if rows else None,
            depth4Plus=sum(r['completedDepth']>=4 for r in rows),
            fallback=sum(r['completedDepth']==0 for r in rows),
            nodes=sum(r['states'] for r in rows),
            meanNps=statistics.mean(r['nps'] for r in rows) if rows else None,
            meanRpcMs=statistics.mean(r['rpcMs'] for r in trace) if trace else None,
            searchesWithLmr=sum(r['searchStats']['lmrReductions']>0 for r in rows),
            **{k:sum(r['searchStats'][k] for r in rows) for k in counters})
    return sides


def aggregate(openings, audit=None):
    report = {}
    for label in comparisons:
        games = []
        for pair, opening in enumerate(openings):
            for color in (0,1):
                file, trace = paths(label, pair, color)
                if file.exists():
                    game = json.loads(file.read_text())
                    if audit:
                        assert game['pair']==pair and game['aColor']==color
                        assert game['trace'] == [json.loads(line) for line in trace.read_text().splitlines()]
                        verify(audit, game, opening, comparisons[label])
                    games.append(game)
        report[label] = dict(caps={n:pair_stats(games,n) for n in (200,400,cap)},
                             byEngine=per_side(games,label),
                             maxPlies=max((g['totalPlies'] for g in games),default=0),
                             verifiedMoves=sum(g['moves'] for g in games))
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage', choices=('bench','run','status','audit'))
    args = parser.parse_args()
    openings = read_setup()
    if args.stage == 'status':
        for label,row in aggregate(openings).items():
            log(label, row['caps'][cap])
        return
    with (out/'run.lock').open('a') as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        if args.stage == 'bench':
            bench(openings)
            return
        if args.stage == 'run':
            assert len(json.loads((out/'bench.json').read_text())) == 40
            jobs=queue.Queue()
            for label in comparisons:
                (out/label).mkdir(exist_ok=True)
            for pair, opening in enumerate(openings):
                for color in (0,1):
                    for label in comparisons:
                        if not paths(label,pair,color)[0].exists():
                            jobs.put((label,pair,color,opening))
            log('start remaining games',jobs.qsize())
            with ThreadPoolExecutor(max_workers=len(cpus)) as pool:
                list(pool.map(lambda cpu:work(cpu,jobs),cpus))
        ref = Worker('baseline',out/'audit.log',cpus[0])
        try:
            report = aggregate(openings,ref)
        finally:
            ref.close()
        assert all(row['caps'][cap]['games']==32 for row in report.values())
        match.atomic_json(out/'summary.json',report)
        for label,row in report.items():
            log('audit complete',label,row['caps'][cap])


if __name__=='__main__':
    main()

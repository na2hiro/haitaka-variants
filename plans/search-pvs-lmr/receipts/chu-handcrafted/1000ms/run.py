from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import select
import statistics
import subprocess
import time

out = Path(__file__).resolve().parent
binaries = out.parent
repo = out.parents[2]
module_path = repo / 'scripts/handcrafted_json_match.py'
spec = importlib.util.spec_from_file_location('json_match', module_path)
referee = importlib.util.module_from_spec(spec)
spec.loader.exec_module(referee)
engines = {name: binaries / name for name in ('baseline', 'pvs', 'lmr')}
comparisons = [('pvs', 'baseline'), ('lmr', 'pvs')]
cpus = [0, 1, 2, 3]
movetime = 1000
max_plies = 200
max_depth = 64
counter_fields = ('pvsProbes', 'pvsResearches', 'lmrReductions', 'lmrResearches')
assert set(cpus) <= os.sched_getaffinity(0)
assert len({Path(f'/sys/devices/system/cpu/cpu{cpu}/topology/core_id').read_text() for cpu in cpus}) == 4

def write_json(path, value):
    referee.atomic_json(path, value)

def pin(worker, cpu):
    os.sched_setaffinity(worker.process.pid, {cpu})

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def make_usi(mv):
    squares = [mv['from'], *mv['path']]
    return ''.join(f'{file}{chr(96 + rank)}' for file, rank in squares) + ('+' if mv['promotion'] else '')

class Usi:
    def __init__(self, binary, log, cpu):
        self.log = log.open('wb')
        self.process = subprocess.Popen(
            ['taskset', '-c', str(cpu), str(binary), 'usi', '--eval', 'handcrafted',
             '--movetime-max-depth', str(max_depth)], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=self.log, bufsize=0,
        )
        self.buffer = b''
        self.send('usi\nisready')
        self.read_until('usiok', 10)
        self.read_until('readyok', 10)

    def send(self, text):
        self.process.stdin.write((text + '\n').encode())
        self.process.stdin.flush()

    def read_until(self, prefix, timeout):
        deadline = time.monotonic() + timeout
        lines = []
        while True:
            while b'\n' not in self.buffer:
                left = deadline - time.monotonic()
                if left <= 0 or not select.select([self.process.stdout], [], [], left)[0]:
                    raise RuntimeError(f'USI watchdog waiting for {prefix}: {lines}')
                data = os.read(self.process.stdout.fileno(), 65536)
                if not data:
                    raise RuntimeError(f'USI exited waiting for {prefix}: {self.process.poll()}')
                self.buffer += data
            raw, self.buffer = self.buffer.split(b'\n', 1)
            line = raw.decode()
            if line.startswith(('info string invalid', 'info string unsupported')):
                raise RuntimeError(line)
            lines.append(line)
            if line.startswith(prefix):
                return lines

    def search(self, history):
        command = 'position startpos'
        if history:
            command += ' moves ' + ' '.join(history)
        began = time.monotonic()
        self.send(f'{command}\ngo movetime {movetime} depth {max_depth}')
        lines = self.read_until('bestmove ', 15)
        wall_ms = 1000 * (time.monotonic() - began)
        infos = [line for line in lines if line.startswith('info depth ')]
        assert len(infos) == 1, lines
        words = infos[0].split(' pv ', 1)[0].replace(' score cp ', ' score ').split()
        stats = {k: int(v) for k, v in zip(words[1::2], words[2::2], strict=True)}
        for key in ('depth', 'time', 'nodes', 'score', *counter_fields):
            assert key in stats, infos
        return dict(best=lines[-1].split()[1], **stats, rpcWallMs=wall_ms)

    def close(self):
        try:
            if self.process.poll() is None:
                self.send('quit')
                try:
                    self.process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait()
        finally:
            self.process.stdin.close()
            self.process.stdout.close()
            self.log.close()

source = repo / 'target/handcrafted-eval-phase0/inputs/chushogi/fixtures.json'
rows = [row for row in json.loads(source.read_text())['fixtures']
        if row['partition'] in ('selection', 'confirmation')]
assert len(rows) == len({row['family'] for row in rows}) == 16
# Freeze openings and their exact, legal, completed-turn history.
validator = referee.Worker(engines['baseline'], out/'fixture-stderr.log', 20)
try:
    start = validator.call('startpos')
    openings = []
    for row in rows:
        pos = validator.call('validate', position=row['request'])
        assert pos['history']['initial'] == start['position'] and pos['pending'] is None
        assert validator.call('inspect', position=pos)['outcome']['kind'] == 'ongoing'
        openings.append(dict(family=row['family'], partition=row['partition'], request=pos))
finally:
    validator.close()
assert len({json.dumps(row['request']['position'], sort_keys=True) for row in openings}) == 16
write_json(out/'openings.json', openings)
write_json(out/'config.json', dict(
    movetimeMs=movetime, depthLimit=max_depth, maxPlies=max_plies, workers=cpus,
    gamesPerComparison=2*len(openings), comparisons=comparisons, fixtureSource=str(source),
    fixtureSha256=digest(source), openingSha256=digest(out/'openings.json'),
    engines={k:dict(path=str(v),sha256=digest(v)) for k,v in engines.items()},
))

def play(agents, ref, initial, black, game_id):
    position = initial
    history = [make_usi(mv) for mv in position['history']['moves']]
    assert len({make_usi(mv) for mv in position['history']['moves']}) > 0
    moves = []
    searches = []
    began = time.monotonic()
    for ply in range(max_plies+1):
        observed = ref.call('inspect', position=position)
        outcome = referee.check_outcome(observed['outcome'])
        if outcome['kind'] != 'ongoing':
            reason = 'rule-' + outcome['kind']
            break
        if ply == max_plies:
            outcome = dict(kind='draw', winner=None)
            reason = 'maximum-ply-capped'
            break
        side = position['position']['sideToMove']
        engine = agents[0] if side == black else agents[1]
        search = engine.search(history)
        search['seat'] = 'A' if side == black else 'B'
        # Decode by matching complete paths against the rules engine.
        legal = ref.call('moves', position=position)
        matching = [mv for mv in legal if make_usi(mv) == search['best']]
        assert len(matching) == 1, (game_id, ply, search['best'])
        move = matching[0]
        position = ref.call('apply', position=position, move=move)
        assert position['position']['sideToMove'] == (side ^ 1)
        assert position['history']['moves'][-1] == move
        history.append(search['best'])
        searches.append(search)
        moves.append(move)
    return dict(
        aColor=black, opening=initial, moves=moves, finalPosition=position,
        outcome=outcome, terminationReason=reason, finalContextHash=observed['contextHash'],
        aScore=.5 if outcome['winner'] is None else float(outcome['winner']==black),
        plies=len(moves), searches=searches, wallSeconds=time.monotonic()-began,
    )

def shard(a,b,slot):
    d = out / f'{a}-vs-{b}'
    d.mkdir(exist_ok=True)
    data = d / f'worker-{slot}.jsonl'
    previous = [json.loads(line) for line in data.read_text().splitlines()] if data.exists() else []
    done = {(g['pairIndex'],g['aColor']) for g in previous}
    assert len(done) == len(previous)
    with ExitStack() as resources:
        ref = referee.Worker(engines['baseline'], d/f'worker-{slot}-referee.log', 20)
        resources.callback(ref.close)
        pin(ref, cpus[slot])
        agents = []
        for seat, label in (('A', a), ('B', b)):
            worker = Usi(engines[label], d/f'worker-{slot}-{seat}.log', cpus[slot])
            resources.callback(worker.close)
            agents.append(worker)
        with data.open('a') as log:
            for pair in range(slot, len(openings), len(cpus)):
                initial = openings[pair]['request']
                for color in (0, 1):
                    if (pair,color) in done:
                        continue
                    game = play(agents, ref, initial, color, (a,b,pair,color))
                    game.update(pairIndex=pair, gameIndex=2*pair+color)
                    log.write(json.dumps(game, separators=(',',':'))+'\n')
                    log.flush()
                    os.fsync(log.fileno())
                    previous.append(game)
                    print(time.strftime('%H:%M:%S'), f'{a}-vs-{b}', f'pair={pair+1} color={color}',
                          f"plies={game['plies']} score={game['aScore']} {game['terminationReason']}", flush=True)
    return previous

def aggregate(games, a, b, labels=engines.keys()):
    games = sorted(games, key=lambda game:game['gameIndex'])
    scores = []
    for index, opening in enumerate(openings):
        first, second = games[2*index:2*index+2]
        assert first['aColor']==0 and second['aColor']==1
        assert first['pairIndex']==second['pairIndex']==index
        assert first['opening']==second['opening']==opening['request']
        scores.append((first['aScore']+second['aScore'])/2)
    counts = Counter(g['aScore'] for g in games)
    mean = statistics.mean(scores)
    bounded = min(.999,max(.001,mean))
    elo = 400 * math.log10(bounded/(1-bounded))
    half = 1.96 * 400 / math.log(10) / (bounded*(1-bounded)) * statistics.stdev(scores)/math.sqrt(len(scores))
    stats = {}
    for seat, label in [('A',a),('B',b)]:
        search = [s for g in games for s in g['searches'] if s['seat']==seat]
        totals = {key:sum(s[key] for s in search) for key in counter_fields}
        if label == 'baseline':
            assert all(v==0 for v in totals.values()), totals
        if label == 'pvs':
            assert totals['pvsProbes'] > 0 and totals['lmrReductions'] == 0, totals
        if label == 'lmr':
            assert totals['lmrReductions'] > 0, totals
        stats[label] = dict(totalSearches=len(search), totalNodes=sum(s['nodes'] for s in search),
                            meanDepth=statistics.mean(s['depth'] for s in search),
                            depthHistogram=dict(sorted(Counter(s['depth'] for s in search).items())),
                            searchesWithLmr=sum(s['lmrReductions']>0 for s in search),
                            meanSearchMs=statistics.mean(s['time'] for s in search),
                            medianRpcMs=statistics.median(s['rpcWallMs'] for s in search),
                            maxRpcMs=max(s['rpcWallMs'] for s in search), **totals)
    return dict(games=len(games),pairs=len(scores),wins=counts[1],losses=counts[0],draws=counts[.5],
                score=mean,pairedElo=elo,pairedElo95Ci=[elo-half,elo+half],pairScores=scores,
                termination=dict(Counter(g['terminationReason'] for g in games)),
                avgPlies=statistics.mean(g['plies'] for g in games),stats=stats)

print(time.strftime('%H:%M:%S'), 'validated 16 openings, 4 distinct physical cores', flush=True)
summary = {}
for a,b in comparisons:
    with ThreadPoolExecutor(max_workers=len(cpus)) as pool:
        shards = list(pool.map(lambda slot: shard(a,b,slot), range(len(cpus))))
    games = [game for group in shards for game in group]
    assert len(games) == 2 * len(openings)
    label = f'{a}-vs-{b}'
    summary[label] = aggregate(games,a,b)
    write_json(out/'summary.json', summary)
    print(time.strftime('%H:%M:%S'), label, json.dumps(summary[label]), flush=True)

"""Continue the 200-ply chushogi trial with its original engines and histories."""
import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
from datetime import datetime
import fcntl
import importlib.util
import json
import math
import os
from pathlib import Path
from queue import Empty, Queue
import statistics
import time

from usi import Usi, digest, make_usi, pin

out = Path(__file__).resolve().parent
previous = out.parent / '1000ms'
repo = out.parents[2]
spec = importlib.util.spec_from_file_location('json_match', repo/'scripts/handcrafted_json_match.py')
referee = importlib.util.module_from_spec(spec)
spec.loader.exec_module(referee)
CAP = 4096
CAPPED = 'maximum-ply-capped'
LABELS = ('pvs-vs-baseline', 'lmr-vs-pvs')
config = json.loads((previous/'config.json').read_text())
cpus = config['workers']
engines = {name: Path(engine['path']) for name, engine in config['engines'].items()}
openings = json.loads((previous/'openings.json').read_text())


def print_progress(*fields):
    print(datetime.now().astimezone().isoformat(timespec='seconds'), *fields, flush=True)


def read_input():
    assert config['movetimeMs'] == 1000 and config['depthLimit'] == 64 and config['maxPlies'] == 200
    assert len(cpus) == 4 and set(cpus) <= os.sched_getaffinity(0)
    assert len({Path(f'/sys/devices/system/cpu/cpu{cpu}/topology/core_id').read_text() for cpu in cpus}) == 4
    assert len(openings) == len({o['family'] for o in openings}) == 16
    assert digest(previous/'openings.json') == config['openingSha256']
    for name, path in engines.items():
        assert digest(path) == config['engines'][name]['sha256'], name

    records = {}
    hashes = {}
    for label in LABELS:
        entries = []
        for path in sorted((previous/label).glob('worker-*.jsonl')):
            hashes[str(path.relative_to(previous))] = digest(path)
            entries.extend(json.loads(line) for line in path.read_text().splitlines())
        records[label] = sorted(entries, key=lambda game: game['gameIndex'])
        assert [game['gameIndex'] for game in records[label]] == list(range(32))
        assert sum(game['terminationReason'] == CAPPED for game in entries) == 22
        for game in entries:
            index = game['gameIndex']
            assert game['aColor'] == index % 2 and game['pairIndex'] == index // 2
            assert game['opening'] == openings[index // 2]['request']
            assert game['finalPosition']['history']['moves'] == game['opening']['history']['moves'] + game['moves']
            assert game['plies'] == len(game['moves']) == len(game['searches']) <= 200
    manifest = dict(config, maxPlies=CAP, resumedFromMaxPlies=200, previousGamesSha256=hashes,
                    capPolicy='exclude-entire-pair', capCountsFrom='paired-opening')
    path = out/'manifest.json'
    if path.exists():
        assert json.loads(path.read_text()) == manifest
    else:
        referee.atomic_json(path, manifest)
    return records


def check_position(ref, game):
    position = game['finalPosition']
    assert ref.call('validate', position=position) == position
    observation = ref.call('inspect', position=position)
    assert observation['contextHash'] == game['finalContextHash']
    outcome = referee.check_outcome(observation['outcome'])
    if game['terminationReason'] == CAPPED:
        assert outcome['kind'] == 'ongoing'
    else:
        assert outcome == game['outcome']
    return outcome


def check_search(search, move, side, a_color):
    assert search['best'] == make_usi(move)
    assert search['seat'] == ('A' if side == a_color else 'B')
    assert search['pvsResearches'] <= search['pvsProbes']
    assert search['lmrResearches'] <= search['lmrReductions']


def checkpoint_path(label, index):
    return out/label/f'game-{index:02}.jsonl'


def journal_entries(path):
    if not path.exists():
        return []
    data = path.read_bytes()
    if data and not data.endswith(b'\n'):
        good_size = data.rfind(b'\n') + 1
        with path.open('r+b') as file:
            file.truncate(good_size)
            os.fsync(file.fileno())
        data = data[:good_size]
        print_progress('recovered incomplete checkpoint', path)
    return [json.loads(line) for line in data.splitlines()]


def verify_game(ref, original, game):
    for key in ('opening', 'aColor', 'pairIndex', 'gameIndex'):
        assert original[key] == game[key]
    assert game['moves'][:original['plies']] == original['moves']
    assert game['searches'][:original['plies']] == original['searches']
    assert game['finalPosition']['history']['moves'] == game['opening']['history']['moves'] + game['moves']
    assert game['plies'] == len(game['moves']) == len(game['searches']) <= CAP
    outcome = check_position(ref, game)
    if game['terminationReason'] == CAPPED:
        assert game['plies'] == CAP and game['aScore'] is None
    else:
        assert game['terminationReason'] == 'rule-' + outcome['kind']
        assert game['aScore'] == (.5 if outcome['winner'] is None else float(outcome['winner'] == game['aColor']))
    side = game['opening']['position']['sideToMove']
    for index, (search, move) in enumerate(zip(game['searches'], game['moves'], strict=True)):
        check_search(search, move, side ^ (index % 2), game['aColor'])


def continue_game(ref, agents, label, game):
    index = game['gameIndex']
    path = checkpoint_path(label, index)
    final_path = path.with_suffix('.json')
    if final_path.exists():
        result = json.loads(final_path.read_text())
        verify_game(ref, game, result)
        return result

    check_position(ref, game)
    position = game['finalPosition']
    history = [make_usi(move) for move in position['history']['moves']]
    moves = game['moves'].copy()
    searches = game['searches'].copy()
    elapsed = game['wallSeconds']
    for entry in journal_entries(path):
        side = position['position']['sideToMove']
        move = entry['move']
        search = entry['search']
        assert entry['ply'] == len(moves) + 1 <= CAP
        check_search(search, move, side, game['aColor'])
        position = ref.call('apply', position=position, move=move)
        assert position['position']['sideToMove'] == (side ^ 1)
        history.append(search['best'])
        moves.append(move)
        searches.append(search)
        elapsed += entry['wallSeconds']

    print_progress('start', label, f'game={index:02}', f'ply={len(moves)}')
    with path.open('a', buffering=1) as journal:
        while True:
            began = time.monotonic()
            observation = ref.call('inspect', position=position)
            outcome = referee.check_outcome(observation['outcome'])
            if outcome['kind'] != 'ongoing':
                reason = 'rule-' + outcome['kind']
                score = .5 if outcome['winner'] is None else float(outcome['winner'] == game['aColor'])
                break
            if len(moves) == CAP:
                reason = CAPPED
                score = None
                break
            side = position['position']['sideToMove']
            player = 0 if side == game['aColor'] else 1
            search = agents[player].search(history)
            search['seat'] = 'A' if player == 0 else 'B'
            legal = ref.call('moves', position=position)
            matching = [move for move in legal if make_usi(move) == search['best']]
            assert len(matching) == 1, (label, index, len(moves), search['best'])
            move = matching[0]
            check_search(search, move, side, game['aColor'])
            position = ref.call('apply', position=position, move=move)
            assert position['position']['sideToMove'] == (side ^ 1)
            assert position['history']['moves'][-1] == move

            history.append(search['best'])
            moves.append(move)
            searches.append(search)
            wall = time.monotonic() - began
            elapsed += wall
            journal.write(json.dumps(dict(ply=len(moves), move=move, search=search, wallSeconds=wall),
                                     separators=(',', ':'))+'\n')
            journal.flush()
            os.fsync(journal.fileno())
            if len(moves) % 100 == 0:
                print_progress('progress', label, f'game={index:02}', f'ply={len(moves)}')
    result = dict(game, finalPosition=position, moves=moves, searches=searches, plies=len(moves),
                  outcome=outcome, terminationReason=reason, aScore=score, wallSeconds=elapsed,
                  finalContextHash=observation['contextHash'])
    verify_game(ref, game, result)
    referee.atomic_json(final_path, result)
    print_progress('done', label, f'game={index:02}', f'ply={len(moves)}', f'score={score}', reason)
    return result


def work(cpu, queue):
    with ExitStack() as stack:
        ref = referee.Worker(engines['baseline'], out/f'referee-cpu{cpu}.log', 30)
        stack.callback(ref.close)
        pin(ref, cpu)
        # Reuse each stable USI process as the first stage did.
        agents_by_label = {}
        while True:
            try:
                label, game = queue.get_nowait()
            except Empty:
                return
            if label not in agents_by_label:
                agents = []
                for seat, engine in zip(('A','B'), label.split('-vs-'), strict=True):
                    agent = Usi(engines[engine], out/label/f'cpu{cpu}-{seat}.log', cpu)
                    stack.callback(agent.close)
                    agents.append(agent)
                agents_by_label[label] = agents
            continue_game(ref, agents_by_label[label], label, game)


def load_results(records, ref):
    complete = {}
    for label, games in records.items():
        result = []
        for game in games:
            if game['terminationReason'] == CAPPED:
                file = checkpoint_path(label, game['gameIndex']).with_suffix('.json')
                if not file.exists():
                    continue
                updated = json.loads(file.read_text())
                verify_game(ref, game, updated)
                result.append(updated)
            else:
                verify_game(ref, game, game)
                result.append(game)
        complete[label] = result
    return complete


def summarize(games, cap):
    counts = Counter(g['aScore'] if g['plies'] <= cap else None for g in games)
    by_index = {g['gameIndex']: g for g in games}
    pair_scores = []
    discarded = 0
    for i in range(0, 32, 2):
        if i not in by_index or i+1 not in by_index:
            continue
        pair = (by_index[i], by_index[i+1])
        if any(g['plies'] > cap or g['aScore'] is None for g in pair):
            discarded += 1
        else:
            pair_scores.append(statistics.mean(g['aScore'] for g in pair))

    paired_score = statistics.mean(pair_scores) if pair_scores else None
    elo = (400 * math.log10(paired_score/(1-paired_score))
           if paired_score is not None and 0 < paired_score < 1 else None)
    interval = None
    if elo is not None and len(set(pair_scores)) >= 2:
        stderr = (400 / math.log(10) / (paired_score * (1 - paired_score))
                  * statistics.stdev(pair_scores) / math.sqrt(len(pair_scores)))
        interval = [elo - 1.96*stderr, elo + 1.96*stderr]
    return dict(games=len(games), wins=counts[1], losses=counts[0], draws=counts[.5],
                capped=counts[None], usablePairs=len(pair_scores), excludedPairs=discarded,
                pairScores=pair_scores, score=paired_score, pairedElo=elo, pairedElo95Ci=interval)


def search_stats(games, label):
    stats = {}
    for seat, name in zip(('A','B'), label.split('-vs-'), strict=True):
        searches = [s for game in games for s in game['searches'] if s['seat'] == seat]
        continuation = [s for game in games for s in game['searches'][200:] if s['seat'] == seat]
        def describe(rows):
            return dict(searches=len(rows),
                        meanDepth=statistics.mean(s['depth'] for s in rows) if rows else None,
                        depth4Plus=sum(s['depth'] >= 4 for s in rows),
                        pvsProbes=sum(s['pvsProbes'] for s in rows),
                        pvsResearches=sum(s['pvsResearches'] for s in rows),
                        lmrReductions=sum(s['lmrReductions'] for s in rows),
                        lmrResearches=sum(s['lmrResearches'] for s in rows),
                        searchesWithLmr=sum(s['lmrReductions'] > 0 for s in rows))
        stats[name] = dict(all=describe(searches), continuation=describe(continuation))
    return stats


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--audit', action='store_true', help='Verify saved games and summarize without starting play.')
    args = parser.parse_args()
    with (out/'run.lock').open('a') as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        records = read_input()
        for label in LABELS:
            (out/label).mkdir(exist_ok=True)
        if not args.audit:
            queue = Queue()
            for index in range(32):
                for label in LABELS:
                    game = records[label][index]
                    if game['terminationReason'] == CAPPED and not checkpoint_path(label, index).with_suffix('.json').exists():
                        queue.put((label, game))
            print_progress('resume', queue.qsize(), 'games', 'limit', CAP)
            with ThreadPoolExecutor(max_workers=len(cpus)) as pool:
                list(pool.map(lambda cpu: work(cpu, queue), cpus))
        ref = referee.Worker(engines['baseline'], out/'audit-referee.log', 30)
        try:
            games = load_results(records, ref)
        finally:
            ref.close()
        report = {label: dict(caps={cap:summarize(games[label], cap)
                                   for cap in (200, 400, 600, 1000, CAP)},
                              maxPlies=max(g['plies'] for g in games[label]),
                              verifiedMoves=sum(g['plies'] for g in games[label]),
                              searchStats=search_stats(games[label],label)) for label in LABELS}
        complete = all(len(games[label]) == 32 for label in LABELS)
        referee.atomic_json(out/('summary.json' if complete else 'partial-summary.json'), report)
        for label in LABELS:
            print_progress('audit', label, report[label]['caps'][CAP])
        if not args.audit:
            assert complete


if __name__ == '__main__':
    main()

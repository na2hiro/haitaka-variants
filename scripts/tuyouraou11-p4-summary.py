#!/usr/bin/env python3
"""P4 browser metrics and predeclared paired, stratified bootstrap results."""
import argparse
import hashlib
import json
from pathlib import Path
import random
import statistics
import subprocess

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'target/tuyouraou11/p4'


def summarize_browser(path):
    data = json.loads(path.read_text())
    assert len(data['preparation']) == 20 and len(data['searches']) == 180
    result = {key: data[key] for key in ('mode', 'browser', 'version', 'positionsHash', 'host')}
    result['preparation'] = {}
    for cache in ('cold', 'warm'):
        rows = [r for r in data['preparation'] if r['cache'] == cache]
        result['preparation'][cache] = {
            key: {'median': statistics.median(r[key] for r in rows), 'max': max(r[key] for r in rows)}
            for key in ('totalMs', 'roundtripMs', 'wasmMs', 'modelMs', 'memoryBytes', 'binaryBytes')
        }
    result['search'] = {}
    for ms in (200, 1000, 3000):
        rows = [r for r in data['searches'] if r['ms'] == ms]
        assert len(rows) == 60 and all(r['bestMove'] for r in rows)
        result['search'][str(ms)] = {
            'count': len(rows),
            'engineOverrunMaxMs': max(r['elapsedMs'] - ms for r in rows),
            'roundtripOverrunMaxMs': max(r['roundtripMs'] - ms for r in rows),
            'memoryMaxBytes': max(r['memoryBytes'] for r in rows),
            'npsMedian': statistics.median(r['nps'] for r in rows),
            'depthMedian': statistics.median(r['completedDepth'] for r in rows),
            'mainTimerTicksMin': min(r['ticks'] for r in rows),
        }
    return result


def summarize_browsers():
    positions = {p['id']: p['sfen'] for p in json.loads((ROOT / 'plans/tuyouraou11/p4-positions.json').read_text())}
    summaries, checks = [], []
    for browser in ('chromium', 'firefox'):
        for mode in ('baseline', 'sfnn'):
            path = OUT / f'{mode}-{browser}.json'
            summary = summarize_browser(path)
            data = json.loads(path.read_text())
            assert max(r['roundtripMs'] for r in data['preparation']) < 10000
            assert max(r['memoryBytes'] for r in data['preparation'] + data['searches']) <= 134217728
            assert max(r['elapsedMs'] - r['ms'] for r in data['searches']) <= 100
            assert max(r['roundtripMs'] - r['ms'] for r in data['searches']) <= 200
            tsv = ''.join(positions[r['positionId']] + '\t' + r['bestMove'] + '\n' for r in data['searches'])
            legality = subprocess.run([str(ROOT / 'target/release/examples/sfnn_verify_moves')], input=tsv, text=True, capture_output=True, check=True)
            summaries.append(summary)
            checks.append({'mode': mode, 'browser': browser, 'rawSha256': hashlib.sha256(path.read_bytes()).hexdigest(), 'legality': legality.stdout.strip(), 'budget': 'PASS'})
    return {'schemaVersion': 1, 'summaries': summaries, 'checks': checks}


def game_scores(directory):
    rows = [json.loads(line) for line in (directory / 'self-play-games.jsonl').read_text().splitlines()]
    assert len(rows) == 100
    assert {r['gameIndex'] for r in rows} == set(range(1, 101))
    openings = (ROOT / 'plans/tuyouraou11/p4-openings.sfen').read_text().splitlines()
    assert len(openings) == len(set(openings)) == 50
    pairs = {}
    counts = {'wins': 0, 'draws': 0, 'losses': 0}
    for row in rows:
        assert row['pairIndex'] == (row['gameIndex'] - 1) // 2
        assert len(row['moves']) == row['plies'] <= 256
        assert row['opening']['suiteIndex'] == row['pairIndex']
        assert row['startSfen'] == openings[row['pairIndex']]
        assert row['aColor'] == ('black' if row['gameIndex'] % 2 else 'white')
        assert row['terminationReason'] in ('no-legal-move', 'king-captured', 'repetition-draw', 'perpetual-check-loss', 'maximum-ply-capped'), row['terminationReason']
        winner = row['winner']
        assert winner in ('A', 'B', None), winner
        score = 1 if winner == 'A' else 0 if winner == 'B' else .5
        counts['wins' if score == 1 else 'losses' if score == 0 else 'draws'] += 1
        pairs.setdefault(row['pairIndex'], []).append(score)
    assert len(pairs) == 50 and all(len(v) == 2 for v in pairs.values())
    return [statistics.mean(pairs[k]) for k in sorted(pairs)], counts


def percentile(values, p):
    # Linear interpolation between adjacent order statistics, explicitly fixed.
    v = sorted(values)
    i = (len(v) - 1) * p
    low = int(i)
    return v[low] + (v[min(low + 1, len(v) - 1)] - v[low]) * (i - low)


def summarize_games():
    strata = []
    result = {'seed': 1, 'bootstrapSamples': 10000, 'samplingUnit': 'starting-position color-swapped pair', 'strata': {}}
    for ms in (200, 1000):
        pairs, counts = game_scores(OUT / f'match-{ms}')
        strata.append(pairs)
        result['strata'][str(ms)] = {**counts, 'scoreRate': statistics.mean(pairs)}
    rng = random.Random(1)
    sampled = [[], []]
    overall = []
    for _ in range(10000):
        means = [statistics.mean(rng.choices(pairs, k=50)) for pairs in strata]
        for target, value in zip(sampled, means):
            target.append(value)
        overall.append(statistics.mean(means))
    for ms, samples in zip((200, 1000), sampled):
        result['strata'][str(ms)]['ci95'] = [percentile(samples, .025), percentile(samples, .975)]
    result['scoreRate'] = statistics.mean([statistics.mean(pairs) for pairs in strata])
    result['ci95'] = [percentile(overall, .025), percentile(overall, .975)]
    result['regression'] = result['ci95'][1] < .5
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=['browser', 'games'])
    parser.add_argument('--records-dir', type=Path, default=OUT,
                        help='Use saved P4 records instead of rerunning measurements')
    args = parser.parse_args()
    OUT = args.records_dir.resolve()
    if args.mode == 'games':
        result = summarize_games()
    else:
        result = summarize_browsers()
    text = json.dumps(result, ensure_ascii=False, indent=2) + '\n'
    (OUT / f'{args.mode}-summary.json').write_text(text)
    print(text)

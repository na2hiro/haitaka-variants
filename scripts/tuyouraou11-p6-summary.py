#!/usr/bin/env python3
"""Validate saved P6 measurements without changing the frozen P4 corpus/budget."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import random
import statistics
import subprocess

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('p4_summary', ROOT / 'scripts/tuyouraou11-p4-summary.py')
p4 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p4)


def checked_browser(path):
    positions = {p['id']: p['sfen'] for p in json.loads((ROOT / 'plans/tuyouraou11/p4-positions.json').read_text())}
    data = json.loads(path.read_text())
    assert data['mode'] == 'sfnn'
    assert data['positionsHash'] == hashlib.sha256((ROOT / 'plans/tuyouraou11/p4-positions.json').read_bytes()).hexdigest()
    summary = p4.summarize_browser(path)
    assert max(r['roundtripMs'] for r in data['preparation']) <= 10000
    assert max(r['memoryBytes'] for r in data['preparation'] + data['searches']) <= 134217728
    assert max(r['elapsedMs'] - r['ms'] for r in data['searches']) <= 100
    assert max(r['roundtripMs'] - r['ms'] for r in data['searches']) <= 200
    assert all(r['ticks'] > 0 for r in data['searches'])
    tsv = ''.join(positions[r['positionId']] + '\t' + r['bestMove'] + '\n' for r in data['searches'])
    result = subprocess.run([str(ROOT / 'target/release/examples/sfnn_verify_moves')], input=tsv, text=True, capture_output=True, check=True)
    return data, {**summary, 'wasmSha256': data['wasmSha256'], 'rawSha256': hashlib.sha256(path.read_bytes()).hexdigest(), 'legality': result.stdout.strip(), 'budget': 'PASS'}


def browser_summary(directory):
    summaries, comparisons = [], []
    for browser in ('chromium', 'firefox'):
        records = {}
        for backend in ('scalar', 'simd'):
            path = directory / f'{backend}-{browser}.json'
            data, summary = checked_browser(path)
            assert data['backend'] == backend
            summaries.append({**summary, 'backend': backend})
            records[backend] = {(r['positionId'], r['ms'], r['repeat']): r for r in data['searches']}
        assert records['scalar'].keys() == records['simd'].keys()
        for ms in (200, 1000, 3000):
            keys = [key for key in records['scalar'] if key[1] == ms]
            ratios = [records['simd'][k]['nps'] / records['scalar'][k]['nps'] for k in keys if records['scalar'][k]['nps'] > 0]
            assert len(ratios) == 60
            comparisons.append({'browser': browser, 'movetimeMs': ms, 'pairedNpsRatioMedian': statistics.median(ratios), 'samples': len(ratios)})
    return {'schemaVersion': 1, 'summaries': summaries, 'comparisons': comparisons, 'result': 'PASS'}


def model_summary(directory):
    summaries, comparisons = [], []
    for browser in ('chromium', 'firefox'):
        records = {}
        for profile in ('micro', 'mini'):
            path = directory / f'{profile}-{browser}.json'
            data, summary = checked_browser(path)
            provenance = json.loads((ROOT / 'plans/tuyouraou11' / ('mini-provenance.json' if profile == 'mini' else 'provenance.json')).read_text())
            assert data['modelId'] == provenance['id'] and data['backend'] == 'simd' and data['profile'] == profile
            summaries.append({**summary, 'profile': profile, 'modelId': data['modelId']})
            records[profile] = {(r['positionId'], r['ms'], r['repeat']): r for r in data['searches']}
        assert records['micro'].keys() == records['mini'].keys()
        pair = summaries[-2:]
        assert pair[0]['version'] == pair[1]['version'] and pair[0]['host'] == pair[1]['host']
        for ms in (200, 1000, 3000):
            keys = [key for key in records['micro'] if key[1] == ms]
            ratios = [records['mini'][k]['nps'] / records['micro'][k]['nps'] for k in keys if records['micro'][k]['nps'] > 0]
            assert len(ratios) == 60
            comparisons.append({'browser': browser, 'movetimeMs': ms, 'miniOverMicroNpsMedian': statistics.median(ratios), 'samples': len(ratios)})
    assert len({row['wasmSha256'] for row in summaries}) == 1, 'all model measurements must use the identical engine binary'
    return {'schemaVersion': 1, 'summaries': summaries, 'comparisons': comparisons, 'result': 'PASS'}


def match_summary(directory):
    rng = random.Random(1)
    strata = {}
    pair_scores = []
    openings = (ROOT / 'plans/tuyouraou11/p4-openings.sfen').read_text().splitlines()
    assert len(openings) == 50
    for ms in (200, 1000):
        path = directory / f'match-{ms}/self-play-games.jsonl'
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        assert len(rows) == 12
        pairs = {}
        for row in rows:
            assert row['terminationReason'] in ('no-legal-move', 'king-captured', 'repetition-draw', 'perpetual-check-loss', 'maximum-ply-capped')
            assert row['failureState'] is None
            assert len(row['moves']) == row['plies'] <= 256
            assert row['pairIndex'] == (row['gameIndex'] - 1) // 2
            assert row['startSfen'] == openings[row['pairIndex']]
            assert row['aColor'] == ('black' if row['gameIndex'] % 2 else 'white')
            assert row['opening']['suiteIndex'] == row['pairIndex']
            pairs.setdefault(row['pairIndex'], []).append(1 if row['winner'] == 'A' else 0 if row['winner'] == 'B' else .5)
        assert len(pairs) == 6 and all(len(v) == 2 for v in pairs.values())
        scores = [statistics.mean(v) for v in pairs.values()]
        pair_scores.append(scores)
        strata[str(ms)] = {'wins': sum(r['winner'] == 'A' for r in rows), 'losses': sum(r['winner'] == 'B' for r in rows), 'draws': sum(r['winner'] is None for r in rows), 'scoreRate': statistics.mean(scores), 'rawSha256': hashlib.sha256(path.read_bytes()).hexdigest()}
    samples = [statistics.mean(statistics.mean(rng.choices(s, k=len(s))) for s in pair_scores) for _ in range(10000)]
    ci = [p4.percentile(samples, .025), p4.percentile(samples, .975)]
    return {'games': 24, 'strata': strata, 'samplingUnit': 'color-swapped opening pair, stratified by movetime', 'bootstrapSeed': 1, 'bootstrapSamples': 10000, 'scoreRate': statistics.mean(statistics.mean(s) for s in pair_scores), 'ci95': ci, 'regression': ci[1] < .5, 'scope': 'native scalar versus detected AVX2/NEON; exploratory, not a Wasm match or proof of strength gain'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=['browser', 'models', 'matches', 'model-matches'])
    parser.add_argument('--records-dir', type=Path, default=ROOT / 'target/tuyouraou11/p6')
    args = parser.parse_args()
    result = browser_summary(args.records_dir) if args.mode == 'browser' else model_summary(args.records_dir) if args.mode == 'models' else match_summary(args.records_dir)
    if args.mode == 'model-matches':
        result['scope'] = 'native mini versus micro, identical detected AVX2/NEON kernels; exploratory, not a Wasm match or strength ranking'
    target = args.records_dir / f'{args.mode}-summary.json'
    target.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(result, ensure_ascii=False, indent=2))

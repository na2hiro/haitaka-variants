#!/usr/bin/env python3
"""Repeatable perft NPS benchmark; leaves are applied, with no bulk counting or TT."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / 'plans/historical-shogi-performance/baseline/perft.json'

def positive(value):
    number = float(value)
    if number <= 0:
        raise argparse.ArgumentTypeError('must be positive')
    return number

def main():
    baseline = json.loads(BASELINE.read_text())
    choices = {r['name']: r for r in baseline['rules']}
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rules', nargs='+', choices=list(choices), default=list(choices))
    parser.add_argument('--output', type=Path, default=ROOT / 'target/perft-benchmark/results.json')
    parser.add_argument('--cpu', type=int, default=None)
    parser.add_argument('--seconds', type=positive, default=3.0)
    parser.add_argument('--samples', type=int, default=3)
    parser.add_argument('--compare', type=Path, help='Previous result with the same rules/depths/CPU/settings')
    args = parser.parse_args()
    if args.seconds < 3 or args.samples < 3 or args.samples % 2 == 0:
        parser.error('use >=3 seconds and an odd sample count >=3')
    if len(set(args.rules)) != len(args.rules):
        parser.error('duplicate rules')
    if not hasattr(os, 'sched_getaffinity') or not shutil.which('taskset'):
        parser.error('Linux sched_getaffinity and taskset are required for CPU pinning')
    allowed = os.sched_getaffinity(0)
    cpu = args.cpu if args.cpu is not None else max(allowed)
    if cpu not in allowed:
        parser.error('requested CPU is outside the current affinity mask')
    bench = ROOT / 'target/perft-benchmark'
    (bench / 'src').mkdir(parents=True, exist_ok=True)
    (bench / 'bins').mkdir(exist_ok=True)
    source = Path(__file__).with_suffix('.rs').read_text()
    (bench / 'src/main.rs').write_text(source)
    version = next(p['version'] for p in tomllib.loads((ROOT / 'Cargo.lock').read_text())['package'] if p['name'] == 'serde_json')
    cargo = f'''[package]
name = "haitaka-perft-benchmark"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
haitaka = {{ path = {json.dumps(str(ROOT / 'haitaka'))}, default-features = false }}
serde_json = "={version}"
[profile.release]
opt-level = 3
codegen-units = 1
lto = "fat"
[features]
extended = []
'''
    for name, r in choices.items():
        if name != 'standard':
            features = (['extended'] if r['extended'] else []) + ['haitaka/' + name]
            cargo += name + ' = ' + json.dumps(features) + '\n'
    (bench / 'Cargo.toml').write_text(cargo)
    shutil.copy2(ROOT / 'Cargo.lock', bench / 'Cargo.lock')
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    diff = subprocess.check_output(['git', 'diff', 'HEAD', '--', 'haitaka', 'haitaka_types'], cwd=ROOT)
    status = subprocess.check_output(['git', 'status', '--porcelain', '--', 'haitaka', 'haitaka_types'], cwd=ROOT, text=True)
    # Build all selected binaries first so compilation cannot contaminate timing.
    for name in args.rules:
        command = ['cargo', 'build', '--offline', '--release', '--manifest-path', str(bench / 'Cargo.toml'), '--target-dir', str(ROOT / 'target'), '--bin', 'haitaka-perft-benchmark']
        if name != 'standard':
            command += ['--features', name]
        log = bench / (name + '-build.log')
        with log.open('w') as output:
            result = subprocess.run(command, cwd=ROOT, stdout=output, stderr=output)
        if result.returncode:
            raise RuntimeError(f'{name} build failed; see {log}')
        shutil.copy2(ROOT / 'target/release/haitaka-perft-benchmark', bench / 'bins' / name)
        print('built', name, flush=True)
    cpu_model = next((s.split(':', 1)[1].strip() for s in Path('/proc/cpuinfo').read_text().splitlines() if s.startswith('model name')), 'unknown')
    report = {
        'schemaVersion': 1, 'gitCommit': commit, 'engineStatus': status,
        'engineTrackedDiffSha256': hashlib.sha256(diff).hexdigest(),
        'baselineSha256': hashlib.sha256(BASELINE.read_bytes()).hexdigest(),
        'harnessSha256': hashlib.sha256(source.encode()).hexdigest(),
        'cpu': cpu_model, 'logicalCpuAffinity': cpu, 'threads': 1,
        'compiler': subprocess.check_output(['rustc', '--version'], text=True).strip(),
        'build': baseline['build'], 'metric': baseline['metric'],
        'minimumBatchSeconds': args.seconds, 'samplesPerRule': args.samples,
        'warmupPerftRuns': 1, 'rules': [],
    }
    previous = json.loads(args.compare.read_text()) if args.compare else None
    if previous:
        for key in ['cpu', 'logicalCpuAffinity', 'compiler', 'minimumBatchSeconds', 'samplesPerRule', 'harnessSha256']:
            if previous.get(key) != report[key]:
                raise ValueError(f'comparison setting differs: {key}')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    for name in args.rules:
        r = choices[name]
        depth = r['result']['depth']
        command = ['taskset', '-c', str(cpu), str(bench / 'bins' / name), name, str(depth), str(int(args.seconds * 1000)), str(args.samples)]
        result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, check=True, timeout=max(180, args.seconds * args.samples * 10))
        measured = json.loads(result.stdout)
        count = measured['result']['nodesPerPerft']
        if count != r['result']['nodesPerPerft']:
            raise ValueError(f'{name}: perft leaf mismatch {count} != {r["result"]["nodesPerPerft"]}')
        samples = measured['result']['samples']
        assert len(samples) == args.samples
        assert all(s['nodesPerPerft'] == count and s['seconds'] >= args.seconds for s in samples)
        spread = max(s['nps'] for s in samples) / min(s['nps'] for s in samples) - 1
        measured.update({'ruleId': r['ruleId'], 'label': r['label'], 'name': name, 'sampleRangePercent': spread * 100, 'needsRemeasurement': spread > .05})
        if previous:
            before = next(q for q in previous['rules'] if q['name'] == name)
            assert before['result']['depth'] == depth and before['result']['nodesPerPerft'] == count
            measured['beforeNps'] = before['result']['medianNps']
            measured['speedup'] = measured['result']['medianNps'] / measured['beforeNps']
        report['rules'].append(measured)
        report['measuredAt'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
        print(name, 'depth', depth, 'nodes', count, 'median NPS', round(measured['result']['medianNps']), 'remeasure' if spread > .05 else '', flush=True)
    print('saved', args.output)

if __name__ == '__main__':
    main()

#!/usr/bin/env python3
"""Execute the frozen CPU-only scale/time development experiment on the assigned MBP."""
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
RUN = ROOT / 'out/anhoku-strength-first/step5-scale-time-20260908-v1'
CONTRACT = ROOT / 'r0/anhoku-strength-first/step5-scale-time-v1.json'
OLD = ROOT / 'out/anhoku-strength-first/step4-objective-search-diagnostic-20260907-v1'

def rows(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line]

def main():
    os.chdir(ROOT)
    config = json.loads(CONTRACT.read_text())
    for a in [config['candidate'], config['openings'], config['parents'], config['priorUnion'], *config['sourceFiles']]:
        b = Path(a['path']).read_bytes()
        assert len(b) == a['bytes'] and hashlib.sha256(b).hexdigest() == a['sha256'], a['path']
    if (RUN / 'started.json').exists():
        started = json.loads((RUN / 'started.json').read_text())['unix']
    else:
        started = time.time()
        (RUN / 'started.json').write_text(json.dumps({'unix': started, 'contractSha256': hashlib.sha256(CONTRACT.read_bytes()).hexdigest()}))
    bins = RUN / 'bin'
    bins.mkdir(exist_ok=True)
    for name in ['haitaka_cli', 'strength_first_step4_diagnostic']:
        if (bins / name).exists():
            assert (bins / name).read_bytes() == (ROOT / 'target/release' / name).read_bytes()
        else:
            shutil.copy2(ROOT / 'target/release' / name, bins / name)
    (RUN / 'build.json').write_text(json.dumps({p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in bins.iterdir()}, indent=2))
    def invoke(command, log):
        if '--output-dir' in command:
            out = Path(command[command.index('--output-dir') + 1])
            if (out / 'receipt.json').exists():
                assert json.loads((out / 'receipt.json').read_text())['passed']
                return
        remaining = config['wallLimitHours'] * 3600 - (time.time() - started)
        assert remaining > 0
        log.parent.mkdir(parents=True, exist_ok=True)
        log.with_suffix('.command.json').write_text(json.dumps([str(x) for x in command], indent=2))
        with log.open('w') as output:
            subprocess.run([str(x) for x in command], stdout=output, stderr=subprocess.STDOUT, check=True, timeout=remaining)
    probe = bins / 'strength_first_step4_diagnostic'
    model = ROOT / config['candidate']['path']
    parents = ROOT / config['parents']['path']
    for scale in [1, 2]:
        jobs = []
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
            for shard in range(8):
                out = RUN / 'probes' / f'scale{scale}' / f'shard-{shard:02}'
                cmd = [probe, '--mode', 'probe', '--input', parents, '--output-dir', out, '--evaluator', 'nnue', '--model', model, '--model-label', f'candidate-scale{scale}', '--nnue-multiplier', scale, '--nodes', 10000, '--start', shard * 16, '--end', (shard + 1) * 16]
                jobs.append(pool.submit(invoke, cmd, RUN / 'logs' / f'probe-{scale}-{shard}.log'))
            for job in jobs:
                job.result()
    by_scale = {scale: {r['parentId']: r for p in sorted((RUN / 'probes' / f'scale{scale}').glob('*/probe.jsonl')) for r in rows(p)} for scale in [1, 2]}
    assert all(len(v) == 128 for v in by_scale.values())
    # Existing labels remain valid: the handcrafted teacher and child positions are unchanged.
    missing = []
    union = rows(OLD / 'union/union-plan.jsonl')
    for parent in union:
        p = by_scale[2][parent['id']]
        moves = [p['bestMove']] if p['legalBestMove'] and p['completedDepth'] > 0 and p['bestMove'] not in parent['unionMoves'] else []
        if moves:
            missing.append({'id': parent['id'], 'sfen': parent['sfen'], 'unionMoves': moves})
    assert len(missing) <= 128
    missing_path = RUN / 'new-child-plan.jsonl'
    missing_path.write_text(''.join(json.dumps(r) + '\n' for r in missing))
    if missing:
        chunk = (len(missing) + 7) // 8
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
            jobs = []
            for shard, start in enumerate(range(0, len(missing), chunk)):
                out = RUN / 'labels' / f'shard-{shard:02}'
                cmd = [probe, '--mode', 'label', '--input', missing_path, '--output-dir', out, '--evaluator', 'handcrafted', '--nodes', 250000, '--start', start, '--end', min(start + chunk, len(missing))]
                jobs.append(pool.submit(invoke, cmd, RUN / 'logs' / f'label-{shard}.log'))
            for job in jobs:
                job.result()
    cli = bins / 'haitaka_cli'
    def engine(side, scale):
        cmd = [f'--{side}-engine', cli]
        args = ['usi', '--eval', 'handcrafted'] if scale is None else ['usi', '--eval', 'nnue', '--nnue', str(model), '--nnue-multiplier', str(scale)]
        return cmd + [f'--{side}-engine-arg={arg}' for arg in args]
    # Alternate budget order between matchups to reduce a systematic time-order effect.
    for index, (name, a, b) in enumerate([('scale2-vs-scale1', 2, 1), ('scale1-vs-handcrafted', 1, None), ('scale2-vs-handcrafted', 2, None)]):
        for ms in ([100, 1000] if index % 2 == 0 else [1000, 100]):
            cell = f'{name}-{ms}ms'
            cmd = [cli, 'self-play', '--games', 64, '--threads', 8, '--movetime-ms', ms, '--max-plies', 200, '--seed', 20260908, '--openings', RUN / 'openings.tsv', '--report-dir', RUN / 'games' / cell, *engine('a', a), *engine('b', b)]
            print(f'{time.strftime("%FT%T")} starting {cell}', flush=True)
            invoke(cmd, RUN / 'logs' / f'{cell}.log')
            records = rows(RUN / 'games' / cell / 'self-play-games.jsonl')
            assert len(records) == 64 and all(r['failureState'] is None for r in records)
            print(f'{time.strftime("%FT%T")} completed {cell}', flush=True)
    (RUN / 'completed.json').write_text(json.dumps({'elapsedSeconds': time.time() - started, 'complete': True}, indent=2))

if __name__ == '__main__':
    main()

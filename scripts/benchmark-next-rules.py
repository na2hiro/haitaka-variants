#!/usr/bin/env python3
"""Build, count and measure the 28 ordinary variants against Annan on one CPU."""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import time
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[1]
NAMES = 'narazu nifu-ari sho-kyo-left sho-kyo-right nakodo queen-king torazu-gin ai-torazu-gin zarikani zarikani-tonbo semai semmai hiroi dobutsu toruichi hisha kaku seireigi kyo-left kyo-right totsugeki ai-happokei happokei patrol lortap shishio hono-shishi nigyoku'.split()

def save(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False) + '\n')

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT, text=True).strip()

def busy():
    # CPU accounting excludes guest fields, which are already included in user/nice.
    v = list(map(int, Path('/proc/stat').read_text().splitlines()[0].split()[1:9]))
    return sum(v) - v[3] - v[4]

def snapshot():
    return {'utc': datetime.now(timezone.utc).isoformat(),
            'loadavg': Path('/proc/loadavg').read_text().strip(),
            'top': subprocess.check_output(['top', '-b', '-n', '1', '-w', '150'], text=True).splitlines()[:22]}

def execute_count(out, rule, depth):
    data = json.loads(subprocess.check_output([out/'bin'/rule, rule, str(depth), '0', '0'], cwd=ROOT, text=True))
    return data['result']['nodesPerPerft']

def prepare(out):
    assert not git('status', '--porcelain', '--untracked-files=no'), 'commit source before building'
    assert not (out/'conditions.json').exists(), 'use a fresh output directory'
    (out/'src').mkdir(parents=True, exist_ok=True)
    (out/'bin').mkdir(exist_ok=True)
    (out/'logs').mkdir(exist_ok=True)
    source = ROOT/'scripts/perft-benchmark.rs'
    shutil.copy2(source, out/'src/main.rs')
    cargo = f'''[package]
name = "haitaka-next-rules-perft"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
haitaka = {{path = {json.dumps(str(ROOT/'haitaka'))}, default-features = false}}
serde_json = "1"
[profile.release]
opt-level = 3
codegen-units = 1
lto = "fat"
[features]
extended = []
annan = ["haitaka/annan"]
'''
    for name in NAMES:
        cargo += f'{name} = ["extended", "haitaka/{name}"]\n'
    (out/'Cargo.toml').write_text(cargo)
    if not (ROOT/'Cargo.lock').exists():
        subprocess.run(['cargo','generate-lockfile','--offline'],cwd=ROOT,check=True)
    shutil.copy2(ROOT/'Cargo.lock', out/'Cargo.lock')
    definitions = json.loads((ROOT/'plans/next-rule-groups/inputs/phase2-definitions.json').read_text())
    assert len(definitions) == len(NAMES) == 28
    rules = [{'name':'annan', 'ruleId':26, 'label':'安南将棋'}] + [
        {'name':n, 'ruleId':d['ruleId'], 'label':d['rule']['name']} for n,d in zip(NAMES, definitions)]
    receipt = {'sourceCommit':git('rev-parse','HEAD'), 'mainCommit':git('rev-parse','origin/main'),
               'compiler':subprocess.check_output(['rustc','-V'],text=True).strip(),
               'build':'native release; opt-level=3, codegen-units=1, lto=fat; cargo -j2',
               'harnessSha256':sha(source), 'lockSha256':sha(ROOT/'Cargo.lock'),
               'buildStarted':snapshot(), 'rules':rules}
    for rule in rules:
        name=rule['name']
        command=['cargo','build','--offline','--release','-j','2','--manifest-path',out/'Cargo.toml','--target-dir',out/'target','--features',name]
        with (out/'logs'/f'{name}-build.log').open('w') as log:
            subprocess.run(list(map(str,command)),cwd=ROOT,stdout=log,stderr=subprocess.STDOUT,check=True)
        shutil.copy2(out/'target/release/haitaka-next-rules-perft',out/'bin'/name)
        rule['binarySha256']=sha(out/'bin'/name)
        rule['perft']={str(d):execute_count(out,name,d) for d in range(1,4)}
        assert rule['perft']['3'] > 0
        print('BUILT',name,rule['perft'],flush=True)
    assert rules[0]['perft']=={'1':28,'2':784,'3':21718},'Annan independent known-count regression'
    assert receipt['sourceCommit']==git('rev-parse','HEAD') and not git('status','--porcelain','--untracked-files=no')
    receipt['effectiveLockSha256']=sha(out/'Cargo.lock')
    receipt['preparedUtc']=datetime.now(timezone.utc).isoformat()
    save(out/'conditions.json',receipt)

def summarize(out):
    conditions=json.loads((out/'conditions.json').read_text())
    trials=json.loads((out/'trials.json').read_text())
    expected={r['name']:r['perft']['3'] for r in conditions['rules']}
    for t in trials:
        assert t['result']['nodesPerPerft']==expected[t['name']]
        assert len(t['result']['samples'])==3
        for sample in t['result']['samples']:
            assert sample['nodesPerPerft']==expected[t['name']] and sample['seconds']>=3
            assert sample['totalLeafNodes']==sample['repetitions']*expected[t['name']]
            assert abs(sample['nps']-sample['totalLeafNodes']/sample['seconds'])<1e-6
    if (out/'core-perft-reference.json').exists():
        reference={r['name']:r['perft'] for r in json.loads((out/'core-perft-reference.json').read_text())}
        assert len(reference)==28
        for rule in conditions['rules'][1:]:
            assert all(rule['perft'][k]==v for k,v in reference[rule['name']].items()),rule['name']
    anchors=[t for t in trials if t['name']=='annan' and not t['contaminated']]
    assert len(anchors)>=3, 'need beginning/middle/end reference measurements'
    reference=statistics.median(s['nps'] for t in anchors for s in t['result']['samples'])
    rows=[]
    for rule in conditions['rules']:
        candidates=[t for t in trials if t['name']==rule['name'] and not t['contaminated']]
        assert candidates,rule['name']
        if rule['name']=='annan':
            samples=[s for t in anchors for s in t['result']['samples']]
        else:
            samples=candidates[-1]['result']['samples']
        nps=statistics.median(s['nps'] for s in samples)
        spread=max(s['nps'] for s in samples)/min(s['nps'] for s in samples)-1
        rows.append({**rule,'nps':nps,'annanRatio':nps/reference,'annanTimesFaster':reference/nps,
                     'spreadPercent':100*spread,'unstable':spread>.05})
    result={'referenceNps':reference,'metric':'leaf nodes/sec; perft depth 3, initial position; all moves applied; no bulk counting, TT, evaluation or pruning; parsing and warmup excluded',
            'rows':rows,'conditions':conditions,'timing':json.loads((out/'timing.json').read_text())}
    save(out/'summary.json',result)
    with (out/'summary.csv').open('w') as f:
        writer=csv.writer(f,lineterminator="\n");writer.writerow(['rule','name','perft1','perft2','perft3','nps','annanRatio','annanTimesFaster','spreadPercent'])
        for r in rows:writer.writerow([r['name'],r['label'],*[r['perft'][str(d)] for d in range(1,4)],round(r['nps']),r['annanRatio'],r['annanTimesFaster'],r['spreadPercent']])
    for r in rows:print(r['name'],round(r['nps']),f'{100*r["annanRatio"]:.2f}%',f'{r["annanTimesFaster"]:.1f}x','UNSTABLE' if r['unstable'] else '',flush=True)

def run(out,cpu):
    conditions=json.loads((out/'conditions.json').read_text())
    assert conditions['sourceCommit']==git('rev-parse','HEAD') and not git('status','--porcelain','--untracked-files=no')
    for r in conditions['rules']:assert sha(out/'bin'/r['name'])==r['binarySha256']
    assert cpu in os.sched_getaffinity(0)
    assert not (out/'trials.json').exists(),'preserve previous trials'
    timing={'cpu':cpu,'siblings':Path(f'/sys/devices/system/cpu/cpu{cpu}/topology/thread_siblings_list').read_text().strip(),
            'cpuModel':next(s.split(':',1)[1].strip() for s in Path('/proc/cpuinfo').read_text().splitlines() if s.startswith('model name')),
            'threads':1,'depth':3,'minimumBatchSeconds':3,'samplesPerTrial':3,'before':snapshot()}
    save(out/'timing.json',timing)
    trials=[]
    queue=['annan']+NAMES[:14]+['annan']+NAMES[14:]+['annan']
    for index,name in enumerate(queue):
        expected=next(r for r in conditions['rules'] if r['name']==name)['perft']['3']
        for attempt in range(1,4):
            before=snapshot();ticks=busy();started=time.monotonic()
            command=['taskset','-c',str(cpu),out/'bin'/name,name,'3','3000','3']
            data=json.loads(subprocess.check_output(list(map(str,command)),cwd=ROOT,text=True,timeout=240))
            elapsed=time.monotonic()-started
            cores=(busy()-ticks)/os.sysconf('SC_CLK_TCK')/elapsed
            result=data['result'];assert result['nodesPerPerft']==expected
            assert len(result['samples'])==3 and all(s['seconds']>=3 and s['nodesPerPerft']==expected for s in result['samples'])
            spread=max(s['nps'] for s in result['samples'])/min(s['nps'] for s in result['samples'])-1
            contaminated=cores>1.5
            trial={'name':name,'index':index,'attempt':attempt,'before':before,'result':result,'systemBusyCores':cores,'contaminated':contaminated}
            trials.append(trial);save(out/'trials.json',trials)
            print('MEASURED',name,round(result['medianNps']),f'spread={100*spread:.2f}%',f'busyCores={cores:.2f}', 'RETRY' if contaminated or spread>.05 else '',flush=True)
            if not contaminated and spread<=.05:break
            # Pause this trial before retrying if other work consumed a core.
            if contaminated:
                print('Other CPU load detected; waiting for a quiet interval',flush=True)
                while True:
                    ticks=busy();start=time.monotonic();time.sleep(5)
                    if (busy()-ticks)/os.sysconf('SC_CLK_TCK')/(time.monotonic()-start)<.5:break
        else:
            assert not contaminated,'CPU competition persisted; keep evidence and rerun later'
    timing['after']=snapshot();save(out/'timing.json',timing)
    summarize(out)

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('phase',choices=['prepare','run','summarize']);p.add_argument('--output',type=Path,default=ROOT/'target/next-rules-perft-20261126');p.add_argument('--cpu',type=int,default=2);a=p.parse_args();out=a.output.resolve()
    if a.phase=='prepare':prepare(out)
    elif a.phase=='run':run(out,a.cpu)
    else:summarize(out)

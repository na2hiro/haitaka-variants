#!/usr/bin/env python3
"""Fixed-input extended perft, sharing the initial-position benchmark's timing loop."""
import argparse,datetime,hashlib,json,os,shutil,subprocess,tomllib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
baseline=json.loads((ROOT/'plans/historical-shogi-performance/baseline/perft.json').read_text())
rules={r['name']:r for r in baseline['rules'] if r['extended']}
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--engine-dir',type=Path,default=ROOT)
parser.add_argument('--positions',type=Path,required=True)
parser.add_argument('--output',type=Path,required=True)
parser.add_argument('--compare',type=Path)
parser.add_argument('--calibrate',action='store_true',help='Freeze expected leaf counts with this engine before timing')
parser.add_argument('--cpu',type=int,default=7)
args=parser.parse_args()
assert args.cpu in os.sched_getaffinity(0)
engine=args.engine_dir.resolve();manifest_path=args.positions.resolve()
manifest=json.loads(manifest_path.read_text());cases=manifest['positions']
assert len({p['id'] for p in cases})==len(cases)
for p in cases:
 assert p['rule'] in rules and 1<=p['depth']<=4
 file=manifest_path.parent/p['file']
 assert hashlib.sha256(file.read_bytes()).hexdigest()==p['inputSha256']
 if not args.calibrate: assert isinstance(p['expectedNodes'],int) and p['expectedNodes']>0
original=Path(__file__).with_name('perft-benchmark.rs').read_text()
prefix=original[:original.index('#[cfg(not(feature = "extended"))]')]
source=prefix+Path(__file__).with_suffix('.rs').read_text()
bench=ROOT/'target/perft-position-benchmark';(bench/'src').mkdir(parents=True,exist_ok=True);(bench/'bins').mkdir(exist_ok=True)
(bench/'src/main.rs').write_text(source)
version=next(p['version'] for p in tomllib.loads((engine/'Cargo.lock').read_text())['package'] if p['name']=='serde_json')
cargo=f'''[package]
name="haitaka-position-perft"
version="0.0.0"
edition="2024"
[workspace]
[dependencies]
haitaka={{path={json.dumps(str(engine/'haitaka'))},default-features=false}}
serde_json="={version}"
[profile.release]
opt-level=3
codegen-units=1
lto="fat"
[features]
'''
for r in rules: cargo+=r+'=["haitaka/'+r+'"]\n'
(bench/'Cargo.toml').write_text(cargo);shutil.copy2(engine/'Cargo.lock',bench/'Cargo.lock')
for rule in dict.fromkeys(p['rule'] for p in cases):
 with (bench/(rule+'-build.log')).open('w') as log:
  subprocess.run(['cargo','build','--offline','--release','--manifest-path',str(bench/'Cargo.toml'),'--target-dir',str(ROOT/'target'),'--features',rule],cwd=ROOT,stdout=log,stderr=log,check=True)
 shutil.copy2(ROOT/'target/release/haitaka-position-perft',bench/'bins'/rule)
 print('built',rule,flush=True)
def run(p,samples):
 file=manifest_path.parent/p['file']
 result=subprocess.check_output(['taskset','-c',str(args.cpu),str(bench/'bins'/p['rule']),p['rule'],str(p['depth']),'3000',str(samples),str(file)],cwd=ROOT,text=True,timeout=300)
 return json.loads(result)
if args.calibrate:
 for p in cases:
  p['expectedNodes']=run(p,0)['nodesPerPerft']
  # Choose depths before any timed comparison, then freeze them. Tiny pending
  # roots otherwise have only a handful of leaves; retain the API's depth cap.
  while 0 < p['expectedNodes'] < 1000 and p['depth'] < 4:
   p['depth'] += 1
   p['expectedNodes']=run(p,0)['nodesPerPerft']
  assert p['expectedNodes']>0
  print('frozen',p['id'],p['expectedNodes'],flush=True)
 manifest_path.write_text(json.dumps(manifest,ensure_ascii=False,indent=2)+'\n')
 args.output.parent.mkdir(parents=True,exist_ok=True)
 args.output.write_text(json.dumps({'calibratedBy':subprocess.check_output(['git','rev-parse','HEAD'],cwd=engine,text=True).strip(),'positions':cases},ensure_ascii=False,indent=2)+'\n')
 raise SystemExit(0)
report={'schemaVersion':1,'gitCommit':subprocess.check_output(['git','rev-parse','HEAD'],cwd=engine,text=True).strip(),'engineDirectory':str(engine),'engineStatus':subprocess.check_output(['git','status','--porcelain','--','haitaka','haitaka_types'],cwd=engine,text=True),'compiler':subprocess.check_output(['rustc','--version'],text=True).strip(),'cpu':next(s.split(':',1)[1].strip() for s in Path('/proc/cpuinfo').read_text().splitlines() if s.startswith('model name')),'logicalCpuAffinity':args.cpu,'threads':1,'dependencyLockSha256':hashlib.sha256((engine/'Cargo.lock').read_bytes()).hexdigest(),'effectiveDependencyLockSha256':hashlib.sha256((bench/'Cargo.lock').read_bytes()).hexdigest(),'harnessSha256':hashlib.sha256(source.encode()).hexdigest(),'sharedTimingLoopSha256':hashlib.sha256(prefix.encode()).hexdigest(),'manifestSha256':hashlib.sha256(manifest_path.read_bytes()).hexdigest(),'minimumBatchSeconds':3,'samplesPerPosition':3,'timingScope':'Leaf-applied internal perft, no TT/bulk/evaluation/pruning; JSON parse/history replay excluded, warmup excluded. Same measure() as initial benchmark.','positions':[]}
previous=json.loads(args.compare.read_text()) if args.compare else None
if previous:
 for key in ['compiler','cpu','logicalCpuAffinity','dependencyLockSha256','effectiveDependencyLockSha256','harnessSha256','manifestSha256','minimumBatchSeconds','samplesPerPosition']:
  assert previous[key]==report[key],key
for p in cases:
 measured=run(p,3);result=measured['result'];assert result['nodesPerPerft']==p['expectedNodes']
 assert len(result['samples'])==3 and all(s['seconds']>=3 and s['nodesPerPerft']==p['expectedNodes'] for s in result['samples'])
 spread=max(s['nps'] for s in result['samples'])/min(s['nps'] for s in result['samples'])-1
 measured.update({'id':p['id'],'inputSha256':p['inputSha256'],'sampleRangePercent':spread*100,'needsRemeasurement':spread>.05})
 if previous:
  old=next(r for r in previous['positions'] if r['id']==p['id'])
  assert old['result']['depth']==result['depth'] and old['result']['nodesPerPerft']==result['nodesPerPerft']
  measured['beforeNps']=old['result']['medianNps'];measured['speedup']=result['medianNps']/measured['beforeNps']
 report['positions'].append(measured);report['measuredAt']=datetime.datetime.now(datetime.timezone.utc).isoformat()
 args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
 print(p['id'],round(result['medianNps']),'remeasure' if spread>.05 else '',flush=True)

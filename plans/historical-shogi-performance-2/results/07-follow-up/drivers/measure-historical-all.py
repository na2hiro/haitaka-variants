from pathlib import Path
import json,hashlib,subprocess,datetime,os,math
root=Path.cwd();base=root/'plans/historical-shogi-performance-2/results';previous=json.loads((base/'06-native-32-after.json').read_text());identities=json.loads((base/'06-native-binary-identity.json').read_text());frozen=json.loads((root/'plans/historical-shogi-performance-2/inputs/positions.json').read_text())
rules='standard heian-8x8 heian-9x8 heian-9x9 shoshogi chushogi torishogi washogi heian-dai daishogi tenjiku-dai dai-dai maka-dai-dai taishogi taikyoku'.split()
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
assert not subprocess.check_output(['git','diff',previous['gitCommit'],'HEAD','--','haitaka','haitaka_types','Cargo.toml'])
assert not subprocess.check_output(['git','status','--porcelain','--','haitaka','haitaka_types'])
assert sha(root/'scripts/perft-benchmark.rs')==previous['harnessSha256']
assert sha(root/'Cargo.lock')==previous['dependencyLockSha256']
assert subprocess.check_output(['rustc','--version'],text=True).strip()==previous['compiler']
assert 7 in os.sched_getaffinity(0)
report={k:previous[k] for k in ['engineDirectory','compiler','cpu','logicalCpuAffinity','threads','dependencyLockSha256','effectiveDependencyLockSha256','harnessSha256','minimumBatchSeconds','samplesPerRule']}
report.update({'sourceCommit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'binaryBuildCommit':previous['gitCommit'],'binaryReuse':'All binary SHAs match the final verified build; engine sources, Cargo.toml, Cargo.lock and benchmark Rust harness are unchanged. No compilation during timing.','method':'Initial position, one warmup, single CPU 7, one thread, actual perft leaf application/restoration; no TT or bulk counting. At least 3 seconds per sample, 3 samples; repeat unstable measurements up to 3 times. Tenjiku/Taikyoku use frozen deeper perft(3) counts.','rules':[]})
output=root/'target/historical-all-perft-20261109.json'
for name in rules:
 r=next(r for r in previous['rules'] if r['name']==name);identity=next(x for x in identities['rules'] if x['rule']==name)
 binary=root/'target/perft-benchmark/bins'/name;assert sha(binary)==identity['afterSha256'],name
 depth=r['result']['depth'];expected=r['result']['nodesPerPerft']
 if name in ['tenjiku-dai','taikyoku']:
  case=next(x for x in frozen['positions'] if x['id']==name+'-initial');depth=case['depth'];expected=case['expectedNodes']
 trials=[]
 for attempt in range(3):
  measured=json.loads(subprocess.check_output(['taskset','-c','7',str(binary),name,str(depth),'3000','3'],text=True,timeout=900))
  v=measured['result'];assert v['nodesPerPerft']==expected;assert len(v['samples'])==3;assert all(s['seconds']>=3 for s in v['samples'])
  spread=max(s['nps'] for s in v['samples'])/min(s['nps'] for s in v['samples'])-1
  trials.append({'attempt':attempt,'sampleRangePercent':spread*100,**measured})
  if spread<=.05:break
 else:raise RuntimeError('Unstable samples: '+name)
 estimate=max(0,int(depth*math.log(3*v['medianNps'])/math.log(expected)))
 entry={'name':name,'label':r['label'],'binarySha256':sha(binary),'sampleRangePercent':spread*100,'threeSecondFullWidthDepthEstimate':estimate,'effectiveBranchingFactor':expected**(1/depth),'secondsPerPerft':expected/v['medianNps'],'result':v,'trials':trials}
 report['rules'].append(entry);report['measuredAt']=datetime.datetime.now(datetime.timezone.utc).isoformat();output.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
 print(name,'perft('+str(depth)+')',expected,'NPS',round(v['medianNps']),'spread',round(spread*100,2),flush=True)
print('saved',output,flush=True)

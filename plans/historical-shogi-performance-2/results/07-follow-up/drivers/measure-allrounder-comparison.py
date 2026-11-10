from pathlib import Path
import subprocess,json,hashlib,datetime
root=Path.cwd();front=Path('/home/na2hiro/proj/shogitter-3');identity=json.loads((root/'plans/historical-shogi-performance-2/results/06-native-binary-identity.json').read_text());base=json.loads((root/'plans/historical-shogi-performance/baseline/perft.json').read_text());previous=json.loads((root/'plans/historical-shogi-performance-2/results/06-native-32-after.json').read_text())
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
assert not subprocess.check_output(['git','diff',previous['gitCommit'],'HEAD','--','haitaka','haitaka_types','Cargo.toml'])
assert not subprocess.check_output(['git','status','--porcelain','--','haitaka','haitaka_types'])
assert sha(root/'Cargo.lock')==previous['dependencyLockSha256'];assert sha(root/'scripts/perft-benchmark.rs')==previous['harnessSha256']
cases=[('standard',3),('heian-8x8',3),('heian-9x8',3),('heian-9x9',3),('shoshogi',3),('chushogi',2),('torishogi',3),('washogi',2),('heian-dai',2),('daishogi',2),('tenjiku-dai',1),('dai-dai',1),('maka-dai-dai',1),('taishogi',1),('taikyoku',1)]
report={'haitakaSourceCommit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'haitakaBinaryBuildCommit':previous['gitCommit'],'shogitterCommit':subprocess.check_output(['git','rev-parse','HEAD'],cwd=front,text=True).strip(),'shogitterStatus':subprocess.check_output(['git','status','--porcelain','--','packages/core','packages/ai'],cwd=front,text=True).strip(),'node':subprocess.check_output(['node','--version'],text=True).strip(),'rustc':previous['compiler'],'cpu':previous['cpu'],'cpuAffinity':7,'threads':1,'minimumSampleSeconds':3,'samplesPerEngine':3,'driverSha256':sha(root/'target/allrounder-perft-benchmark.ts'),'bundledSourceSha256':sha(root/'target/allrounder-perft-benchmark.cjs'),'method':'Actual PossibilityScoringShogiGame + existing AI perft, without evaluation/pruning/leaf TT/bulk. Move-list cache within one traversal retained as production; a fresh game wrapper for each traversal prevents cross-run cache reuse. Native perft retains its production scoped generation cache and clears it on return. Identical starting rules/depth per pair; AI counts command steps, Native counts completed turns. Multi-hop leaf differences retained and explicitly reported, not relabeled as same work. Node and Native alternate three >=3s samples; setup/import/init excluded.','rules':[]}
output=root/'target/allrounder-vs-haitaka-perft-20261109.json'
if output.exists():
 old=json.loads(output.read_text());report['rules']=old['rules'];report['previousDriverSha256']=old['driverSha256'];report['previousBundleSha256']=old['bundledSourceSha256']
report['samplingNote']='First five stable rules: >=3s warmup and >=3s samples. Remaining rules: >=10s warmup and >=5s samples after Chu timing instability. Each pair uses identical sample duration. All retain three alternating samples.'
for name,depth in cases:
 if any(r['name']==name for r in report['rules']):continue
 r=next(r for r in base['rules'] if r['name']==name);binary=root/'target/perft-benchmark/bins'/name;ident=next(r for r in identity['rules'] if r['rule']==name);assert sha(binary)==ident['afterSha256'],name
 trials=[]
 for attempt in range(3):
  cmd=['taskset','-c','7','node',str(root/'target/allrounder-perft-benchmark.cjs'),str(r['ruleId']),str(depth),str(binary),name]
  p=subprocess.run(cmd,capture_output=True,text=True,timeout=900)
  if p.returncode:raise RuntimeError(name+' '+p.stdout[-2000:]+' '+p.stderr[-4000:])
  result=json.loads(p.stdout);trials.append(result)
  with (root/'target/allrounder-comparison-trials.jsonl').open('a') as f:f.write(json.dumps({'name':name,'attempt':attempt,'result':result})+'\n')
  if not(result['haitaka']['needsRemeasurement'] or result['allrounder']['needsRemeasurement']):break
 else:
  result=min(trials,key=lambda t:max(t['haitaka']['sampleRangePercent'],t['allrounder']['sampleRangePercent']));print('Timing remains variable',name,flush=True)
 result.update({'name':name,'label':r['label'],'haitakaBinarySha256':sha(binary),'benchmarkBundleSha256':sha(root/'target/allrounder-perft-benchmark.cjs'),'trials':trials.copy()})
 # Trials store independent decoded outputs; do not create a self-reference.
 result['trials']=[{k:v for k,v in t.items() if k!='trials'} for t in trials]
 report['rules'].append(result);report['measuredAt']=datetime.datetime.now(datetime.timezone.utc).isoformat();output.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
 print(name,'depth',depth,'H',round(result['haitaka']['medianNps']),'A',round(result['allrounder']['medianNps']),'leaves',result['haitaka']['nodesPerPerft'],result['allrounder']['nodesPerPerft'],flush=True)
print('saved',output,flush=True)

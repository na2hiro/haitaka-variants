#!/usr/bin/env python3
"""Profile isolated extended-rule snapshots; production code is never instrumented."""
import argparse,hashlib,io,json,re,shutil,subprocess,tarfile,tomllib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
baseline=json.loads((ROOT/'plans/historical-shogi-performance/baseline/perft.json').read_text())
choices={r['name']:r for r in baseline['rules'] if r['extended']}
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--revision')
parser.add_argument('--label',required=True)
parser.add_argument('--rules',nargs='+',choices=list(choices),required=True)
parser.add_argument('--output',type=Path,required=True)
parser.add_argument('--cpu',type=int,default=7)
args=parser.parse_args()
if not re.fullmatch('[a-z0-9-]+',args.label): parser.error('invalid label')
dest=ROOT/'target'/('hotspot-'+args.label)
if dest.exists(): parser.error('snapshot exists; use a new label')
dest.mkdir(parents=True)
if args.revision:
 with tarfile.open(fileobj=io.BytesIO(subprocess.check_output(['git','archive',args.revision,'haitaka','haitaka_types'],cwd=ROOT))) as archive: archive.extractall(dest,filter='data')
else:
 for name in ['haitaka','haitaka_types']: shutil.copytree(ROOT/name,dest/name)
fingerprint=hashlib.sha256()
for p in sorted((dest/'haitaka/src').rglob('*.rs')):
 fingerprint.update(str(p.relative_to(dest)).encode());fingerprint.update(p.read_bytes())
def span_function(source,name,index):
 pattern=r'(\bfn '+name+r'\b[^{]*\{)'
 source,n=re.subn(pattern,lambda m:m[1]+f'\nlet _probe=crate::hot_probe::span({index});',source,count=1)
 assert n==1,name
 return source
def span_call(source,offset,prefix,index):
 start=source.find(prefix,offset)
 if start<0: return source,False
 begin=start+len(prefix)
 at=source.index('(',begin)
 depth=1;end=at+1
 while depth:
  if source[end]=='(': depth+=1
  elif source[end]==')': depth-=1
  end+=1
 return source[:begin]+'{let _probe=crate::hot_probe::span('+str(index)+');'+source[begin:end]+'}'+source[end:],True
large=dest/'haitaka/src/extended/large.rs'
s=large.read_text()
generation = 'enumerate_entries' if 'fn enumerate_entries(' in s else 'enumerate'
for name,index in [('protected',2),('finish',3),('active',4),('order',5),(generation,6)]: s=span_function(s,name,index)
s,enum_instrumented=span_call(s,s.index("impl Enumeration"),'for t in ',0)
s,push_instrumented=span_call(s,s.index('fn push(&mut self, m: LargeMove)'),'let t = ',1)
large.write_text(s)
classical=dest/'haitaka/src/extended/classical.rs';s=classical.read_text()
for name,index in [('classical_acceptable',7),('classical_apply',8)]: s=span_function(s,name,index)
classical.write_text(s)
lib=dest/'haitaka/src/lib.rs';lib.write_text(lib.read_text()+'\npub mod hot_probe;\n')
(dest/'haitaka/src/hot_probe.rs').write_text('''use std::{cell::RefCell,time::Instant};
thread_local!{static STATS:RefCell<([u64;9],[u128;9])>=const{RefCell::new(([0;9],[0;9]))};}
pub fn reset(){STATS.with(|s|*s.borrow_mut()=([0;9],[0;9]));}
pub fn stats()->([u64;9],[u128;9]){STATS.with(|s|*s.borrow())}
pub struct Span(usize,Instant);
pub fn span(id:usize)->Span{Span(id,Instant::now())}
impl Drop for Span{fn drop(&mut self){let ns=self.1.elapsed().as_nanos();STATS.with(|s|{let mut s=s.borrow_mut();s.0[self.0]+=1;s.1[self.0]+=ns;});}}
''')
bench=dest/'bench';(bench/'src').mkdir(parents=True)
(bench/'src/main.rs').write_text(Path(__file__).with_suffix('.rs').read_text())
shutil.copy2(ROOT/'Cargo.lock',bench/'Cargo.lock')
version=next(p['version'] for p in tomllib.loads((ROOT/'Cargo.lock').read_text())['package'] if p['name']=='serde_json')
manifest=f'''[package]
name = "haitaka-hotspot-profile"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
haitaka = {{path="../haitaka",default-features=false}}
serde_json = "={version}"
[profile.release]
opt-level=3
codegen-units=1
lto="fat"
[features]
'''
for rule in choices: manifest+=rule+'=["haitaka/'+rule+'"]\n'
(bench/'Cargo.toml').write_text(manifest)
build=ROOT/'target/hotspot-probe-build';(dest/'bins').mkdir()
for rule in args.rules:
 with (dest/(rule+'-build.log')).open('w') as log:
  subprocess.run(['cargo','build','--offline','--release','--manifest-path',str(bench/'Cargo.toml'),'--target-dir',str(build),'--features',rule],cwd=ROOT,stdout=log,stderr=log,check=True)
 shutil.copy2(build/'release/haitaka-hotspot-profile',dest/'bins'/rule)
 print('built',rule,flush=True)
names=['enumeration_targets','push_targets','protected_inclusive','finish_inclusive','active_inclusive','order_inclusive','enumerate_inclusive','classical_candidate_inclusive','classical_apply_inclusive']
report={'revision':args.revision or 'working-tree','sourceSha256':fingerprint.hexdigest(),'cpuAffinity':args.cpu,'compiler':subprocess.check_output(['rustc','--version'],text=True).strip(),'names':names,'instrumentedEnumerationTargets':enum_instrumented,'instrumentedPushTargets':push_instrumented,'method':'Isolated snapshot, inclusive timers perturb code/layout and overlap; do not sum. Warmup excluded, >=3s x3. Allocations counted separately in one warmed perft; includes alloc/realloc requested bytes, excludes dealloc. Not production NPS.','rules':[]}
for rule in args.rules:
 expected=choices[rule]['result']
 p=json.loads(subprocess.check_output(['taskset','-c',str(args.cpu),str(dest/'bins'/rule),rule,str(expected['depth'])],cwd=ROOT,text=True))
 assert p['nodesPerPerft']==expected['nodesPerPerft']
 assert all(s['seconds']>=3 for s in p['samples'])
 report['rules'].append(p)
 args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(report,indent=2)+'\n')
 print('profiled',rule,flush=True)

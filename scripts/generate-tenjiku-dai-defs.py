#!/usr/bin/env python3
import json,collections,subprocess
from pathlib import Path
import argparse
parser=argparse.ArgumentParser(description='Generate large-rule movement tables from resolved Shogitter definitions')
parser.add_argument('input',type=Path)
parser.add_argument('--ruleset',default='tenjiku-dai',choices=['tenjiku-dai','dai-dai','maka-dai-dai','taishogi'])
args=parser.parse_args()
q=json.loads(args.input.read_text());r=q['rule'];size=r['size'][0];bases=list(r['nari']);pieces={p['species']:p for p in q['pieces']};incoming={v:k for k,v in r['nari'].items()if v}
expected={'tenjiku-dai':(59,16),'dai-dai':(60,17),'maka-dai-dai':(61,19),'taishogi':(63,25)}[args.ruleset]
if (q['ruleId'],size)!=expected or r['size']!=[size,size]:
 raise SystemExit('resolved rule ID/geometry does not match --ruleset')
ids=list(pieces);variants=[(id,None)for id in ids]
reverse=collections.defaultdict(list)
for base,to in r['nari'].items():
 if to:reverse[to].append(base)
if args.ruleset in ['maka-dai-dai','taishogi']:
 for to,fronts in reverse.items():
  if len(fronts)>1:variants.extend((to,base)for base in fronts)
idx={pair:i for i,pair in enumerate(variants)}
text=[f'// Generated from resolved Shogitter rule {q["ruleId"]}; IDs, aliases and movement types remain distinct.','const PIECES:&[PieceDef]=&['];metadata=[]
fmt=lambda x:'&['+','.join(x)+']'
def requirements(p,t):
 rs=p.get('mustNotBeEmpty',{}).get(str(t),[])
 if isinstance(rs,dict):rs=[rs]
 return fmt(['Requirement{delta:('+str(r['move'][0])+','+str(r['move'][1])+'),kind:'+str(r['type'])+'}'for r in rs])
def prim(p):
 return fmt(['Primitive{delta:('+str(m['move'][0])+','+str(m['move'][1])+'),kind:'+str(t)+',limit:'+str(p.get('limit',{}).get(str(t),0))+',skip:'+str(p.get('skip',{}).get(str(t),0))+',next:'+str(p.get('change',{}).get(str(t),-2))+',occupied:'+requirements(p,t)+'}'for m in p['move']for t in(m['type']if isinstance(m['type'],list)else[m['type']])])

for id,origin in variants:
 p=pieces[id];base=origin or (id if id in bases else incoming[id]);promo=r['nari'].get(id);dead=0
 for y in range(size):
  if any(0<=size//2+m['move'][0]<size and 0<=y+m['move'][1]<size for m in p['move']):break
  dead+=1
 if args.ruleset!='tenjiku-dai':dead=0 # Capture strategy permits dead-rank survivors.
 value=2500 if id in r['winkoma']else {'aa':100,'fe':2600,'zg':2600,'fp':2800,'fk':2200,'fl':2200,'fm':1800,'fn':1500,'bl':1800,'dh':1800,'fc':2200,'kr':2200,'fd':2000,'zi':2000,'af':750,'ag':900,'fh':1600}.get(id,450)
 if args.ruleset!='tenjiku-dai' and id not in r['winkoma']:
  value={'aa':100,'ab':300,'ad':450,'ae':550,'af':750,'ag':900,'am':1000,'an':1150,'bl':1600,'dh':1600,'ie':2100,'gf':2200,'gh':2500,'hc':2200,'0c':2500,'go':1400,'lf':1400,'ig':1700,'ix':2600,'iz':2600,'jb':2800,'do':550}.get(id,max(250,min(2000,sum(100 if (m['type']==2) else 30*p.get('limit',{}).get(str(m['type']),1) for m in p['move']))))
 text.append('PieceDef{id:"'+id+'",steps:&[],rays:&[],promotion:'+('Some(PieceId('+str(idx[(promo,id if len(reverse[promo])>1 else None)])+'))'if promo else'None')+',base:'+str(bases.index(base))+',royal:'+str(id in r['winkoma']).lower()+',dead_ranks:'+str(dead)+',value:'+str(value)+'},')
 ex=[z for a in p.get('jumpException',[])for z in(a if isinstance(a,list)else[a])]
 metadata.append('Movement{root:'+prim(p)+',states:'+fmt([prim({**s,'skip':p.get('skip',{}),'limit':p.get('limit',{}),'mustNotBeEmpty':p.get('mustNotBeEmpty',{})})for s in p.get('status',[])])+',exceptions:'+fmt(['"'+s+'"'for s in ex])+',stop_capture:'+str(p.get('stopWhenCapture',False)).lower()+',no_stop:'+str(p.get('nopass',False)).lower()+',file_limit:'+str(max(0,p.get('nifu',0)-1))+'}')
text+= ['];','const ORIGINS:&[Option<&str>]=&['+','.join('Some("'+o+'")'if o else 'None'for _,o in variants)+'];','const MOVEMENT:&[Movement]=&['+','.join(metadata)+'];']
inv=collections.Counter(c[1]for f in q['initial']['ban']for c in f if c and c[0]==0)
text+=['const INVENTORY:&[usize]=&['+','.join(str(inv[b])for b in bases)+'];','const INITIAL:&[([u16;2],u16,u8)]=&[']
for x,f in enumerate(q['initial']['ban']):
 for y,c in enumerate(f):
  if c:text.append('(['+str(x+1)+','+str(y+1)+'],'+str(idx[(c[1],None)])+','+str(c[0])+'),')
text.append('];')
generated=Path(__file__).resolve().parents[1].joinpath('haitaka/src/extended/large/'+('tenjiku' if args.ruleset=='tenjiku-dai' else args.ruleset.replace('-','_'))+'_defs.rs')
generated.write_text('\n'.join(text)+'\n')
subprocess.run(['rustfmt','--edition','2024',str(generated)],check=True)
Path('target/'+args.ruleset+'-protocol-config.json').write_text(json.dumps({'ids':ids,'promotion':r['nari'],'royals':r['winkoma'],'origins':{k:v for k,v in reverse.items()if len(v)>1}},indent=2)+'\n')
print(len(ids),sum(inv.values()))

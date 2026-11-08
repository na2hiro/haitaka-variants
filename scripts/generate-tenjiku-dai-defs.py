#!/usr/bin/env python3
import json,collections,subprocess
from pathlib import Path
import argparse
parser=argparse.ArgumentParser(description='Generate rule-59 movement tables from resolved Shogitter definitions')
parser.add_argument('input',type=Path)
args=parser.parse_args()
q=json.loads(args.input.read_text());r=q['rule'];bases=list(r['nari']);pieces={p['species']:p for p in q['pieces']};incoming={v:k for k,v in r['nari'].items()if v};assert len(incoming)==sum(bool(v)for v in r['nari'].values())
ids=list(pieces);idx={id:i for i,id in enumerate(ids)}
text=['// Generated from resolved Shogitter rule 59; IDs, aliases and movement types remain distinct.','const PIECES:&[PieceDef]=&['];metadata=[]
fmt=lambda x:'&['+','.join(x)+']'
prim=lambda p:fmt(['Primitive{delta:('+str(m['move'][0])+','+str(m['move'][1])+'),kind:'+str(t)+',limit:'+str(p.get('limit',{}).get(str(t),0))+',skip:'+str(p.get('skip',{}).get(str(t),0))+',next:'+str(p.get('change',{}).get(str(t),-2))+',enemy:'+str(str(t)in p.get('mustNotBeEmpty',{})).lower()+'}'for m in p['move']for t in(m['type']if isinstance(m['type'],list)else[m['type']])])
for id,p in pieces.items():
 base=id if id in bases else incoming[id];promo=r['nari'].get(id);dead=0
 for y in range(16):
  if any(0<=8+m['move'][0]<16 and 0<=y+m['move'][1]<16 for m in p['move']):break
  dead+=1
 value=2500 if id in r['winkoma']else {'aa':100,'fe':2600,'zg':2600,'fp':2800,'fk':2200,'fl':2200,'fm':1800,'fn':1500,'bl':1800,'dh':1800,'fc':2200,'kr':2200,'fd':2000,'zi':2000,'af':750,'ag':900,'fh':1600}.get(id,450)
 text.append('PieceDef{id:"'+id+'",steps:&[],rays:&[],promotion:'+('Some(PieceId('+str(idx[promo])+'))'if promo else'None')+',base:'+str(bases.index(base))+',royal:'+str(id in r['winkoma']).lower()+',dead_ranks:'+str(dead)+',value:'+str(value)+'},')
 ex=[z for a in p.get('jumpException',[])for z in(a if isinstance(a,list)else[a])]
 metadata.append('Movement{root:'+prim(p)+',states:'+fmt([prim({**s,'skip':p.get('skip',{})})for s in p.get('status',[])])+',exceptions:'+fmt(['"'+s+'"'for s in ex])+',stop_capture:'+str(p.get('stopWhenCapture',False)).lower()+',no_stop:'+str(p.get('nopass',False)).lower()+',file_limit:'+str(max(0,p.get('nifu',0)-1))+'}')
text+= ['];','const MOVEMENT:&[Movement]=&['+','.join(metadata)+'];']
inv=collections.Counter(c[1]for f in q['initial']['ban']for c in f if c and c[0]==0)
text+=['const INVENTORY:&[usize]=&['+','.join(str(inv[b])for b in bases)+'];','const INITIAL:&[([u16;2],u16,u8)]=&[']
for x,f in enumerate(q['initial']['ban']):
 for y,c in enumerate(f):
  if c:text.append('(['+str(x+1)+','+str(y+1)+'],'+str(idx[c[1]])+','+str(c[0])+'),')
text.append('];')
generated=Path(__file__).resolve().parents[1].joinpath('haitaka/src/extended/large/tenjiku_defs.rs')
generated.write_text('\n'.join(text)+'\n')
subprocess.run(['rustfmt','--edition','2024',str(generated)],check=True)
Path('target/tenjiku-dai-protocol-config.json').write_text(json.dumps({'ids':ids,'promotion':r['nari'],'royals':r['winkoma']},indent=2)+'\n')
print(len(ids),sum(inv.values()))

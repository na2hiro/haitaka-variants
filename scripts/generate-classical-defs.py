#!/usr/bin/env python3
import json,collections
from pathlib import Path
import argparse
parser=argparse.ArgumentParser(description='Generate stage-4 rule tables from Shogitter resolved definitions')
parser.add_argument('input',type=Path)
parser.add_argument('--config',type=Path,default=Path('target/classical-protocol-config.json'))
args=parser.parse_args()
rules=json.loads(args.input.read_text())
settings={16:('Torishogi','TORI',7,'Hands<7>', 'ClassicalState',2,True,True,False,False,False),50:('Washogi','WA',11,'NoHands','ClassicalState',3,False,True,True,False,False),15:('HeianDai','HEIAN_DAI',13,'NoHands','ClassicalState',3,False,True,False,True,False),49:('Daishogi','DAI',15,'NoHands','LionState',5,False,False,True,False,True)}
text=['// Adopted Shogitter stage-4 rule and movement definitions.','// Same wire IDs with different origins retain different internal piece IDs.']
manifest=[]
for q in rules:
 rid=q['ruleId'];r=q['rule'];name,prefix,size,hands,state,dan,forced,check,stale,kings,lions=settings[rid]
 defs={p['species']:p for p in q['pieces']};bases=list(r['nari']);incoming=collections.defaultdict(list)
 for a,b in r['nari'].items():
  if b:incoming[b].append(a)
 entries=[(b,None,b)for b in bases]
 for target,sources in incoming.items():
  if len(sources)==1 and target not in bases:entries.append((target,None,sources[0]))
  elif len(sources)>1:
   entries.extend((target,source,source)for source in sources)
 index={(id,origin):i for i,(id,origin,b)in enumerate(entries)}
 inv=collections.Counter(c[1]for f in q['initial']['ban']for c in f if c and c[0]==0)
 text.append('const '+prefix+'_PIECES:&[PieceDef]=&[')
 metas=[];continuations=[]
 fmt=lambda a:'&['+','.join('('+','.join(str(x)for x in t)+')'for t in a)+']'
 for id,origin,base in entries:
  d=defs[id];steps=[];rays=[];limits=[]
  for m in d['move']:
   typ=m['type'];xy=m['move'];change=d.get('change',{}).get(str(typ))
   if typ==1:steps.append(xy)
   elif typ==2:rays.append(xy)
   elif change is not None:
    steps.append(xy);follow=d['status'][change]['move'];assert all(v['type']==1 for v in follow)
    continuations.append((index[(id,origin)],xy,[v['move']for v in follow]))
   else:
    assert str(typ)in d.get('limit',{}),(rid,id,typ)
    rays.append(xy);limits.append((*xy,d['limit'][str(typ)]))
  dead=0
  for y in range(size):
   if any(0<=size//2+dx<size and 0<=y+dy<size for dx,dy in steps+rays):break
   dead+=1
  promotion=r['nari'].get(id)
  if promotion:
   targetorigin=id if len(incoming[promotion])>1 else None
   promo='Some(PieceId('+str(index[(promotion,targetorigin)])+'))'
  else:promo='None'
  royal=id in r['winkoma']
  value={'aa':100,'bb':100,'ab':300,'ac':300,'ad':450,'ae':550,'ba':650,'bc':1000,'bd':500,'aw':500,'ax':350,'ay':550,'az':550,'em':100,'af':750,'ag':900,'bl':2200,'dh':2200,'cz':1700,'db':1900,'ct':1600,'dj':1600}.get(id,350+80*len(rays)+20*len(steps))
  if royal:value=2500
  text.append('PieceDef{id:"'+id+'",steps:'+fmt(steps)+',rays:'+fmt(rays)+',promotion:'+promo+',base:'+str(bases.index(base))+',royal:'+str(royal).lower()+',dead_ranks:'+str(dead)+',value:'+str(value)+'},')
  capture='Some(PieceId('+str(index[(base,None)])+'))'if rid==16 else'None'
  metas.append('Meta{origin:'+('Some("'+origin+'")'if origin else'None')+',capture_return:'+capture+',file_limit:'+str(max(0,d.get('nifu',0)-1))+',limits:'+fmt(limits)+'}')
 text+= ['];','const '+prefix+'_META:&[Meta]=&['+','.join(metas)+'];','const '+prefix+'_INVENTORY:&[usize]=&['+','.join(str(inv[b])for b in bases)+'];','const '+prefix+'_INITIAL:&[([u16;2],u16,u8)]=&[']
 for x,f in enumerate(q['initial']['ban']):
  for y,c in enumerate(f):
   if c:text.append('(['+str(x+1)+','+str(y+1)+'],'+str(index[(c[1],None)])+','+str(c[0])+'),')
 text+= ['];','fn '+prefix.lower()+'_continuation(id:PieceId,delta:(isize,isize))->Option<&\'static[(isize,isize)]>{match(id.0,delta){']
 for id,xy,follow in continuations:text.append('('+str(id)+',('+str(xy[0])+','+str(xy[1])+'))=>Some('+fmt(follow)+'),')
 text+=['_=>None,}}']
 ruleset={16:'torishogi',50:'washogi',15:'heian-dai',49:'daishogi'}[rid]
 uchi='Some(PieceId('+str(index[('bb',None)])+'))'if rid==16 else'None'
 text.append('classical_rule!('+name+','+str(size)+','+state+','+hands+','+str(rid)+',"'+ruleset+'",'+str(dan)+','+str(forced).lower()+','+str(check).lower()+','+str(stale).lower()+','+str(kings).lower()+','+str(lions).lower()+','+uchi+','+prefix+'_PIECES,'+prefix+'_META,'+prefix+'_INVENTORY,'+prefix+'_INITIAL,'+prefix.lower()+'_continuation);')
 manifest.append({'ruleId':rid,'name':ruleset,'width':size,'height':size,'royals':r['winkoma'],'useHands':rid==16,'lion':lions,'pieces':[{'id':id,**({'origin':origin}if origin else{})}for id,origin,base in entries],'handIds':bases if rid==16 else[]})
Path(__file__).resolve().parents[1].joinpath('haitaka/src/extended/classical/defs.rs').write_text('\n'.join(text)+'\n')
args.config.write_text(json.dumps(manifest,ensure_ascii=False,indent=2)+'\n')

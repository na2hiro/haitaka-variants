from pathlib import Path
import json,subprocess,time,statistics,datetime
root=Path('/home/na2hiro/proj/haitaka-variants');work=root/'target/honshogi-historical-impact';out=work/'native-results.json'
while not 'after build completed' in Path('/tmp/honshogi-historical-impact-build.log').read_text():time.sleep(2)
cases=[('初期局面','lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1',5,19861490),('持駒の多い終盤','ln1g5/1r4k2/p2pppn2/2ps2p2/1p7/2P6/PPSPPPPLP/2G2K1pr/LN4G1b b BG2SLPnp 61',3,3107043)]
report={'metric':'Native leaf-applied perft; no bulk/TT/evaluation/pruning; SFEN parse excluded. Same harness/checkout path; warmup before every >=5s sample, five alternating samples per version, CPU 7.','cases':[]}
for label,sfen,depth,expected in cases:
 samples={'before':[],'after':[]}
 for i in range(5):
  for side in (['before','after'] if i%2==0 else ['after','before']):
   result=json.loads(subprocess.check_output(['taskset','-c','7',str(work/side/'native'),sfen,str(depth),'5000'],text=True,timeout=300));assert result['nodesPerPerft']==expected
   sample=result['samples'][0];assert sample['seconds']>=5;samples[side].append(sample)
 row={'label':label,'sfen':sfen,'depth':depth,'nodesPerPerft':expected}
 for side in samples:
  a=samples[side];med=statistics.median(x['nps'] for x in a);spread=max(x['nps'] for x in a)/min(x['nps'] for x in a)-1
  row[side]={'samples':a,'medianNps':med,'sampleRangePercent':spread*100}
 row['npsChangePercent']=(row['after']['medianNps']/row['before']['medianNps']-1)*100;report['cases'].append(row);report['measuredAt']=datetime.datetime.now(datetime.timezone.utc).isoformat();out.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n');print(label,round(row['before']['medianNps']),round(row['after']['medianNps']),round(row['npsChangePercent'],3),flush=True)

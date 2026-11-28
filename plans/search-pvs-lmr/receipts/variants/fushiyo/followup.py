from collections import Counter
import json
from pathlib import Path
import shlex
import subprocess
import time

root = Path(__file__).resolve().parent
repo = root.parents[3]
markers = [root / f'{a}-vs-{b}-100ms' / 'self-play-report.json'
           for a,b in [('pvs','baseline'),('lmr','pvs')]]
deadline = time.monotonic() + 600
while not all(m.is_file() for m in markers):
    if time.monotonic() >= deadline:
        raise RuntimeError('random-opening runs still missing')
    time.sleep(3)
assert sum(json.loads(m.read_text())['summary']['draws'] for m in markers) >= 60
fixtures = json.loads(
    (repo/'target/handcrafted-eval-phase0/inputs/fushiyo/fixtures.json').read_text())['fixtures']
rows = [f for f in fixtures if f['partition']=='calibration' and f['stage'] in ('progressed','sparse')]
assert len(rows)==len({f['family'] for f in rows})==16
suite = root/'calibration-openings.jsonl'
suite.write_text(''.join(json.dumps(f['request'])+'\n' for f in rows))
output = {}
for a,b in [('pvs','baseline'),('lmr','pvs')]:
    label = f'{a}-vs-{b}-100ms-calibration'
    folder=root/label
    cmd=list(map(str,[root/'baseline','self-play','--games',32,'--threads',4,
        '--movetime-ms',100,'--max-plies',120,'--opening-unit','pair','--seed',1,
        '--openings',suite,
        '--a-engine',root/a,'--a-engine-arg','usi','--a-engine-arg=--eval','--a-engine-arg','handcrafted',
        '--b-engine',root/b,'--b-engine-arg','usi','--b-engine-arg=--eval','--b-engine-arg','handcrafted',
        '--report-dir',folder]))
    (root/(label+'.command')).write_text(shlex.join(cmd)+'\n')
    print(label,'started',flush=True)
    with (root/(label+'.log')).open('w') as log:
        subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT,check=True)
    report=json.loads((folder/'self-play-report.json').read_text())
    games=[json.loads(line) for line in (folder/'self-play-games.jsonl').read_text().splitlines()]
    assert len(games)==32 and report['summary']['pairCount']==16
    stage={}
    for i,entry in enumerate(rows):
        pair=[g for g in games if g['pairIndex']==i]
        assert len(pair)==2 and {g['aColor'] for g in pair}=={'black','white'}
        assert pair[0]['startSfen']==pair[1]['startSfen']
        assert pair[0]['opening']['baseSfen']==pair[1]['opening']['baseSfen']==entry['request']['baseSfen']
        assert pair[0]['opening']['prefixMoves']==pair[1]['opening']['prefixMoves']==entry['request']['moves']
        value=stage.setdefault(entry['stage'], {'wins':0,'losses':0,'draws':0,'capped':0})
        for g in pair:
            value[{'A':'wins','B':'losses',None:'draws'}[g['winner']]]+=1
            value['capped']+=g['terminationReason']=='maximum-ply-capped'
    summary=report['summary']
    output[label]={**{k:summary[k] for k in ('games','pairCount','aWins','bWins','draws','scoreRate','pairedElo','pairedElo95Ci')},
                   'terminationReasons':dict(Counter(g['terminationReason'] for g in games)), 'stages':stage}
    print(label,output[label],flush=True)
(root/'calibration-summary.json').write_text(json.dumps(output,indent=2)+'\n')

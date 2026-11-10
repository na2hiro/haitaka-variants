#!/usr/bin/env python3
"""Bounded root/teacher diagnosis and native legacy-control match on the MBP."""
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
ROOT=Path(__file__).resolve().parents[1]
RUN=ROOT/'out/anhoku-strength-first/step6-browser-export-20260908-v1'
OLD=ROOT/'out/anhoku-strength-first/step4-objective-search-diagnostic-20260907-v1'
CONTRACT=ROOT/'r0/anhoku-strength-first/step6-browser-export-v1.json'
def rows(p):return [json.loads(x) for x in p.read_text().splitlines() if x]
def main():
    os.chdir(ROOT)
    config=json.loads(CONTRACT.read_text())
    assert not (RUN/'remote-started.json').exists()
    started=time.time()
    (RUN/'remote-started.json').write_text(json.dumps({'unix':started,'contractSha256':hashlib.sha256(CONTRACT.read_bytes()).hexdigest()}))
    def invoke(cmd,log,timeout=14400):
        log.parent.mkdir(parents=True,exist_ok=True)
        log.with_suffix('.command.json').write_text(json.dumps([str(x) for x in cmd],indent=2)+'\n')
        with log.open('w') as output:subprocess.run([str(x) for x in cmd],stdout=output,stderr=subprocess.STDOUT,check=True,timeout=timeout)
    probe=RUN/'bin/strength_first_step4_diagnostic'
    model=ROOT/config['candidate']['path']
    parents=rows(RUN/'parents.jsonl')
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        jobs=[pool.submit(invoke,[probe,'--mode','probe','--input',RUN/'parents.jsonl','--output-dir',RUN/'corrected-probes'/f'shard-{s:02}','--evaluator','nnue','--model',model,'--model-label','corrected-scale1','--nodes',10000,'--start',s*16,'--end',(s+1)*16],RUN/'remote-logs'/f'probe-{s}.log') for s in range(8)]
        for job in jobs:job.result()
    corrected={x['parentId']:x for p in (RUN/'corrected-probes').glob('*/probe.jsonl') for x in rows(p)}
    legacy={x['parentId']:x for p in (OLD/'remote/initial/candidate').glob('*/probe.jsonl') for x in rows(p)}
    handcrafted={x['parentId']:x for p in (OLD/'remote/initial/handcrafted').glob('*/probe.jsonl') for x in rows(p)}
    static={x['parentId']:x for x in rows(RUN/'export-analysis/choices.jsonl')}
    union=[]
    for p in parents:
        pid=p['id']
        observations={'corrected':corrected[pid],'legacy':legacy[pid],'handcrafted':handcrafted[pid]}
        choices={k:v['bestMove'] for k,v in observations.items() if v['legalBestMove'] and v['completedDepth']>0}
        if 'fullMove' in static[pid]:choices['fullStatic']=static[pid]['fullMove']
        moves=sorted(set(choices.values()))
        assert 0<len(moves)<=4
        union.append({**p,'unionMoves':moves,'rootChoices':choices,'rootObservations':observations})
    plan=RUN/'union-plan.jsonl'
    plan.write_text(''.join(json.dumps(x)+'\n' for x in union))
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        jobs=[pool.submit(invoke,[probe,'--mode','label','--input',plan,'--output-dir',RUN/'labels'/f'shard-{s:02}','--evaluator','handcrafted','--nodes',250000,'--start',s*16,'--end',(s+1)*16],RUN/'remote-logs'/f'label-{s}.log') for s in range(8)]
        for job in jobs:job.result()
    diagnostic_nodes=sum(x['totalNodes'] for x in corrected.values())+sum(x['totalNodes'] for p in (RUN/'labels').glob('*/labels.jsonl') for x in rows(p))
    assert diagnostic_nodes<=config['audit']['maximumSearchNodes'] and time.time()-started<=14400
    (RUN/'remote-diagnostic-completed.json').write_text(json.dumps({'complete':True,'elapsedSeconds':time.time()-started,'searchNodes':diagnostic_nodes},indent=2)+'\n')
    cli=RUN/'bin/haitaka_cli'
    legacy_cli=ROOT/'out/anhoku-strength-first/step5-scale-time-20260908-v1/bin/haitaka_cli'
    lines=[x for x in (ROOT/'haitaka_learn/openings/anhoku-v3.tsv').read_text().splitlines() if x and not x.startswith('#')][:32]
    openings=RUN/'native-openings.sfen';openings.write_text('\n'.join(x.split('\t',1)[1] for x in lines)+'\n')
    protocol={'parentContractSha256':hashlib.sha256(CONTRACT.read_bytes()).hexdigest(),'openingsSha256':hashlib.sha256(openings.read_bytes()).hexdigest(),'openingIds':[x.split('\t')[0] for x in lines],'games':64,'threads':8,'movetimeMs':1000,'maxPlies':200,'aBinarySha256':hashlib.sha256(cli.read_bytes()).hexdigest(),'bBinarySha256':hashlib.sha256(legacy_cli.read_bytes()).hexdigest(),'candidateSha256':hashlib.sha256(model.read_bytes()).hexdigest(),'frozenBeforeGameSearch':True}
    (RUN/'native-protocol.json').write_text(json.dumps(protocol,indent=2)+'\n')
    def engine(side,binary):
        return [f'--{side}-engine',binary]+[f'--{side}-engine-arg={x}' for x in ['usi','--eval','nnue','--nnue',str(model),'--nnue-multiplier','1']]
    cmd=[cli,'self-play','--games',64,'--threads',8,'--movetime-ms',1000,'--max-plies',200,'--seed',20260908,'--openings',openings,'--report-dir',RUN/'native-corrected-vs-legacy',*engine('a',cli),*engine('b',legacy_cli)]
    print('starting native corrected-versus-legacy',flush=True)
    invoke(cmd,RUN/'remote-logs/native-corrected-vs-legacy.log',7200)
    data=rows(RUN/'native-corrected-vs-legacy/self-play-games.jsonl')
    assert len(data)==64 and all(x['failureState'] is None for x in data)
    (RUN/'remote-completed.json').write_text(json.dumps({'complete':True,'elapsedSeconds':time.time()-started},indent=2)+'\n')
if __name__=='__main__':main()

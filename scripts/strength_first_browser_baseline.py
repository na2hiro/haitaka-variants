#!/usr/bin/env python3
"""Run the frozen corrected-WASM development baseline, one browser game at a time."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
ROOT=Path(__file__).resolve().parents[1]
RUN=ROOT/'out/anhoku-strength-first/step6-browser-export-retry-20260910-v2'
ASSETS=ROOT/'out/anhoku-strength-first/step6-browser-export-20260908-v1'
CONTRACT=ROOT/'r0/anhoku-strength-first/step6-browser-export-v2.json'
NODE=Path('/home/na2hiro/.nvm/versions/node/v26.8.1/bin/node')
os.chdir(ROOT)
config=json.loads(CONTRACT.read_text())
for a in [config['candidate'],*config['wasm'],*config['sources'],config['browser']['openings'],config['browser']['secondaryOpenings']]:
    b=Path(a['path']).read_bytes()
    assert len(b)==a['bytes'] and hashlib.sha256(b).hexdigest()==a['sha256'],a['path']
assert json.loads((ASSETS/'wasm-parity.json').read_text())['passed']
assert json.loads((ASSETS/'export-analysis/result.json').read_text())['nativeReferenceTraceParityPassed']
assert NODE.is_file(),NODE
node_version=subprocess.run([NODE,'--version'],check=True,text=True,capture_output=True).stdout.strip()
assert node_version=='v26.8.1',node_version
assert not (RUN/'browser-started.json').exists(),'immutable browser run already started'
RUN.mkdir(parents=True,exist_ok=True)
started=time.time()
(RUN/'browser-started.json').write_text(json.dumps({'unix':started,'pid':os.getpid(),'contractSha256':hashlib.sha256(CONTRACT.read_bytes()).hexdigest(),'node':str(NODE),'nodeVersion':node_version},indent=2)+'\n')
for name,ms,openings,hours in [('primary',1000,'openings.tsv',8),('secondary',100,'secondary-openings.tsv',1)]:
    output=RUN/f'browser-{name}-{ms}ms.json'
    assert not output.exists()
    cmd=[str(NODE),'scripts/strength_first_browser_harness.mjs','--pkg-dir',str(ASSETS/'wasm-256'),'--model',config['candidate']['path'],'--openings',str(ASSETS/openings),'--movetime-ms',str(ms),'--max-plies','200','--timeout-ms',str(hours*3600000),'--contract',str(CONTRACT),'--output',str(output)]
    (RUN/f'browser-{name}.command.json').write_text(json.dumps(cmd,indent=2)+'\n')
    print(f'{time.strftime("%FT%T")} start {name} {ms}ms',flush=True)
    with (RUN/f'browser-{name}.log').open('w') as log:
        subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT,check=True,timeout=hours*3600+60)
    data=json.loads(output.read_text())
    assert data['status']=='passed' and data['summary']['missingMoves']==0 and data['summary']['emergencyFallbacks']==0 and data['summary']['unsearchedMoves']==0
    assert len(data['rawGames'])==(128 if name=='primary' else 32)
    print(f'{time.strftime("%FT%T")} complete {name} {ms}ms',flush=True)
(RUN/'browser-completed.json').write_text(json.dumps({'complete':True,'elapsedSeconds':time.time()-started},indent=2)+'\n')

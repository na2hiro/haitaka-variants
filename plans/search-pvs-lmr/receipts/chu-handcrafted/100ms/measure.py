from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
import math
from pathlib import Path
import shlex
import statistics
import subprocess
import time

out = Path(__file__).resolve().parent
repo = out.parents[1]
builds = ('baseline','pvs','lmr')
deadline = time.monotonic() + 600
for engine in builds:
    while not (out/engine).is_file():
        if time.monotonic() > deadline:
            raise RuntimeError(f'build missing: {engine}')
        time.sleep(2)

fixtures = [
    ('start', 'startpos', (2, 3)),
    ('balanced', 'sfen k11/12/12/3g5g2/12/12/12/3G5G2/12/12/12/11K b - 1', (4, 5)),
    ('lion', 'sfen k11/12/12/12/5p6/6N5/12/12/12/12/12/K11 b - 1', (3, 4)),
    ('promotion', 'sfen k11/12/7S4/12/12/12/12/12/12/12/12/K11 b - 1', (3, 4)),
]
fixed = []
for label, position, depths in fixtures:
    for depth in depths:
        rows = {}
        for engine in builds:
            with subprocess.Popen([out/engine, 'usi'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True) as proc:
                proc.stdin.write(f'position {position}\ngo depth {depth}\n')
                proc.stdin.flush()
                info = proc.stdout.readline().split()
                best = proc.stdout.readline().split()
                assert info[:3] == ['info', 'depth', str(depth)], (label, depth, info)
                assert best[:1] == ['bestmove'], (label, depth, best)
                if 'pv' in info:
                    info = info[:info.index('pv')]
                info.remove('cp')
                fields = dict(zip(info[1::2],info[2::2]))
                rows[engine] = {'move': best[1], **{k:int(fields[k]) for k in (
                    'score','nodes','time','pvsProbes','pvsResearches','lmrReductions','lmrResearches')}}
                proc.stdin.write('quit\n')
                proc.stdin.flush()
        fixed.append(dict(fixture=label,position=position,depth=depth,engines=rows))
        (out/'fixed-depth.json').write_text(json.dumps(fixed,indent=2)+'\n')
        assert rows['baseline']['score']==rows['pvs']['score'], (label,depth,rows)
        print('fixed',label,depth,rows,flush=True)
assert sum(r['engines']['pvs']['pvsProbes'] for r in fixed)>0
# LMR is checked at a deterministic narrow-window node in the Rust suite.
print('fixed LMR counts',sum(r['engines']['lmr']['lmrReductions'] for r in fixed),flush=True)

script = repo/'scripts/handcrafted_json_match.py'
spec = importlib.util.spec_from_file_location('match',script)
matcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(matcher)
input_file = repo/'target/handcrafted-eval-phase0/inputs/chushogi/fixtures.json'
openings = [r['request'] for r in json.loads(input_file.read_text())['fixtures'] if r['partition']=='selection']
assert len(openings)==8
for i in range(4):
    (out/f'openings-{i}.jsonl').write_text(''.join(json.dumps(game)+'\n' for game in openings[i*2:i*2+2]))

def shard(a,b,i):
    directory=out/f'{a}-vs-{b}'/f'shard-{i}'
    directory.parent.mkdir(exist_ok=True)
    args = ['--a-engine',str(out/a),'--b-engine',str(out/b),'--depth','64','--movetime-ms','100',
            '--max-plies','120','--games','4','--seed',str(1+2*i),'--report-dir',str(directory),
            '--openings',str(out/f'openings-{i}.jsonl')]
    directory.with_suffix('.command').write_text(shlex.join(['python3','-B',str(script),*args])+'\n')
    if not (directory/'report.json').exists():
        matcher.run(matcher.parser().parse_args(args))
    report=json.loads((directory/'report.json').read_text())
    assert report['status']=='complete' and report['completedGames']==4
    assert [e['info']['ruleId'] for e in report['engines']]==[48,48]
    games=[json.loads(line) for line in (directory/'games.jsonl').read_text().splitlines()]
    assert len(games)==4
    print(a,b,i,'complete',flush=True)
    return games

summary = {}
matched = []
for a,b in [('pvs','baseline'),('lmr','pvs')]:
    with ThreadPoolExecutor(max_workers=4) as pool:
        results=list(pool.map(lambda i: shard(a,b,i),range(4)))
    games=[g for group in results for g in group]
    pair_scores=[]
    for i in range(0,len(games),2):
        left,right=games[i:i+2]
        assert left['aColor']==0 and right['aColor']==1 and left['opening']==right['opening']
        if a=='pvs':
            matched.append(left['opening'])
        else:
            assert matched[i//2]==left['opening']
        pair_scores.append((left['aScore']+right['aScore'])/2)
    rates=Counter(g['aScore'] for g in games)
    score=statistics.mean(pair_scores)
    mid=min(.999,max(.001,score))
    elo=400*math.log10(mid/(1-mid))
    half=1.96*400/math.log(10)/(mid*(1-mid))*statistics.stdev(pair_scores)/math.sqrt(len(pair_scores))
    result=dict(games=len(games),pairs=len(pair_scores),wins=rates[1],losses=rates[0],
                draws=rates[.5],score=score,elo=elo,ci95=[elo-half,elo+half],
                termination=dict(Counter(g['terminationReason'] for g in games)))
    result['meanDepth']={seat:statistics.mean(s['completedDepth'] for g in games
                          for s in g['searches'] if s['seat']==seat) for seat in ('A','B')}
    summary[f'{a}-vs-{b}']=result
    (out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    print(a,b,result,flush=True)

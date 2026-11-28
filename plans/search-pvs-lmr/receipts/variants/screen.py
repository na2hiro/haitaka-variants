from collections import Counter, defaultdict
import json
import math
from pathlib import Path
import shlex
import subprocess
import sys
import time

out = Path(__file__).resolve().parent
repo = out.parents[2]
rules = tuple(sys.argv[1:]) or ('annan', 'minishogi', 'fushiyo', 'oute', 'nekoneko')
builds = ('baseline', 'pvs', 'lmr')
movetime = 100
num_games = 32
max_plies = 120
workers = 4
seed = 1
opening_plies = 4
summary_path = out / "summary.json"
summary = json.loads(summary_path.read_text()) if summary_path.exists() else {}
for rule in rules:
    for build in builds:
        path = out / rule / build
        deadline = time.monotonic() + 20 * 60
        while not path.is_file():
            if time.monotonic() >= deadline:
                raise RuntimeError(f'missing built engine: {path}')
            time.sleep(3)


def fixed(rule):
    rows = json.loads((repo / 'target/handcrafted-eval-phase0/inputs' / rule / 'fixtures.json').read_text())['fixtures']
    fixtures = [('startpos', 'startpos')]
    for stage in ('early', 'progressed', 'sparse'):
        row = next(r for r in rows if r['partition'] == 'calibration' and r['stage'] == stage)
        req = row['request']
        command = f"sfen {req['baseSfen']}"
        if req['moves']:
            command += ' moves ' + ' '.join(req['moves'])
        fixtures.append((stage, command))
    if rule == 'nekoneko':
        fixtures = [('startpos', 'startpos'), ('few-pieces', 'sfen 4k4/9/9/3p5/9/5P3/9/9/4K4 b - 1')]
    result_path = out / rule / 'fixed-depth.json'
    if rule == 'nekoneko' and result_path.exists():
        results = json.loads(result_path.read_text())
        assert len(results) == 12 and {r['fixture'] for r in results} == {'startpos', 'few-pieces'}
    else:
        results = []
        for label, position in fixtures:
            for depth in (3, 4):
                for engine in builds:
                    request = f'isready\nposition {position}\ngo depth {depth}\nquit\n'
                    print(rule, label, depth, engine, flush=True)
                    output = subprocess.check_output(
                        [out / rule / engine, 'usi', '--eval', 'handcrafted'],
                        input=request, text=True, timeout=180,
                    ).splitlines()
                    assert output[0] == 'readyok' and len(output) == 3, (rule, position, output)
                    info = output[1].split()
                    assert info[:3] == ['info','depth',str(depth)], (rule,position,output)
                    fields = dict(zip(info[1::2],info[2::2]))
                    result = {
                        'fixture': label, 'position': position, 'depth': depth, 'engine': engine,
                        'score': int(fields['lastCompletedIterationValue']),
                        'move': output[2].split()[1],
                    }
                    for key in ('nodes','qnodes','time','qsearchCapHits','qsearchDeltaPrunes','pvsProbes',
                                'pvsResearches','lmrReductions','lmrResearches'):
                        result[key] = int(fields.get(key,'0'))
                    results.append(result)
        (out / rule / 'fixed-depth.json').write_text(json.dumps(results, indent=2) + '\n')
    score_differences = []
    for i in range(0,len(results),3):
        baseline, pvs, lmr = results[i:i+3]
        if pvs['score'] != baseline['score']:
            score_differences.append([baseline,pvs])
        assert not (baseline['pvsProbes'] or baseline['lmrReductions'] or pvs['lmrReductions'])
    trials = {b: {k:sum(row[k] for row in results if row['engine']==b)
                  for k in ('pvsProbes','pvsResearches','lmrReductions','lmrResearches')}
              for b in builds}
    assert trials['pvs']['pvsProbes'] > 0 and trials['lmr']['pvsProbes'] > 0, (rule, trials)
    print(rule, 'fixed-depth', 'differences', score_differences, 'trial counts', trials, flush=True)
    return score_differences, trials


def verify_match(path, a, b, rule, suite):
    report = json.loads((path / 'self-play-report.json').read_text())
    records = list(map(json.loads,(path/'self-play-games.jsonl').read_text().splitlines()))
    command = report['command']
    assert [command[k] for k in ('games','threads','movetimeMs','maxPlies','openingUnit',
                                  'seed','openingRandomPlies')] == [num_games,workers,movetime,max_plies,'pair',seed,opening_plies]
    assert Path(report['engines'][0]['command']) == out/rule/a and Path(report['engines'][1]['command']) == out/rule/b
    assert all(e['args']==['usi','--eval','handcrafted'] for e in report['engines'])
    assert sorted(r['gameIndex'] for r in records)==list(range(1,num_games+1))
    pairs = defaultdict(list)
    for record in records:
        pairs[record['pairIndex']].append(record)
    assert len(pairs)==num_games//2
    scores = []
    points = {'A':1.0,'B':0.0,None:0.5}
    for i,pair in sorted(pairs.items()):
        assert len(pair)==2 and {r['aColor'] for r in pair}=={'black','white'}
        assert pair[0]['startSfen']==pair[1]['startSfen'] and pair[0]['opening']==pair[1]['opening']
        assert len(pair[0]['opening']['prefixMoves'])==opening_plies
        identity = pair[0]['startSfen'],pair[0]['opening']
        assert suite.setdefault(i,identity)==identity
        scores.append(sum(points[r['winner']] for r in pair)/2)
    result = report['summary']
    wins = Counter(r['winner'] for r in records)
    assert [wins['A'],wins['B'],wins[None]] == [result[k] for k in ('aWins','bWins','draws')]
    assert result['pairCount'] == len(scores)
    assert result['pairScoreBins'] == [scores.count(k/4) for k in range(5)]
    mean = sum(scores)/len(scores)
    assert math.isclose(result['scoreRate'],mean)
    bounded = min(0.999,max(0.001,mean))
    elo = 400*math.log10(bounded/(1-bounded))
    se = math.sqrt(sum((x-mean)**2 for x in scores)/((len(scores)-1)*len(scores)))
    half_width = 1.96*400/math.log(10)/(bounded*(1-bounded))*se
    assert math.isclose(elo,result['pairedElo'])
    assert all(math.isclose(x,y) for x,y in zip([elo-half_width,elo+half_width],result['pairedElo95Ci']))
    counters = Counter(r['terminationReason'] for r in records)
    return {**{k:result[k] for k in ('games','aWins','bWins','draws','pairCount','pairScoreBins',
                                     'scoreRate','pairedElo','pairedElo95Ci')}, 'terminationReasons':dict(counters)}


def match(rule,a,b,suite):
    label = f'{a}-vs-{b}-{movetime}ms'
    run = out/rule/label
    command = list(map(str,[out/rule/'baseline','self-play','--games',num_games,'--threads',workers,
        '--movetime-ms',movetime,'--max-plies',max_plies,'--opening-unit','pair',
        '--opening-random-plies',opening_plies,'--seed',seed,
        '--a-engine',out/rule/a,'--a-engine-arg','usi','--a-engine-arg=--eval','--a-engine-arg','handcrafted',
        '--b-engine',out/rule/b,'--b-engine-arg','usi','--b-engine-arg=--eval','--b-engine-arg','handcrafted',
        '--report-dir',run]))
    (out/rule/(label+'.command')).write_text(shlex.join(command)+'\n')
    if not (run/'self-play-report.json').exists():
        print(rule,label,'started',flush=True)
        with (out/rule/(label+'.log')).open('w') as log:
            subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,check=True)
    result = verify_match(run,a,b,rule,suite)
    print(rule,label,result,flush=True)
    return result


failures = {}
for rule in rules:
    try:
        differences,trials = fixed(rule)
        row = {'fixedDepths': [3,4], 'pvsScoreDifferences':differences, 'trialCounts':trials}
        if not differences:
            suite = {}
            row['matches'] = {f'{a}-vs-{b}':match(rule,a,b,suite) for a,b in (('pvs','baseline'),('lmr','pvs'))}
        summary[rule] = row
    except (AssertionError, subprocess.SubprocessError) as error:
        detail = repr(error)
        failures[rule] = detail
        print(rule, 'failed', detail, flush=True)
    (out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    (out/'failures.json').write_text(json.dumps(failures,indent=2)+'\n')
if failures:
    raise SystemExit(1)

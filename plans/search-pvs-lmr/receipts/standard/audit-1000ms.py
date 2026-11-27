from collections import Counter, defaultdict
import json
import math
from pathlib import Path

out = Path(__file__).resolve().parent
old = {}
with (out / 'pvs-vs-baseline-100ms' / 'self-play-games.jsonl').open() as file:
    for line in file:
        game = json.loads(line)
        old[game['pairIndex']] = (game['startSfen'], game['opening'])
assert len(old) == 100

summary = {}
for a,b in [('pvs','baseline'),('lmr','pvs')]:
    label = f'{a}-vs-{b}-1000ms'
    directory = out / label
    report = json.loads((directory/'self-play-report.json').read_text())
    games = [json.loads(line) for line in (directory/'self-play-games.jsonl').read_text().splitlines()]
    command = report['command']
    assert report['ruleset'] == 'standard'
    assert [command[k] for k in ('games','threads','movetimeMs','openingUnit','openingRandomPlies','seed','maxPlies')] == [
        200,4,1000,'pair',4,1,200]
    for engine,name in zip(report['engines'],(a,b),strict=True):
        assert Path(engine['command']).resolve()==out/name
        assert engine['args']==['usi','--eval','handcrafted']
    assert sorted(g['gameIndex'] for g in games)==list(range(1,201))
    pairs = defaultdict(list)
    for g in games:
        assert (g['startSfen'],g['opening'])==old[g['pairIndex']]
        assert {g['aColor'],g['bColor']}=={'black','white'}
        pairs[g['pairIndex']].append(g)
    assert len(pairs)==100
    bins = [0]*5
    game_points = {'A':2,'B':0,None:1}
    for pair in pairs.values():
        assert len(pair)==2 and {g['aColor'] for g in pair}=={'black','white'}
        bins[sum(game_points[g['winner']] for g in pair)]+=1
    outcomes = Counter(g['winner'] for g in games)
    observed = report['summary']
    assert [outcomes[k] for k in ('A','B',None)]==[observed[k] for k in ('aWins','bWins','draws')]
    assert bins==observed['pairScoreBins']
    pair_scores = [i/4 for i,count in enumerate(bins) for _ in range(count)]
    mean = sum(pair_scores)/100
    se = math.sqrt(sum((v-mean)**2 for v in pair_scores)/(100*99))
    elo = 400*math.log10(mean/(1-mean))
    radius = 1.96 * 400 / math.log(10) / (mean*(1-mean)) * se
    assert math.isclose(elo,observed['pairedElo'])
    assert math.isclose(observed['scoreRate'],mean)
    assert all(math.isclose(x,y) for x,y in zip(observed['pairedElo95Ci'],(elo-radius,elo+radius)))
    summary[label] = {
        **{k:observed[k] for k in ('games','pairCount','aWins','bWins','draws','scoreRate','pairedElo','pairedElo95Ci')},
        'terminationReasons': dict(Counter(g['terminationReason'] for g in games)),
    }
(out/'standard-1000ms-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps(summary,indent=2))

from collections import Counter
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import statistics

out = Path(__file__).resolve().parent
repo = out.parents[2]
spec = importlib.util.spec_from_file_location('referee', repo/'scripts/handcrafted_json_match.py')
referee = importlib.util.module_from_spec(spec)
spec.loader.exec_module(referee)
cfg = json.loads((out/'config.json').read_text())
summaries = json.loads((out/'summary.json').read_text())
openings = json.loads((out/'openings.json').read_text())
assert hashlib.sha256((out/'openings.json').read_bytes()).hexdigest() == cfg['openingSha256']
for engine in cfg['engines'].values():
    assert hashlib.sha256(Path(engine['path']).read_bytes()).hexdigest() == engine['sha256']
assert len(openings) == len({row['family'] for row in openings}) == 16
assert Counter(o['partition'] for o in openings) == {'selection':8, 'confirmation':8}
names = {'pvs-vs-baseline':'PVS 対 基準', 'lmr-vs-pvs':'PVS＋LMR 対 PVS'}
labels = {'baseline':'基準', 'pvs':'PVS', 'lmr':'PVS＋LMR'}
bench = {}
assert cfg['movetimeMs'] == 1000 and cfg['maxPlies'] == 200 and cfg['depthLimit'] == 64
worker = referee.Worker(cfg['engines']['baseline']['path'], out/'audit-stderr.log', 30)

def usi(mv):
    return ''.join(f'{x}{chr(96+y)}' for x,y in [mv['from'],*mv['path']]) + ('+' if mv['promotion'] else '')

try:
    for label in names:
        directory = out/label
        records = [json.loads(line) for file in sorted(directory.glob('worker-*.jsonl'))
                    for line in file.read_text().splitlines()]
        games = sorted(records, key=lambda g:g['gameIndex'])
        assert [g['gameIndex'] for g in games]==list(range(32))
        pair_scores = []
        for index, row in enumerate(openings):
            pair = games[2*index:2*index+2]
            assert [g['aColor'] for g in pair]==[0,1]
            for g in pair:
                opening = row['request']
                assert g['opening']==opening and g['pairIndex']==index
                assert len(g['searches'])==len(g['moves'])==g['plies']<=cfg['maxPlies']
                assert g['finalPosition']['history']['moves']==opening['history']['moves']+g['moves']
                final = worker.call('validate', position=g['finalPosition'])
                assert final==g['finalPosition']
                inspected = worker.call('inspect', position=final)
                assert inspected['contextHash']==g['finalContextHash']
                outcome = inspected['outcome']
                if g['terminationReason']=='maximum-ply-capped':
                    assert outcome['kind']=='ongoing' and g['plies']==cfg['maxPlies']
                    assert g['outcome']=={'kind':'draw','winner':None}
                else:
                    assert outcome['kind']!='ongoing' and g['outcome']==outcome
                expected = .5 if g['outcome']['winner'] is None else float(g['outcome']['winner']==g['aColor'])
                assert g['aScore']==expected
                first = opening['position']['sideToMove']
                for ply, (mv,s) in enumerate(zip(g['moves'],g['searches'],strict=True)):
                    assert s['best']==usi(mv)
                    side = first ^ (ply % 2)
                    assert s['seat']==('A' if side==g['aColor'] else 'B')
                    assert s['depth']<=cfg['depthLimit']
                    assert s['pvsResearches']<=s['pvsProbes']
                    assert s['lmrResearches']<=s['lmrReductions']
            pair_scores.append(statistics.mean(g['aScore'] for g in pair))
        data=summaries[label]
        assert data['games']==32 and data['pairs']==16 and data['pairScores']==pair_scores
        counts=Counter(g['aScore'] for g in games)
        assert (data['wins'],data['losses'],data['draws'])==(counts[1],counts[0],counts[.5])
        score=statistics.mean(pair_scores)
        assert math.isclose(data['score'], score)
        bounded=min(.999,max(.001,score))
        logit=lambda x:400*math.log10(x/(1-x))
        elo=logit(bounded)
        half=1.96*400/math.log(10)/(bounded*(1-bounded))*statistics.stdev(pair_scores)/math.sqrt(16)
        assert math.isclose(data['pairedElo'],elo)
        assert all(math.isclose(x,y) for x,y in zip(data['pairedElo95Ci'],[elo-half,elo+half]))
        for seat, name in zip(('A','B'), label.split('-vs-')):
            ss=[x for g in games for x in g['searches'] if x['seat']==seat]
            totals=data['stats'][name]
            assert len(ss)==totals['totalSearches']
            for field in ('pvsProbes','pvsResearches','lmrReductions','lmrResearches'):
                assert sum(x[field] for x in ss)==totals[field]
            assert sum(x['lmrReductions']>0 for x in ss)==totals['searchesWithLmr']
        bench[label]=dict(verifiedGames=len(games), verifiedPairs=len(pair_scores),
                          verifiedMoves=sum(len(g['moves']) for g in games))
finally:
    worker.close()
(out/'audit.json').write_text(json.dumps(bench,indent=2)+'\n')
print(bench)

def number(x):
    return f'{x:,}'

line=[
    '',
    '## 1手1秒の先後ペア対局',
    '',
    '同じ手作り評価のrelease 3構成を使用した。既存の選別8局面と確認8局面を履歴付きで固定し、各比較で先後を交換した。'
    '各32局・16ペア、4物理コアに4ワーカーを固定し、1手1000 ms、深さ上限64、対局上限200完了手とした。',
    'ネイティブ USI の `info` から探索統計を取得し、JSON のルールエンジンで合法手、完成局面、履歴、勝敗を検証した。',
    '',
    '| 比較 | 勝–敗–分 | 打切り | 得点率 | ペア Elo | 95% CI |',
    '|---|---:|---:|---:|---:|---:|',
]
for key in names:
    r=summaries[key]
    lo,hi=r['pairedElo95Ci']
    line.append(f"| {names[key]} | {r['wins']}–{r['losses']}–{r['draws']} | {r['termination'].get('maximum-ply-capped',0)} | "
                f"{r['score']:.2%} | {r['pairedElo']:+.1f} | [{lo:+.1f}, {hi:+.1f}] |")
line += ['', '打切りは0.5点として集計した。Eloの区間は先後ペアごとの得点を標本にした近似値。', '',
         '| 比較 | 構成 | 平均完了深さ | 探索手数 | 深さ4以上 | LMRを使った探索 | PVS試し読み / 再探索 | 短縮 / 再探索 |',
         '|---|---|---:|---:|---:|---:|---:|---:|']
for key in names:
    for engine,r in summaries[key]['stats'].items():
        deep=sum(n for depth,n in r['depthHistogram'].items() if int(depth)>=4)
        line.append(f"| {names[key]} | {labels[engine]} | {r['meanDepth']:.2f} | {number(r['totalSearches'])} | "
                    f"{number(deep)} | {number(r['searchesWithLmr'])} | {number(r['pvsProbes'])} / {number(r['pvsResearches'])} | "
                    f"{number(r['lmrReductions'])} / {number(r['lmrResearches'])} |")
for key in names:
    r=summaries[key]
    lo,hi=r['pairedElo95Ci']
    conclusion='区間はプラス側だった' if lo>0 else '区間はマイナス側だった' if hi<0 else '区間が0を含んだ'
    line += ['', f'{names[key]}の95%信頼{conclusion}。']
line+=['', '実行と照合は、保存した3バイナリと固定した局面を使用する。設定・ハッシュ・棋譜・集計は `target/chu-search-trials/1000ms/` に保存した。', '',
       '```bash', 'python3 -B -u target/chu-search-trials/1000ms/run.py', 'python3 -B target/chu-search-trials/1000ms/audit.py', '```', '']
p=repo/'plans/wasm-search-pvs-lmr-chu-results.md'
previous=p.read_text()
section='\n'.join(line)
heading='## 1手1秒の先後ペア対局'
if heading in previous:
    assert section.strip() in previous, 'The recorded one-second results differ from the verified results'
else:
    p.write_text(previous.rstrip()+'\n'+section)

from collections import Counter
import json
from pathlib import Path

out = Path(__file__).resolve().parent
results = json.loads((out/'summary.json').read_text())
extra = json.loads((out/'fushiyo/calibration-summary.json').read_text())
names = {'annan':'安南', 'nekoneko':'NekoNeko', 'minishogi':'5五将棋',
         'fushiyo':'持ち駒不使用', 'oute':'王手将棋'}
engine_names = {'pvs-vs-baseline':'PVS 対 基準探索', 'lmr-vs-pvs':'PVS＋LMR 対 PVS'}
assert set(results)==set(names)
games = sum(m['games'] for r in results.values() for m in r['matches'].values())
assert games == 320 and sum(r['games'] for r in extra.values())==64
lines = [
    '# 代表ルールの PVS・1 ply LMR 実験結果',
    '',
    '計測日 2026-11-21〜22。既存の100 ms代表ルール検査と、前回の探索実験をもとに、共通の盤面・探索を使う5ルールを選んだ。安南と NekoNeko は駒の動きが周囲の駒に依存する。5五将棋は小盤、持ち駒不使用は取り駒の扱い、王手将棋は勝利条件が異なる。JSON ルールの探索は `haitaka::extended` にある。',
    '',
    '変則ルールの候補は `search-variant-experiments` と `search-pvs` または `search-lmr` の組合せで有効になる。両候補は実験 feature として切り替える。',
    '',
    '## 固定深さ',
    '',
    '開始局面と既存の校正用局面を、手作り評価・新しい TT・期限なしの深さ3と4で照合した。NekoNeko の校正用局面では1ケースが180秒を超えたため、開始局面と少駒局面の2局面で再検証した。',
    '',
    '| ルール | PVS と基準探索のスコア一致 | PVS の試し読み / 再探索 | PVS＋LMR の短縮 / 再探索 |',
    '|---|---:|---:|---:|',
]
for rule,title in names.items():
    r=results[rule]
    fixed=json.loads((out/rule/'fixed-depth.json').read_text())
    assert len(fixed)==(12 if rule=='nekoneko' else 24)
    mismatches=len(r['pvsScoreDifferences'])
    assert mismatches==0
    cases=len(fixed)//3
    p=r['trialCounts']['pvs']
    l=r['trialCounts']['lmr']
    assert p['pvsProbes']>0 and l['lmrReductions']>0 and l['lmrResearches']>0
    lines.append(f"| {title} | {cases-mismatches}/{cases} | {p['pvsProbes']:,} / {p['pvsResearches']:,} | {l['lmrReductions']:,} / {l['lmrResearches']:,} |")
lines += [
    '',
    '安南の開始局面・深さ4は、通常のデルタ枝刈りで基準探索32、PVS 20だった。デルタ枝刈り無効では両方2だった。安南は3構成とも実験 feature を付けてデルタ枝刈りを無効に揃えた。他の4ルールは各ルールの通常の qsearch 設定で比較した。NekoNeko の少駒局面は `4k4/9/9/3p5/9/5P3/9/9/4K4 b - 1`。',
    '',
    '安南の実験 feature あり・なしと通常将棋の LMR 構成で、`cargo test --offline --release -p haitaka_wasm --lib` が成功した。安南の実験 feature なしでは試し読みと短縮がゼロだった。',
    '',
    '## 100 ms の対局',
    '',
    '各行は32局、16先後ペア。手作り評価、seed 1、ランダム初手順4 ply、4ワーカー、対局上限120 ply。ルール内の2比較で同じ初手順と先後の組を使ったことを棋譜から確認し、CLI のペア Elo と95%信頼区間を再計算した。並行して本将棋の1秒対局を実行した計測環境での結果。',
    '',
    '| ルール | 比較 | 勝–敗–分 | 打切り | 得点率 | ペア Elo | 95% CI |',
    '|---|---|---:|---:|---:|---:|---:|',
]
def row(title,cmp,r):
    lo,hi=r['pairedElo95Ci']
    score=f"{r['aWins']}–{r['bWins']}–{r['draws']}"
    capped=r['terminationReasons'].get('maximum-ply-capped',0)
    rating=f"{r['pairedElo']:+.1f} | [{lo:+.1f}, {hi:+.1f}]"
    if capped==r['games']:
        rating='— | —'
    return f"| {title} | {cmp} | {score} | {capped} | {r['scoreRate']:.2%} | {rating} |"
for rule,title in names.items():
    for key,value in results[rule]['matches'].items():
        lines.append(row(title,engine_names[key],value))

lines += [
    '',
    '5五将棋の LMR がこのスクリーニングで最も高く、PVS 単体に22勝8敗2分だった。16ペアの結果で、区間は各比較についてのもの。持ち駒不使用は初手順からの64局中62局が120 plyに達した。',
    '',
    '## 持ち駒不使用の追加対局',
    '',
    '既存の校正用進行局面8件と少駒局面8件を選び、同じ100 ms・120 plyで各32局を追加した。初手順からの成績と別に集計した。',
    '',
    '| ルール | 比較 | 勝–敗–分 | 打切り | 得点率 | ペア Elo | 95% CI |',
    '|---|---|---:|---:|---:|---:|---:|',
]
for k,v in extra.items():
    label=k.removesuffix('-100ms-calibration')
    lines.append(row('持ち駒不使用',engine_names[label],v))
lines += [
    '',
    '追加対局でも64局中50局が打切りとなり、両比較の得点率は50%。PVS＋LMR 対 PVS の全ペアが1勝1敗または2分で、ペア得点の分散は0だった。この条件で得られる棋力差の情報は限られる。',
    '',
    '## 再現コマンド',
    '',
    'バイナリ、固定深さ JSON、起動コマンド、棋譜、集計 JSON は `target/search-pvs-lmr/variants/` に保存した。ビルド例は次のとおり。',
    '',
    '```bash',
    'rule=minishogi # annan, nekoneko, fushiyo, oute',
    'out="$PWD/target/search-pvs-lmr/variants/$rule"',
    'mkdir -p "$out"',
    'baseline_features=$rule',
    'if [[ "$rule" == annan ]]; then',
    '    baseline_features="$rule,search-variant-experiments"',
    'fi',
    'cargo build --offline --release -p haitaka_cli --features "$baseline_features"',
    'cp target/release/haitaka_cli "$out/baseline"',
    'cargo build --offline --release -p haitaka_cli --features "$rule,search-variant-experiments,search-pvs"',
    'cp target/release/haitaka_cli "$out/pvs"',
    'cargo build --offline --release -p haitaka_cli --features "$rule,search-variant-experiments,search-lmr"',
    'cp target/release/haitaka_cli "$out/lmr"',
    '```',
    '',
    '対局は保存済みの `*-100ms.command` で再現できる。各ディレクトリの JSON に実際の対局条件と手順が記録されている。',
    '',
    '```bash',
    '"$out/baseline" self-play \\',
    '    --games 32 --threads 4 --movetime-ms 100 --max-plies 120 \\',
    '    --opening-unit pair --opening-random-plies 4 --seed 1 \\',
    '    --a-engine "$out/pvs" --a-engine-arg usi --a-engine-arg=--eval --a-engine-arg handcrafted \\',
    '    --b-engine "$out/baseline" --b-engine-arg usi --b-engine-arg=--eval --b-engine-arg handcrafted \\',
    '    --report-dir "$out/pvs-vs-baseline-100ms"',
    '```',
    '',
]
(out/'variant-report.md').write_text('\n'.join(lines))

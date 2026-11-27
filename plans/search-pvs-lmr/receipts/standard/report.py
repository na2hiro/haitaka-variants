from collections import Counter, defaultdict
from itertools import batched
import hashlib
import json
import math
from pathlib import Path
import re

data = Path('target/search-pvs-lmr')
fixed = json.loads((data / 'fixed-depth.json').read_text())
assert len(fixed) == 48
assert all(b['score'] == p['score'] for b, p, _ in batched(fixed, 3))
matches = []
opening_by_pair = {}
for a, b, ms in (('pvs', 'baseline', 20), ('lmr', 'pvs', 20),
                 ('pvs', 'baseline', 100), ('lmr', 'pvs', 100)):
    directory = data / f'{a}-vs-{b}-{ms}ms'
    report = json.loads((directory / 'self-play-report.json').read_text())
    games = [json.loads(line) for line in (directory / 'self-play-games.jsonl').read_text().splitlines()]
    command = report['command']
    assert report['ruleset'] == 'standard'
    assert [command[k] for k in ('games', 'threads', 'movetimeMs', 'openingUnit',
                                'openingRandomPlies', 'seed', 'maxPlies')] == [200, 4, ms, 'pair', 4, 1, 200]
    for engine, name in zip(report['engines'], [a,b], strict=True):
        assert Path(engine['command']).resolve() == (data / name).resolve()
        assert engine['args'] == ['usi', '--eval', 'handcrafted']
    assert sorted(game['gameIndex'] for game in games) == list(range(1, 201))
    pairs = defaultdict(list)
    for game in games:
        assert game['pairIndex'] == (game['gameIndex'] - 1) // 2
        pairs[game['pairIndex']].append(game)
    assert len(pairs) == 100
    result_to_halfpoints = {'A': 2, 'B': 0, None: 1}
    bins = [0] * 5
    for index, pair in pairs.items():
        assert len(pair) == 2
        assert {g['aColor'] for g in pair} == {'black', 'white'}
        assert pair[0]['startSfen'] == pair[1]['startSfen']
        assert pair[0]['opening'] == pair[1]['opening']
        opening = pair[0]['opening']
        assert len(opening['prefixMoves']) == 4 and opening['randomPlies'] == 4
        assert opening['randomSeed'] == ((index << 32) ^ 1)
        assert opening_by_pair.setdefault(index, opening) == opening
        bins[sum(result_to_halfpoints[g['winner']] for g in pair)] += 1
    summary = report['summary']
    outcomes = Counter(g['winner'] for g in games)
    assert [outcomes[k] for k in ['A', 'B', None]] == [
        summary['aWins'], summary['bWins'], summary['draws']]
    assert summary['pairCount'] == 100 and summary['pairScoreBins'] == bins
    scores = [i / 4 for i, n in enumerate(bins) for _ in range(n)]
    mean = sum(scores) / 100
    score_se = math.sqrt(sum((score - mean) ** 2 for score in scores) / (99 * 100))
    expected = 400 * math.log10(mean / (1 - mean))
    radius = 1.96 * 400 / math.log(10) / (mean * (1 - mean)) * score_se
    assert math.isclose(summary['pairedElo'], expected)
    assert all(math.isclose(reported, calculated) for reported, calculated in
               zip(summary['pairedElo95Ci'], [expected - radius, expected + radius]))
    matches.append((a, b, ms, summary, Counter(game['terminationReason'] for game in games)))

label = {'baseline': '現行', 'pvs': 'PVS', 'lmr': 'PVS＋LMR'}
def rate(n, d):
    return f'{n:,}/{d:,} ({100*n/d:.2f}%)' if d else '—'
def tail_status(name):
    suffix = '-final' if name in ('search-pvs', 'search-lmr') else ''
    log = (data / f'test-{name}{suffix}.log').read_text()
    result = re.search(r'test result: ok\. (\d+) passed; 0 failed; (\d+) ignored;', log)
    assert result, name
    return result.groups()

lines = [
    '# 通常将棋の PVS・1 ply LMR 実験結果',
    '',
    '計測日 2026-11-21。通常将棋の release ビルドで手作り評価を使用した。現行探索は変更前の `7f29506d`。実験は `search-pvs` と `search-lmr` で切り替える。',
    '',
    '## 固定深さの検証',
    '',
    'PVS 単体は、開始局面、既存の序盤4局面、戦術3局面、詰み、王手回避、千日手と連続王手の成立直前の局面で現行探索とルートスコアが一致した。局面ごとに固定した2深さを、通常の qsearch 設定とデルタ枝刈り無効の両方で照合した。探索ごとに TT と手順付けを初期化し、期限は設けなかった。成立済みの千日手と連続王手も、終局値と試し読みゼロを確認した。',
    '',
    'LMR の回帰テストでは5手目の TT 深さが1 ply 浅いこと、短縮 fail-low の親に TT を保存しないこと、`alpha` 超過と短縮時の詰み値から通常深さへ戻ることを確認した。通常の再探索中のノード予算切れと、qsearch 上限からの再実行中の中断を用い、確定済みのルート手だけが残って履歴が復元されることを確認した。',
    '',
    '安南と NekoNeko は `search-lmr` を有効にして深さ4の USI 統計を確認し、PVS と LMR の各カウンタはすべてゼロだった。',
    '',
    '| 構成 | 成功 | 既存の無効化済みテスト |',
    '|---|---:|---:|',
]
for name, title in [('default', '現行'), ('search-pvs', 'PVS'), ('search-lmr', 'PVS＋LMR'),
                    ('annan,search-lmr', '安南＋実験 feature'), ('nekoneko,search-lmr', 'NekoNeko＋実験 feature')]:
    passed, ignored = tail_status(name)
    lines.append(f'| {title} | {passed} | {ignored} |')

lines += [
    '',
    '各構成について `cargo test --offline --release -p haitaka_wasm --lib` を実行した。実験 feature 付きの `wasm32-unknown-unknown` と、学習データ生成に使う既定構成の `haitaka_learn` は `cargo check --offline` を通過した。USI の `go depth`、`go movetime`、`go nodes` から実験カウンタを取得できることも確認した。',
    '',
    '## 固定深さの計測',
    '',
    'Intel Core i7-6700、rustc 1.98.1。各局面と深さについてビルドの順番を交替しながら3回計測した。各回で USI プロセスを起動し直し、初期 TT と初期手順付けで `go depth` を実行した。実時間は `go` 送信から `bestmove` 受信までの中央値。ノード数、スコア、選択手、再探索件数は3回一致した。スコアは手番側から見た `lastCompletedIterationValue`。',
    '',
    '各セルは **スコア / 選択手 / 主探索ノード / qnodes / 実時間 ms** の順。',
    '',
    '| 局面 | 深さ | 現行 | PVS | PVS＋LMR |',
    '|---|---:|---|---|---|',
]
def cell(r):
    return f"{r['score']} / `{r['move']}` / {r['nodes']:,} / {r['qnodes']:,} / {r['elapsedMs']:.2f}"
for b,p,l in batched(fixed,3):
    lines.append(f"| {b['fixture']} | {b['depth']} | {cell(b)} | {cell(p)} | {cell(l)} |")

lines += ['', '再探索率の分母は PVS の試し読み件数または LMR の短縮件数。',
          '', '| 局面 | 深さ | PVS の再探索率 | PVS＋LMR の PVS 再探索率 | PVS＋LMR の LMR 再探索率 |',
          '|---|---:|---:|---:|---:|']
for b,p,l in batched(fixed,3):
    lines.append(f"| {b['fixture']} | {b['depth']} | {rate(p['pvsResearches'],p['pvsProbes'])} | "
                 f"{rate(l['pvsResearches'],l['pvsProbes'])} | {rate(l['lmrResearches'],l['lmrReductions'])} |")

lines += ['', '8局面の合計に対する増減率。実時間は各局面の中央値を合計した。',
          '', '| 深さ | 比較 | 主探索ノード | qnodes | 合計ノード | 実時間 |',
          '|---|---|---:|---:|---:|---:|']
for depth in (4, 5):
    stats = {}
    for name in label:
        runs = [r for r in fixed if (r['depth'], r['engine']) == (depth, name)]
        main, quiet, ms = (sum(r[k] for r in runs) for k in ('nodes', 'qnodes', 'elapsedMs'))
        stats[name] = [main, quiet, main + quiet, ms]
    for a,b in (('pvs','baseline'),('lmr','pvs')):
        deltas = [f'{100*(x/y-1):+.1f}%' for x,y in zip(stats[a],stats[b])]
        lines.append(f"| {depth} | {label[a]} 対 {label[b]} | " + ' | '.join(deltas) + ' |')

lines += [
    '',
    'PVS のスコアと選択手は全16ケースで現行と一致した。PVS＋LMR は opening-1 の深さ5で 208 から 204 になり、手が `3c2b` から `3c7g` に変わった。デルタ枝刈り無効でも同じスコア差が出た。短縮を外した参照テストでは 208 に戻った。bishop-promotion の深さ5でも手が変わり、スコアは 942 だった。',
    '',
    '## 先後を入れ替えた自己対局',
    '',
    '各比較は200局、同一初手順を先後入れ替えした100ペア。全4比較で seed 1、ランダム初手順4 ply、4ワーカー、対局上限200 plyを使用した。棋譜から、4比較が同じ100初手順を使い、全ペアで先後が反転したことを照合した。Elo と95%信頼区間は CLI のペア単位推定。棋譜からペアの得点分布を再計算し、CLI の数値と照合した。',
    '',
    '| A 対 B | 1手 | A 勝–B 勝–引分 | A 得点率 | ペア Elo | 95% CI |',
    '|---|---:|---:|---:|---:|---:|',
]
for a,b,ms,s,_ in matches:
    lo, hi = s['pairedElo95Ci']
    lines.append(f"| {label[a]} 対 {label[b]} | {ms} ms | {s['aWins']}–{s['bWins']}–{s['draws']} | "
                 f"{s['scoreRate']:.2%} | {s['pairedElo']:+.1f} | [{lo:+.1f}, {hi:+.1f}] |")

lines += ['', 'RESULT_INTERPRETATION', '',
          '## 実行コマンドと保存先', '',
          '現行バイナリは実装前にビルドした。', '',
          '```bash',
          'run_dir="$PWD/target/search-pvs-lmr"',
          'mkdir -p "$run_dir"',
          'cargo build --offline --release -p haitaka_cli',
          'cp target/release/haitaka_cli "$run_dir/baseline"',
          'cargo build --offline --release -p haitaka_cli --features search-pvs',
          'cp target/release/haitaka_cli "$run_dir/pvs"',
          'cargo build --offline --release -p haitaka_cli --features search-lmr',
          'cp target/release/haitaka_cli "$run_dir/lmr"',
          '```',
          '',
          '固定深さ計測に使用したスクリプトと生データは `target/search-pvs-lmr/fixed-depth.py` と `target/search-pvs-lmr/fixed-depth.json`。たとえば開始局面の深さ5は以下で再現できる。',
          '',
          '```bash',
          'for build in baseline pvs lmr',
          'do',
          "    printf '%s\\n' isready 'position startpos' 'go depth 5' quit |",
          '        "$run_dir/$build" usi --eval handcrafted',
          'done',
          '```',
          '',
          '固定局面は既存の `ORDERING_TEST_OPENINGS` の順と qsearch の戦術フィクスチャ3件。',
          '',
          '| 局面 | SFEN |',
          '|---|---|',
]
for name, sfen in zip(['opening-1','opening-2','opening-3','opening-4','rook-capture','silver-promotion','bishop-promotion'], [
    'lnsg1gsnl/1r2k2b1/1ppppp+Bpp/p8/9/2P6/PP1PPPPPP/7R1/LNSGKGSNL b P 5',
    'lnsgkgsnl/2r4b1/ppppp1ppp/5p3/7P1/9/PPPPPPP1P/1B5R1/LNSGKGSNL b - 5',
    'lnsgk1snl/1r4gb1/pp1pppppp/2p6/9/9/PPPPPPPPP/1B2GK1R1/LNSG2SNL b - 5',
    'lns1k1snl/1r1g1g1b1/ppppppppp/9/9/9/PPPPPPPPP/1B1RK4/LNSG1GSNL b - 5',
    '9/9/k8/9/4Rr3/9/9/9/4K4 b - 1',
    '4k4/9/4S4/9/9/9/9/9/4K4 b - 1',
    '4k4/9/4B4/9/9/9/9/9/4K4 b - 1',
]):
    lines.append(f'| {name} | `{sfen}` |')

lines += [
    '',
    '対局ログ、実際のコマンド、JSON 集計と棋譜を `target/search-pvs-lmr/` に保存した。4比較は同時に走らせず順番に実施した。一部の対局は低優先度のテストビルドと並行した。',
    '',
    '```bash',
    'run_match() {',
    '    local time_ms=$1 challenger=$2 opponent=$3',
    '    "$run_dir/baseline" self-play \\',
    '        --games 200 --threads 4 --movetime-ms "$time_ms" \\',
    '        --opening-unit pair --opening-random-plies 4 --seed 1 --max-plies 200 \\',
    '        --a-engine "$run_dir/$challenger" --a-engine-arg usi --a-engine-arg=--eval --a-engine-arg handcrafted \\',
    '        --b-engine "$run_dir/$opponent" --b-engine-arg usi --b-engine-arg=--eval --b-engine-arg handcrafted \\',
    '        --report-dir "$run_dir/$challenger-vs-$opponent-${time_ms}ms"',
    '}',
    'run_match 20 pvs baseline',
    'run_match 20 lmr pvs',
    'run_match 100 pvs baseline',
    'run_match 100 lmr pvs',
    '```',
    '',
    '| release バイナリ | SHA-256 |',
    '|---|---|',
]
for binary in label:
    digest = hashlib.sha256((data / binary).read_bytes()).hexdigest()
    lines.append(f'| {binary} | `{digest}` |')
lines.append('')

for a,b,ms,s,termination in matches:
    print(a,b,ms,s['pairedElo'],s['pairedElo95Ci'],dict(termination))
(data / 'report-draft.md').write_text('\n'.join(lines))

"""Record the audited 4096-ply continuation in the chushogi experiment report."""
import json
from continue_match import CAP, CAPPED, LABELS, out, repo

summary = json.loads((out/'summary.json').read_text())
report = repo/'plans/wasm-search-pvs-lmr-chu-results.md'
heading = '## 上限4096手での継続対局'
names = {'pvs-vs-baseline': 'PVS 対 基準', 'lmr-vs-pvs': 'PVS＋LMR 対 PVS'}
engine_name = {'baseline':'基準', 'pvs':'PVS', 'lmr':'PVS＋LMR'}


def score_display(value, format_spec):
    return '—' if value is None else format(value, format_spec)


def capped_game_details():
    for label in LABELS:
        for path in sorted((out/label).glob('game-*.json')):
            game = json.loads(path.read_text())
            if game['terminationReason'] != CAPPED:
                continue
            board = {tuple(piece['at']):piece['color']
                     for piece in game['opening']['position']['pieces']}
            last_capture = None
            for ply, move in enumerate(game['moves'], start=1):
                color = board.pop(tuple(move['from']))
                for destination in map(tuple, move['path']):
                    if destination in board:
                        assert board.pop(destination) != color
                        last_capture = ply
                board[destination] = color
            assert board == {tuple(piece['at']):piece['color']
                             for piece in game['finalPosition']['position']['pieces']}
            yield (label, game['pairIndex']+1, '先手' if game['aColor']==0 else '後手',
                   last_capture, list(board.values()).count(0), list(board.values()).count(1))


def main():
    lines = [heading, '',
        '1手1秒の64局のうち、200手で打ち切った44局を保存した全棋譜から再開した。'
        '同一ハッシュのリリースバイナリと固定局面を使い、4ワーカーで1手1000 ms、深さ上限64。'
        '手数は開始局面から数え、獅子の二段階移動も完了着手1手とした。',
        'ルール上の終局を先に裁定する。上限に達した局を別に数え、その局を含む先後ペアは得点率とEloの集計から除外する。',
        '',
        '| 比較 | 上限 | 勝–敗–分 | 打ち切り局 | 集計ペア / 総ペア | 得点率 | ペアElo | 95%信頼区間 |',
        '|---|---:|---:|---:|---:|---:|---:|---:|']
    for label in LABELS:
        assert summary[label]['caps'][str(CAP)]['games'] == 32
        for cap in (200, 400, CAP):
            row = summary[label]['caps'][str(cap)]
            lo_hi = row['pairedElo95Ci']
            lo_hi = '—' if lo_hi is None else f'[{lo_hi[0]:+.1f}, {lo_hi[1]:+.1f}]'
            lines.append(
                f"| {names[label]} | {cap} | {row['wins']}–{row['losses']}–{row['draws']} | {row['capped']} | "
                f"{row['usablePairs']} / 16 | {score_display(row['score'], '.2%')} | "
                f"{score_display(row['pairedElo'], '+.1f')} | {lo_hi} |")
    lines += ['', '勝–敗–分はルール上終局した局の内訳。200手と400手の結果は実際の棋譜を各上限で区切って集計した。'
              '区間は先後ペア得点を標本にしたデルタ法による近似値。ペア得点にばらつきがない場合は省略した。', '',
              '| 比較 | 構成 | 継続分の探索手数 | 平均完了深さ | 深さ4以上 | PVS試し読み / 再探索 | LMR短縮 / 再探索 |',
              '|---|---|---:|---:|---:|---:|---:|']
    for label in LABELS:
        for engine, stats in summary[label]['searchStats'].items():
            row = stats['continuation']
            lines.append(f"| {names[label]} | {engine_name[engine]} | {row['searches']:,} | "
                         f"{row['meanDepth']:.2f} | {row['depth4Plus']:,} | {row['pvsProbes']:,} / {row['pvsResearches']:,} | "
                         f"{row['lmrReductions']:,} / {row['lmrResearches']:,} |")
    max_ply = max(row['maxPlies'] for row in summary.values())
    audited = sum(row['verifiedMoves'] for row in summary.values())
    lines += ['', f'最長局は{max_ply:,}手。全64局の{audited:,}手を検証し、旧棋譜との一致、着手の合法性、終局を照合した。']
    capped = list(capped_game_details())
    assert len(capped) == sum(row['caps'][str(CAP)]['capped'] for row in summary.values())
    if capped:
        lines += ['', '| 比較 | 開始局面番号 | 比較の左側 | 最後に駒を取った手 | 終了時の駒数（先手 / 後手） |',
                  '|---|---:|---|---:|---:|']
        for label, pair, color, last_capture, black, white in capped:
            lines.append(f'| {names[label]} | {pair} | {color} | {last_capture or "—"} | {black} / {white} |')
    lines += ['', '実験の設定、再開用の棋譜、集計JSONは `target/chu-search-trials/4096plies/` に記録した。', '',
              '```bash',
              'python3 -B -u target/chu-search-trials/4096plies/continue_match.py',
              'python3 -B target/chu-search-trials/4096plies/continue_match.py --audit',
              'python3 -B target/chu-search-trials/4096plies/write_report.py',
              '```', '']
    content = '\n'.join(lines)
    previous = report.read_text()
    if heading in previous:
        assert content.strip() in previous
        return
    with report.open('a') as file:
        file.write(('\n' if previous.endswith('\n') else '\n\n') + content)


if __name__ == '__main__':
    main()

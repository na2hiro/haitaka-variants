import json
from pathlib import Path
import statistics
import subprocess
from time import perf_counter

root = Path(__file__).resolve().parent
fixtures = {
    'startpos': 'startpos',
    'opening-1': 'sfen lnsg1gsnl/1r2k2b1/1ppppp+Bpp/p8/9/2P6/PP1PPPPPP/7R1/LNSGKGSNL b P 5',
    'opening-2': 'sfen lnsgkgsnl/2r4b1/ppppp1ppp/5p3/7P1/9/PPPPPPP1P/1B5R1/LNSGKGSNL b - 5',
    'opening-3': 'sfen lnsgk1snl/1r4gb1/pp1pppppp/2p6/9/9/PPPPPPPPP/1B2GK1R1/LNSG2SNL b - 5',
    'opening-4': 'sfen lns1k1snl/1r1g1g1b1/ppppppppp/9/9/9/PPPPPPPPP/1B1RK4/LNSG1GSNL b - 5',
    'rook-capture': 'sfen 9/9/k8/9/4Rr3/9/9/9/4K4 b - 1',
    'silver-promotion': 'sfen 4k4/9/4S4/9/9/9/9/9/4K4 b - 1',
    'bishop-promotion': 'sfen 4k4/9/4B4/9/9/9/9/9/4K4 b - 1',
}
engine_names = ['baseline', 'pvs', 'lmr']
repetitions = 3
rows = []
for name, position in fixtures.items():
    for depth in [4, 5]:
        results = {engine: [] for engine in engine_names}
        for rep in range(repetitions):
            for engine in engine_names[rep:] + engine_names[:rep]:
                with subprocess.Popen(
                    [root / engine, 'usi', '--eval', 'handcrafted'],
                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True
                ) as proc:
                    proc.stdin.write(f'isready\nposition {position}\n')
                    proc.stdin.flush()
                    assert proc.stdout.readline().strip() == 'readyok'
                    started = perf_counter()
                    proc.stdin.write(f'go depth {depth}\n')
                    proc.stdin.flush()
                    info = proc.stdout.readline().split()
                    best = proc.stdout.readline().split()
                    elapsed = 1000 * (perf_counter() - started)
                    proc.stdin.write('quit\n')
                    proc.stdin.flush()
                    assert info[:3] == ['info', 'depth', str(depth)], info
                    assert best[0] == 'bestmove', best
                    fields = dict(zip(info[1::2], info[2::2]))
                    assert fields['qsearchCapHits'] == '0' or int(fields['qnodes']) <= 1_000_000
                    row = {
                        'score': int(fields['lastCompletedIterationValue']),
                        'move': best[1],
                        **{key: int(fields.get(key, '0')) for key in [
                            'nodes', 'qnodes', 'qsearchDeltaPrunes', 'qsearchCapHits',
                            'pvsProbes', 'pvsResearches', 'lmrReductions', 'lmrResearches',
                        ]},
                    }
                    results[engine].append((row, elapsed))
        for engine, runs in results.items():
            row = runs[0][0]
            assert all(other == row for other, _ in runs), runs
            result = dict(fixture=name, depth=depth, engine=engine, **row,
                          elapsedMs=round(statistics.median(ms for _, ms in runs), 3))
            rows.append(result)
            print(json.dumps(result), flush=True)
        assert results['baseline'][0][0]['score'] == results['pvs'][0][0]['score'], results
(root / 'fixed-depth.json').write_text(json.dumps(rows, indent=2) + '\n')

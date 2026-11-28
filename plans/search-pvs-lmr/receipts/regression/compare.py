"""Compare feat/search-pvs-lmr builds with origin/main and with the builds used for measurement."""
import ast
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

out = Path(sys.argv[1])
repo = Path('/home/na2hiro/proj/haitaka-variants')
spec = importlib.util.spec_from_file_location('match', repo/'scripts/handcrafted_json_match.py')
match = importlib.util.module_from_spec(spec)
spec.loader.exec_module(match)
report = {}
ordinary = sys.argv[2:]


def usi(binary, position, go):
    lines = subprocess.check_output([binary, 'usi', '--eval', 'handcrafted'], text=True, timeout=120,
                                    input=f'isready\nposition {position}\n{go}\nquit\n').splitlines()
    assert len(lines) == 3 and lines[0] == 'readyok' and lines[2].startswith('bestmove '), lines
    tokens = lines[1].split()
    assert tokens[0] == 'info' and len(tokens) % 2 == 1, lines
    fields = dict(zip(tokens[1::2], tokens[2::2]))
    for key in ('time', 'nps', 'qnps'):
        fields.pop(key, None)
    return lines[2], fields


# Standard shogi: the default build must match main exactly; the experiment builds are
# compared with the binaries that produced the recorded results.
tree = ast.parse((repo/'target/search-pvs-lmr/fixed-depth.py').read_text())
fixtures = next(ast.literal_eval(row.value) for row in tree.body
                if isinstance(row, ast.Assign) and getattr(row.targets[0], 'id', '') == 'fixtures')
commands = [(name, position, f'go depth {d}') for name, position in fixtures.items() for d in (4, 5)]
repetition = 'sfen 4k4/9/9/9/9/9/9/9/4K4 b - 1 moves ' + ' '.join(['5i4i', '5a4a', '4i5i', '4a5a'] * 2 + ['5i4i', '5a4a', '4i5i'])
commands += [(name, position, f'go nodes {budget}')
             for name, position in [('startpos', 'startpos'), ('repetition', repetition)]
             for budget in (1, 128, 30000)]
standard = {}
for label, before, after, required in (
        ('default-vs-main', out/'main-standard', out/'standard', True),
        ('pvs-vs-measured', repo/'target/search-pvs-lmr/pvs', out/'standard-pvs', False),
        ('lmr-vs-measured', repo/'target/search-pvs-lmr/lmr', out/'standard-lmr', False)):
    same, diffs = 0, []
    for name, position, go in commands:
        a, b = usi(before, position, go), usi(after, position, go)
        if a == b:
            same += 1
        else:
            diffs.append(dict(fixture=name, go=go, before=a, after=b))
    assert not (required and diffs), (label, diffs[:3])
    standard[label] = dict(checks=len(commands), identical=same, differences=diffs)
    print('standard', label, same, '/', len(commands))
report['standard'] = standard


# Chu shogi JSON v2 with the handcrafted evaluation: the default build must match main.
fields = {'schemaVersion', 'bestMove', 'completedDepth', 'timedOut', 'nodeLimitReached', 'elapsedMs',
          'states', 'nps', 'score', 'outcome'}
positions = [json.loads(line) for line in (repo/'target/chushogi-reference-input.jsonl').read_text().splitlines()]


def json_search(binary, position, *args):
    return json.loads(subprocess.check_output([out/binary, 'json', 'search', *map(str, args)],
                                              input=json.dumps(position), text=True, timeout=120))


checks = 0
for i, position in enumerate(positions):
    for nodes in (1, 128, 1000):
        a, b = [json_search(binary, position, '--depth', 8, '--timeout-ms', 30000, '--nodes', nodes)
                for binary in ('main-chushogi', 'chushogi')]
        for row in (a, b):
            assert set(row) == fields and not row['timedOut'], (i, nodes, row)
        assert {k: a[k] for k in fields - {'elapsedMs', 'nps'}} == {k: b[k] for k in fields - {'elapsedMs', 'nps'}}, (i, nodes, a, b)
        checks += 1
report['chuHandcraftedDefaultVsMain'] = dict(positions=len(positions), checks=checks, nodeBudgets=[1, 128, 1000],
                                             depth=8, fields=sorted(fields - {'elapsedMs', 'nps'}))
print('chu handcrafted default-vs-main', checks)


def cli(binary, *args, stdin=''):
    return json.loads(subprocess.check_output([out/binary, 'json', *map(str, args)], input=stdin, text=True, timeout=120))


# Ordinary rules (JSON v3) with the handcrafted evaluation: the default build must match main
# on positions from a fixed walk through the legal moves.
report['ordinaryDefaultVsMain'] = {}
for rule in ordinary:
    position = json.dumps(cli(f'main-{rule}', 'startpos'))
    walk = [position]
    for ply in range(40):
        moves = cli(f'main-{rule}', 'moves', stdin=position)
        if not moves or cli(f'main-{rule}', 'inspect', stdin=position)['outcome']['kind'] != 'ongoing':
            break
        position = json.dumps(cli(f'main-{rule}', 'apply', '--move', json.dumps(moves[(ply * 7) % len(moves)]),
                                  stdin=position))
        if ply % 4 == 3:
            walk.append(position)
    checks = 0
    for position in walk:
        for nodes in (1, 128, 1000):
            a, b = [cli(binary, 'search', '--depth', 8, '--timeout-ms', 30000, '--nodes', nodes, stdin=position)
                    for binary in (f'main-{rule}', rule)]
            assert not a['timedOut'] and set(a) == set(b), (rule, nodes, a, b)
            keys = set(a) - {'elapsedMs', 'nps'}
            assert {k: a[k] for k in keys} == {k: b[k] for k in keys}, (rule, nodes, a, b)
            checks += 1
    report['ordinaryDefaultVsMain'][rule] = dict(positions=len(walk), checks=checks, nodeBudgets=[1, 128, 1000], depth=8)
    print('ordinary', rule, 'default-vs-main', checks)

# Chu shogi with Minase NNUE: each build must reproduce the measured fixed-depth results.
trial = repo/'target/chu-search-trials/minase-main-8630a75-mba'
bench = json.loads((trial/'bench.json').read_text())
setup = json.loads((out/'models/setup.json').read_text())
openings = setup['openings']
fixed = [openings[0], *openings[1:4]]
fixed += [row['request'] for row in json.loads((repo/'target/chu-search-trials/1000ms/openings.json').read_text())[::3]]
models = {'v3': str(out/'models/reference.nnue'),
          'v2.6': '/tmp/minase-v260/crates/minase/nets/nnue.bin'}
binaries = {'baseline': 'chushogi', 'pvs': 'chushogi-pvs', 'lmr': 'chushogi-lmr'}
nnue = 0
for record in bench:
    position = fixed[record['position']]
    for build, binary in binaries.items():
        result = json.loads(subprocess.check_output(
            [out/binary, 'json', 'search', '--nnue', models[record['model']], '--depth', str(record['depth']),
             '--timeout-ms', '30000'], input=json.dumps(position), text=True, timeout=60))
        expected = record['builds'][build]
        for key in ('score', 'bestMove', 'states', 'completedDepth', 'outcome'):
            assert result[key] == expected[key], (record['position'], record['depth'], record['model'], build, key)
        nnue += 1
report['chuNnueVsMeasured'] = dict(checks=nnue, positions=len(fixed), depths=[3, 4], models=list(models),
                                   fields=['score', 'bestMove', 'states', 'completedDepth', 'outcome'])
print('chu nnue vs measured', nnue)
match.atomic_json(out/'regression.json', report)

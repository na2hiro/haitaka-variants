#!/usr/bin/env python3
"""Audit the frozen real-browser match experiment and reproduce paired CIs."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import random
import re
import statistics
import zlib

ROOT = Path(__file__).resolve().parents[1]
META = ROOT / 'plans/tuyouraou11'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def percentile(values, quantile):
    values = sorted(values)
    index = (len(values) - 1) * quantile
    lo = int(index)
    return values[lo] + (values[min(lo + 1, len(values) - 1)] - values[lo]) * (index - lo)


def interval(strata, shared=False):
    rng = random.Random(1)
    samples = []
    for _ in range(10000):
        indices = rng.choices(range(len(strata[0])), k=len(strata[0])) if shared else None
        samples.append(statistics.mean(
            statistics.mean(values[i] for i in (indices if shared else rng.choices(range(len(values)), k=len(values))))
            for values in strata))
    return [percentile(samples, .025), percentile(samples, .975)]


def audit_browser(directory, browser, protocol, openings, smoke, part=None):
    directory = directory / browser
    meta = json.loads((directory / 'metadata.json').read_text())
    assert not meta['errors']
    if part == 'retained':
        assert meta.get('result') is None and meta.get('endedAt') is None
        for name, digest in protocol['resume']['interruptedArtifacts'][browser].items():
            assert sha(directory / name) == digest, name
    else:
        assert meta['result'] == 'PASS'
    assert meta['smoke'] == smoke and meta['browser'] == browser
    assert meta['version'] == protocol['browserVersions'][browser]
    for key in ['models', 'wasmSha256', 'glueSha256', 'runtimeSha256', 'capabilitySha256', 'bridgeSha256', 'cliSha256', 'openingsSha256']:
        expected = protocol['resume']['bridgeSha256'] if part == 'continuation' and key == 'bridgeSha256' else protocol[key]
        assert meta[key] == expected, key
    assert meta['affinity'].endswith(','.join(map(str, protocol['affinity'][browser])))
    assert len(meta['workers']) == 4
    workers = {w['slot']: w for w in meta['workers']}
    assert len(workers) == 4
    assert {p: sum(w['profile'] == p for w in workers.values()) for p in ['mini', 'micro']} == {'mini': 2, 'micro': 2}
    for worker in meta['workers']:
        model = protocol['models'][worker['profile']]
        assert worker['simd'] and worker['prepared'] == {'ruleId': 0, 'evaluator': 'sfnn', 'modelId': model['id'], 'arch': model['arch']}
    times = [100] if smoke else [1000] if part == 'continuation' else protocol['movetimeMs']
    count = 100
    max_plies = 4 if smoke else 256
    assert [m['ms'] for m in meta['matches']] == ([200] if part == 'retained' else times)
    assert all(m['exitCode'] == 0 and m['signal'] is None for m in meta['matches'])
    rows, strata, summaries = {}, [], []
    for ms in times:
        count = 32 if part == 'retained' and ms == 1000 else 68 if part == 'continuation' else 100
        path = directory / f'match-{ms}/self-play-games.jsonl'
        games = [json.loads(line) for line in path.read_text().splitlines()]
        if part == 'retained' and ms == 1000:
            assert {r['gameIndex'] for r in games} == set(range(1, count + 1)) | set(protocol['resume']['excludedGameIds'][browser])
            games = [r for r in games if r['gameIndex'] <= count]
            report = None
        else:
            report = json.loads((path.parent / 'self-play-report.json').read_text())
        assert len(games) == count and {r['gameIndex'] for r in games} == set(range(1, count + 1))
        if report is not None:
            command = report['command']
            assert command['games'] == count and command['threads'] == 2
            assert command['movetimeMs'] == ms and command['aDepth'] == command['bDepth'] == 64
            assert command['maxPlies'] == max_plies and command['seed'] == 1
            assert command['openingOrder'] == 'sequential' and command['openingRandomPlies'] == 0
            assert [e['args'][-1] for e in report['engines']] == ['mini', 'micro']
        pairs = {}
        for row in games:
            ident = row['gameIndex']
            assert row['pairIndex'] == (ident - 1) // 2
            assert row['aColor'] == ('black' if ident % 2 else 'white')
            assert row['bColor'] != row['aColor']
            assert row['startSfen'] == openings[row['pairIndex']]
            assert row['opening']['suiteIndex'] == row['pairIndex'] and row['opening']['randomPlies'] == 0
            assert len(row['moves']) == row['plies'] <= max_plies
            assert row['failureState'] is None
            assert row['terminationReason'] in ('no-legal-move', 'king-captured', 'repetition-draw', 'perpetual-check-loss', 'maximum-ply-capped')
            assert row['winner'] in ('A', 'B', None)
            if row['terminationReason'] == 'maximum-ply-capped':
                assert row['winner'] is None and row['plies'] == max_plies
            pairs.setdefault(row['pairIndex'], []).append(1 if row['winner'] == 'A' else 0 if row['winner'] == 'B' else .5)
            rows[(ms, ident)] = row
        assert len(pairs) == count // 2 and all(len(v) == 2 for v in pairs.values())
        values = [statistics.mean(pairs[i]) for i in sorted(pairs)]
        strata.append(values)
        summaries.append({'browser': browser, 'movetimeMs': ms, 'games': count,
                          'miniWins': sum(r['winner'] == 'A' for r in games),
                          'microWins': sum(r['winner'] == 'B' for r in games),
                          'draws': sum(r['winner'] is None for r in games),
                          'cappedDraws': sum(r['terminationReason'] == 'maximum-ply-capped' for r in games),
                          'scoreRate': statistics.mean(values), 'ci95': interval([values]),
                          'plies': sum(r['plies'] for r in games), 'gamesSha256': sha(path)})
    # Match every played move to the actual browser Worker USI transcript.
    transcript = directory / 'usi-transcript.jsonl.gz'
    if part != 'retained':
        assert sha(transcript) == meta['transcriptSha256']
    leases, seen = {}, set()
    stats = {p: {'searches': 0, 'workerOverrunMaxMs': -1e9, 'brokerOverrunMaxMs': -1e9, 'memoryMaxBytes': 0} for p in ['mini', 'micro']}
    if part == 'retained':
        inflater = zlib.decompressobj(31)
        recovered = inflater.decompress(transcript.read_bytes())
        assert not inflater.eof
        lines = recovered.split(b'\n')[:-1]
    else:
        with gzip.open(transcript, 'rt') as stream:
            lines = list(stream)
    for line in lines:
        event = json.loads(line)
        assert event['event'] != 'error', event
        if event['event'] == 'lease':
            assert event['lease'] not in leases
            assert workers[event['slot']]['profile'] == event['profile']
            leases[event['lease']] = {'profile': event['profile'], 'slot': event['slot'], 'position': None, 'game': None, 'goCount': 0}
        elif event['event'] == 'command':
            lease = leases[event['lease']]
            assert event['profile'] == lease['profile'] and event['slot'] == lease['slot']
            command = event['command']
            if command.startswith('position sfen '):
                tokens = command.split()
                start = ' '.join(tokens[2:6])
                assert len(tokens) == 6 or tokens[6] == 'moves'
                moves = tokens[7:]
                assert start in openings
                lease['position'] = (start, moves)
            elif command.startswith('go '):
                match = re.fullmatch(r'go movetime (\d+) depth 64', command)
                assert match, command
                ms = int(match[1])
                start, moves = lease['position']
                black_turn = (start.split()[1] == 'b') != bool(len(moves) % 2)
                mini_black = black_turn if lease['profile'] == 'mini' else not black_turn
                ident = 2 * openings.index(start) + (1 if mini_black else 2)
                key = (ms, ident)
                assert lease['game'] in (None, key)
                lease['game'] = key
                if part == 'retained' and key not in rows:
                    continue
                row = rows[key]
                ply = len(moves)
                assert moves == row['moves'][:ply] and ply < row['plies']
                best = [l.split()[1] for l in event['lines'] if l.startswith('bestmove ')]
                assert best == [row['moves'][ply]], (key, ply, best)
                assert (ms, ident, ply) not in seen
                seen.add((ms, ident, ply))
                lease['goCount'] += 1
                stat = stats[lease['profile']]
                stat['searches'] += 1
                stat['workerOverrunMaxMs'] = max(stat['workerOverrunMaxMs'], event['elapsedMs'] - ms)
                stat['brokerOverrunMaxMs'] = max(stat['brokerOverrunMaxMs'], event['roundtripMs'] - ms)
                stat['memoryMaxBytes'] = max(stat['memoryMaxBytes'], event['memoryBytes'])
    assert len(seen) == sum(row['plies'] for row in rows.values())
    if part != 'retained':
        assert len(leases) == len(rows) * 2 and all(l['game'] is not None for l in leases.values())
    else:
        for key in rows:
            used = [l for l in leases.values() if l['game'] == key]
            assert len(used) == 2 and {l['profile'] for l in used} == {'mini', 'micro'}
    return strata, summaries, {'browser': browser, 'segment': part, 'games': len(rows), 'browserRepliesMatched': len(seen),
                               'transcriptSha256': sha(transcript), 'profiles': stats,
                               'provenance': 'PASS', 'everyMoveMatchesBrowserTranscript': 'PASS',
                               'legalMovesAndTerminalRules': 'native CLI validates every move with try_play and history adjudication'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--records-dir', type=Path, default=ROOT / 'target/tuyouraou11/p6-strength')
    parser.add_argument('--smoke', action='store_true')
    parser.add_argument('--continuation-dir', type=Path)
    args = parser.parse_args()
    protocol = json.loads((META / 'p6-strength-protocol.json').read_text())
    openings_path = META / 'p4-openings.sfen'
    assert sha(openings_path) == protocol['openingsSha256']
    openings = openings_path.read_text().splitlines()
    assert len(openings) == len(set(openings)) == 50
    strata, summaries, audits, browsers = [], [], [], []
    for browser in protocol['browsers']:
        if args.continuation_dir:
            first_values, first_summary, first_audit = audit_browser(args.records_dir, browser, protocol, openings, False, 'retained')
            rest_values, rest_summary, rest_audit = audit_browser(args.continuation_dir, browser, protocol, openings[16:], False, 'continuation')
            values = [first_values[0], first_values[1] + rest_values[0]]
            assert [len(v) for v in values] == [50, 50]
            summary = [first_summary[0]]
            a, b = first_summary[1], rest_summary[0]
            combined = {'browser': browser, 'movetimeMs': 1000, 'games': 100, 'scoreRate': statistics.mean(values[1]), 'ci95': interval([values[1]]), 'sourceGamesSha256': [a['gamesSha256'], b['gamesSha256']]}
            for key in ['miniWins', 'microWins', 'draws', 'cappedDraws', 'plies']:
                combined[key] = a[key] + b[key]
            summary.append(combined)
            audits.extend([first_audit, rest_audit])
        else:
            values, summary, audit = audit_browser(args.records_dir, browser, protocol, openings, args.smoke)
            audits.append(audit)
        strata.extend(values)
        summaries.extend(summary)
        browsers.append({'browser': browser, 'scoreRate': statistics.mean(map(statistics.mean, values)), 'ci95': interval(values), 'openingBlockCi95': interval(values, shared=True)})
    ci = interval(strata)
    block_ci = interval(strata, shared=True)
    result = {'schemaVersion': 1, 'experiment': 'smoke' if args.smoke else 'full',
              'assignment': protocol['assignment'], 'games': sum(s['games'] for s in summaries),
              'strata': summaries, 'browsers': browsers, 'scoreRate': statistics.mean(map(statistics.mean, strata)),
              'ci95': ci, 'openingBlockCi95': block_ci, 'bootstrapSamples': 10000, 'bootstrapSeed': 1,
              'miniStrongerInTestedConditions': ci[0] > .5 and block_ci[0] > .5,
              'p4RegressionCriterion': ci[1] < .5, 'audits': audits, 'result': 'PASS'}
    target = args.records_dir / 'strength-summary.json'
    target.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()

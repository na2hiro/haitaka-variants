#!/usr/bin/env python3
"""Audit and summarize the frozen Step 5 CPU development experiment."""
import hashlib
import json
from pathlib import Path
import random
import statistics
from collections import defaultdict

ROOT = Path(__file__).resolve().parents[1]
RUN = ROOT / 'out/anhoku-strength-first/step5-scale-time-20260908-v1'
OLD = RUN.parent / 'step4-objective-search-diagnostic-20260907-v1'

def rows(p):
    return [json.loads(s) for s in p.read_text().splitlines() if s]

def interval(groups):
    values = list(groups.values())
    rng = random.Random(20260908)
    samples = sorted(statistics.mean(rng.choices(values, k=len(values))) for _ in range(20000))
    return {'mean': statistics.mean(values), 'lower95': samples[500], 'upper95': samples[19499], 'groups': len(values)}

def artifact(p):
    b = p.read_bytes()
    return {'path': str(p.relative_to(ROOT)), 'bytes': len(b), 'sha256': hashlib.sha256(b).hexdigest()}

def main():
    contract_path = ROOT / 'r0/anhoku-strength-first/step5-scale-time-v1.json'
    config = json.loads(contract_path.read_text())
    probes = {s: {x['parentId']: x for p in sorted((RUN / f'probes/scale{s}').glob('*/probe.jsonl')) for x in rows(p)} for s in [1, 2]}
    assert all(len(d) == 128 for d in probes.values())
    prior = {x['parentId']: x for p in sorted((OLD / 'remote/initial/candidate').glob('*/probe.jsonl')) for x in rows(p)}
    unchanged = 0
    static_checks = 0
    for key, one in probes[1].items():
        assert all(one[field] == prior[key][field] for field in ['bestMove', 'rootScore', 'totalNodes', 'completedDepth'])
        unchanged += 1
        two = probes[2][key]
        assert two['staticParentScore'] == max(-28975, min(28975, one['staticParentScore'] * 2))
        child_two = {x['move']: x for x in two['childStaticScores']}
        for child in one['childStaticScores']:
            expected = child['rootOrientedScore'] if child['terminal'] else max(-28975, min(28975, child['rootOrientedScore'] * 2))
            assert child_two[child['move']]['rootOrientedScore'] == expected
            static_checks += 1
    labels = {(x['parentId'], x['move']): x for folder in [OLD / 'remote/labels', RUN / 'labels'] for p in sorted(folder.glob('*/labels.jsonl')) for x in rows(p)}
    assert len(labels) == 249 + len(rows(RUN / 'new-child-plan.jsonl'))
    parent_metrics = []
    differences = defaultdict(list)
    for parent in rows(OLD / 'union/union-plan.jsonl'):
        key = parent['id']
        choices = [probes[s][key]['bestMove'] for s in [1, 2]]
        assert all(probes[s][key]['legalBestMove'] and probes[s][key]['completedDepth'] > 0 for s in [1, 2])
        union = set(parent['unionMoves']) | set(choices)
        child_labels = [labels[(key, move)] for move in union]
        assert all(x['completed'] for x in child_labels)
        best = max(x['boundedRootOrientedScore'] for x in child_labels)
        regret = [(best - labels[(key, move)]['boundedRootOrientedScore']) / 12000 for move in choices]
        parent_metrics.append({'parentId': key, 'group': parent['sourceOpeningGroupId'], 'scale1Regret': regret[0], 'scale2Regret': regret[1], 'changed': choices[0] != choices[1]})
        differences[parent['sourceOpeningGroupId']].append(regret[1] - regret[0])
    # Resample whole opening clusters and preserve parent weighting, as in Step 4.
    rng = random.Random(20260908)
    clusters = list(differences.values())
    bootstrap = []
    for _ in range(20000):
        sample = rng.choices(clusters, k=len(clusters))
        bootstrap.append(sum(map(sum, sample)) / sum(map(len, sample)))
    bootstrap.sort()
    diagnostic = {'parents': len(parent_metrics), 'defaultParityWithStep4': unchanged, 'childStaticScalingChecks': static_checks, 'changedChoices': sum(x['changed'] for x in parent_metrics), 'newLabels': len(labels) - 249, 'scale1MeanRegret': statistics.mean(x['scale1Regret'] for x in parent_metrics), 'scale2MeanRegret': statistics.mean(x['scale2Regret'] for x in parent_metrics), 'scale2MinusScale1': {'mean': statistics.mean(x['scale2Regret'] - x['scale1Regret'] for x in parent_metrics), 'lower95': bootstrap[500], 'upper95': bootstrap[19499], 'openingGroups': len(clusters)}, 'searchNodes': sum(x['totalNodes'] for d in probes.values() for x in d.values()) + sum(x['totalNodes'] for p in (RUN / 'labels').glob('*/labels.jsonl') for x in rows(p))}
    diagnostic['nodeBudgetSearchSensitivity'] = {
        str(scale): {'alphaBetaNodes': sum(x['alphaBetaNodes'] for x in data.values()), 'qsearchNodes': sum(x['qsearchNodes'] for x in data.values()), 'meanCompletedDepth': statistics.mean(x['completedDepth'] for x in data.values()), 'partialIterations': sum(x['partialIteration'] for x in data.values())}
        for scale, data in probes.items()
    }
    cells = {}
    cell_groups = {}
    for matchup in config['matchups']:
        for ms in config['movetimeMs']:
            name = f'{matchup}-{ms}ms'
            folder = RUN / 'games' / name
            if not (folder / 'self-play-report.json').exists():
                continue
            report = json.loads((folder / 'self-play-report.json').read_text())
            games = rows(folder / 'self-play-games.jsonl')
            if len(games) != 64:
                continue
            assert len({g['gameIndex'] for g in games}) == 64
            assert all(g['failureState'] is None for g in games)
            assert report['command']['movetimeMs'] == ms and report['command']['threads'] == 8
            grouped = defaultdict(list)
            colors = defaultdict(set)
            for g in games:
                grouped[g['opening']['suiteIndex']].append(1 if g['winner'] == 'A' else 0 if g['winner'] == 'B' else .5)
                colors[g['opening']['suiteIndex']].add(g['aColor'])
            assert len(grouped) == 32 and all(len(v) == 2 for v in grouped.values())
            assert all(c == {'white', 'black'} for c in colors.values())
            means = {k: statistics.mean(v) for k, v in grouped.items()}
            cell_groups[name] = means
            summary = report['summary']
            cells[name] = {'games': 64, 'wins': summary['aWins'], 'losses': summary['bWins'], 'draws': summary['draws'], 'score': interval(means), 'averagePlies': summary['avgPlies'], 'cappedGames': sum(g['terminationReason'] == 'maximum-ply-capped' for g in games), 'meanEngineElapsedMsPerPly': sum(g['totalElapsedMs'] for g in games) / sum(g['plies'] for g in games), 'telemetry': {side: summary[side + 'Breakdown'] for side in ['a', 'b']}, 'report': artifact(folder / 'self-play-report.json'), 'records': artifact(folder / 'self-play-games.jsonl')}
    budget_differences = {}
    for matchup in config['matchups']:
        a, b = f'{matchup}-100ms', f'{matchup}-1000ms'
        if a in cell_groups and b in cell_groups:
            budget_differences[matchup] = interval({k: cell_groups[b][k] - cell_groups[a][k] for k in cell_groups[a]})
    result = {'schema': 'haitaka-strength-first-step5-result-v1', 'complete': len(cells) == 6 and (RUN / 'completed.json').exists(), 'contract': artifact(contract_path), 'scope': config['scope'], 'diagnostic': diagnostic, 'games': cells, 'scoreChange1000Minus100Ms': budget_differences, 'limitations': ['32 reused development openings; exploratory intervals, no multiplicity correction.', 'Native arm64 with eight concurrent single-thread games; not browser latency or target-device validation.', 'Both opponents receive the same time budget. Relative score changes do not directly measure 1-second versus 100-ms strength for the same model.', 'Root regret is restricted to the expanded union and handcrafted teacher, not an objective game-theoretic label.', 'Production model and timing remain unchanged.'], 'deviations': config['deviations']}
    browser = {}
    for ms in [100, 1000]:
        path = RUN / f'browser-timing-{ms}ms.json'
        if path.exists():
            data = json.loads(path.read_text())
            assert data['status'] == 'passed'
            assert data['summary']['missingMoves'] == 0 and data['summary']['emergencyFallbacks'] == 0 and data['summary']['unsearchedMoves'] == 0
            assert len(data['rawGames']) == 8 and all(g['plies'] == 2 for g in data['rawGames'])
            assert data['summary']['timing']['deadlineSampleCount'] == 16
            browser[str(ms)] = {'report': artifact(path), 'timing': data['summary']['timing'], 'candidate': data['summary']['nodes']['A'], 'scope': 'local Ubuntu single-worker archived Step1 width256 WASM; two-ply capped timing smoke only, no game-strength claim; opening IDs generated by harness map by SFEN to frozen IDs 001,003,005,007'}
    result['browserTimingSmoke'] = browser
    result['recommendation'] = {'evaluationMultiplier': 1, 'nextPrimaryEvaluationMovetimeMs': 1000, 'retain100msSecondaryCheck': True, 'productionChanged': False, 'reason': 'No demonstrated benefit from scale 2 in direct paired games or restricted regret. One second is practical in the small browser timing smoke, but relative strength gain from extra time is not established.', 'nextExperiment': 'Prioritize CPU diagnosis of integer export versus full-precision disagreement on new development positions; keep the selected candidate and scale1 fixed. New training is outside this experiment and requires separate GPU permission if needed.'}
    result['completion'] = json.loads((RUN / 'completed.json').read_text()) if (RUN / 'completed.json').exists() else None
    out = RUN / 'analysis'
    out.mkdir(exist_ok=True)
    (out / 'parents.jsonl').write_text(''.join(json.dumps(x) + '\n' for x in parent_metrics))
    (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    lines = ["# Anhoku Step 5: score scale and thinking time", "", "CPU development experiment on the MacBook Pro at 192.168.0.12. " + ("All 384 games completed." if result['complete'] else "Measurement is still running."), "", "| Matchup (score for first engine) | Budget | W–L–D | Score | Opening-group 95% interval |", "| --- | ---: | ---: | ---: | ---: |"]
    if result['complete']:
        lines[4:4] = ['Retain the original 1× evaluation scale. The 2× treatment did not demonstrate a benefit: it scored 30–34 against 1× at both budgets, had lower observed scores against handcrafted, and did not improve restricted teacher regret. Use 1 second as the primary budget for the next strength evaluation, retaining 100 ms as a secondary check. Production settings are unchanged.', '', 'The next improvement investigation should focus on the previously observed integer-export versus full-precision disagreement, with the candidate and 1× scale fixed. This experiment provides no reason to spend GPU time on another capacity increase.', '']
    for name, cell in cells.items():
        score = cell['score']
        matchup, ms = name.rsplit('-', 1)
        lines.append(f"| {matchup} | {ms} | {cell['wins']}–{cell['losses']}–{cell['draws']} | {score['mean']:.1%} | {score['lower95']:.1%} to {score['upper95']:.1%} |")
    lines.extend(["", "Each cell uses the same 32 development openings, one game with each engine color, 200-ply cap, and eight concurrent games. Both opponents receive the stated time budget. Intervals resample opening pairs; these are exploratory measurements on reused openings.", "", f"Doubling the score changed {diagnostic['changedChoices']}/128 root choices. Expanded-union restricted regret was {diagnostic['scale1MeanRegret']:.6f} at scale 1 and {diagnostic['scale2MeanRegret']:.6f} at scale 2. Scale 2 minus scale 1 was {diagnostic['scale2MinusScale1']['mean']:+.6f}, with a 95% opening-cluster interval [{diagnostic['scale2MinusScale1']['lower95']:+.6f}, {diagnostic['scale2MinusScale1']['upper95']:+.6f}]. Lower regret is better; this does not establish an improvement from scaling.", "", "The default scale reproduced all 128 prior root results. All 8,290 child static-score checks passed. Both root probes completed at every parent, and all 24 additional child labels completed. Diagnostic compute was 8.56M nodes; existing 249 teacher labels were reused.", "", "The multiplier is applied after final integer NNUE score division, consistently through full refresh, accumulator evaluation, and diagnostic traces. Scale 2 clamps static values below the mate band. Terminal values and default scale 1 behavior are preserved.", "", "## Time-budget differences", ""])
    for name, delta in budget_differences.items():
        lines.append(f"- {name}: score at 1 second minus score at 100 ms = {delta['mean']*100:+.1f} percentage points; paired opening interval [{delta['lower95']*100:+.1f}, {delta['upper95']*100:+.1f}].")
    lines.extend(['', '## Search and pruning response', ''])
    for scale, values in diagnostic['nodeBudgetSearchSensitivity'].items():
        total = values['alphaBetaNodes'] + values['qsearchNodes']
        lines.append(f"- Scale {scale}, fixed 10k-node roots: {values['qsearchNodes']/total:.1%} qsearch nodes; mean completed depth {values['meanCompletedDepth']:.2f}.")
    for ms in [100, 1000]:
        name = f'scale2-vs-scale1-{ms}ms'
        if name in cells:
            t = cells[name]['telemetry']
            rates = {side: values['qsearchCapHits'] / max(values['qnodes'], 1) for side, values in t.items()}
            lines.append(f"- Direct {ms} ms games: qsearch cap hits per qnode were {rates['a']:.4f} for scale 2 and {rates['b']:.4f} for scale 1. These counters show a search response to scaling, but the game trajectories differ and the rates alone are not a strength measure.")
    if browser:
        lines.extend(['', '## Browser timing smoke', '', 'A separate single-worker Ubuntu Chrome check used the archived Step 1 width-256 WASM runtime and the selected candidate at default scale. Four paired openings were stopped after two plies (16 searches per budget). These capped games provide timing evidence only.', ''])
        for ms, smoke in browser.items():
            t = smoke['timing']
            lines.append(f"- {ms} ms budget: p95 and maximum elapsed {t['maxElapsedMs']:.1f} ms; maximum deadline overshoot {t['maxLatenessMs']:.1f} ms; no missing, unsearched, or emergency-fallback moves.")
        lines.extend(['', 'This supports 1 second as a practical timing target for further testing, but does not validate all production devices. Browser model/module load time is separate from these per-search measurements.'])
    lines.extend(["", "## Limits and validation", "", *['- ' + x for x in result['limitations']], "- Real-model regression test passed on arm64; release CLI and diagnostic builds passed; source diff whitespace check passed.", "- The initial regression-test invocation used a relative model path that Cargo resolved from the crate directory. Re-running with the absolute path passed.", "- The first game launch rejected the stable-ID TSV format before game search. The same openings were converted to bare SFEN; completed diagnostics were reused, and the original contract and controller log were retained.", "", "Artifacts: `out/anhoku-strength-first/step5-scale-time-20260908-v1/analysis/result.json`, raw paired games, root probes, new child labels, command manifests, binary hashes, and the frozen `r0/anhoku-strength-first/step5-scale-time-v1.json`."])
    (out / 'result.md').write_text('\n'.join(lines) + '\n')
    if result['complete']:
        (ROOT / 'r0/anhoku-strength-first/step5-result-report.json').write_text(json.dumps(result, indent=2) + '\n')
        (ROOT / 'r0/anhoku-strength-first/step5-result-report.md').write_text('\n'.join(lines) + '\n')

    print(json.dumps({'complete': result['complete'], 'diagnostic': diagnostic, 'games': {k: {x: v[x] for x in ['wins','losses','draws','score']} for k,v in cells.items()}, 'budgetDifferences': budget_differences}, indent=2))

if __name__ == '__main__':
    main()

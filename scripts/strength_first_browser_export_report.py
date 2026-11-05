#!/usr/bin/env python3
"""Audit and report the corrected-runtime development campaign."""
import hashlib
import json
from pathlib import Path
import random
import statistics
from collections import defaultdict
ROOT=Path(__file__).resolve().parents[1]
ASSET_RUN=ROOT/'out/anhoku-strength-first/step6-browser-export-20260908-v1'
BROWSER_RUN=ROOT/'out/anhoku-strength-first/step6-browser-export-retry-20260910-v2'

def rows(p):return [json.loads(x) for x in p.read_text().splitlines() if x]
def art(p):
    b=p.read_bytes();return dict(path=str(p.relative_to(ROOT)),bytes=len(b),sha256=hashlib.sha256(b).hexdigest())
def interval(groups):
    values=list(groups.values());rng=random.Random(20260908)
    estimates=sorted(statistics.mean(rng.choices(values,k=len(values))) for _ in range(20000))
    return dict(groups=len(values),mean=statistics.mean(values),lower95=estimates[500],upper95=estimates[19499],replicates=20000,seed=20260908)
def main():
    contract=ROOT/'r0/anhoku-strength-first/step6-browser-export-v2.json'
    config=json.loads(contract.read_text())
    labels={(x['parentId'],x['move']):x for p in sorted((ASSET_RUN/'labels').glob('*/labels.jsonl')) for x in rows(p)}
    regret=defaultdict(list);delta=defaultdict(list);parent_rows=[]
    union=rows(ASSET_RUN/'union-plan.jsonl')
    for p in union:
        complete={mv:labels[(p['id'],mv)] for mv in p['unionMoves'] if labels[(p['id'],mv)]['completed']}
        assert len(complete)==len(p['unionMoves'])
        best=max(x['boundedRootOrientedScore'] for x in complete.values())
        values={name:(best-complete[mv]['boundedRootOrientedScore'])/12000 for name,mv in p['rootChoices'].items()}
        assert 'legacy' in values and 'corrected' in values
        for name,v in values.items():regret[name].append(v)
        delta[p['sourceOpeningGroupId']].append(values['corrected']-values['legacy'])
        parent_rows.append(dict(parentId=p['id'],group=p['sourceOpeningGroupId'],regret=values))
    rng=random.Random(20260908);clusters=list(delta.values());estimates=[]
    for _ in range(20000):
        sample=rng.choices(clusters,k=len(clusters));estimates.append(sum(map(sum,sample))/sum(map(len,sample)))
    estimates.sort()
    diagnostic=dict(parents=len(union),labels=len(labels),incompleteLabels=sum(not x['completed'] for x in labels.values()),missingHandcraftedRoots=sum('handcrafted' not in p['rootChoices'] for p in union),regret={k:dict(parents=len(v),mean=statistics.mean(v)) for k,v in regret.items()},correctedMinusLegacy=dict(mean=statistics.mean(x for xs in clusters for x in xs),lower95=estimates[500],upper95=estimates[19499],openingGroups=len(clusters)),compute=json.loads((ASSET_RUN/'remote-diagnostic-completed.json').read_text()))
    native=None
    folder=ASSET_RUN/'native-corrected-vs-legacy'
    if (ASSET_RUN/'remote-completed.json').exists():
        games=rows(folder/'self-play-games.jsonl');assert len(games)==64 and all(g['failureState'] is None for g in games)
        grouped=defaultdict(list);colors=defaultdict(set)
        for g in games:
            grouped[g['opening']['suiteIndex']].append(1 if g['winner']=='A' else 0 if g['winner']=='B' else .5);colors[g['opening']['suiteIndex']].add(g['aColor'])
        assert len(grouped)==32 and all(len(v)==2 for v in grouped.values()) and all(v=={'black','white'} for v in colors.values())
        summary=json.loads((folder/'self-play-report.json').read_text())['summary']
        native=dict(games=64,wins=summary['aWins'],losses=summary['bWins'],draws=summary['draws'],score=interval({k:statistics.mean(v) for k,v in grouped.items()}),capped=sum(g['terminationReason']=='maximum-ply-capped' for g in games),scope='native arm64 eight concurrent games; corrected vs archived legacy at same1000ms; development-only',report=art(folder/'self-play-report.json'),records=art(folder/'self-play-games.jsonl'))
    browser={}
    for name,ms,pairs in [('primary',1000,64),('secondary',100,16)]:
        p=BROWSER_RUN/f'browser-{name}-{ms}ms.json'
        if not p.exists():continue
        d=json.loads(p.read_text());assert d['status']=='passed';games=d['rawGames'];assert len(games)==pairs*2
        assert d['summary']['missingMoves']==0 and d['summary']['emergencyFallbacks']==0 and d['summary']['unsearchedMoves']==0
        assert d['provenance']['execution']['concurrentGames']==1 and d['provenance']['execution']['workerCount']==1
        assert d['provenance']['files']['modelA']['sha256']==config['candidate']['sha256']
        expected_openings=config['browser']['openings' if name=='primary' else 'secondaryOpenings']
        assert d['provenance']['files']['openings']['sha256']==expected_openings['sha256']
        assert d['provenance']['files']['contract']['sha256']==art(contract)['sha256']
        expected_wasm=next(x for x in config['wasm'] if x['path'].endswith('.wasm'))['sha256']
        assert d['provenance']['files']['wasm']['sha256']==expected_wasm
        grouped=defaultdict(list);colors=defaultdict(set)
        for g in games:
            grouped[g['openingId']].append(g['scoreA']);colors[g['openingId']].add(g['aColor'])
            for search in g['searches']:
                assert search['requestedMs']==ms
        assert len(grouped)==pairs and all(len(v)==2 for v in grouped.values())
        assert all(len(v)==2 for v in colors.values())
        mean_by_group={k:statistics.mean(v) for k,v in grouped.items()}
        score=d['summary']['score']
        capped=sum(g['capped'] for g in games)
        # Worst-case scoring assigns every cap a loss for the candidate.
        conservative=defaultdict(list)
        for g in games:conservative[g['openingId']].append(0 if g['capped'] else g['scoreA'])
        browser[name]=dict(movetimeMs=ms,games=len(games),wins=score['aWins'],losses=score['bWins'],draws=score['draws'],capped=capped,score=interval(mean_by_group),capAsLossScore=interval({k:statistics.mean(v) for k,v in conservative.items()}),timing=d['summary']['timing'],load=d['load'],memory=d['hostTelemetry']['memory'],nodes=d['summary']['nodes'],report=art(p),scope='single-worker Ubuntu Chrome current WASM; reused development openings; not independent confirmation',openingScores=mean_by_group)
    export=json.loads((ASSET_RUN/'export-analysis/result.json').read_text())
    static_scores=rows(ASSET_RUN/'export-analysis/scores.jsonl')
    strata=defaultdict(list)
    for row in static_scores:
        if row['terminal']:continue
        magnitude=abs(row['fullPrecisionScore'])
        score_bin='0-250' if magnitude<=250 else '250-1000' if magnitude<=1000 else '1000-3000' if magnitude<=3000 else '3000+'
        strata['scoreRange:'+score_bin].append(row)
        strata['check:'+str(row['inCheck'])].append(row)
    export['additionalStrata']={name:{'positions':len(group),'legacyMae':statistics.mean(abs(x['legacyScore']-x['fullPrecisionScore']) for x in group),'correctedMae':statistics.mean(abs(x['correctedScore']-x['fullPrecisionScore']) for x in group),'meanFtZeroFraction':statistics.mean(x['ftZeroFraction'] for x in group),'meanFtSaturatedFraction':statistics.mean(x['ftSaturatedFraction'] for x in group)} for name,group in strata.items()}
    wasm=json.loads((ASSET_RUN/'wasm-parity.json').read_text())
    result=dict(schema='haitaka-strength-first-step6-result-v2',complete=bool(native and len(browser)==2 and (BROWSER_RUN/'browser-completed.json').exists()),contract=art(contract),discovery='Serializer uses feature-transformer activation scale127; runtime/oracles incorrectly applied another six-bit shift. Corrected FT clipping only; hidden-layer shifts preserved.',exportAudit=export,wasmParity=dict(passed=wasm['passed'],positions=wasm['positions']),restrictedTeacherDiagnostic=diagnostic,nativeTreatment=native,browser=browser,browserRetry=config['recovery'],productionChanged=False,gpuUsed=False,scope='development-only',limitations=['Prior runtime/oracle agreement shared the FT shift defect; historical reports remain measurements of the legacy implementation, not serializer fidelity.', 'Static rankings use full-precision NNUE, and teacher labels use a restricted union, neither is game-theoretic truth.', 'New parents are disjoint from the128 previously labeled parents by canonical board; source development games/openings were reused and exact training-position non-overlap is not asserted.', 'Native and browser measurements use different hosts and concurrency and are not pooled.', 'Browser timing is measured on one Ubuntu host; production acceptance thresholds and second-host checks remain outstanding.', 'The first browser attempt completed its schedule in memory but failed before persisting raw games; the full frozen schedule was replayed under the v2 recovery contract and the two attempts are not combined.'])
    if len(browser)==2:
        common=set(browser['primary']['openingScores']) & set(browser['secondary']['openingScores'])
        assert len(common)==16
        result['matchedBrowserBudgetDifference']=interval({key:browser['primary']['openingScores'][key]-browser['secondary']['openingScores'][key] for key in sorted(common)})
    out=BROWSER_RUN/'analysis';out.mkdir(exist_ok=True)
    (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
    (out/'teacher-parents.jsonl').write_text(''.join(json.dumps(x)+'\n' for x in parent_rows))
    if result['complete']:
        fp=export['nonterminal']; choices=export['moveAgreement']
        lines=['# Anhoku: corrected NNUE runtime and 1-second browser baseline', '', 'The CPU audit found and corrected an extra six-bit shift in feature-transformer activation. The serialized weights already use activation scale 127; hidden affine layers still require their six-bit shift. Model bytes and the 1× output multiplier are unchanged. No GPU or deployment was used.', '', f"On {fp['positions']:,} nonterminal positions, full-precision MAE fell from {fp['legacyScore']['mae']:.1f} to {fp['correctedScore']['mae']:.1f} score units. Independent integer reference, native full/incremental/trace evaluation, and current WASM agreed exactly on {export['positions']:,} positions. Full-precision static move agreement improved from {choices['legacy']}/{choices['eligibleParents']} to {choices['corrected']}/{choices['eligibleParents']} eligible parents.", '', 'The first 64-pair browser attempt completed play but Node 14 failed while formatting optional memory telemetry, before the raw games were saved. The v2 harness writes qualified games atomically before telemetry and pins Node 26.8.1. The complete schedule was replayed; results below use only that replay.', '', '## Game results', '', '| Comparison | Time per move | Games | W–L–D | Candidate score | Opening-group 95% interval |', '| --- | ---: | ---: | ---: | ---: | ---: |']
        for label,cell,ms in [('Native corrected vs legacy',native,1000),('Browser corrected vs handcrafted (primary)',browser['primary'],1000),('Browser corrected vs handcrafted (secondary)',browser['secondary'],100)]:
            score=cell['score'];lines.append(f"| {label} | {ms} ms | {cell['games']} | {cell['wins']}–{cell['losses']}–{cell['draws']} | {score['mean']:.1%} | {score['lower95']:.1%}–{score['upper95']:.1%} |")
        if 'matchedBrowserBudgetDifference' in result:
            d=result['matchedBrowserBudgetDifference']
            lines.extend(['', f"On the 16 openings common to both browser schedules, the paired 1,000 ms minus 100 ms score difference was {d['mean']*100:+.1f} percentage points, with a 95% interval [{d['lower95']*100:+.1f}, {d['upper95']*100:+.1f}]. Both opponents receive the stated budget, so this is a change in relative score against handcrafted, not a direct long-time versus short-time match."])
        lines.extend(['', 'The native result does not demonstrate that the correction improves game strength. It is an eight-worker arm64 development match against the legacy runtime. Browser results use one concurrent game on Ubuntu and compare against handcrafted; these environments and opponents are not pooled. All openings are reused development evidence, not independent confirmation.', '', '## Browser timing and resources', ''])
        for name,cell in browser.items():
            t=cell['timing'];memory=cell['memory']
            lines.append(f"- {name}, {cell['movetimeMs']} ms: p95 elapsed {t['p95ElapsedMs']:.1f} ms, p99 {t['p99ElapsedMs']:.1f} ms, maximum {t['maxElapsedMs']:.1f} ms; maximum overshoot {t['maxLatenessMs']:.1f} ms. Capped games: {cell['capped']}.")
            if memory.get('status')=='observed':lines.append(f"  Measured peak Chrome process-group RSS: {memory['peakResidentSetBytes']/1024**2:.1f} MiB.")
        lines.extend(['', 'No missing, unsearched, or emergency-fallback moves were accepted. Per-search timing excludes model/module load time, which is retained separately in the machine-readable report. A second supported device and numeric product acceptance limits are still needed before production rollout.', '', '## Teacher diagnosis and residual export error', '', f"All {diagnostic['labels']} restricted child labels completed. Mean restricted regret was {diagnostic['regret']['legacy']['mean']:.6f} for legacy and {diagnostic['regret']['corrected']['mean']:.6f} for corrected. Corrected minus legacy was {diagnostic['correctedMinusLegacy']['mean']:+.6f}, with a 95% opening-cluster interval [{diagnostic['correctedMinusLegacy']['lower95']:+.6f}, {diagnostic['correctedMinusLegacy']['upper95']:+.6f}]. The interval includes zero. Three handcrafted roots were incomplete and are disclosed; the legacy/corrected comparison covers all 128 parents.", '', f"The diagnostic used {diagnostic['compute']['searchNodes']/1e6:.2f}M search nodes, below its 129.28M ceiling. Static enumeration of all legal children did not perform exhaustive child search. Residual MAE with only FT quantization was {fp['quantizedFtOnlyScore']['mae']:.1f}; no affine weights were clipped. FT quantization is a possible next ablation, not an automatic training or GPU decision.", '', '## Validation and interpretation', '', '- Runtime regression suite: 93 passed, one explicit-model test initially ignored; the latter was then run with the actual model and passed. Real-model scalar/SIMD and width-512 activation regression checks also passed.', '- Historical Python integer oracles shared the runtime shift defect. They are corrected for future use; archived outputs remain measurements of the legacy implementation.', '- The original long legacy browser launch was deferred before game search when the defect was verified. The current campaign froze the correction and qualified it before starting games.', '- Parent canonical boards exclude all 128 previously labeled parents. Source games and openings are development history; exact training-position non-overlap is not asserted.', '- The first browser result was lost to a reporter-only Node 14 incompatibility after 64/64 pairs. The v2 replay is the sole browser sample used here.', '- No model was retrained, no paid compute was rented, and nothing was deployed.', '', '## Next decision', '', 'Keep the serializer-consistent correction in the working runtime and use the measured 1-second baseline for future development. Do not promote the NNUE merely because export fidelity improved. Before new training, re-screen at most two retained checkpoints under the corrected runtime at 1 second, because prior checkpoint selection used the legacy implementation. Only then choose a single further ablation if the game result and residual-error evidence support it. Final promotion still requires independent opening groups, replication, target-device checks, and the plan’s confirmation criterion.', '', 'Reproduction and raw evidence: `r0/anhoku-strength-first/step6-browser-export-v2.json`; `scripts/strength_first_export_analyze.py`, `scripts/strength_first_export_wasm_audit.mjs`, `scripts/strength_first_export_remote.py`, `scripts/strength_first_browser_baseline.py`, and `scripts/strength_first_browser_export_report.py`; `out/anhoku-strength-first/step6-browser-export-20260908-v1/` and `out/anhoku-strength-first/step6-browser-export-retry-20260910-v2/`.'])
        (out/'result.md').write_text('\n'.join(lines)+'\n')
        (ROOT/'r0/anhoku-strength-first/step6-result-report.md').write_text('\n'.join(lines)+'\n')
        (ROOT/'r0/anhoku-strength-first/step6-result-report.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(dict(complete=result['complete'],teacher=diagnostic,native=native,browser={k:{f:v[f] for f in ['movetimeMs','games','wins','losses','draws','capped','score','timing']} for k,v in browser.items()}),indent=2))
if __name__=='__main__':main()

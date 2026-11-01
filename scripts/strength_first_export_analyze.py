#!/usr/bin/env python3
"""CPU-only export discrepancy decomposition with exact native-trace checks."""
import hashlib
import json
from pathlib import Path
import statistics
from collections import defaultdict
import numpy as np
import strength_first_export_reference as reference

ROOT = Path(__file__).resolve().parents[1]
RUN = ROOT / 'out/anhoku-strength-first/step6-browser-export-20260908-v1'

def main():
    config = json.loads((ROOT / 'r0/anhoku-strength-first/step6-browser-export-v1.json').read_text())
    for key in ['candidate', 'fullPrecisionWeights', 'parents']:
        a = config[key]
        assert hashlib.sha256((ROOT / a['path']).read_bytes()).hexdigest() == a['sha256']
    with np.load(RUN / 'full-precision-weights.npz') as archive:
        weights = {key: archive[key] for key in archive.files}
    _, integer = reference.parity.parse_integer_network(ROOT / config['candidate']['path'])
    bft, wft, psqt_w, buckets = integer
    rows = [json.loads(x) for x in (RUN / 'static-audit/positions.jsonl').read_text().splitlines()]
    outputs = []
    clipping = []
    for bucket in range(8):
        for layer in range(3):
            w = weights[f'b{bucket}_l{layer}_weight']
            scale = 64 if layer < 2 else 9600/127
            maximum = 127/scale
            clipping.append(dict(bucket=bucket,layer=layer,weights=w.size,clipped=int(np.count_nonzero(np.abs(w)>maximum)),maximumExcess=float(np.maximum(np.abs(w)-maximum,0).max())))
    def dense(x, bucket, quantized=False):
        for layer in range(3):
            if quantized:
                bias, weight = buckets[bucket][layer]
                bs = 8128 if layer < 2 else 9600
                ws = 64 if layer < 2 else 9600/127
                w = weight[:,:len(x)].astype(np.float64)/ws
                b = bias.astype(np.float64)/bs
            else:
                w = weights[f'b{bucket}_l{layer}_weight']
                b = weights[f'b{bucket}_l{layer}_bias']
            x = w @ x + b
            if layer < 2: x=np.clip(x,0,1)
        return float(x[0])
    for row in rows:
        corrected = reference.integer_trace(integer, row, 0)
        for key, source in [('score','nativeScore'),('transformed','transformed'),('hidden1','hidden1'),('hidden2','hidden2'),('output','output'),('psqt','psqt')]:
            assert corrected[key] == row[source], (row['id'], key)
        legacy = reference.integer_trace(integer,row,6)['score']
        idx = [sorted(row[c]['base']+row[c]['donor']) for c in ['black','white']]
        accum = [weights['ft_bias'] + weights['ft_weights'][indices].sum(axis=0) for indices in idx]
        our=0 if row['sfen'].split()[1]=='b' else 1
        bucket=row['outputBucket']
        x=np.clip(np.concatenate([accum[our][:256],accum[1-our][:256]]),0,1)
        psqt=float((accum[our][256+bucket]-accum[1-our][256+bucket])*.5)
        full=(dense(x,bucket)+psqt)*600
        qx=np.array(corrected['transformed'],dtype=np.float64)/127
        qpsqt=corrected['psqt']/9600
        quant_ft=(dense(qx,bucket)+qpsqt)*600
        quant_weights_float_activation=(dense(qx,bucket,True)+qpsqt)*600
        outputs.append(dict(id=row['id'],parentId=row['parentId'],move=row['move'],sfen=row['sfen'],phase=row['phase'],group=row['sourceOpeningGroupId'],terminal=row['terminal'],inCheck=row['inCheck'],fullPrecisionScore=full,legacyScore=legacy,correctedScore=corrected['score'],quantizedFtOnlyScore=quant_ft,quantizedWeightsFloatActivationScore=quant_weights_float_activation,ftZeroFraction=row['transformerZeroCount']/512,ftSaturatedFraction=row['transformerSaturatedCount']/512))
    def stats(data):
        result={'positions':len(data)}
        for key in ['legacyScore','correctedScore','quantizedFtOnlyScore','quantizedWeightsFloatActivationScore']:
            errors=np.array([x[key]-x['fullPrecisionScore'] for x in data])
            result[key]={'mae':float(np.abs(errors).mean()),'rmse':float(np.sqrt(np.mean(errors**2))),'p95Absolute':float(np.percentile(np.abs(errors),95)),'maxAbsolute':float(np.abs(errors).max()),'meanSigned':float(errors.mean()),'signMismatches':sum((x[key]>0)-(x[key]<0) != (x['fullPrecisionScore']>0)-(x['fullPrecisionScore']<0) for x in data)}
        return result
    by_parent=defaultdict(list)
    for x in outputs:
        if x['move'] is not None:by_parent[x['parentId']].append(x)
    choices=[]
    for key, children in by_parent.items():
        # Terminal game value is exact and must not be replaced with a network score.
        nonterminal=[x for x in children if not x['terminal']]
        if any(x['terminal'] for x in children) or not nonterminal:
            choices.append(dict(parentId=key,excluded='terminal-child-present'))
            continue
        pick=lambda name: min(nonterminal,key=lambda x:(x[name],x['move']))
        fp=pick('fullPrecisionScore');legacy=pick('legacyScore');fixed=pick('correctedScore')
        choices.append(dict(parentId=key,fullMove=fp['move'],legacyMove=legacy['move'],correctedMove=fixed['move'],legacyAgrees=legacy['move']==fp['move'],correctedAgrees=fixed['move']==fp['move'],legacyFullStaticRegret=legacy['fullPrecisionScore']-fp['fullPrecisionScore'],correctedFullStaticRegret=fixed['fullPrecisionScore']-fp['fullPrecisionScore']))
    eligible=[x for x in choices if 'excluded' not in x]
    result={'schema':'haitaka-strength-first-step6-export-analysis-v1','nativeReferenceTraceParityPassed':True,'parents':128,'positions':len(outputs),'nonterminal':stats([x for x in outputs if not x['terminal']]),'allPositions':stats(outputs),'byPhase':{phase:stats([x for x in outputs if x['phase']==phase and not x['terminal']]) for phase in ['early','middle','late']},'moveAgreement':{'eligibleParents':len(eligible),'excludedTerminalParents':128-len(eligible),'legacy':sum(x['legacyAgrees'] for x in eligible),'corrected':sum(x['correctedAgrees'] for x in eligible),'legacyMeanFullStaticRegret':statistics.mean(x['legacyFullStaticRegret'] for x in eligible),'correctedMeanFullStaticRegret':statistics.mean(x['correctedFullStaticRegret'] for x in eligible)},'affineWeightClipping':clipping,'scope':'Static move agreement/regret uses full-precision NNUE, not a search teacher or game-theoretic value. All legal children were statically evaluated, not exhaustively searched.'}
    out=RUN/'export-analysis';out.mkdir(exist_ok=True)
    (out/'scores.jsonl').write_text(''.join(json.dumps(x)+'\n' for x in outputs))
    (out/'choices.jsonl').write_text(''.join(json.dumps(x)+'\n' for x in choices))
    (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({k:v for k,v in result.items() if k not in ['affineWeightClipping','byPhase','allPositions']},indent=2))
if __name__=='__main__':main()

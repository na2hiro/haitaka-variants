#!/usr/bin/env python3
"""Independent serializer-derived integer inference, with explicit legacy control."""
import numpy as np
import strength_first_candidate_parity as parity

def integer_trace(network, position, shift=0):
    bias, weights, psqt_weights, buckets = network
    feature_rows = [sorted(position[color]['base'] + position[color]['donor']) for color in ['black', 'white']]
    accum = [bias.astype(np.int64) + weights[idx].astype(np.int64).sum(axis=0) for idx in feature_rows]
    assert all(np.all((x >= -32768) & (x <= 32767)) for x in accum)
    psqts = [psqt_weights[idx].astype(np.int64).sum(axis=0) for idx in feature_rows]
    our = 0 if position['sfen'].split()[1] == 'b' else 1
    transformed = np.concatenate([np.clip(accum[our] >> shift, 0, 127), np.clip(accum[1-our] >> shift, 0, 127)])
    bucket = int(position['outputBucket'])
    (b1,w1),(b2,w2),(bo,wo) = buckets[bucket]
    h1 = np.clip((b1.astype(np.int64) + w1.astype(np.int64) @ transformed) >> 6, 0, 127)
    h2 = np.clip((b2.astype(np.int64) + w2[:,:16].astype(np.int64) @ h1) >> 6, 0, 127)
    output = int(bo[0]) + int(wo[0].astype(np.int64) @ h2)
    psqt = parity.trunc_div(int(psqts[our][bucket]) - int(psqts[1-our][bucket]), 2)
    return dict(score=parity.trunc_div(psqt+output,16), transformed=transformed.tolist(), hidden1=h1.tolist(), hidden2=h2.tolist(), output=output, psqt=psqt)

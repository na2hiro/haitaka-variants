#!/usr/bin/env python3
"""Extract immutable CPU tensors and audit serialized scales; never train."""
import hashlib
import json
import os
from pathlib import Path
import sys
import types
import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import r5_trainer as r5
import strength_first_candidate_parity as parity

run = ROOT / 'out/anhoku-strength-first/step6-browser-export-20260908-v1'
contract = json.loads((ROOT / 'r0/anhoku-strength-first/step4-diagnostic-v1.json').read_text())
checkpoint = ROOT / contract['models']['candidateCheckpoint']['path']
network = ROOT / contract['models']['candidateExport']['path']
for p, a in [(checkpoint, contract['models']['candidateCheckpoint']), (network, contract['models']['candidateExport'])]:
    assert hashlib.sha256(p.read_bytes()).hexdigest() == a['sha256']
temp, context = r5.prepare_context(contract['models']['candidateWidth'], 'cmake')
try:
    stub = types.ModuleType('cupy')
    stub.RawKernel = lambda *_args, **_kwargs: (_ for _ in ()).throw(RuntimeError('GPU prohibited'))
    sys.modules['cupy'] = stub
    torch, _, module, _ = r5.import_trainer(context)
    torch.set_num_threads(1)
    model = torch.load(checkpoint, map_location='cpu', weights_only=False).cpu().eval()
    with torch.no_grad():
        arrays = {'ft_weights': module.coalesce_ft_weights(model, model.input).cpu().numpy(), 'ft_bias': model.input.bias.cpu().numpy()}
        for bucket, layers in enumerate(model.layer_stacks.get_coalesced_layer_stacks()):
            for index, layer in enumerate(layers):
                arrays[f'b{bucket}_l{index}_weight'] = layer.weight.cpu().numpy()
                arrays[f'b{bucket}_l{index}_bias'] = layer.bias.cpu().numpy()
        np.savez(run / 'full-precision-weights.npz', **arrays)
    _, integer = parity.parse_integer_network(network)
    bias, weights, psqt, _ = integer
    expected_bias = np.rint(arrays['ft_bias'][:256] * 127).astype(np.int16)
    expected_weights = np.rint(arrays['ft_weights'][:, :256] * 127).astype(np.int16)
    result = {'checkpointSha256': contract['models']['candidateCheckpoint']['sha256'], 'networkSha256': contract['models']['candidateExport']['sha256'], 'featureTransformerBiasMatchesScale127': bool(np.array_equal(bias, expected_bias)), 'featureTransformerWeightsMatchScale127': bool(np.array_equal(weights, expected_weights)), 'weightScale': 127, 'runtimeAdditionalRightShift': 6, 'serializerSha256': hashlib.sha256((context / 'serialize.py').read_bytes()).hexdigest(), 'fullPrecisionWeightsSha256': hashlib.sha256((run / 'full-precision-weights.npz').read_bytes()).hexdigest(), 'cpuOnly': True}
    (run / 'serializer-scale-audit.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))
finally:
    os.chdir(ROOT)
    temp.cleanup()

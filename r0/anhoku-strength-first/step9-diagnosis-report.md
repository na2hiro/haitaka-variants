# Step 9 post-confirmation diagnosis

**Decision remains unchanged: confirmation failed; handcrafted remains production. `deploymentAuthorized: false`.**

This is bounded exploratory development evidence on the native MBP at 100 ms per move. It does not establish 1,000 ms strength, shipped-WASM qualification, or deployment authorization.

## Scope and validation

Manifest: `out/anhoku-strength-first/step9-post-confirmation-diagnosis-20260914-v1/manifest.json` (`f3cca7338b695bc3366394e3bc75b8b837b21a9bd47e5ccd31461aa6b4552b8c`). The audit passed, all four new cells completed 4,096 games, every block passed strict validation, and outcomes were opened only after the fixed schedule completed. The abandoned 1,000 ms partial remains unopened and excluded.

## Six matched cells

| Model / suite | Source | Games | W–L–D | Candidate pair score |
| --- | --- | ---: | ---: | ---: |
| A u2,688 / Suite A | historical-selected-subset | 1024 | 586–437–1 | 57.2754% |
| A u2,688 / Suite B | new-fixed-cell | 1024 | 605–419–0 | 59.0820% |
| B u2,688 / Suite A | new-fixed-cell | 1024 | 483–540–1 | 47.2168% |
| B u2,688 / Suite B | historical-selected-subset | 1024 | 511–513–0 | 49.9023% |
| B u768 / Suite A | new-fixed-cell | 1024 | 599–425–0 | 58.4961% |
| B u768 / Suite B | new-fixed-cell | 1024 | 594–430–0 | 58.0078% |

## Matched contrasts

All intervals are paired-opening bootstrap 95% descriptive intervals, 10,000 replicates, exploratory and unadjusted.

| Contrast | Estimate | 95% interval |
| --- | ---: | ---: |
| A u2,688 − B u2,688, Suite A | +10.0586% | [+6.1035%, +13.8672%] |
| A u2,688 − B u2,688, Suite B | +9.1797% | [+5.3711%, +13.0859%] |
| A u2,688 − B u2,688, suite-balanced | +9.6191% | [+6.9092%, +12.2803%] |
| B u768 − B u2,688, Suite A | +11.2793% | [+7.4219%, +15.2344%] |
| B u768 − B u2,688, Suite B | +8.1055% | [+4.2969%, +11.9141%] |
| B u768 − B u2,688, suite-balanced | +9.6924% | [+7.0795%, +12.4023%] |

## Audit observations

The two recipes match on feature family, width, robust-clipped Huber objective, output scale, Ranger optimizer, loader semantics, learning-rate schedule, diagnostics, and serializer family. Differences are the training seed, corpus manifests/content, trainer wrapper/output history, checkpoint schedule, experiment identity, and selection outcome. The audit found no identity, export, orientation, recipe, or data-integrity defect.

B u768 has a passed parity receipt bound to the exact u768 network and frozen runtime. The historical A u2,688 parity receipt passes its network checks but records a prior runtime hash; it is retained as historical evidence and is not used as exact-runtime parity evidence. Existing B u768 and B u2,688 loss, saturation, output-scale, and update-magnitude diagnostics are preserved in the machine-readable audit.

## Interpretation and limits

- A exceeds B u2,688 descriptively on both matched suites; this supports a B-model generalization-weakness hypothesis.
- B u768 exceeds B u2,688 descriptively on both matched suites; this supports an earlier-checkpoint-generalizes-better hypothesis.

The historical diagonal cells ran in an earlier session, so contrasts containing them retain session and thermal confounding. The crossover can describe model-by-suite interaction, but cannot causally isolate corpus, initialization, recipe, or session effects. A favorable u768 result cannot rescue Step 9 or authorize promotion.

**Bounded next proposal:** If NNUE work is resumed after review, use one separately reviewed prospective lane that isolates the B-model generalization hypothesis; keep corpus, seed, recipe, and session factors explicitly unresolved and predeclare one fixed endpoint.

Remaining owned processes: none.

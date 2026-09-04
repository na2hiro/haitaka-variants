## Phase R6: nested strength-scale learning curve

### Question

Does the frozen recipe continue to improve with genuinely new data, and at what
scale does equal-node and fixed-time strength saturate?

The Fairy-Stockfish trainer guidance says that data count and depth are the main
strength factors, that at least 100M positions are usually needed for decent
results, and that depths 4-5 often work well. Its training guide historically
defines one epoch as 20M positions. These are scale references, not substitutes
for an Anhoku learning curve:

- [Fairy-Stockfish training-data guidance](https://github.com/fairy-stockfish/variant-nnue-pytorch/wiki/Training-data-generation)
- [Fairy-Stockfish NNUE training guidance](https://github.com/fairy-stockfish/variant-nnue-pytorch/wiki/NNUE-training)

### Scale ladder

1. Train to the frozen plateau rule on the nested 10M prefix.
2. Replicate the 10M result with a second training seed.
3. Generate/train 50M only if the preregistered grouped offline and paired-game
   decisions show a credible positive 1M-to-10M slope and the R4 cost ceiling
   remains affordable.
4. Generate/train 100M only if the corresponding 10M-to-50M slope remains
   credibly positive and the 100M forecast is affordable. An uncertain slope
   may authorize one terminal 100M probe only if that exception, its minimum
   plausible gain, and its cost cap were registered before R6 began; simply
   failing to reach the strength target never authorizes more data.
5. Reproduce the best scale with an independent generation seed, not merely a
   different optimizer seed.

Do not compare scales at the same small optimizer-step count. Compare converged
recipes and publish both loss-versus-examples-seen and loss-versus-unique-data
curves.

### Evaluation at each scale

Use the ladder:

1. finite validation and development metrics;
2. the disjoint one-shot stage block for that scale after its single
   predeclared candidate is selected; after opening, the block becomes
   development history;
3. equal-node match versus the preceding scale and handcrafted;
4. target fixed-time screen only after equal-node quality is competitive;
5. production runtime benchmark.

For offline deltas, resample the highest independent source group and apply the
registered multiplicity rule. For games, predeclare exactly one valid decision
mode: a fixed number of complete opening pairs with its grouped interval, a
validated pair/pentanomial SPRT reported by its declared decision, or an
anytime-valid confidence sequence. Never attach an ordinary post-stop 95%
interval after sequential peeking. Select one checkpoint offline, then play it;
do not rank every checkpoint by thousands of games.

### Scale decisions

- **Both offline and equal-node improve:** continue to the next affordable
  nested scale.
- **Training improves but held-out metrics flatten:** increase independent data
  diversity or regularization; do not merely repeat records.
- **Full-precision improves but quantized does not:** add quantization-aware
  training or revise scaling before more data.
- **Offline and equal-node flatten after convergence:** authorize R7 residual
  analysis.
- **Equal-node improves but fixed-time does not:** route to the compact model or
  R8 runtime work.

Every scale jump requires a measured node, wall-time, storage, GPU, and money
forecast plus an immutable artifact-retention plan.


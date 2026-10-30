## Phase R7: hard examples, objective alignment, and bounded architecture work

### Question

After convergence and scale are real, what explains the residual search errors?

### R7-A: hard-position replay

Mine positions from:

- candidate-versus-handcrafted games where evaluation swings precede a loss;
- candidate/deep-teacher move disagreements with high regret;
- tactical failures, rare donor configurations, and under-covered feature rows;
- independent policies, not only candidate self-play.

The candidate may propose positions but never labels them. Relabel every retained
board with the frozen stronger teacher. Cap prioritized-sampling weights and mix
hard data with a declared broad replay fraction so the network does not forget
general positions. Mine only from development games and registered disagreement
pools—never from one-shot stage blocks, the final sealed static/search set, or
sealed promotion openings.

Run one controlled broad-only versus broad-plus-hard comparison. If it improves
development regret and equal-node play, select one candidate and open a fresh
one-shot R7 stage block. Any further hard-mining round must still use development
evidence and consume a newly registered, disjoint stage block, with the maximum
number of rounds frozen before the first block is opened. An opened block becomes
development history and is never mined for its failures.

### R7-B: quantization-aware training

Authorize QAT only when full precision consistently wins and the serialized
network loses a material fraction of that gain. The gate compares the same
checkpoint recipe with and without fake-quantization/quantization-aware
fine-tuning and requires runtime integer parity.

### R7-C: representation or capacity

New feature geometry is authorized only if all are true:

- the loader, objective, schedule, and quantization gates pass;
- train and held-out curves have converged at a meaningful scale;
- residuals form a stable, reproducible category that the current input cannot
  distinguish or the current width cannot fit;
- a written oracle shows how the proposed feature/capacity change represents
  that category;
- the comparison changes only that factor at matched data and compute.

Failure of an exactly representable R1 oracle is a correctness/trainer/oracle
failure and stays in R1/R2; it never authorizes a feature expansion. Earlier
representation work is authorized only by a proven non-representable collision
with frequent, material regret after parity passes. Otherwise, do not revive
DonorReceiverPairV2 or invent Phase 12 features from intuition.


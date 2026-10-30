# Model Registry

No official shogi-variant NNUE model is published from this repository yet.
The first useful model for Annan and other variants is a launch goal for the community.

For a receipt-gated workflow that trains, exports, checks native/WASM parity,
and evaluates a candidate before independent confirmation, see
[Strength-first NNUE workflow](nnue-strength-first-workflow.md).

When you've built a model, please reach out to me on [the official Discord server](https://discord.com/invite/CYRWPQMGXu), create an issue or any other way!

When sharing a model, I'd like to know:

- variant name
- NNUE architecture (in case you changed training code)
- training config
- dataset size
- hardware
- training duration
- validation result
- `haitaka_learn verify` output
- license (MIT is assumed, unless specified)
- download location (do not commit to the repository!)

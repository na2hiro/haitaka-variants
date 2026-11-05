# Hand-rule implementation verification

Verified on 2026-11-05 in `feat/composable-hand-rules`, based on PR #33's
`feat/composable-outcome-rules` at `55d8229`.

## Results

| Check | Result |
| --- | --- |
| Workspace regression (`cargo test --workspace --offline`) | 529 passed, 6 existing external-model tests ignored |
| Seven individual rules | Core legality/state tests, search, CLI profile/ID checks passed |
| Donor composition | All 84 combinations (12 donors × 7 hand/capture/outcome rules) compiled |
| Further composition/validation | 115 checks total including the donor matrix; five composite workspace builds and legal-move rollouts, all ten conflicting hand pairs, all four internal-marker entrypoints, outcome precedence, and NNUE width 256 |
| Check-win composition | Nakamaware/Suika retain unmoved checking pieces and detect touching Kings; `oute` compositions passed |
| Post-review check refresh | Nine targeted checks passed, including both capture-King profiles, their `oute` combinations, search, learning, and the standard core suite (the already-passed long DFPN regression excluded from this repeat) |
| R5 learning | All six supported rules passed data round-trip and fixed-node self-play generation tests |
| Actual C++/PyTorch integration | 13 fixture records loaded; six CPU optimizer steps completed; a 159,495,770-byte NNUE exported and loaded under all six profiles |
| Shared hand capacity | Maximum shared stock exercised 78 real rows and 156 factorized rows |
| Incremental R5 | Accumulators matched full refresh for captures, friendly captures, shared drops, cancellation and cascaded fusion; exported-network integer output matched full refresh |
| Release WASM | All seven profiles built using `wasm-pack --release --no-opt --target web` |
| Native/WASM comparison | 39 positions agreed on move counts and USI terminal notifications; all 31 eligible NNUE integer evaluations agreed |
| Packaging | All seven archives verified for rule ID, profile name, and exact bundled WASM bytes |
| Python/browser regressions | 49 Python tests and the existing three browser-worker test files passed |

The CPU smoke is an integration fixture, not a release model or a strength
measurement. The packages contain the release WASM without this smoke model.
Growth (`zoshoku`) NNUE is intentionally rejected; the other six rules use the
existing R5 feature layout. Incompatible TuyouraOu11 SFNN profiles are also
rejected. Capture-King profiles return `Unknown` from checking-move DFPN.

## Inspect and reproduce

[Rule configuration and semantics](hand-rules.md) describe the feature flags,
configuration fields, and compatibility limits. New CI jobs exercise the
seven rules and the complete donor matrix.

Local raw logs and machine-readable reports are under
`target/hand-verification/`, including `matrix-results.json`,
`check-refresh-results.json`, `cpu-smoke/report.json`, `wasm-results.json`,
`package-results.json`, and `workspace-final.log`.

The optional `hand_rules_native_wasm_fixtures` integration test writes native
fixtures when `HAITAKA_HAND_WASM_FIXTURES` is set. Set
`HAITAKA_HAND_SMOKE_MODEL` to the exported smoke network for six supported
profiles; the Growth fixture deliberately omits a model. Compare with:

```console
node scripts/verify_hand_rule_wasm.mjs \
  target/hand-verification/native-wasm \
  target/hand-verification/wasm \
  target/hand-verification/cpu-smoke/smoke.nnue
```

Single-rule archives are in `target/hand-verification/packages/`. Their IDs
are 24, 54, 57, 93, 143, 146, and 155. A bundle with all seven archives and
these instructions is `target/hand-verification/hand-rule-packages.zip`.

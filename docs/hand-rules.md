# Composable hand rules

These rules extend the compile-time profiles introduced in PR #33. Select at
most one donor rule, any supported outcome rules, at most one king constraint,
and the independent capture/hand options below.

| Cargo feature | Rule ID | Behavior |
| --- | ---: | --- |
| `nakamaware` | 24 | Capture friendly pieces; checks need not be evaded; capture the opposing King to win. Capturing your own King loses. |
| `fushiyo` | 54 | Discard captured pieces; no drops. |
| `share` | 57 | Captures add to both hands; a drop removes from both. |
| `zoshoku` | 93 | Captures add two copies. |
| `babanuki` | 143 | A pair of the same hand piece disappears. |
| `tensuu` | 146 | Win with at least 15 hand points: Pawn 1, Lance 2, Knight 3, Silver 4, Gold 5. Bishop/Rook score zero. |
| `suika` | 155 | Pairs fuse through Pawn → Lance → Knight → Silver → Gold → Bishop → Rook → King, including cascades. Acquire or synthesize a hand King to win; checks need not be evaded. |

`fushiyo`, `share`, `zoshoku`, `babanuki`, and `suika` are mutually exclusive.
`nakamaware` and `tensuu` can combine with each of them. King constraints always
apply to the physical King. In capture-King profiles, ordinary geometric check
is still available to `oute`, but does not force an evasion or constitute mate.
Existing immediate-loss precedence remains: for example, Capital Return can
reverse a move that otherwise wins. DFPN returns `Unknown` for capture-King
profiles because checking-move mate search is not applicable.

## Configuration and packaging

```toml
[rules]
ruleset = "annan"
capture_rules = ["nakamaware"]
hand_rule = "share"
win_conditions = ["tensuu"]
rule_id = 65001 # Explicit identity for a composite profile.
```

`capture_rules` defaults to `[]`; `hand_rule` defaults to `"standard"`.
For Suika, use `win_conditions = ["suika"]` and omit `hand_rule`. The Suika
feature activates its inventory transform and capture-King behavior together.
Single-rule standard profiles infer the IDs in the table. Composite profiles
require an explicit ID. Existing profile names are unchanged; new capture and
hand suffixes follow the existing outcome and King-constraint suffixes.

```console
cargo xtask package --ruleset standard+suika
cargo xtask package --ruleset annan+tensuu+nakamaware+share --rule-id 65001
cargo run -p haitaka_cli --features zoshoku -- usi
```

The profile is carried through data generation, training, verification,
checkpoint ranking, self-play reports, and package manifests. `cargo train`
and other learning wrappers derive all rule features from `[rules]`.

## Positions and evaluation

Hand counts use `u32`, with checked arithmetic and deterministic hashes for
counts above the original 19-piece table. Growth profiles allow increased
material. Ordinary profiles retain their material limits; shared profiles
count their duplicated hand only once. Shared SFENs must list equal hands for
both colors. Cancellation and fusion SFENs must already be normalized (at
most one of each hand species). Capture-King terminal SFENs can include `K`
or `k` in the hand. Kings cannot be dropped. No-use SFENs require empty hands. Tsume setup with
these profiles preserves explicitly supplied defensive hands instead of
inferring the complement of the standard stock.

All seven rules support handcrafted evaluation, native/USI play, WASM, and
packaging. R5 NNUE training and inference support six rules; `zoshoku` is
rejected before learning or loading a network. Its unbounded material growth
requires a separate data/feature format. R5 terminal King positions are
adjudicated before feature extraction and are not stored as NNUE inputs.

TuyouraOu11 SFNN keeps its original 38-nonking material contract: `share`,
`suika`, and `zoshoku` are rejected. Compatible hand/capture profiles use full
refresh updates. A model trained for ordinary shogi may load where the feature
layout is compatible; this does not establish its strength under new rules.

## Verification

```console
cargo test -p haitaka --test hand_rules --features suika
cargo test -p haitaka_wasm --features share hand_rules
cargo test -p haitaka_learn --bin haitaka_learn --features share hand_rules
cargo test -p xtask hand_rules
```

To exercise the actual C++ sparse loader, CPU training, and NNUE export, run
`hand_rules_training_data_roundtrip` for each supported single rule with
`HAITAKA_HAND_SMOKE_OUTPUT` set to an absolute fixture directory, then run
`scripts/hand_rule_nnue_smoke.py --data <fixtures> --output <smoke-output>`
using the pinned trainer's Python environment. The shared fixture includes
all 78 reachable base rows. Set `HAITAKA_HAND_SMOKE_MODEL` to the resulting
`smoke.nnue` when rerunning the NNUE incremental tests. This is a disposable
integration smoke, not a trained release model or a strength measurement.

[Recorded implementation verification](hand-rules-verification.md).

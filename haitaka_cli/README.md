# haitaka_cli

`haitaka_cli` contains local launch tools for Haitaka Variants.

It is the command-line entry point for playing against the engine, running quick
engine-vs-engine comparisons, and creating Shogitter Engine Package v1 archives.

## Commands

### Play Or Debug

Ask the engine for one move and exit:

```bash
cargo run -p haitaka_cli -- play --human none --depth 3
```

Play as black against the engine:

```bash
cargo run -p haitaka_cli -- play --human black --depth 3
```

Use a custom SFEN:

```bash
cargo run -p haitaka_cli -- play \
  --human none \
  --depth 3 \
  --sfen "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"
```

Use Annan rules:

```bash
cargo run -p haitaka_cli --features annan -- play --human none --depth 3
```

### Self-Play

Run a small engine-vs-engine comparison with a fixed time budget per move:

```bash
cargo run -p haitaka_cli --release -- self-play \
  --games 4 \
  --threads 0 \
  --movetime-ms 100
```

Compare an NNUE model against the handcrafted evaluator and report the Elo-style
gap as `A - B`:

```bash
cargo run -p haitaka_cli --release -- self-play \
  --games 10 \
  --threads 0 \
  --movetime-ms 100 \
  --a-eval nnue \
  --opening-random-plies 4 \
  --seed 1 \
  --nnue path/to/model.nnue
```

This treats engine A as `nnue` and engine B as `handcrafted`. Use `--a-nnue`
or `--b-nnue` when each side should load a different model. `--threads 0`
selects available parallelism automatically. Prefer `--movetime-ms` for
strength checks because fixed depth can miss improvements that make search
faster. With movetime, omitted `--a-depth` and `--b-depth` mean uncapped
iterative deepening; use them mainly when you want an explicit cap for
deterministic debugging or quick smoke tests.

`--opening-random-plies` is the important knob when matches look too
deterministic from one fixed start position. The same randomized opening is used
for each consecutive color-swapped pair of games.

The Elo output is only a small-sample estimate. It is useful for quick local
checks, not for publishing serious engine ratings.

For the complete workflow, including raw external USI engines, native engine
archives, opening suites, and JSON reports, see
[`docs/self-play.md`](../docs/self-play.md).

### Shogitter Engine Package v1

From the repository root, create a standard package with:

```bash
cargo pack
```

Create an Annan package with:

```bash
cargo pack-annan
```

These Cargo aliases are defined in `.cargo/config.toml` and run the workspace
`xtask` helper, which builds WASM and then invokes `haitaka_cli package`.

For manual debugging, build WASM first:

```bash
wasm-pack build haitaka_wasm --target web --out-dir pkg --release
```

Then create a package:

```bash
cargo run -p haitaka_cli --release -- package \
  --wasm-dir haitaka_wasm/pkg \
  --output target/haitaka-variants.tgz
```

For Annan:

```bash
wasm-pack build haitaka_wasm --target web --out-dir pkg --release --features annan
cargo run -p haitaka_cli --release --features annan -- package \
  --wasm-dir haitaka_wasm/pkg \
  --ruleset annan \
  --rule-id 26 \
  --output target/haitaka-variants-annan.tgz
```

For a metadata-only smoke test without built WASM artifacts:

```bash
cargo run -p haitaka_cli -- package --allow-missing-wasm
```

Archives created with `--allow-missing-wasm` are not loadable by Shogitter.

## Package Contents

The `.tgz` archive contains:

```text
shogitter-engine.json
README.txt
engine/
  haitaka_wasm.js
  haitaka_wasm_bg.wasm
  haitaka_wasm.d.ts
  haitaka_wasm_bg.wasm.d.ts
  package.json
  README.md
  model.nnue
```

`model.nnue` is included only when `--nnue` is passed. The `.d.ts`,
`package.json`, and `README.md` files are copied when present in the
`wasm-pack` output directory.

The root `shogitter-engine.json` manifest is authoritative. It declares:

- `runtime.kind = "wasm-bindgen"`
- `runtime.module = "engine/haitaka_wasm.js"`
- `runtime.wasm = "engine/haitaka_wasm_bg.wasm"`
- one Shogitter rule mapping using SFEN positions and USI moves

See [`../docs/shogitter-package.md`](../docs/shogitter-package.md).

## Tests

```bash
cargo test -p haitaka_cli
cargo test -p haitaka_cli --features annan
```

# Shogitter Engine Package v1

`haitaka_cli package` creates a Shogitter Engine Package v1 `.tgz` archive.
This is the Shogitter-owned first-party contract for direct `wasm-bindgen`
engines such as Haitaka. It is separate from full-blown USI engine compiled with Emscripten, like Fairy-Stockfish and
YaneuraOu, which Shogitter still detects with its `.js`/`.worker.js`/`.wasm` heuristic.

Run these commands from the repository root.

The preferred commands are Cargo aliases backed by the workspace `xtask` helper:

```bash
cargo pack
cargo pack-annan
```

`cargo pack` builds the standard WASM package and writes
`target/haitaka-variants.tgz`. `cargo pack-annan` builds with the
`annan` feature and writes `target/haitaka-variants-annan.tgz`.

For manual debugging, build the WASM artifacts first:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
wasm-pack build haitaka_wasm --target web --out-dir pkg --release
```

For Annan, build WASM with the Annan feature:

```bash
wasm-pack build haitaka_wasm --target web --out-dir pkg --release --features annan
```

Then create the package:

```bash
cargo run -p haitaka_cli -- package \
  --wasm-dir haitaka_wasm/pkg \
  --output target/haitaka-variants.tgz
```

For Annan, also build the package command with the Annan feature:

```bash
cargo run -p haitaka_cli --release --features annan -- package \
  --wasm-dir haitaka_wasm/pkg \
  --ruleset annan \
  --rule-id 26 \
  --output target/haitaka-shogitter-annan.tgz
```

For a metadata-only smoke test:

```bash
cargo run -p haitaka_cli -- package --allow-missing-wasm
```

Archives created with `--allow-missing-wasm` are not loadable by Shogitter
because their manifest-declared runtime artifacts are absent.

## Archive Layout

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

`model.nnue` is present only when `--nnue` is passed. The `.d.ts`,
`package.json`, and `README.md` files are copied from `haitaka_wasm/pkg` when
they exist. `haitaka_wasm.js` and `haitaka_wasm_bg.wasm` are required unless
`--allow-missing-wasm` is passed.

## Manifest

`shogitter-engine.json` is authoritative. Haitaka emits schema version 1 with
profile-scoped rule and NNUE declarations:

```json
{
  "schema": "shogitter-engine-package",
  "schemaVersion": 1,
  "engine": {
    "id": "haitaka-variants",
    "name": "Haitaka Variants (annan)",
    "version": "0.1.0",
    "commit": "<git commit or unknown>"
  },
  "runtime": {
    "kind": "wasm-bindgen",
    "module": "engine/haitaka_wasm.js",
    "wasm": "engine/haitaka_wasm_bg.wasm"
  },
  "capabilities": {
    "protocols": ["shogitter-direct-v1"],
    "commands": ["search", "iterative-search", "perft", "dfpn"],
    "supportsPonder": false,
    "supportsMovetime": true,
    "supportsDepth": true
  },
  "profiles": [
    {
      "id": "annan-default",
      "name": "Annan default",
      "rules": [
        {
          "ruleId": 26,
          "variant": "annan",
          "positionFormat": "sfen",
          "moveFormat": "usi",
          "startpos": "lnsgkgsnl/1r5b1/p1ppppp1p/1p5p1/9/1P5P1/P1PPPPP1P/1B5R1/LNSGKGSNL b - 1"
        }
      ],
      "nnue": null
    }
  ]
}
```

When `--nnue path/to/model.nnue` is passed, the selected profile's `nnue` is:

```json
{
  "path": "engine/model.nnue",
  "format": "nnue"
}
```

## Fields

- `schema`: always `shogitter-engine-package`.
- `schemaVersion`: currently `1`.
- `engine`: Shogitter display and provenance metadata. The display name
  includes the packaged ruleset, such as `Haitaka Variants (annan)`, so users
  can distinguish registered engines.
- `runtime.kind`: `wasm-bindgen` for Haitaka packages.
- `runtime.module`: archive-relative path to the generated JS glue module.
- `runtime.wasm`: archive-relative path to the generated WASM binary.
- `capabilities`: direct-call commands Shogitter can expect from the engine.
- `profiles`: packaged engine profiles. Each profile is a selectable
  engine/NNUE configuration with its own rule list.
- `profiles[].nnue`: optional archive-relative NNUE model descriptor for that
  profile.

For the default standard build, Haitaka emits one `standard` profile using
`ruleId = 0`, `variant = "standard"`, `positionFormat = "sfen"`, and
`moveFormat = "usi"`. For `--features annan`, Haitaka emits one `annan`
profile using `ruleId = 26` and `variant = "annan"`.

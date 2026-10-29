# Anhoku NNUE v0.6 Phase 8A data-generation handoff

Phase 8A is prepared but not launch-passing yet. This handoff stops before
production data generation; the generated manifests and audits must be added
after the CPU machines finish.

## Frozen identity

- C/16 control: `out/anhoku-v0.6-phase7.1-preserved/lane-c-step-16.nnue`
- C/16 SHA-256: `049f72f3a3adcfeb260710264af6669da6346af35bd34092f6f6fa0ef531cfe0`
- Source suite: `haitaka_learn/openings/anhoku-v2.tsv`
- Suite: 64 opening IDs, validated by `validate-openings`
- OOD-v2: `anhoku-v2-053` through `anhoku-v2-064` (12 IDs)
- Label budget: 50,000 combined alpha-beta/qsearch nodes, depth cap 64
- Incomplete labels: `reject-position`, counted and excluded
- Attempted roots: 64 per game, independent of accepted root/leaf records
- Data seed: 75; split seed: 76; shuffle seed: 77
- Later trainer initialization seeds: 80, 81, 82

The root and leaf configs retain separate output directories for compatibility
with the existing trainer pipeline. They use the same non-teacher identity and
each manifest records a `candidate_identity_sha256` over the sampled root
sequence. Run the manifest comparison before any training; equal hashes are a
required gate.

## Preflight

```bash
sha256sum out/anhoku-v0.6-phase7.1-preserved/lane-c-step-16.nnue
python3 scripts/phase8_prepare.py check \
  --output out/anhoku-v0.6-phase8a-preflight.json
cargo run --release -p haitaka_learn --features anhoku -- validate-openings \
  --config haitaka_learn.anhoku-v0.6-phase8-root.toml
```

## Bounded generation

Run the root and leaf pilots sequentially. A lane may be divided into
contiguous shard ranges on different machines, but every machine must use this
source revision and the exact config file. Freeze and record the revision
before starting:

```bash
git rev-parse HEAD
git status --short
```

The status must be clean after these changes are committed. Do not use
`--ignore-identity-mismatch`.

```bash
cargo generate haitaka_learn.anhoku-v0.6-phase8-root.pilot.toml --shard 1-4/16
cargo generate haitaka_learn.anhoku-v0.6-phase8-leaf.pilot.toml --shard 1-4/16
```

Give the remaining non-overlapping shard ranges to the other machines. Copy
complete output directories back to one host and merge each lane:

```bash
cargo merge haitaka_learn.anhoku-v0.6-phase8-root.pilot.toml \
  --input path/to/root-machine-a-output \
  --input path/to/root-machine-b-output
cargo merge haitaka_learn.anhoku-v0.6-phase8-leaf.pilot.toml \
  --input path/to/leaf-machine-a-output \
  --input path/to/leaf-machine-b-output
```

Then audit both splits and compare the matched manifests:

```bash
python3 scripts/phase8_prepare.py check-matched \
  --root-output out/anhoku-v0.6-phase8a-root-pilot \
  --leaf-output out/anhoku-v0.6-phase8a-leaf-pilot \
  --output out/anhoku-v0.6-phase8a-matched.json
```

Do not train from these pilots. Record per-split side/outcome balance,
incomplete-label rates, terminal/mate rejection counts, generation CPU time,
positions per second, and projected 262k/1M CPU cost in the preparation result.
Phase 8B remains blocked until those gates pass.

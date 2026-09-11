# Anhoku independent-seed development replication

The third and final initial-tournament training run completed all 4,096 optimizer updates on the unchanged audited 880k training corpus. It consumed 66,753,664 examples, or 75.86 equivalent passes. The lowest validation loss was **0.012594 at update 2,688**.

The scheduled one-second development screens improved with training:

| Checkpoint | Games | W–L–D | Score | Opening-group 95% interval |
| --- | ---: | ---: | ---: | ---: |
| u256 | 128 | 48–80–0 | 37.5% | 30.1%–44.9% |
| u1024 | 128 | 50–77–1 | 39.5% | 31.8%–47.1% |
| u2048 | 128 | 63–65–0 | 49.2% | 39.6%–58.8% |

The frozen final comparison evaluated u2,048, the best scheduled screen, and u2,688, the lowest validation loss, over 256 equal-node games each. They scored 49.2% and 57.0% respectively. The ranking rule selected **u2,688**.

Update 2,688 passed full-precision/integer/native/WASM qualification on 1,024 positions with zero parity mismatches. Its 83,048,706-byte selected network has SHA-256 `6c4f31ad…`. Quantization MAE was 9.55 score units, with p95 45.35 and maximum 113.31.

The selected model then completed the frozen primary evaluation on the MacBook Pro: 256 games, 128 color-swapped pairs, 64 opening groups, one concurrent game, and 1,000 ms per move. It scored **163–93–0, or 63.67%**, with a grouped 95% interval of **55.58%–71.77%** (about +97.5 Elo from the point estimate). This replicates the compact recipe's development strength across initialization seed.

The result still reuses the same training corpus and development openings. It nominates the recipe and u2,688 as model A for independent confirmation; handcrafted remains the production recommendation, and no deployment is authorized.

The selected u2,688 checkpoint, u2,048 checkpoint, final u4,096 checkpoint, selected integer network, match sources, and receipts are preserved locally. The original u256 checkpoint transfer was interrupted at 89,751,552 of 163,438,925 bytes, and the original u1,024 checkpoint was omitted before instance 50523521 was destroyed. A bounded same-image RTX 4090 replay matched the initial prediction digest and boundary metrics but diverged at sub-metric precision after CUDA updates, so it could not recreate either historical file byte for byte. Vast had no automatic backup. This gap prevents a complete rehash of every historical selection-graph leaf; it does not affect the exact selected model or its preserved parity and match evidence. Recovery instance 50558431 was destroyed after $0.241 in charges.

The next bounded lane is CPU preparation for a separately generated model-B corpus and unseen opening-source confirmation. Any GPU training for that lane requires a new concrete rental envelope and explicit permission.

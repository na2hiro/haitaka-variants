# Anhoku strength-first Step 3 width-512 preflight

Prepared 2026-09-07T14:08:08+09:00. Status: **ready, awaiting explicit GPU permission**.

This run tests one training-recipe change: `recipe.width.l1` increases from 256 to 512. The training and validation data, seed, scratch initialization, feature family, batch and loader semantics, robust teacher-only objective, Ranger settings, learning-rate schedule, 4,096-update exposure, checkpoint schedule, openings, search budgets, stopping rule, and final ranking remain matched to the completed width-256 run. The separate experiment identity, runner, output directory, preflight path, resource forecast, and quote are operational changes required to keep the evidence independent and auditable.

The dedicated contract and runner pass closed-world validation, all input hashes, and the receipt/stopping/selection self-test. The width-512 native runtime passes `cargo check`, and all 92 native/WASM correctness tests pass. The actual pinned NNUE/Ranger/production-loader CPU fixture also passes: loss decreases, the finite partial batch is observed, and resumed versus uninterrupted weights, optimizer state, predictions, trace, cursor, and serialized network are exact. Both fixture exports have SHA-256 `97badf9392feef2d25cfbabcd1858b5aaf761fea605ae65e685ec51c42c19504`.

The exact contract is SHA-256 `e5a6c642728d0f448c18587595ac05fbcef9a0f7091ee6605de648eee0632228`. It uses dedicated width-512 trainer SHA-256 `6ac656cd2a35c49cc05f319267880c34acf05cabffb3934d934607eeaf7aa54e` and parity tool SHA-256 `205837ed94854e733070f5efb1bf655f916253cb8ca14467f33b54b639374b3b`.

The concrete paid target is the existing stopped Vast.ai instance 50101026: RTX 4090 with 24,564 MB GPU memory and 50 GB disk. It is currently `actual_status=exited`, `intended_status=stopped`; its observed combined rate is $0.462593/hour. The run cap is four additional rented hours, no more than $0.75/hour, and no more than $5 additional cost. The prior observed program charge was $1.213, and the program ceiling remains $25.

The width-256 measured rate was 5,832.9 updates/hour. Conservatively scaling training time by two gives about 1.40 GPU-hours; the contract rounds this to 1.5 hours and reserves 2.0 hours for training and validation inside the four-hour envelope. Peak new storage is capped at 15 GiB. The first 128 updates are a mandatory throughput probe, and its measured forecast must replace this estimate before continuation beyond update 256.

After permission, execution will restart the existing instance, recheck the rate and retained remote inputs, upload the hash-bound width-512 contract and tools, run remote validation and the production-path fixture, and train only through update 128. Later continuation remains gated by measured cost, parity, and each scheduled strength receipt. No result from these reused openings can authorize promotion or deployment.

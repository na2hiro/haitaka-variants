# Anhoku strength-first Step 2 runner

Generated 2026-09-07T06:31:42+09:00. Status: **complete before GPU execution**.

The versioned v3 runner strictly validates every resolved training, optimizer, data, checkpoint, strength-screen, stopping, and cost field. It preserves deterministic finite partial batches, fixed-order validation, atomic resumable state, immutable named checkpoints, atomic checkpoint-specific export, and separate exposure counters. Every continuation is bound to a measured throughput horizon and the complete raw-game screen evidence needed for the two-successive-low stopping rule.

The final real CPU fixture used the pinned NNUE model, Ranger optimizer, and production sparse loader. Across 12 updates and 54 examples it observed batches `5,4` for six epochs, reduced loss from 0.0169239847 to 0.0169128693, and produced exact resumed versus uninterrupted weights, optimizer state, predictions, trace, cursor, and integer network. Both exports have SHA-256 `7d2185dfdaad9eaeb6a2b727cf908f9bdeb258865018c7553b3f0bf03722c8dd`.

The fixture is deliberately CPU-scoped. CUDA RNG and optimizer state, the production CUDA training/continuation commit path, candidate quantization, and integer/native/WASM parity remain mandatory on the approved instance. No candidate training or new-model game has started.

The resolved first-rental target is verified Vast.ai offer 12239064: one RTX 4090 with 24,564 MB GPU memory and 50 GiB storage at a quoted combined $0.462593/hour. The host has 22.5 GB/s measured PCIe bandwidth, 4,438 MB/s disk bandwidth, CUDA 13.2 support, and reliability 0.9981716. It remained available across the repeated selection checks. The accepted ceiling is $0.75/hour, four rented hours, and $5 total. Fresh training is hard-stopped at update 128; the throughput probe itself is capped at 0.25 GPU-hour. Continuation requires a recomputed measured forecast, an approved maximum update horizon, and later hash-bound strength screens.

The conservative first-run reserve is four rented hours and $5. The three-run forecast reserves 12 rented hours and $15, below the program ceilings of 24 GPU-hours and $25. These later runs are not authorized by the first-rental approval.

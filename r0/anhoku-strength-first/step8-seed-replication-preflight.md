# Anhoku Step 8 seed-replication preflight

Step 7 selected corrected-runtime update 2,048 after it scored 76–52 against handcrafted at 1 second per move (59.4%, opening-paired 95% interval 51.6%–67.2%) on reused development openings. The next bounded lane is one fresh width-256 seed on the unchanged audited 880k/60k corpus. It isolates initialization variance; it is not the independent-data replication required for final confirmation.

The closed-world config validator and deterministic runner self-test pass. The real CPU fixture also passes: resumed and uninterrupted weights, optimizer state, predictions, cursors, and serialized exports are exact. Training is gated at update 128; continuation requires measured throughput and hash-bound evidence.

The current public Vast.ai quote is one verified RTX 4090 at $0.2742/hour before any 60 GiB adjustment. The requested ceiling is one RTX 4090, 60 GiB, at most $0.75/hour, four rented hours, and $5 total. The offer will be rechecked before creation. The prior matching width-256 run recorded 263.54 seconds across shuffle, training, validation, checkpoints, and export, so this envelope is conservative.

No GPU has been rented or used. The paid instance will be stopped before long CPU strength games whenever export and transfer are complete. Production remains unchanged.

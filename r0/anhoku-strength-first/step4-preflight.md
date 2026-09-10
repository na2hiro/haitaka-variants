# Anhoku strength-first Step 4 preflight

The frozen CPU-only Step 4 diagnostic passed its local preflight. The subsequently amended final contract is `step4-diagnostic-v1.json` at SHA-256 `65966edcfc2f700323316084ec8f8ab41af1d922fadc71c505b9b44c2077d418`.

The deterministic source transformation retained 384 unique parent boards: 128 early, 128 middle, and 128 late positions across all 64 source opening groups. One real 10k-node probe passed for handcrafted, C/16, and the selected width-256 candidate. Each returned a legal move, a completed depth, the expected model identity, and exact alpha-beta plus qsearch accounting.

The restricted-label path completed an exact 250k-node child search in 1.239 seconds with the optimized local build. Feature export, full-precision checkpoint evaluation, integer-export evaluation, and the final analysis path also passed on the one-parent fixture bound to the final contract.

The user corrected the MBP endpoint to `192.168.0.12`. Its presented SSH keys exactly matched the existing `mbp` identity, and the bounded workflow completed there. The final result supersedes this preflight record and is reported in `step4-result-report.md`. No GPU was used.

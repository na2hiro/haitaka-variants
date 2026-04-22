# Roadmap

This roadmap starts after launch. Basic local play, self-play, benchmarks, and
package generation are pre-launch requirements, not future promises.

## Stronger Annan AI

- Build and publish the first useful Annan NNUE model.
- Improve search strength and time management.
- Collect player-reported bad moves and turn them into tests or training data.

## More Variants

- Document the minimum rule evidence needed to add a variant.
- Add variants incrementally with rule tests and perft-style checks.
- Keep compile-time feature flags for zero-cost rule specialization unless the
  architecture needs to change.

## Shared Training Effort

- Maintain a public model registry with compatible commits, configs, hardware,
  dataset sizes, and verification results.
- Make training reports easy to compare across contributors.
- Improve methodology for variant NNUE data generation and evaluation.

## Shogitter Integration

- Consume generated engine packages in Shogitter staging.
- Expose stronger bots to players.
- Use player games and reports to guide engine and model improvements.

## Benchmark Automation

- Publish stable benchmark baselines.
- Add automated benchmark comparison for performance-sensitive pull requests.
- Track move generation, search, DFPN, and NNUE inference separately.

# Roadmap

This roadmap starts after launch. Basic local play, self-play, benchmarks, and
package generation are pre-launch requirements, not future promises.

## Stronger AI for Annan and other variants

- Train and publish the first useful Annan NNUE model.
- Improve search strength and time management.
- Collect player-reported bad moves and turn them into tests or training data.

## More Variants

- Assess Shogitter rule list for the next variant targets to implement.
- Add variants incrementally with rule tests and perft-style checks.

## Shared Training Effort

- Maintain a public model registry with compatible commits, configs, hardware,
  dataset sizes, and verification results.
- Make training reports easy to compare across contributors.
- Improve methodology for variant NNUE data generation and evaluation.

## Shogitter Integration

- Allow NNUE to be uploaded and directly consumed by players
- Use player games and reports to guide engine and model improvements.

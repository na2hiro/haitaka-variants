use criterion::{Criterion, criterion_group, criterion_main};
use haitaka::{Board, DfpnOptions};

#[cfg(not(feature = "annan"))]
const BENCH_POSITIONS: &[(&str, &str)] = &[
    ("one-ply mate", "8k/6G2/7B1/9/9/9/9/9/K8 b R 1"),
    ("tsume", "lpg6/3s2R2/1kpppp3/p8/9/P8/2N6/9/9 b BGN 1"),
];

#[cfg(feature = "annan")]
const BENCH_POSITIONS: &[(&str, &str)] = &[
    ("one-ply mate", "8k/6G2/7B1/9/9/9/9/9/K8 b R 1"),
    ("one-ply no mate", "4k4/9/9/9/9/9/9/9/4K4 b - 1"),
];

fn bench_dfpn(c: &mut Criterion) {
    let options = DfpnOptions::default();

    for &(name, sfen) in BENCH_POSITIONS {
        let board = Board::from_sfen(sfen)
            .or_else(|_| Board::tsume(sfen))
            .expect("benchmark SFEN should be valid");
        c.bench_function(name, |b| {
            b.iter(|| {
                let _ = board.dfpn(&options);
            });
        });
    }
}

criterion_group!(benches, bench_dfpn);
criterion_main!(benches);

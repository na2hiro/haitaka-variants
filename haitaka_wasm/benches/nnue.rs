use std::path::PathBuf;
use std::sync::Arc;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use haitaka::Board;
use haitaka_wasm::{
    NnueModel, SearchEvalMode, search_impl_handcrafted, search_impl_with_eval_mode,
    search_iterative_deepening_impl, search_iterative_deepening_impl_with_dfpn_mode,
};

fn load_test_nnue() -> Option<Arc<NnueModel>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shogi-878ca61334a7.nnue");
    let bytes = std::fs::read(path).ok()?;
    let model = NnueModel::from_bytes(&bytes).ok()?;
    Some(Arc::new(model))
}

fn standard_positions() -> Vec<&'static str> {
    vec![
        haitaka::SFEN_STARTPOS,
        haitaka::SFEN_6PIECE_HANDICAP,
        "lnsgkgsnl/1r5b1/ppppp1ppp/5p3/4P4/9/PPPP1PPPP/1B5R1/LNSGKGSNL w - 2",
        "lnsgk1snl/1r3g1b1/pppppp1pp/6p2/4P4/2P6/PP1P1PPPP/1B5R1/LNSGKGSNL b - 1",
        "9/9/k8/9/4Rr3/9/9/9/4K4 b - 1",
    ]
}

fn eval_positions(model: &NnueModel) -> Vec<(Board, haitaka_wasm::NnuePositionState)> {
    standard_positions()
        .into_iter()
        .map(|sfen| {
            let board = Board::from_sfen(sfen).unwrap();
            let state = model.build_position_state_full(&board);
            (board, state)
        })
        .collect()
}

fn criterion_benchmark(criterion: &mut Criterion) {
    let Some(model) = load_test_nnue() else {
        return;
    };

    let eval_positions = eval_positions(&model);

    for (board, state) in &eval_positions {
        assert_eq!(
            model.evaluate_full_refresh(board),
            model.evaluate_from_state(board, state)
        );
    }

    let mut eval_group = criterion.benchmark_group("nnue_eval");
    eval_group.bench_function("full_refresh", |b| {
        b.iter(|| {
            for (board, _) in &eval_positions {
                black_box(model.evaluate_full_refresh(black_box(board)));
            }
        });
    });
    eval_group.bench_function("incremental_state", |b| {
        b.iter(|| {
            for (board, state) in &eval_positions {
                black_box(model.evaluate_from_state(black_box(board), black_box(state)));
            }
        });
    });
    eval_group.finish();

    let mut search_group = criterion.benchmark_group("nnue_search");
    for &(name, sfen, depth) in &[
        ("startpos_d3", haitaka::SFEN_STARTPOS, 3u8),
        ("startpos_d4", haitaka::SFEN_STARTPOS, 4u8),
        ("tactical_d3", "9/9/k8/9/4Rr3/9/9/9/4K4 b - 1", 3u8),
    ] {
        let full_refresh =
            search_impl_with_eval_mode(sfen, depth, model.clone(), SearchEvalMode::FullRefresh)
                .unwrap();
        let incremental =
            search_impl_with_eval_mode(sfen, depth, model.clone(), SearchEvalMode::Incremental)
                .unwrap();
        assert_eq!(
            incremental.best_move, full_refresh.best_move,
            "best move parity for {name}"
        );
        assert_eq!(
            incremental.best_score, full_refresh.best_score,
            "score parity for {name}"
        );

        search_group.bench_function(format!("{name}_full_refresh"), |b| {
            b.iter(|| {
                black_box(
                    search_impl_with_eval_mode(
                        black_box(sfen),
                        black_box(depth),
                        model.clone(),
                        SearchEvalMode::FullRefresh,
                    )
                    .unwrap(),
                );
            });
        });
        search_group.bench_function(format!("{name}_incremental"), |b| {
            b.iter(|| {
                black_box(
                    search_impl_with_eval_mode(
                        black_box(sfen),
                        black_box(depth),
                        model.clone(),
                        SearchEvalMode::Incremental,
                    )
                    .unwrap(),
                );
            });
        });
    }
    search_group.finish();

    let mut iterative_group = criterion.benchmark_group("iterative_search");
    for &(name, sfen, depth) in &[
        ("startpos_d4", haitaka::SFEN_STARTPOS, 4u8),
        ("tactical_d3", "9/9/k8/9/4Rr3/9/9/9/4K4 b - 1", 3u8),
    ] {
        let fixed = search_impl_handcrafted(sfen, depth).unwrap();
        let iterative = search_iterative_deepening_impl(sfen, depth, 5_000).unwrap();
        assert_eq!(
            iterative.best_move, fixed.best_move,
            "iterative parity for {name}"
        );
        assert_eq!(
            iterative.completed_depth, depth,
            "completed depth for {name}"
        );

        iterative_group.bench_function(format!("{name}_fixed"), |b| {
            b.iter(|| {
                black_box(search_impl_handcrafted(black_box(sfen), black_box(depth)).unwrap());
            });
        });
        iterative_group.bench_function(format!("{name}_iterative"), |b| {
            b.iter(|| {
                black_box(
                    search_iterative_deepening_impl(black_box(sfen), black_box(depth), 5_000)
                        .unwrap(),
                );
            });
        });
    }

    let mate_sfen = "8k/6G2/7B1/9/9/9/9/9/K8 b R 1";
    let dfpn_enabled =
        search_iterative_deepening_impl_with_dfpn_mode(mate_sfen, 4, 5_000, true).unwrap();
    let dfpn_disabled =
        search_iterative_deepening_impl_with_dfpn_mode(mate_sfen, 4, 5_000, false).unwrap();
    assert_eq!(dfpn_enabled.completed_depth, 0);
    assert!(dfpn_enabled.dfpn.as_ref().is_some_and(|dfpn| dfpn.selected));
    assert!(dfpn_disabled.dfpn.is_none());
    assert!(dfpn_enabled.best_move.is_some());
    assert!(dfpn_disabled.best_move.is_some());

    iterative_group.bench_function("mate_dfpn_enabled", |b| {
        b.iter(|| {
            black_box(
                search_iterative_deepening_impl_with_dfpn_mode(
                    black_box(mate_sfen),
                    4,
                    5_000,
                    true,
                )
                .unwrap(),
            );
        });
    });
    iterative_group.bench_function("mate_dfpn_disabled", |b| {
        b.iter(|| {
            black_box(
                search_iterative_deepening_impl_with_dfpn_mode(
                    black_box(mate_sfen),
                    4,
                    5_000,
                    false,
                )
                .unwrap(),
            );
        });
    });
    iterative_group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);

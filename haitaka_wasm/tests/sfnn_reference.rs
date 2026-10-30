//! Explicit real-model verification: never silently passes without model files.
#![cfg(not(target_arch = "wasm32"))]
use haitaka::Board;
use haitaka_wasm::sfnn::{MICRO_ARCH, SfnnModel};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, str::FromStr};

#[test]
#[ignore = "requires fetched micro model; run scripts/tuyouraou11.py verify"]
fn micro_matches_external_reference_and_rejects_corruption() {
    let dir = PathBuf::from(
        std::env::var_os("HAITAKA_SFNN_MODEL_DIR").expect("HAITAKA_SFNN_MODEL_DIR required"),
    );
    let nn = std::fs::read(dir.join("nn.bin")).expect("nn.bin required");
    let progress = std::fs::read(dir.join("progress.bin")).expect("progress.bin required");
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../plans/tuyouraou11/reference-fixture.json"
    ))
    .unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&nn)),
        fixtures["model"]["nnSha256"].as_str().unwrap()
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(&progress)),
        fixtures["model"]["progressSha256"].as_str().unwrap()
    );
    let model = SfnnModel::from_bytes(&nn, &progress, MICRO_ARCH).unwrap();
    let cases = fixtures["cases"].as_array().unwrap();
    assert!(cases.len() >= 64);
    for case in cases {
        let board = Board::from_str(case["sfen"].as_str().unwrap()).unwrap();
        let actual = model.evaluate_full_refresh(&board);
        let expected = &case["progress"];
        let id = case["id"].as_str().unwrap();
        assert_eq!(
            actual.progress_sums_q16[0].to_string(),
            expected["sumsQ16"]["black"].as_str().unwrap(),
            "{id}: black progress"
        );
        assert_eq!(
            actual.progress_sums_q16[1].to_string(),
            expected["sumsQ16"]["white"].as_str().unwrap(),
            "{id}: white progress"
        );
        assert_eq!(
            actual.selected_sum_q16.to_string(),
            expected["selectedSumQ16"].as_str().unwrap(),
            "{id}: selected progress"
        );
        assert_eq!(
            actual.bucket,
            expected["bucket"].as_u64().unwrap() as usize,
            "{id}: bucket"
        );
        assert_eq!(
            i64::from(actual.eval_cp),
            case["evalCp"].as_i64().unwrap(),
            "{id}: eval; raw={}",
            actual.raw_output
        );
    }
    let reject_nn = |b: &[u8]| {
        let e = SfnnModel::from_bytes(b, &progress, MICRO_ARCH)
            .err()
            .expect("must reject NN");
        assert!(e.0.starts_with("SFNN_INVALID_NN:"), "{e}");
    };
    reject_nn(&nn[..nn.len() - 1]);
    let mut bad = nn.clone();
    bad.push(0);
    reject_nn(&bad);
    for at in [0, 4, 12, 105, 109] {
        let mut bad = nn.clone();
        bad[at] ^= 0x01;
        reject_nn(&bad);
    }
    let reject_progress = |b: &[u8]| {
        let e = SfnnModel::from_bytes(&nn, b, MICRO_ARCH)
            .err()
            .expect("must reject progress");
        assert!(e.0.starts_with("SFNN_INVALID_PROGRESS:"), "{e}");
    };
    reject_progress(&[]);
    reject_progress(&progress[..progress.len() - 1]);
    let mut bad = progress.clone();
    bad.push(0);
    reject_progress(&bad);
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut bad = progress.clone();
        bad[..8].copy_from_slice(&v.to_le_bytes());
        reject_progress(&bad);
    }
    for arch in ["", "SFNN_ka2_4096_8_64_progress8", "HalfKAv2^"] {
        assert!(
            SfnnModel::from_bytes(&nn, &progress, arch)
                .err()
                .unwrap()
                .0
                .starts_with("SFNN_UNSUPPORTED_ARCH:")
        );
    }
    println!(
        "matched {} external reference positions exactly; real-file corruption checks passed",
        cases.len()
    );
}

#[test]
#[ignore = "requires micro; explicit P2 verification"]
fn incremental_states_and_search_match_full_refresh() {
    use haitaka::{Move, Piece};
    use haitaka_wasm::{SearchEvalMode, search_impl_sfnn};
    use std::sync::Arc;
    let dir = PathBuf::from(
        std::env::var_os("HAITAKA_SFNN_MODEL_DIR").expect("model directory required"),
    );
    let model = Arc::new(
        SfnnModel::from_bytes(
            &std::fs::read(dir.join("nn.bin")).unwrap(),
            &std::fs::read(dir.join("progress.bin")).unwrap(),
            MICRO_ARCH,
        )
        .unwrap(),
    );
    let mut rng = 1u64;
    let (mut nodes, mut captures, mut drops, mut promotions, mut kings, mut bucket_changes) =
        (0, 0, 0, 0, 0, 0);
    let mut corpus = Vec::new();
    for _ in 0..32 {
        let mut board = Board::startpos();
        let mut state = model.build_position_state_full(&board);
        let mut moves = Vec::new();
        let trace = |b: &Board, s: &haitaka_wasm::sfnn::SfnnPositionState| {
            let e = model.evaluate_from_state(b, s);
            serde_json::json!({"progress": {"sumsQ16": {"black": e.progress_sums_q16[0].to_string(),
                "white": e.progress_sums_q16[1].to_string()}, "selectedSumQ16": e.selected_sum_q16.to_string(),
                "bucket": e.bucket}, "evalCp": e.eval_cp, "fullRefreshMatches": true})
        };
        let mut traces = vec![trace(&board, &state)];
        for _ in 0..256 {
            let mut legal = Vec::new();
            board.generate_moves(|m| {
                legal.extend(m);
                false
            });
            if legal.is_empty() {
                break;
            }
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let mv = legal[(rng % legal.len() as u64) as usize];
            captures += usize::from(board.colored_piece_on(mv.to()).is_some());
            drops += usize::from(mv.is_drop());
            promotions += usize::from(mv.is_promotion());
            if let Move::BoardMove { from, .. } = mv {
                kings += usize::from(board.colored_piece_on(from).unwrap().piece == Piece::King);
            }
            // Verify an independent sibling from the same parent (state must not leak).
            if nodes % 32 == 0 {
                let sibling_mv = *legal.last().unwrap();
                let mut sibling = board.clone();
                sibling.play(sibling_mv);
                let sibling_state = model.apply_move(&board, &sibling, &state, sibling_mv);
                assert_eq!(sibling_state, model.build_position_state_full(&sibling));
            }
            let mut child = board.clone();
            child.play(mv);
            let child_state = model.apply_move(&board, &child, &state, mv);
            let full = model.build_position_state_full(&child);
            assert_eq!(child_state, full, "node {nodes}, move {mv}, parent {board}");
            let actual = model.evaluate_from_state(&child, &child_state);
            assert_eq!(actual, model.evaluate_full_refresh(&child));
            bucket_changes +=
                usize::from(actual.bucket != model.evaluate_from_state(&board, &state).bucket);
            state = child_state;
            board = child;
            nodes += 1;
            moves.push(mv.to_string());
            traces.push(trace(&board, &state));
            if nodes >= 2048 {
                break;
            }
        }
        corpus.push(serde_json::json!({"startSfen": Board::startpos().to_string(), "moves": moves, "traces": traces}));
        if nodes >= 2048 {
            break;
        }
    }
    assert_eq!(nodes, 2048);
    assert!(captures > 0 && drops > 0 && promotions > 0 && kings > 0 && bucket_changes > 0);
    if let Some(path) = std::env::var_os("HAITAKA_SFNN_CORPUS") {
        std::fs::write(path, serde_json::to_vec(&corpus).unwrap()).unwrap();
    }
    println!(
        "SFNN delta: nodes={nodes} captures={captures} drops={drops} promotions={promotions} king_moves={kings} bucket_changes={bucket_changes}"
    );
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../plans/tuyouraou11/reference-fixture.json"
    ))
    .unwrap();
    for index in [0, 8, 24, 100] {
        let sfen = fixture["cases"][index]["sfen"].as_str().unwrap();
        let full = search_impl_sfnn(sfen, 2, model.clone(), SearchEvalMode::FullRefresh).unwrap();
        let incremental =
            search_impl_sfnn(sfen, 2, model.clone(), SearchEvalMode::Incremental).unwrap();
        assert_eq!(
            full.root_result, incremental.root_result,
            "search root {index}"
        );
        assert_eq!(full.states, incremental.states, "search nodes {index}");
        let board = Board::from_str(sfen).unwrap();
        assert!(board.is_legal(incremental.best_move.unwrap().parse().unwrap()));
    }
}

//! Explicit real-model compatibility checks; these are not strength tests.
#![cfg(all(feature = "suika", not(target_arch = "wasm32")))]
use haitaka::{Board, Color, Move, Piece};
use haitaka_wasm::sfnn::{MICRO_ARCH, SfnnModel};
use haitaka_wasm::{SearchEvalMode, UsiSession, search_impl_sfnn};
use std::{path::PathBuf, sync::Arc};

fn model() -> Arc<SfnnModel> {
    let dir = PathBuf::from(
        std::env::var_os("HAITAKA_SFNN_MODEL_DIR").expect("real micro model required"),
    );
    Arc::new(
        SfnnModel::from_bytes(
            &std::fs::read(dir.join("nn.bin")).unwrap(),
            &std::fs::read(dir.join("progress.bin")).unwrap(),
            MICRO_ARCH,
        )
        .unwrap(),
    )
}

#[test]
#[ignore = "requires HAITAKA_SFNN_MODEL_DIR with real micro model"]
fn suika_sfnn_real_model_handles_fusion_drops_and_rollouts() {
    let model = model();
    let mut board = Board::from_sfen("8k/9/9/4p4/4R4/9/9/9/K8 b GSNLP 1").unwrap();
    let mut state = model.build_position_state_full(&board);
    for text in ["5e5d", "1a1b", "B*4f"] {
        let mv: Move = text.parse().unwrap();
        let mut child = board.clone();
        child.try_play(mv).unwrap();
        state = model.apply_move(&board, &child, &state, mv);
        assert_eq!(state, model.build_position_state_full(&child));
        assert_eq!(
            model.evaluate_from_state(&child, &state),
            model.evaluate_full_refresh(&child)
        );
        board = child;
    }
    let high_rooks =
        Board::from_sfen("8k/RRRRRRRRR/RRRRRRRRR/RRRRRRRRR/RRRRRRRRR/RR7/9/9/K8 b - 1").unwrap();
    model.evaluate_full_refresh(&high_rooks);
    let mut random = 155u64;
    let mut checked = 0;
    let mut terminal = 0;
    let mut captures = 0;
    let mut drops = 0;
    let mut promotions = 0;
    for _ in 0..32 {
        let mut board = Board::startpos();
        let mut state = model.build_position_state_full(&board);
        for _ in 0..128 {
            let mut moves = Vec::new();
            board.generate_moves(|group| {
                moves.extend(group);
                false
            });
            if moves.is_empty() {
                break;
            }
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let mv = moves[random as usize % moves.len()];
            match mv {
                Move::Drop { .. } => drops += 1,
                Move::BoardMove { to, promotion, .. } => {
                    captures += u32::from(board.colored_piece_on(to).is_some());
                    promotions += u32::from(promotion);
                }
            }
            let mut child = board.clone();
            child.try_play(mv).unwrap();
            if child.winner().is_some() {
                terminal += 1;
                let result = search_impl_sfnn(
                    &child.to_string(),
                    1,
                    model.clone(),
                    SearchEvalMode::Incremental,
                )
                .unwrap();
                assert!(result.best_move.is_none());
                break;
            }
            state = model.apply_move(&board, &child, &state, mv);
            assert_eq!(state, model.build_position_state_full(&child));
            assert_eq!(
                model.evaluate_from_state(&child, &state),
                model.evaluate_full_refresh(&child)
            );
            checked += 1;
            board = child;
        }
    }
    assert!(checked >= 512 && captures > 0 && drops > 0 && promotions > 0 && terminal > 0);
    println!(
        "{checked} nonterminal transitions, {captures} captures, {drops} drops, {promotions} promotions, {terminal} terminal positions checked"
    );
}

#[test]
#[ignore = "requires HAITAKA_SFNN_MODEL_DIR with real micro model"]
fn suika_sfnn_search_prioritizes_king_acquisition_over_static_eval() {
    let model = model();
    for sfen in [
        // Multiple fusions yield a King without capturing the physical King.
        "8k/9/9/4p4/4R4/9/9/9/K8 b RBGSNLP 1",
        // The winning capture remains legal even while our King is attacked.
        "r7k/9/9/4p4/4R4/9/9/9/K8 b RBGSNLP 1",
        // Direct capture removes the opposing physical King.
        "9/9/9/4k4/4R4/9/9/9/K8 b - 1",
    ] {
        let board = Board::from_sfen(sfen).unwrap();
        let mut child = board.clone();
        child.try_play("5e5d".parse().unwrap()).unwrap();
        assert_eq!(child.num_in_hand(Color::Black, Piece::King), 1);
        assert_eq!(child.winner(), Some(Color::Black));
        for mode in [SearchEvalMode::Incremental, SearchEvalMode::FullRefresh] {
            let result = search_impl_sfnn(sfen, 1, model.clone(), mode).unwrap();
            assert_eq!(result.best_move.as_deref(), Some("5e5d"));
            assert_eq!(result.best_score, Some(29999));
            let terminal = search_impl_sfnn(&child.to_string(), 1, model.clone(), mode).unwrap();
            assert_eq!(terminal.best_move, None);
            assert_eq!(terminal.best_score, Some(-30000));
        }
        let dir = PathBuf::from(std::env::var_os("HAITAKA_SFNN_MODEL_DIR").unwrap());
        let mut session = UsiSession::default();
        session
            .load_sfnn(
                &std::fs::read(dir.join("nn.bin")).unwrap(),
                &std::fs::read(dir.join("progress.bin")).unwrap(),
                MICRO_ARCH,
            )
            .unwrap();
        assert!(
            session
                .handle_line(&format!("position sfen {sfen} moves 5e5d"))
                .is_empty()
        );
        assert_eq!(
            session.handle_line("go depth 1").last().unwrap(),
            "bestmove resign"
        );
    }
}

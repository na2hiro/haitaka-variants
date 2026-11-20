#![cfg(all(feature = "heian-9x9", not(target_arch = "wasm32")))]
use haitaka::{Board, Color, Move, Piece, RuleSpec, SelectedRule};
use haitaka_wasm::sfnn::{MICRO_ARCH, MINI_ARCH, SfnnModel};
use haitaka_wasm::{SearchEvalMode, UsiSession, search_impl_sfnn};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};

fn model() -> (Arc<SfnnModel>, Vec<u8>, Vec<u8>, &'static str) {
    let mini = std::env::var("HAITAKA_SFNN_PROFILE").as_deref() == Ok("mini");
    let dir = PathBuf::from(
        std::env::var_os("HAITAKA_SFNN_MODEL_DIR").expect("real model directory required"),
    );
    let nn = std::fs::read(dir.join("nn.bin")).unwrap();
    let progress = std::fs::read(dir.join("progress.bin")).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&nn)),
        if mini {
            "45323c65135da65d6c2c364eea93a5af6a285c27ebfb07eab4afbe737ef74508"
        } else {
            "7f9545dbf73b87acf312146daabf59c4f17d0845c87d587fa19caed51707dddb"
        }
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(&progress)),
        "a2d18f0564639c945b5d6469e9b437cd64a526ad4fbcef02ab1d8f547d08e5f1"
    );
    let arch = if mini { MINI_ARCH } else { MICRO_ARCH };
    (
        Arc::new(SfnnModel::from_bytes(&nn, &progress, arch).unwrap()),
        nn,
        progress,
        arch,
    )
}

#[test]
#[ignore = "requires verified micro or mini model files"]
fn heian_sfnn_incremental_matches_refresh_through_captures_and_gold_promotions() {
    let (model, _, _, _) = model();
    let mut seed = 0x571ab13_u64;
    let (mut checked, mut captures, mut promotions) = (0, [0; 2], [0; 2]);
    for _ in 0..12 {
        let mut board = Board::startpos();
        let mut state = model.build_position_state_full(&board);
        for _ in 0..192 {
            let mut moves = Vec::new();
            board.generate_moves(|group| {
                moves.extend(group);
                false
            });
            if moves.is_empty() {
                break;
            }
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let mv = moves[seed as usize % moves.len()];
            let color = board.side_to_move() as usize;
            if board.piece_on(mv.to()).is_some() {
                captures[color] += 1;
            }
            if mv.is_promotion() {
                promotions[color] += 1;
            }
            let mut child = board.clone();
            child.try_play(mv).unwrap();
            let next = model.apply_move(&board, &child, &state, mv);
            assert_eq!(
                next,
                model.build_position_state_full(&child),
                "{board} {mv}"
            );
            assert_eq!(
                model.evaluate_from_state(&child, &next),
                model.evaluate_full_refresh(&child)
            );
            assert!(Color::ALL.iter().all(|&c| child.is_hand_empty(c)));
            state = next;
            board = child;
            checked += 1;
        }
    }
    assert!(
        checked >= 1000 && captures.iter().all(|&n| n > 0) && promotions.iter().all(|&n| n > 0)
    );
    println!("{checked} exact transitions; captures={captures:?}; gold promotions={promotions:?}");
}

#[test]
#[ignore = "requires verified micro or mini model files"]
fn heian_sfnn_search_usi_and_only_kings_draw_match_both_modes() {
    let (model, nn, progress, arch) = model();
    for sfen in [SelectedRule::STARTPOS, "8k/9/6g2/6S2/9/9/9/9/K8 b - 1"] {
        let a = search_impl_sfnn(sfen, 2, model.clone(), SearchEvalMode::Incremental).unwrap();
        let b = search_impl_sfnn(sfen, 2, model.clone(), SearchEvalMode::FullRefresh).unwrap();
        assert_eq!(a.best_move, b.best_move);
        assert_eq!(a.best_score, b.best_score);
        assert!(
            Board::from_sfen(sfen)
                .unwrap()
                .is_legal(a.best_move.unwrap().parse::<Move>().unwrap())
        );
    }
    let draw = "8k/9/9/9/9/9/9/9/K8 b - 1";
    for mode in [SearchEvalMode::Incremental, SearchEvalMode::FullRefresh] {
        let result = search_impl_sfnn(draw, 2, model.clone(), mode).unwrap();
        assert_eq!(result.best_score, Some(0));
        assert_eq!(result.best_move, None);
    }
    let mut session = UsiSession::default();
    session.load_sfnn(&nn, &progress, arch).unwrap();
    assert!(
        session
            .handle_line("position startpos moves 7g7f 3c3d")
            .is_empty()
    );
    let board = Board::from_sfen(&session.board_sfen()).unwrap();
    assert!(
        session
            .handle_line("go nodes 1000")
            .iter()
            .filter_map(|s| s.strip_prefix("bestmove "))
            .any(|m| board.is_legal(m.parse().unwrap()))
    );
    assert!(
        session
            .handle_line(&format!("position sfen {draw}"))
            .is_empty()
    );
    let lines = session.handle_line("go depth 2");
    assert!(lines.iter().any(|s| s.contains("only-kings-draw")));
    assert!(lines.iter().any(|s| s == "bestmove resign"));
    assert!(Board::from_sfen(draw).unwrap().pieces(Piece::King).len() == 2);
}

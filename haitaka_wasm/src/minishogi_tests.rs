use super::*;

fn legal_best(board: &Board, best: &Option<String>) {
    let mv: Move = best
        .as_deref()
        .expect("search should choose a move")
        .parse()
        .unwrap();
    assert!(board.is_legal(mv), "illegal bestmove {mv}");
}

#[test]
fn direct_search_and_perft_use_the_selected_geometry() {
    let board = Board::startpos();
    assert_eq!(perft_bulk(&board, 5), 533_203);
    let result = search_impl_handcrafted(haitaka::SFEN_STARTPOS, 2).unwrap();
    legal_best(&board, &result.best_move);
    assert!(result.root_result.play_move_was_searched);
}

#[test]
fn fixed_node_budget_is_respected_and_returns_a_legal_move() {
    let board = Board::startpos();
    let mut workspace = SearchWorkspace::default();
    let result =
        search_board_impl_handcrafted_with_node_budget_in_workspace(&board, 100, 8, &mut workspace)
            .unwrap();
    legal_best(&board, &result.best_move);
    assert!(result.total_nodes <= 100);
    assert!(result.exhausted);
}

#[test]
fn usi_position_movetime_hash_and_invalid_position_are_isolated() {
    let mut session = UsiSession::default();
    assert!(session.handle_line("usi").iter().any(|s| s == "usiok"));
    assert_eq!(session.handle_line("isready"), ["readyok"]);
    assert!(
        session
            .handle_line("setoption name Hash value 1")
            .is_empty()
    );
    assert!(
        session
            .handle_line("position startpos moves 5d5c")
            .is_empty()
    );
    let board = Board::from_sfen(&session.board_sfen()).unwrap();
    let output = session.handle_line("go movetime 1");
    let text = output
        .iter()
        .find_map(|s| s.strip_prefix("bestmove "))
        .unwrap();
    assert!(board.is_legal(text.parse().unwrap()));
    let previous = session.board_sfen();
    assert!(
        session.handle_line("position sfen 9/9/9/9/9/9/9/9/9 b - 1")[0]
            .contains("invalid position")
    );
    assert_eq!(session.board_sfen(), previous);
}

#[test]
fn history_is_used_by_usi_and_search() {
    let sfen = "4k/5/5/5/K4 b - 1";
    let cycle = "5e5d 1a1b 5d5e 1b1a ".repeat(3);
    let command = format!("position sfen {sfen} moves {}", cycle.trim());
    let (board, history) = parse_usi_position_with_history(&command).unwrap();
    assert_eq!(history.adjudication(), HistoryAdjudication::RepetitionDraw);
    let result = search_board_impl_handcrafted_with_history(&board, &history, 2).unwrap();
    assert!(result.best_move.is_none());
    assert_eq!(result.best_score, Some(0));
    let mut session = UsiSession::default();
    assert!(session.handle_line(&command).is_empty());
    let lines = session.handle_line("go depth 2");
    assert!(lines.iter().any(|line| {
        line.contains("repetition-draw rules minishogi-shogitter-history-adjudication-v1")
    }));
    assert!(lines.iter().any(|line| line == "bestmove resign"));
}

#[test]
fn mate_and_model_rejection_are_consistent_across_entrypoints() {
    let result = search_impl_handcrafted("4k/3G1/3R1/5/K4 w - 1", 2).unwrap();
    assert!(result.best_move.is_none());
    assert_eq!(
        Board::from_sfen("4k/3G1/3R1/5/K4 w - 1").unwrap().status(),
        haitaka::GameStatus::Won
    );
    assert!(
        NnueModel::from_bytes(&[])
            .unwrap_err()
            .to_string()
            .contains("UNSUPPORTED_RULESET")
    );
    assert!(
        SfnnModel::from_bytes(&[], &[], sfnn::MINI_ARCH)
            .unwrap_err()
            .to_string()
            .contains("UNSUPPORTED_RULESET")
    );
    assert!(
        load_nnue_impl(&[])
            .unwrap_err()
            .contains("UNSUPPORTED_RULESET")
    );
    assert!(
        load_sfnn_impl(&[], &[], sfnn::MICRO_ARCH)
            .unwrap_err()
            .contains("UNSUPPORTED_RULESET")
    );
    let mut session = UsiSession::default();
    assert!(
        session
            .load_nnue(&[])
            .unwrap_err()
            .contains("UNSUPPORTED_RULESET")
    );
    assert!(
        session
            .load_sfnn(&[], &[], sfnn::MINI_ARCH)
            .unwrap_err()
            .contains("UNSUPPORTED_RULESET")
    );
}

#[test]
fn tt_moves_roundtrip_all_compact_squares_without_widening_entries() {
    use crate::tt::PackedMove;
    for from in haitaka::Square::ALL {
        for to in haitaka::Square::ALL {
            if from == to {
                continue;
            }
            for promotion in [false, true] {
                let mv = Move::BoardMove {
                    from,
                    to,
                    promotion,
                };
                assert_eq!(PackedMove::from_move(mv).to_move(), Some(mv));
            }
        }
    }
    assert_eq!(std::mem::size_of::<PackedMove>(), 2);
}

use super::*;
use haitaka::RuleSpec;

// Independently obtained with Fairy-Stockfish go perft 3.
fn reference_perft3() -> u64 {
    match haitaka::SelectedRule::NAME {
        "minishogi" => 2512,
        "judkins" => 6183,
        "gorogoro" => 4166,
        "gorogoro-plus" => 44436,
        "minishogi-kingin" => 46127,
        _ => unreachable!(),
    }
}

#[test]
fn selected_startpos_perft_and_search_match_geometry() {
    let board = Board::startpos();
    assert_eq!(perft_bulk(&board, 3), reference_perft3());
    let result = search_impl_handcrafted(haitaka::SelectedRule::STARTPOS, 2).unwrap();
    let mv: Move = result.best_move.unwrap().parse().unwrap();
    assert!(board.is_legal(mv));
}

#[test]
fn selected_usi_startpos_and_model_rejection_work() {
    let mut session = UsiSession::default();
    assert_eq!(session.board_sfen(), haitaka::SelectedRule::STARTPOS);
    assert!(session.handle_line("position startpos").is_empty());
    let output = session.handle_line("go depth 2");
    let best = output
        .iter()
        .find_map(|line| line.strip_prefix("bestmove "))
        .unwrap();
    assert!(Board::startpos().is_legal(best.parse().unwrap()));
    assert!(
        NnueModel::from_bytes(&[])
            .unwrap_err()
            .to_string()
            .contains("UNSUPPORTED_RULESET")
    );
    assert!(
        SfnnModel::from_bytes(&[], &[], sfnn::MICRO_ARCH)
            .unwrap_err()
            .to_string()
            .contains("UNSUPPORTED_RULESET")
    );
}

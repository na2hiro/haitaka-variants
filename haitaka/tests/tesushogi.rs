#![cfg(feature = "tesushogi")]
use haitaka::{
    Board, Color, DfpnInterruptionReason, DfpnOptions, DfpnStatus, HistoryAdjudication, Move,
    PositionHistory,
};

fn at_ply(sfen: &str, next_move: u16, limit: u16) -> Board {
    let mut board = Board::from_sfen(sfen).unwrap();
    board.set_move_number(next_move);
    board.set_move_limit(limit).unwrap();
    board
}

#[test]
fn tesushogi_deadline_for_all_three_limits() {
    for limit in [49, 99, 149] {
        let mut board = at_ply(haitaka::SFEN_STARTPOS, limit, limit);
        assert_eq!(board.immediate_winner(), None);
        board.try_play("7g7f".parse::<Move>().unwrap()).unwrap();
        assert_eq!(board.winner(), Some(Color::White));
        assert!(!board.generate_moves(|_| true));
        assert!(!board.is_legal("3c3d".parse().unwrap()));
        assert!(board.try_play("3c3d".parse().unwrap()).is_err());
    }
}

#[test]
fn tesushogi_final_move_mate_precedes_deadline_but_check_does_not() {
    for limit in [49, 99, 149] {
        let mut mate = at_ply("8k/6G2/7B1/9/9/9/9/9/K8 b R 1", limit, limit);
        mate.try_play("R*1b".parse().unwrap()).unwrap();
        assert_eq!(mate.winner(), Some(Color::Black));
        mate.set_move_number(limit + 2);
        assert_eq!(mate.winner(), Some(Color::White));
        let mut check = at_ply("8k/9/9/9/9/9/9/9/K8 b R 1", limit, limit);
        check.try_play("R*1d".parse().unwrap()).unwrap();
        assert!(!check.checkers().is_empty());
        assert_eq!(check.winner(), Some(Color::White));
    }
}

#[test]
fn tesushogi_keys_include_elapsed_moves_and_limit() {
    let original = Board::startpos();
    let mut later = original.clone();
    later.set_move_number(5);
    let mut longer = original.clone();
    longer.set_move_limit(99).unwrap();
    for other in [&later, &longer] {
        assert_ne!(original.hash(), other.hash());
        assert!(!original.same_position(other));
        assert_ne!(
            PositionHistory::new(original.clone()).tt_key(),
            PositionHistory::new(other.clone()).tt_key()
        );
    }
    assert_eq!(longer.color_swapped().move_limit(), 99);
    for bad in [0, 1, 48, 50, 100, 150, u16::MAX] {
        assert!(longer.set_move_limit(bad).is_err());
        assert_eq!(longer.move_limit(), 99);
    }
}

#[test]
fn tesushogi_repetition_disabled_and_dfpn_unsupported() {
    let board = Board::startpos();
    let mut history = PositionHistory::new(board.clone());
    for _ in 0..4 {
        history.push(board.clone());
    }
    assert_eq!(history.adjudication(), HistoryAdjudication::Ongoing);
    for result in [
        board.dfpn(&DfpnOptions::default()),
        board
            .dfpn_with_history(&DfpnOptions::default(), &history)
            .unwrap(),
    ] {
        assert_eq!(result.status, DfpnStatus::Unknown);
        assert!(!result.completed);
        assert_eq!(result.stats.nodes, 0);
        assert_eq!(
            result.interruption_reason,
            Some(DfpnInterruptionReason::UnsupportedRuleset)
        );
    }
}

#[test]
fn tesushogi_final_move_keeps_pawn_drop_mate_legality() {
    for limit in [49, 99, 149] {
        let illegal = at_ply("8k/6G2/7B1/9/9/9/9/9/K8 b P 1", limit, limit);
        assert!(!illegal.is_legal("P*1b".parse().unwrap()));
        let mut legal = at_ply("8k/9/9/9/9/9/9/9/K8 b P 1", limit, limit);
        legal.try_play("P*1b".parse().unwrap()).unwrap();
        assert_eq!(legal.winner(), Some(Color::White));
    }
}

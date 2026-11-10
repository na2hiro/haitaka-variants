use super::*;
use haitaka::{File, Rank, Square};
fn position(pieces: &[(&str, char)], side: Color) -> String {
    let mut cells = vec![vec![None; File::NUM]; Rank::NUM];
    for &(name, piece) in pieces {
        let sq: Square = name.parse().unwrap();
        cells[sq.rank() as usize][sq.file() as usize] = Some(piece);
    }
    let rows = cells
        .iter()
        .map(|row| {
            let mut text = String::new();
            let mut empty = 0;
            for cell in row.iter().rev() {
                if let Some(piece) = cell {
                    if empty > 0 {
                        text.push(char::from_digit(empty, 10).unwrap());
                        empty = 0;
                    }
                    text.push(*piece);
                } else {
                    empty += 1;
                }
            }
            if empty > 0 {
                text.push(char::from_digit(empty, 10).unwrap());
            }
            text
        })
        .collect::<Vec<_>>()
        .join("/");
    format!(
        "{rows} {side} - {}",
        if side == Color::Black { 1 } else { 2 }
    )
}
#[test]
fn selected_geometry_perft_and_direct_search_are_legal() {
    let board = Board::startpos();
    assert_eq!(perft_bulk(&board, 1), if File::NUM == 8 { 22 } else { 26 });
    assert_eq!(
        perft_bulk(&board, 2),
        if File::NUM == 8 { 484 } else { 676 }
    );
    assert_eq!(
        perft_bulk(&board, 3),
        if File::NUM == 8 { 10296 } else { 17108 }
    );
    let result = search_iterative_deepening_impl(SelectedRule::STARTPOS, 2, 1000).unwrap();
    assert!(board.is_legal(result.best_move.unwrap().parse().unwrap()));
    assert!(result.dfpn.is_none());
}
#[test]
fn rule_draw_has_zero_score_and_no_fallback_or_moves() {
    let draw = position(&[("1a", 'k'), ("5e", 'K')], Color::Black);
    for depth in [1, 3] {
        let result = search_impl_handcrafted(&draw, depth).unwrap();
        assert_eq!(result.best_score, Some(0));
        assert!(result.best_move.is_none());
        assert!(!result.root_result.emergency_fallback_used);
    }
    let result = search_iterative_deepening_impl(&draw, 3, 1000).unwrap();
    assert_eq!(result.best_score, Some(0));
    assert!(result.best_move.is_none());
    assert_eq!(perft_bulk(&draw.parse().unwrap(), 3), 0);
    let mut session = UsiSession::default();
    assert!(
        session
            .handle_line(&format!("position sfen {draw}"))
            .is_empty()
    );
    assert!(
        session
            .handle_line("go nodes 1")
            .iter()
            .any(|s| s == "bestmove resign")
    );
    assert!(
        session
            .handle_line("go depth 1")
            .iter()
            .any(|line| line.contains("only-kings-draw"))
    );
    let before = position(&[("1a", 'k'), ("4d", 'K'), ("3c", 'p')], Color::Black);
    let mut b: Board = before.parse().unwrap();
    let mut history = PositionHistory::new(b.clone());
    b.play("4d3c".parse().unwrap());
    history.push(b.clone());
    assert_eq!(terminal_score_for_side_to_move(&b, 1), Some(0));
}
#[test]
fn usi_promotion_and_capture_roundtrip_and_low_budget() {
    let king = Square::new(File::ALL[File::NUM - 1], Rank::ALL[Rank::NUM - 1]).to_string();
    let before = position(
        &[("1a", 'k'), (&king, 'K'), ("3d", 'S'), ("3b", 'g')],
        Color::Black,
    );
    let mut session = UsiSession::default();
    assert!(
        session
            .handle_line(&format!("position sfen {before} moves 3d3c+ 3b3c"))
            .is_empty()
    );
    let b: Board = session.board_sfen().parse().unwrap();
    assert!(b.is_hand_empty(Color::White));
    assert!(b.is_hand_empty(Color::Black));
    assert!(!session.board_sfen().contains('+'));
    let output = session.handle_line("go nodes 1");
    let best = output
        .iter()
        .find_map(|s| s.strip_prefix("bestmove "))
        .unwrap();
    assert!(b.is_legal(best.parse().unwrap()));
    let previous = session.board_sfen();
    assert!(session.handle_line("position sfen 9/9/9 b - 1")[0].contains("invalid position"));
    assert_eq!(session.board_sfen(), previous);
}
#[test]
fn repetition_and_models_and_dfpn_are_rule_aware() {
    let king = Square::new(File::ALL[File::NUM - 1], Rank::ALL[Rank::NUM - 1]);
    let up = king.offset(0, -1);
    let before = position(
        &[("1a", 'k'), (&king.to_string(), 'K'), ("4d", 'G')],
        Color::Black,
    );
    let cycle = format!("{king}{up} 1a1b {up}{king} 1b1a ").repeat(3);
    let command = format!("position sfen {before} moves {}", cycle.trim());
    let (b, h) = parse_usi_position_with_history(&command).unwrap();
    assert_eq!(h.adjudication(), HistoryAdjudication::RepetitionDraw);
    let result = search_board_impl_handcrafted_with_history(&b, &h, 2).unwrap();
    assert!(result.best_move.is_none());
    assert_eq!(result.best_score, Some(0));
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
    assert!(
        load_nnue_impl(&[])
            .unwrap_err()
            .contains("UNSUPPORTED_RULESET")
    );
    assert!(
        dfpn_impl(SelectedRule::STARTPOS, Some(100), None, 1, 4)
            .unwrap_err()
            .contains("DFPN_UNSUPPORTED_RULESET")
    );
}

use std::cmp::Reverse;

use haitaka::{Board, Color, Move, Piece};
use wasm_bindgen::prelude::*;

const INF_SCORE: i32 = 1_000_000;
const MATE_SCORE: i32 = 100_000;
const MOBILITY_WEIGHT: i32 = 2;
const HAND_PIECES: [Piece; Piece::HAND_NUM] = [
    Piece::Pawn,
    Piece::Lance,
    Piece::Knight,
    Piece::Silver,
    Piece::Bishop,
    Piece::Rook,
    Piece::Gold,
];

fn best_move_impl(sfen: &str, depth: u8) -> Result<Option<String>, String> {
    let board = Board::from_sfen(sfen)
        .map_err(|err| format!("failed to parse SFEN: {err}"))?;
    let depth = depth.max(1);

    Ok(search_best_move(&board, depth).map(|mv| mv.to_string()))
}

#[wasm_bindgen(js_name = best_move)]
pub fn best_move(sfen: &str, depth: u8) -> Result<Option<String>, JsValue> {
    best_move_impl(sfen, depth).map_err(|err| JsValue::from_str(&err))
}

fn search_best_move(board: &Board, depth: u8) -> Option<Move> {
    let moves = legal_moves(board);
    if moves.is_empty() {
        return None;
    }

    let mut alpha = -INF_SCORE;
    let beta = INF_SCORE;
    let mut best_score = -INF_SCORE;
    let mut best_move = None;

    for mv in moves {
        let mut child = board.clone();
        child.play_unchecked(mv);
        let score = -negamax(&child, depth.saturating_sub(1), -beta, -alpha, 1);
        if score > best_score {
            best_score = score;
            best_move = Some(mv);
        }
        alpha = alpha.max(score);
    }

    best_move
}

fn negamax(board: &Board, depth: u8, mut alpha: i32, beta: i32, ply: i32) -> i32 {
    let moves = legal_moves(board);
    if moves.is_empty() {
        return -MATE_SCORE + ply;
    }
    if depth == 0 {
        return evaluate(board);
    }

    let mut best_score = -INF_SCORE;
    for mv in moves {
        let mut child = board.clone();
        child.play_unchecked(mv);
        let score = -negamax(&child, depth - 1, -beta, -alpha, ply + 1);
        if score > best_score {
            best_score = score;
        }
        if score > alpha {
            alpha = score;
        }
        if alpha >= beta {
            break;
        }
    }

    best_score
}

fn evaluate(board: &Board) -> i32 {
    let us = board.side_to_move();
    let them = !us;
    material_score(board, us)
        - material_score(board, them)
        + MOBILITY_WEIGHT * (count_legal_moves(board) as i32 - opponent_mobility(board) as i32)
}

fn material_score(board: &Board, color: Color) -> i32 {
    let mut score = 0;

    for &piece in &Piece::ALL {
        score += board.colored_pieces(color, piece).len() as i32 * piece_value(piece);
    }

    for &piece in &HAND_PIECES {
        score += i32::from(board.num_in_hand(color, piece)) * piece_value(piece);
    }

    score
}

fn opponent_mobility(board: &Board) -> usize {
    board.null_move()
        .map(|opponent_board| count_legal_moves(&opponent_board))
        .unwrap_or(0)
}

fn count_legal_moves(board: &Board) -> usize {
    let mut count = 0;
    board.generate_moves(|moves| {
        count += moves.len();
        false
    });
    count
}

fn legal_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|piece_moves| {
        moves.extend(piece_moves);
        false
    });
    moves.sort_by_key(|mv| move_order_key(board, *mv));
    moves
}

fn move_order_key(board: &Board, mv: Move) -> (Reverse<i32>, Reverse<u8>, String) {
    (
        Reverse(capture_value(board, mv)),
        Reverse(u8::from(mv.is_promotion())),
        mv.to_string(),
    )
}

fn capture_value(board: &Board, mv: Move) -> i32 {
    match mv {
        Move::BoardMove { to, .. } => board
            .color_on(to)
            .filter(|color| *color != board.side_to_move())
            .and_then(|_| board.piece_on(to))
            .map(piece_value)
            .unwrap_or(0),
        Move::Drop { .. } => 0,
    }
}

fn piece_value(piece: Piece) -> i32 {
    match piece {
        Piece::Pawn => 100,
        Piece::Lance => 300,
        Piece::Knight => 300,
        Piece::Silver => 400,
        Piece::Gold => 500,
        Piece::Bishop => 700,
        Piece::Rook => 800,
        Piece::King => 0,
        Piece::Tokin | Piece::PLance | Piece::PKnight | Piece::PSilver => 550,
        Piece::PBishop => 900,
        Piece::PRook => 1000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_legal_best_move(sfen: &str, depth: u8) {
        let board = Board::from_sfen(sfen).unwrap();
        let best = best_move_impl(sfen, depth)
            .unwrap()
            .expect("expected a legal move");
        let mv: Move = best.parse().unwrap();
        assert!(
            board.is_legal(mv),
            "best move {best} should be legal for {sfen}"
        );
    }

    #[test]
    #[cfg(not(feature = "annan"))]
    fn returns_legal_move_for_start_position() {
        assert_legal_best_move(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            2,
        );
    }

    #[test]
    #[cfg(not(feature = "annan"))]
    fn returns_legal_move_for_handicap_position() {
        assert_legal_best_move(
            "2sgkgs2/9/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL w - 2",
            2,
        );
    }

    #[test]
    #[cfg(feature = "annan")]
    fn returns_legal_move_for_annan_start_position() {
        assert_legal_best_move(haitaka::SFEN_STARTPOS, 2);
    }

    #[test]
    fn returns_none_when_side_to_move_has_no_legal_move() {
        let sfen = "lns4+Rl/1r1g5/p1p1pSp1p/1p1p1p3/8k/7NG/PPPPPPP1P/1B7/LNSGKGSNL w B2p 26";
        assert_eq!(best_move_impl(sfen, 2).unwrap(), None);
    }

    #[test]
    fn prefers_capturing_a_hanging_rook_in_a_simple_tactical_position() {
        let sfen = "9/9/k8/9/4Rr3/9/9/9/4K4 b - 1";
        assert_eq!(best_move_impl(sfen, 1).unwrap().as_deref(), Some("5e4e"));
    }
}

//! Annan Shogi helpers.
//!
//! In Annan Shogi, a piece moves like the friendly piece directly behind it
//! (one square toward its own camp). If no friendly piece is behind it,
//! the piece moves normally.

use crate::*;

/// Compute pseudo-legal moves for a piece type on a given square.
///
/// This is a runtime dispatch version of the compile-time `Commoner::pseudo_legals`,
/// needed because under Annan rules the effective piece type is only known at runtime.
#[inline]
pub fn pseudo_legals_for(piece: Piece, color: Color, square: Square, blockers: BitBoard) -> BitBoard {
    match piece {
        Piece::Pawn => pawn_attacks(color, square),
        Piece::Lance => get_lance_moves(color, square, blockers),
        Piece::Knight => knight_attacks(color, square),
        Piece::Silver => silver_attacks(color, square),
        Piece::Gold => gold_attacks(color, square),
        Piece::Bishop => get_bishop_moves(color, square, blockers),
        Piece::Rook => get_rook_moves(color, square, blockers),
        Piece::King => king_attacks(color, square),
        Piece::Tokin => gold_attacks(color, square),
        Piece::PLance => gold_attacks(color, square),
        Piece::PKnight => gold_attacks(color, square),
        Piece::PSilver => gold_attacks(color, square),
        Piece::PBishop => get_bishop_moves(color, square, blockers) | gold_attacks(color, square),
        Piece::PRook => get_rook_moves(color, square, blockers) | silver_attacks(color, square),
    }
}

/// Shift a bitboard forward (toward the opponent) by one rank for the given color.
#[inline]
pub fn shift_forward(bb: BitBoard, color: Color) -> BitBoard {
    match color {
        Color::Black => bb.shift_north(1),
        Color::White => bb.shift_south(1),
    }
}

/// Annan backing information for one color.
pub struct AnnanBacking {
    /// `backed_by[p]` = bitboard of friendly pieces that have piece type `p` directly behind them.
    pub backed_by: [BitBoard; Piece::NUM],
    /// Union of all `backed_by` entries — pieces that have any backer.
    pub has_backer: BitBoard,
}

impl AnnanBacking {
    /// Compute Annan backing information for the given color.
    pub fn compute(board: &Board, color: Color) -> Self {
        let friendly = board.colors(color);
        let mut backed_by = [BitBoard::EMPTY; Piece::NUM];
        let mut has_backer = BitBoard::EMPTY;

        for &piece in &Piece::ALL {
            let backers = board.colored_pieces(color, piece);
            if backers.is_empty() {
                continue;
            }
            // Shift backers forward: the square ahead of a backer is the square it backs
            let shifted = shift_forward(backers, color);
            let backed = shifted & friendly;
            backed_by[piece as usize] = backed;
            has_backer |= backed;
        }

        Self { backed_by, has_backer }
    }
}

/// Returns the square directly behind `square` for the given color (toward own camp).
///
/// Returns `None` if `square` is on the back rank.
#[inline]
pub fn backer_square(color: Color, square: Square) -> Option<Square> {
    match color {
        Color::Black => square.try_offset(0, 1),
        Color::White => square.try_offset(0, -1),
    }
}

/// Returns the effective piece type for a piece at `square` under Annan rules.
///
/// If a friendly piece is directly behind `square`, the piece moves like that backer.
/// Otherwise it moves as its own type.
#[inline]
pub fn effective_piece(board: &Board, color: Color, square: Square) -> Piece {
    if let Some(behind) = backer_square(color, square) {
        if board.colors(color).has(behind) {
            if let Some(backer) = board.piece_on(behind) {
                return backer;
            }
        }
    }
    board.piece_on(square).unwrap()
}

/// Returns true if the given piece type has slider movement (can move along rays).
#[inline]
pub fn is_slider_movement(piece: Piece) -> bool {
    matches!(
        piece,
        Piece::Lance | Piece::Bishop | Piece::Rook | Piece::PBishop | Piece::PRook
    )
}

/// Returns the pseudo-attack rays for a slider piece type from a given square.
/// Returns `BitBoard::EMPTY` for non-slider piece types.
#[inline]
pub fn slider_pseudo_attacks(piece: Piece, color: Color, square: Square) -> BitBoard {
    match piece {
        Piece::Lance => lance_pseudo_attacks(color, square),
        Piece::Bishop | Piece::PBishop => bishop_pseudo_attacks(square),
        Piece::Rook | Piece::PRook => rook_pseudo_attacks(square),
        _ => BitBoard::EMPTY,
    }
}

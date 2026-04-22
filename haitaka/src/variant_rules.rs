//! Compile-time piece-influence variant helpers.
//!
//! These variants change a piece's effective movement based on adjacent
//! friendly donor pieces, while captures, promotion, and drops still use the
//! physical moving piece.

use crate::*;

/// Compute pseudo-legal moves for a piece type on a given square.
///
/// This is a runtime dispatch version of the compile-time `Commoner::pseudo_legals`,
/// needed because the effective piece type is only known at runtime in influence
/// variants.
#[inline(always)]
pub fn pseudo_legals_for(
    piece: Piece,
    color: Color,
    square: Square,
    blockers: BitBoard,
) -> BitBoard {
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

/// Returns true if the given piece type has slider movement.
#[inline(always)]
pub fn is_slider_movement(piece: Piece) -> bool {
    matches!(
        piece,
        Piece::Lance | Piece::Bishop | Piece::Rook | Piece::PBishop | Piece::PRook
    )
}

/// Returns the pseudo-attack rays for a slider piece type from a given square.
/// Returns `BitBoard::EMPTY` for non-slider piece types.
#[inline(always)]
pub fn slider_pseudo_attacks(piece: Piece, color: Color, square: Square) -> BitBoard {
    match piece {
        Piece::Lance => lance_pseudo_attacks(color, square),
        Piece::Bishop | Piece::PBishop => bishop_pseudo_attacks(square),
        Piece::Rook | Piece::PRook => rook_pseudo_attacks(square),
        _ => BitBoard::EMPTY,
    }
}

/// Set of effective movement piece types.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MovementSet {
    mask: u16,
}

impl MovementSet {
    #[inline(always)]
    pub const fn empty() -> Self {
        Self { mask: 0 }
    }

    #[inline(always)]
    pub const fn single(piece: Piece) -> Self {
        Self {
            mask: 1 << (piece as u16),
        }
    }

    #[inline(always)]
    pub fn insert(&mut self, piece: Piece) {
        self.mask |= 1 << (piece as u16);
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.mask == 0
    }

    #[inline(always)]
    pub const fn contains(self, piece: Piece) -> bool {
        (self.mask & (1 << (piece as u16))) != 0
    }

    #[inline(always)]
    pub fn pseudo_legals(self, color: Color, square: Square, blockers: BitBoard) -> BitBoard {
        if self.mask.count_ones() == 1 {
            let piece = Piece::index_const(self.mask.trailing_zeros() as usize);
            return pseudo_legals_for(piece, color, square, blockers);
        }

        let mut moves = BitBoard::EMPTY;
        for &piece in &Piece::ALL {
            if self.contains(piece) {
                moves |= pseudo_legals_for(piece, color, square, blockers);
            }
        }
        moves
    }

    #[inline(always)]
    pub fn has_slider(self) -> bool {
        Piece::ALL
            .iter()
            .any(|&piece| self.contains(piece) && is_slider_movement(piece))
    }
}

/// Movement influence information for one color.
pub struct MovementInfluence {
    /// `influenced_by[p]` = friendly pieces that gain movement type `p`.
    ///
    /// In Antouzai, one square may appear in multiple entries.
    pub influenced_by: [BitBoard; Piece::NUM],
    /// Union of all influenced squares.
    pub has_influence: BitBoard,
}

impl MovementInfluence {
    /// Compute movement influence for the given color.
    #[inline(always)]
    pub fn compute(board: &Board, color: Color) -> Self {
        let friendly = board.colors(color);
        let mut influenced_by = [BitBoard::EMPTY; Piece::NUM];
        let mut has_influence = BitBoard::EMPTY;

        for &piece in &Piece::ALL {
            let donors = board.colored_pieces(color, piece);
            if donors.is_empty() {
                continue;
            }

            let influenced = influence_targets_from_donors(donors, color) & friendly;
            influenced_by[piece as usize] = influenced;
            has_influence |= influenced;
        }

        Self {
            influenced_by,
            has_influence,
        }
    }

    /// Effective movement types for the physical piece on `square`.
    #[inline(always)]
    pub fn effective_movements(&self, native_piece: Piece, square: Square) -> MovementSet {
        if !self.has_influence.has(square) {
            return MovementSet::single(native_piece);
        }

        let mut pieces = MovementSet::empty();
        for &piece in &Piece::ALL {
            if self.influenced_by[piece as usize].has(square) {
                pieces.insert(piece);
            }
        }
        debug_assert!(!pieces.is_empty());
        pieces
    }
}

/// Returns the effective movement types for a piece at `square`.
#[inline(always)]
pub fn effective_movements(board: &Board, color: Color, square: Square) -> MovementSet {
    let native_piece = board.piece_on(square).unwrap();
    MovementInfluence::compute(board, color).effective_movements(native_piece, square)
}

/// Returns the single effective movement piece for single-donor variants.
#[cfg(any(feature = "annan", feature = "anhoku"))]
#[inline(always)]
pub fn effective_piece(board: &Board, color: Color, square: Square) -> Piece {
    if let Some(donor) = donor_candidate_square(color, square) {
        if board.colors(color).has(donor) {
            if let Some(piece) = board.piece_on(donor) {
                return piece;
            }
        }
    }
    board.piece_on(square).unwrap()
}

/// Returns the friendly donor squares currently influencing `square`.
#[inline(always)]
pub fn influencing_donor_squares(board: &Board, color: Color, square: Square) -> BitBoard {
    donor_candidate_squares(color, square) & board.colors(color)
}

#[cfg(feature = "annan")]
#[inline(always)]
fn influence_targets_from_donors(donors: BitBoard, color: Color) -> BitBoard {
    shift_forward(donors, color)
}

#[cfg(feature = "anhoku")]
#[inline(always)]
fn influence_targets_from_donors(donors: BitBoard, color: Color) -> BitBoard {
    shift_backward(donors, color)
}

#[cfg(feature = "antouzai")]
#[inline(always)]
fn influence_targets_from_donors(donors: BitBoard, _color: Color) -> BitBoard {
    donors.shift_east(1) | donors.shift_west(1)
}

#[cfg(feature = "annan")]
#[inline(always)]
fn donor_candidate_square(color: Color, square: Square) -> Option<Square> {
    match color {
        Color::Black => square.try_offset(0, 1),
        Color::White => square.try_offset(0, -1),
    }
}

#[cfg(feature = "annan")]
#[inline(always)]
fn donor_candidate_squares(color: Color, square: Square) -> BitBoard {
    donor_candidate_square(color, square).map_or(BitBoard::EMPTY, Square::bitboard)
}

#[cfg(feature = "anhoku")]
#[inline(always)]
fn donor_candidate_square(color: Color, square: Square) -> Option<Square> {
    match color {
        Color::Black => square.try_offset(0, -1),
        Color::White => square.try_offset(0, 1),
    }
}

#[cfg(feature = "anhoku")]
#[inline(always)]
fn donor_candidate_squares(color: Color, square: Square) -> BitBoard {
    donor_candidate_square(color, square).map_or(BitBoard::EMPTY, Square::bitboard)
}

#[cfg(feature = "antouzai")]
#[inline(always)]
fn donor_candidate_squares(_color: Color, square: Square) -> BitBoard {
    let left = square
        .try_offset(1, 0)
        .map_or(BitBoard::EMPTY, Square::bitboard);
    let right = square
        .try_offset(-1, 0)
        .map_or(BitBoard::EMPTY, Square::bitboard);
    left | right
}

/// Shift a bitboard forward (toward the opponent) by one rank for the given color.
#[cfg(feature = "annan")]
#[inline(always)]
fn shift_forward(bb: BitBoard, color: Color) -> BitBoard {
    match color {
        Color::Black => bb.shift_north(1),
        Color::White => bb.shift_south(1),
    }
}

/// Shift a bitboard backward (toward own camp) by one rank for the given color.
#[cfg(feature = "anhoku")]
#[inline(always)]
fn shift_backward(bb: BitBoard, color: Color) -> BitBoard {
    match color {
        Color::Black => bb.shift_south(1),
        Color::White => bb.shift_north(1),
    }
}

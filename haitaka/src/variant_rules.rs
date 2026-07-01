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
    /// Compute movement influence for the given color (fixed-offset donor variants).
    #[cfg(not(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko",
        feature = "tenkyo"
    )))]
    #[inline(always)]
    pub fn compute(board: &Board, color: Color) -> Self {
        let friendly = board.colors(color);
        let donor_color = donor_color(color);
        let mut influenced_by = [BitBoard::EMPTY; Piece::NUM];
        let mut has_influence = BitBoard::EMPTY;

        for &piece in &Piece::ALL {
            let donors = board.colored_pieces(donor_color, piece);
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

    /// Compute movement influence for the given color (neko run-reflection variants).
    ///
    /// Each line (file for `neko`/`nekoneko`, rank for `yokoneko`/`yokonekoneko`)
    /// is segmented into maximal runs and the `i`-th piece from one end swaps
    /// abilities with the `i`-th piece from the other end. The middle piece of an
    /// odd-length run keeps its native movement.
    #[cfg(any(feature = "tenkyo"))]
    #[inline(always)]
    pub fn compute(board: &Board, color: Color) -> Self {
        let mut influenced_by = [BitBoard::EMPTY; Piece::NUM];
        let mut has_influence = BitBoard::EMPTY;

        for sq in board.colors(color) {
            let donor_sq = sq.flip();
            if let Some(donor_piece) = board.piece_on(donor_sq) {
                influenced_by[donor_piece as usize] |= sq.bitboard();
                has_influence |= sq.bitboard();
            }
        }

        Self {
            influenced_by,
            has_influence,
        }
    }

    /// Compute movement influence for the given color (neko run-reflection variants).
    ///
    /// Each line (file for `neko`/`nekoneko`, rank for `yokoneko`/`yokonekoneko`)
    /// is segmented into maximal runs and the `i`-th piece from one end swaps
    /// abilities with the `i`-th piece from the other end. The middle piece of an
    /// odd-length run keeps its native movement.
    #[cfg(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    ))]
    #[inline(always)]
    pub fn compute(board: &Board, color: Color) -> Self {
        let mut influenced_by = [BitBoard::EMPTY; Piece::NUM];
        let mut has_influence = BitBoard::EMPTY;

        for line in 0..neko::LINE_COUNT {
            let mut pos = 0;
            while pos < neko::LINE_LEN {
                if !neko::in_run(board, color, line, pos) {
                    pos += 1;
                    continue;
                }
                let start = pos;
                while pos < neko::LINE_LEN && neko::in_run(board, color, line, pos) {
                    pos += 1;
                }
                let len = pos - start;
                for i in 0..len {
                    let j = len - 1 - i;
                    if j == i {
                        continue; // middle of an odd-length run keeps native movement
                    }
                    let sq = neko::line_square(line, start + i);
                    // Only `color`'s own pieces have their moves generated here.
                    if board.color_on(sq) != Some(color) {
                        continue;
                    }
                    let partner_sq = neko::line_square(line, start + j);
                    let partner_piece = board.piece_on(partner_sq).unwrap();
                    influenced_by[partner_piece as usize] |= sq.bitboard();
                    has_influence |= sq.bitboard();
                }
            }
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
        #[cfg(feature = "tenjiku")]
        pieces.insert(native_piece);
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

/// Returns the single effective movement piece for fixed-offset single-donor variants.
#[cfg(any(
    feature = "annan",
    feature = "anhoku",
    feature = "taimen",
    feature = "haimen"
))]
#[inline(always)]
pub fn effective_piece(board: &Board, color: Color, square: Square) -> Piece {
    if let Some(donor) = donor_candidate_square(color, square) {
        if board.colors(donor_color(color)).has(donor) {
            if let Some(piece) = board.piece_on(donor) {
                return piece;
            }
        }
    }
    board.piece_on(square).unwrap()
}

/// Returns the single effective movement piece for Tenkyo point-symmetry donors.
#[cfg(feature = "tenkyo")]
#[inline(always)]
pub fn effective_piece(board: &Board, _color: Color, square: Square) -> Piece {
    board
        .piece_on(square.flip())
        .unwrap_or_else(|| board.piece_on(square).unwrap())
}

/// Returns the single effective movement piece for neko run-reflection variants.
#[cfg(any(
    feature = "neko",
    feature = "nekoneko",
    feature = "yokoneko",
    feature = "yokonekoneko"
))]
#[inline(always)]
pub fn effective_piece(board: &Board, color: Color, square: Square) -> Piece {
    if let Some(partner) = neko::run_partner_square(board, color, square) {
        return board.piece_on(partner).unwrap();
    }
    board.piece_on(square).unwrap()
}

/// Returns the donor squares currently influencing `square`.
///
/// Donors are friendly pieces in same-side variants (annan/anhoku/antouzai) and
/// enemy pieces in face-off variants (taimen/haimen). The neko run-reflection
/// variants do not use this narrowing optimization (see `target_squares`).
#[cfg(not(any(
    feature = "neko",
    feature = "nekoneko",
    feature = "yokoneko",
    feature = "yokonekoneko",
    feature = "tenkyo"
)))]
#[inline(always)]
pub fn influencing_donor_squares(board: &Board, color: Color, square: Square) -> BitBoard {
    donor_candidate_squares(color, square) & board.colors(donor_color(color))
}

/// Color of the pieces that donate movement to `color`'s pieces.
///
/// Same-side variants (annan/anhoku/antouzai) use friendly donors; face-off
/// variants (taimen/haimen) use enemy donors. Unused by the neko run-reflection
/// variants, which look up partners by run rather than donor color.
#[cfg(not(any(
    feature = "neko",
    feature = "nekoneko",
    feature = "yokoneko",
    feature = "yokonekoneko",
    feature = "tenkyo"
)))]
#[inline(always)]
fn donor_color(color: Color) -> Color {
    #[cfg(any(feature = "taimen", feature = "haimen"))]
    {
        !color
    }
    #[cfg(not(any(feature = "taimen", feature = "haimen")))]
    {
        color
    }
}

// `annan` (friendly behind) and `haimen` (enemy behind) share the same geometry:
// a donor influences the piece one rank in front of it.
#[cfg(any(feature = "annan", feature = "haimen", feature = "tenjiku"))]
#[inline(always)]
fn influence_targets_from_donors(donors: BitBoard, color: Color) -> BitBoard {
    shift_forward(donors, color)
}

// `anhoku` (friendly in front) and `taimen` (enemy in front) share the same
// geometry: a donor influences the piece one rank behind it.
#[cfg(any(feature = "anhoku", feature = "taimen"))]
#[inline(always)]
fn influence_targets_from_donors(donors: BitBoard, color: Color) -> BitBoard {
    shift_backward(donors, color)
}

#[cfg(feature = "antouzai")]
#[inline(always)]
fn influence_targets_from_donors(donors: BitBoard, _color: Color) -> BitBoard {
    donors.shift_east(1) | donors.shift_west(1)
}

#[cfg(feature = "anki")]
#[inline(always)]
fn influence_targets_from_donors(donors: BitBoard, _color: Color) -> BitBoard {
    donors.shift_east(1).shift_north(2)
        | donors.shift_west(1).shift_north(2)
        | donors.shift_east(2).shift_north(1)
        | donors.shift_west(2).shift_north(1)
        | donors.shift_east(2).shift_south(1)
        | donors.shift_west(2).shift_south(1)
        | donors.shift_east(1).shift_south(2)
        | donors.shift_west(1).shift_south(2)
}

// Donor sits behind the influenced piece (annan: friendly, haimen: enemy).
#[cfg(any(feature = "annan", feature = "haimen", feature = "tenjiku"))]
#[inline(always)]
fn donor_candidate_square(color: Color, square: Square) -> Option<Square> {
    match color {
        Color::Black => square.try_offset(0, 1),
        Color::White => square.try_offset(0, -1),
    }
}

// Donor sits in front of the influenced piece (anhoku: friendly, taimen: enemy).
#[cfg(any(feature = "anhoku", feature = "taimen"))]
#[inline(always)]
fn donor_candidate_square(color: Color, square: Square) -> Option<Square> {
    match color {
        Color::Black => square.try_offset(0, -1),
        Color::White => square.try_offset(0, 1),
    }
}

#[cfg(any(
    feature = "annan",
    feature = "anhoku",
    feature = "taimen",
    feature = "haimen",
    feature = "tenjiku"
))]
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

#[cfg(feature = "anki")]
#[inline(always)]
fn donor_candidate_squares(_color: Color, square: Square) -> BitBoard {
    const OFFSETS: [(i8, i8); 8] = [
        (1, 2),
        (-1, 2),
        (-2, 1),
        (-2, -1),
        (-1, -2),
        (1, -2),
        (2, -1),
        (2, 1),
    ];
    let mut donors = BitBoard::EMPTY;
    for &(file, rank) in &OFFSETS {
        if let Some(sq) = square.try_offset(file, rank) {
            donors |= sq.bitboard();
        }
    }
    donors
}

/// Shift a bitboard forward (toward the opponent) by one rank for the given color.
#[cfg(any(feature = "annan", feature = "haimen", feature = "tenjiku"))]
#[inline(always)]
fn shift_forward(bb: BitBoard, color: Color) -> BitBoard {
    match color {
        Color::Black => bb.shift_north(1),
        Color::White => bb.shift_south(1),
    }
}

/// Shift a bitboard backward (toward own camp) by one rank for the given color.
#[cfg(any(feature = "anhoku", feature = "taimen"))]
#[inline(always)]
fn shift_backward(bb: BitBoard, color: Color) -> BitBoard {
    match color {
        Color::Black => bb.shift_south(1),
        Color::White => bb.shift_north(1),
    }
}

/// Board-dependent run-reflection donors for the neko family.
///
/// Each "line" is segmented into maximal runs of contiguous occupied squares
/// (broken by empty squares, and additionally by a color change for the
/// friendly-only `neko`/`yokoneko` variants). Within a run the `i`-th piece from
/// one end swaps abilities with the `i`-th from the other end; the middle piece
/// of an odd-length run keeps its native movement.
#[cfg(any(
    feature = "neko",
    feature = "nekoneko",
    feature = "yokoneko",
    feature = "yokonekoneko"
))]
pub(crate) mod neko {
    use crate::*;

    /// Number of lines to scan (9 files for vertical, 9 ranks for horizontal).
    pub(crate) const LINE_COUNT: usize = 9;
    /// Number of squares in each line.
    pub(crate) const LINE_LEN: usize = 9;

    /// Whether runs are segmented horizontally (within a rank) instead of
    /// vertically (within a file).
    const HORIZONTAL: bool = cfg!(any(feature = "yokoneko", feature = "yokonekoneko"));
    /// Whether enemy pieces participate in a run (nekoneko/yokonekoneko); for the
    /// friendly-only variants a color change breaks the run.
    const ANY_COLOR: bool = cfg!(any(feature = "nekoneko", feature = "yokonekoneko"));

    /// The square at position `pos` along line `line`.
    #[inline(always)]
    pub(crate) fn line_square(line: usize, pos: usize) -> Square {
        if HORIZONTAL {
            // line = rank, pos = file
            Square::new(File::index_const(pos), Rank::index_const(line))
        } else {
            // line = file, pos = rank
            Square::new(File::index_const(line), Rank::index_const(pos))
        }
    }

    /// Whether the square at (`line`, `pos`) is a member of a run for `color`.
    #[inline(always)]
    pub(crate) fn in_run(board: &Board, color: Color, line: usize, pos: usize) -> bool {
        let sq = line_square(line, pos);
        if ANY_COLOR {
            board.piece_on(sq).is_some()
        } else {
            board.color_on(sq) == Some(color)
        }
    }

    /// The swap-partner square of `square` within its run for `color`, or `None`
    /// when `square` is unpaired (the middle of an odd-length run) or not a run
    /// member.
    #[inline(always)]
    pub(crate) fn run_partner_square(
        board: &Board,
        color: Color,
        square: Square,
    ) -> Option<Square> {
        let (line, pos) = if HORIZONTAL {
            (square.rank() as usize, square.file() as usize)
        } else {
            (square.file() as usize, square.rank() as usize)
        };

        if !in_run(board, color, line, pos) {
            return None;
        }

        let mut lo = pos;
        while lo > 0 && in_run(board, color, line, lo - 1) {
            lo -= 1;
        }
        let mut hi = pos;
        while hi + 1 < LINE_LEN && in_run(board, color, line, hi + 1) {
            hi += 1;
        }

        let partner_pos = lo + hi - pos;
        if partner_pos == pos {
            return None; // middle of an odd-length run keeps native movement
        }
        Some(line_square(line, partner_pos))
    }
}

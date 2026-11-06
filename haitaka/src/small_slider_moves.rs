//! Compact-board sliders share the precomputed first-blocker ray backend.
use crate::*;
#[inline(always)]
pub const fn get_rook_moves(_color: Color, square: Square, occupied: BitBoard) -> BitBoard {
    small_rook_moves(square, occupied)
}
#[inline(always)]
pub const fn get_bishop_moves(_color: Color, square: Square, occupied: BitBoard) -> BitBoard {
    small_bishop_moves(square, occupied)
}

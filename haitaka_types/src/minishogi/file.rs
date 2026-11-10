//! Compact file coordinates for the selected small board.
use crate::*;
include!(concat!(env!("OUT_DIR"), "/file.rs"));
impl File {
    pub const fn flip(self) -> Self {
        Self::index_const(Self::NUM - 1 - self as usize)
    }
    pub const fn bitboard(self) -> BitBoard {
        BitBoard(((1 << Rank::NUM) - 1) << (Rank::NUM * self as usize))
    }
    pub const fn west(self) -> BitBoard {
        let shift = Rank::NUM * (self as usize + 1);
        if shift >= BitBoardWord::BITS as usize {
            BitBoard::EMPTY
        } else {
            BitBoard::new(BitBoard::FULL.0 << shift)
        }
    }
    pub const fn east(self) -> BitBoard {
        let shift = Rank::NUM * (File::NUM - self as usize);
        if shift >= BitBoardWord::BITS as usize {
            BitBoard::EMPTY
        } else {
            BitBoard::new(BitBoard::FULL.0 >> shift)
        }
    }
}

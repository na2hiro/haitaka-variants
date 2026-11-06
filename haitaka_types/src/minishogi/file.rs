//! Compact file coordinates for the selected small board.
use crate::*;
include!(concat!(env!("OUT_DIR"), "/file.rs"));
crate::helpers::enum_char_conv! { File, FileParseError { One = '1', Two = '2', Three = '3', Four = '4', Five = '5' } }
impl File {
    pub const fn flip(self) -> Self {
        Self::index_const(Self::NUM - 1 - self as usize)
    }
    pub const fn bitboard(self) -> BitBoard {
        BitBoard(((1 << Rank::NUM) - 1) << (Rank::NUM * self as usize))
    }
    pub const fn west(self) -> BitBoard {
        BitBoard::new(BitBoard::FULL.0 << (Rank::NUM * (self as usize + 1)))
    }
    pub const fn east(self) -> BitBoard {
        BitBoard::new(BitBoard::FULL.0 >> (Rank::NUM * (File::NUM - self as usize)))
    }
}

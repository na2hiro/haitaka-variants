//! Compact file-major square coordinates.
use crate::*;
use core::{convert::TryInto, str::FromStr};
include!(concat!(env!("OUT_DIR"), "/square.rs"));
crate::helpers::simple_error! { pub struct SquareParseError = "The value was not a valid Square."; }
impl FromStr for Square {
    type Err = SquareParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut c = s.chars();
        let f = c
            .next()
            .and_then(|v| v.try_into().ok())
            .ok_or(SquareParseError)?;
        let r = c
            .next()
            .and_then(|v| v.try_into().ok())
            .ok_or(SquareParseError)?;
        if c.next().is_some() {
            return Err(SquareParseError);
        }
        Ok(Self::new(f, r))
    }
}
impl core::fmt::Display for Square {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}{}", self.file(), self.rank())
    }
}
impl Square {
    pub const fn new(file: File, rank: Rank) -> Self {
        Self::index_const(file as usize * Rank::NUM + rank as usize)
    }
    pub const fn file(self) -> File {
        File::index_const(self as usize / Rank::NUM)
    }
    pub const fn rank(self) -> Rank {
        Rank::index_const(self as usize % Rank::NUM)
    }
    pub const fn bitboard(self) -> BitBoard {
        BitBoard(1 << self as usize)
    }
    pub const fn try_offset(self, dx: i8, dy: i8) -> Option<Self> {
        let f = self.file() as i8 + dx;
        let r = self.rank() as i8 + dy;
        if f < 0 || f >= File::NUM as i8 || r < 0 || r >= Rank::NUM as i8 {
            return None;
        }
        Some(Self::new(
            File::index_const(f as usize),
            Rank::index_const(r as usize),
        ))
    }
    pub const fn offset(self, dx: i8, dy: i8) -> Self {
        match self.try_offset(dx, dy) {
            Some(s) => s,
            None => panic!("Offset puts square out of bounds"),
        }
    }
    pub const fn flip_file(self) -> Self {
        Self::new(self.file().flip(), self.rank())
    }
    pub const fn flip_rank(self) -> Self {
        Self::new(self.file(), self.rank().flip())
    }
    pub const fn flip(self) -> Self {
        Self::index_const(Self::NUM - 1 - self as usize)
    }
    pub const fn relative_to(self, color: Color) -> Self {
        match color {
            Color::Black => self,
            Color::White => self.flip(),
        }
    }
    pub const fn up_diagonal(self) -> BitBoard {
        self.diagonal(false)
    }
    pub const fn down_diagonal(self) -> BitBoard {
        self.diagonal(true)
    }
    const fn diagonal(self, down: bool) -> BitBoard {
        let mut out = BitBoard::EMPTY;
        let mut i = 0;
        while i < Self::NUM {
            let sq = Self::index_const(i);
            let df = sq.file() as i8 - self.file() as i8;
            let dr = sq.rank() as i8 - self.rank() as i8;
            if (down && df == -dr) || (!down && df == dr) {
                out.0 |= 1 << i;
            }
            i += 1;
        }
        out
    }
}

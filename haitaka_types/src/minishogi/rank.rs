//! Promotion and drop regions of the selected small board.
use crate::*;
include!(concat!(env!("OUT_DIR"), "/rank.rs"));
crate::helpers::enum_char_conv! { Rank, RankParseError { A = 'a', B = 'b', C = 'c', D = 'd', E = 'e' } }
impl Rank {
    pub const RANK: [BitBoard; Self::NUM] = {
        let mut out = [BitBoard::EMPTY; Self::NUM];
        let mut f = 0;
        while f < File::NUM {
            let mut r = 0;
            while r < Self::NUM {
                out[r].0 |= 1 << (f * Self::NUM + r);
                r += 1;
            }
            f += 1;
        }
        out
    };
    pub const NORTH: [BitBoard; Self::NUM] = {
        let mut out = [BitBoard::EMPTY; Self::NUM];
        let mut r = 1;
        while r < Self::NUM {
            out[r] = out[r - 1].bitor(Self::RANK[r - 1]);
            r += 1;
        }
        out
    };
    pub const SOUTH: [BitBoard; Self::NUM] = {
        let mut out = [BitBoard::EMPTY; Self::NUM];
        let mut r = Self::NUM - 1;
        while r > 0 {
            out[r - 1] = out[r].bitor(Self::RANK[r]);
            r -= 1;
        }
        out
    };
    pub const fn bitboard(self) -> BitBoard {
        Self::RANK[self as usize]
    }
    pub const fn flip(self) -> Self {
        Self::index_const(Self::NUM - 1 - self as usize)
    }
    pub const fn north(self) -> BitBoard {
        Self::NORTH[self as usize]
    }
    pub const fn south(self) -> BitBoard {
        Self::SOUTH[self as usize]
    }
    pub const fn relative_to(self, color: Color) -> Self {
        match color {
            Color::White => self,
            Color::Black => self.flip(),
        }
    }
}
pub const fn no_fly_zone(color: Color, piece: Piece) -> BitBoard {
    let ranks = match piece {
        Piece::Pawn | Piece::Lance => 1,
        Piece::Knight => 2,
        _ => 0,
    };
    let mut out = BitBoard::EMPTY;
    let mut r = 0;
    while r < ranks {
        let index = match color {
            Color::Black => r,
            Color::White => Rank::NUM - 1 - r,
        };
        out = out.bitor(Rank::RANK[index]);
        r += 1;
    }
    out
}
pub const fn drop_zone(color: Color, piece: Piece) -> BitBoard {
    no_fly_zone(color, piece).not()
}
pub const fn prom_zone(color: Color) -> BitBoard {
    let mut out = BitBoard::EMPTY;
    let mut r = 0;
    while r < SelectedRule::PROMOTION_RANKS {
        let index = match color {
            Color::Black => r,
            Color::White => Rank::NUM - 1 - r,
        };
        out = out.bitor(Rank::RANK[index]);
        r += 1;
    }
    out
}
pub const fn must_prom_zone(color: Color, piece: Piece) -> BitBoard {
    no_fly_zone(color, piece)
}

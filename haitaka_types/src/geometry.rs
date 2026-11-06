//! Compile-time board geometry, independent of the game played on it.

use crate::fixed_bitboard::{MultiBitBoard, ScalarBitBoard, SquareSet};
use crate::packed_coordinates::{CoordinateCode, GeometryMoveCode};

/// A geometry chooses its storage and compact square index statically.
pub trait BoardGeometry {
    type Bits: SquareSet;
    type Index: Copy;
    type MoveCode: CoordinateCode;
    const WIDTH: usize;
    const HEIGHT: usize;
    const SQUARES: usize = Self::WIDTH * Self::HEIGHT;

    fn index(file: usize, rank: usize) -> Option<usize> {
        if file >= Self::WIDTH || rank >= Self::HEIGHT {
            return None;
        }
        Some(file * Self::HEIGHT + rank)
    }
    fn coordinates(index: usize) -> Option<(usize, usize)> {
        (index < Self::SQUARES).then_some((index / Self::HEIGHT, index % Self::HEIGHT))
    }
    fn offset(index: usize, dx: isize, dy: isize) -> Option<usize> {
        let (file, rank) = Self::coordinates(index)?;
        Self::index(file.checked_add_signed(dx)?, rank.checked_add_signed(dy)?)
    }
    fn flip(index: usize) -> Option<usize> {
        (index < Self::SQUARES).then(|| Self::SQUARES - 1 - index)
    }
    /// Shift squares geometrically, without wrapping around the board edges.
    fn shifted(set: Self::Bits, dx: isize, dy: isize) -> Self::Bits {
        let mut remaining = set;
        let mut out = Self::Bits::EMPTY;
        while let Some(index) = remaining.first() {
            remaining = remaining.without(index);
            if let Some(next) = Self::offset(index, dx, dy) {
                out = out.with(next);
            }
        }
        out
    }
    /// Rectangular ray, including the first blocker. `playable` supports holes.
    fn ray(
        from: usize,
        dx: isize,
        dy: isize,
        occupied: Self::Bits,
        playable: Self::Bits,
    ) -> Self::Bits {
        assert!(dx != 0 || dy != 0, "ray direction must be nonzero");
        if !playable.contains(from) {
            return Self::Bits::EMPTY;
        }
        let mut out = Self::Bits::EMPTY;
        let mut current = from;
        while let Some(next) = Self::offset(current, dx, dy) {
            if !playable.contains(next) {
                break;
            }
            out = out.with(next);
            if occupied.contains(next) {
                break;
            }
            current = next;
        }
        out
    }
}

/// A rectangular board. Implementations select the smallest scalar or word array.
pub struct Rect<const WIDTH: usize, const HEIGHT: usize>;

macro_rules! geometry {
    ($w:literal, $h:literal, $storage:ty, $index:ty) => {
        impl BoardGeometry for Rect<$w, $h> {
            type Bits = $storage;
            type Index = $index;
            type MoveCode = <$storage as GeometryMoveCode>::Code;
            const WIDTH: usize = $w;
            const HEIGHT: usize = $h;
        }
        const _: () = {
            assert!(<$storage as SquareSet>::BITS == $w * $h);
            assert!($w * $h <= (1usize << (core::mem::size_of::<$index>() * 8)));
            assert!(
                $w * $h <= <<$storage as GeometryMoveCode>::Code as CoordinateCode>::MAX_SQUARES
            );
        };
    };
}
// The distinct bounding rectangles in the current Shogitter rule database.
geometry!(3, 4, ScalarBitBoard<u64, 12>, u8);
geometry!(4, 5, ScalarBitBoard<u64, 20>, u8);
geometry!(5, 5, ScalarBitBoard<u64, 25>, u8);
geometry!(5, 6, ScalarBitBoard<u64, 30>, u8);
geometry!(6, 6, ScalarBitBoard<u64, 36>, u8);
geometry!(7, 6, ScalarBitBoard<u64, 42>, u8);
geometry!(7, 7, ScalarBitBoard<u64, 49>, u8);
geometry!(9, 7, ScalarBitBoard<u64, 63>, u8);
geometry!(8, 8, ScalarBitBoard<u64, 64>, u8);
geometry!(9, 8, ScalarBitBoard<u128, 72>, u8);
geometry!(9, 9, ScalarBitBoard<u128, 81>, u8);
geometry!(9, 10, ScalarBitBoard<u128, 90>, u8);
geometry!(9, 11, ScalarBitBoard<u128, 99>, u8);
geometry!(10, 10, ScalarBitBoard<u128, 100>, u8);
geometry!(11, 11, ScalarBitBoard<u128, 121>, u8);
geometry!(12, 12, MultiBitBoard<3, 144>, u8);
geometry!(13, 13, MultiBitBoard<3, 169>, u8);
geometry!(15, 15, MultiBitBoard<4, 225>, u8);
geometry!(16, 16, MultiBitBoard<4, 256>, u8);
geometry!(17, 17, MultiBitBoard<5, 289>, u16);
geometry!(19, 19, MultiBitBoard<6, 361>, u16);
geometry!(25, 25, MultiBitBoard<10, 625>, u16);
geometry!(36, 36, MultiBitBoard<21, 1296>, u16);

/// Rule-level configuration, separate from geometry and storage capacity.
pub trait RuleSpec {
    type Geometry: BoardGeometry;
    const NAME: &'static str;
    const RULE_ID: u32;
    const PROMOTION_RANKS: usize;
    const STARTPOS: &'static str;
    /// Native inventory in Piece's hand order: P, L, N, S, B, R, G.
    const INVENTORY: [u8; 7];
    const REPETITION: RepetitionRule;
    const CAPTURE_TO_HAND: bool = true;
    /// Canonical gold promotions lose their original material identity.
    const GOLD_AGGREGATE_INVENTORY: bool = false;
    const ALLOWED_PIECES: u16 = (1 << crate::Piece::NUM) - 1;
    const ALLOW_PROMOTED_SFEN: bool = true;
    const ONLY_KINGS_DRAW: bool = false;
    const ALLOW_DFPN: bool = true;
    const ALLOW_STANDARD_MODELS: bool = Self::Geometry::WIDTH == 9 && Self::Geometry::HEIGHT == 9;
    const PROMOTIONS: [Option<crate::Piece>; crate::Piece::NUM] = {
        use crate::Piece::*;
        [
            Some(Tokin),
            Some(PLance),
            Some(PKnight),
            Some(PSilver),
            Some(PBishop),
            Some(PRook),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ]
    };
    /// Keep the original branch-free shogi promotion implementation when its
    /// mapping is unchanged. Derived from the map so custom rules cannot forget it.
    const USE_STANDARD_PROMOTIONS: bool = {
        use crate::Piece::*;
        let standard = [
            Some(Tokin),
            Some(PLance),
            Some(PKnight),
            Some(PSilver),
            Some(PBishop),
            Some(PRook),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ];
        let mut same = true;
        let mut i = 0;
        while i < crate::Piece::NUM {
            let a = match Self::PROMOTIONS[i] {
                Some(piece) => piece as u8,
                None => u8::MAX,
            };
            let b = match standard[i] {
                Some(piece) => piece as u8,
                None => u8::MAX,
            };
            if a != b {
                same = false;
                break;
            }
            i += 1;
        }
        same
    };
    const MAX_BOARD_PIECES: usize = {
        let mut count = 2;
        let mut i = 0;
        while i < Self::INVENTORY.len() {
            count += Self::INVENTORY[i] as usize;
            i += 1;
        }
        count
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepetitionRule {
    Draw,
}

pub struct StandardShogi;
impl RuleSpec for StandardShogi {
    type Geometry = Rect<9, 9>;
    const NAME: &'static str = "standard";
    const RULE_ID: u32 = 0;
    const PROMOTION_RANKS: usize = 3;
    const STARTPOS: &'static str =
        "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
    const INVENTORY: [u8; 7] = [18, 4, 4, 4, 2, 2, 4];
    const REPETITION: RepetitionRule = RepetitionRule::Draw;
}

/// Metadata for the existing 9x9 feature modes; movement remains specialized
/// by their existing compile-time implementations.
pub struct ConfiguredShogi;
macro_rules! configured_shogi {
    ($($feature:literal, $id:literal, $start:expr);* $(;)?) => {
        const fn configured_metadata() -> (&'static str, u32, &'static str) {
            $(if cfg!(feature = $feature) { return ($feature, $id, $start); })*
            (StandardShogi::NAME, StandardShogi::RULE_ID, StandardShogi::STARTPOS)
        }
    };
}
configured_shogi!(
    "annan", 26, "lnsgkgsnl/1r5b1/p1ppppp1p/1p5p1/9/1P5P1/P1PPPPP1P/1B5R1/LNSGKGSNL b - 1";
    "anhoku", 55, StandardShogi::STARTPOS;
    "antouzai", 95, StandardShogi::STARTPOS;
    "taimen", 72, StandardShogi::STARTPOS;
    "haimen", 74, StandardShogi::STARTPOS;
    "neko", 130, StandardShogi::STARTPOS;
    "nekoneko", 131, StandardShogi::STARTPOS;
    "yokoneko", 132, StandardShogi::STARTPOS;
    "yokonekoneko", 133, StandardShogi::STARTPOS;
    "tenkyo", 151, StandardShogi::STARTPOS;
    "tenjiku", 56, StandardShogi::STARTPOS;
    "anki", 94, StandardShogi::STARTPOS;
);
impl RuleSpec for ConfiguredShogi {
    type Geometry = Rect<9, 9>;
    const NAME: &'static str = configured_metadata().0;
    const RULE_ID: u32 = configured_metadata().1;
    const STARTPOS: &'static str = configured_metadata().2;
    const PROMOTION_RANKS: usize = StandardShogi::PROMOTION_RANKS;
    const INVENTORY: [u8; 7] = StandardShogi::INVENTORY;
    const REPETITION: RepetitionRule = StandardShogi::REPETITION;
}

pub struct Minishogi;
impl RuleSpec for Minishogi {
    type Geometry = Rect<5, 5>;
    const NAME: &'static str = "minishogi";
    const RULE_ID: u32 = 51;
    const PROMOTION_RANKS: usize = 1;
    const STARTPOS: &'static str = "rbsgk/4p/5/P4/KGSBR b - 1";
    const INVENTORY: [u8; 7] = [2, 0, 0, 2, 2, 2, 2];
    const REPETITION: RepetitionRule = RepetitionRule::Draw;
}

macro_rules! small_rule {
    ($type:ident, $name:literal, $id:literal, $w:literal, $h:literal, $promotion:literal, $start:literal, $inventory:expr) => {
        pub struct $type;
        impl RuleSpec for $type {
            type Geometry = Rect<$w, $h>;
            const NAME: &'static str = $name;
            const RULE_ID: u32 = $id;
            const PROMOTION_RANKS: usize = $promotion;
            const STARTPOS: &'static str = $start;
            const INVENTORY: [u8; 7] = $inventory;
            const REPETITION: RepetitionRule = RepetitionRule::Draw;
        }
    };
}
small_rule!(
    Judkins,
    "judkins",
    52,
    6,
    6,
    2,
    "rbnsgk/5p/6/6/P5/KGSNBR b - 1",
    [2, 0, 2, 2, 2, 2, 2]
);
small_rule!(
    Gorogoro,
    "gorogoro",
    96,
    5,
    6,
    2,
    "sgkgs/5/1ppp1/1PPP1/5/SGKGS b - 1",
    [6, 0, 0, 4, 0, 0, 4]
);
small_rule!(
    GorogoroPlus,
    "gorogoro-plus",
    97,
    5,
    6,
    2,
    "sgkgs/5/1ppp1/1PPP1/5/SGKGS b NLnl 1",
    [6, 2, 2, 4, 0, 0, 4]
);
small_rule!(
    MinishogiKingin,
    "minishogi-kingin",
    110,
    5,
    5,
    1,
    "sgkgs/5/5/5/SGKGS b RBrb 1",
    [0, 0, 0, 4, 2, 2, 4]
);

macro_rules! heian_rule {
    ($type:ident, $feature:literal, $id:literal, $w:literal, $h:literal, $start:literal, $inventory:expr) => {
        pub struct $type;
        impl RuleSpec for $type {
            type Geometry = Rect<$w, $h>;
            const NAME: &'static str = $feature;
            const RULE_ID: u32 = $id;
            const PROMOTION_RANKS: usize = 3;
            const STARTPOS: &'static str = $start;
            const INVENTORY: [u8; 7] = $inventory;
            const REPETITION: RepetitionRule = RepetitionRule::Draw;
            const CAPTURE_TO_HAND: bool = false;
            const GOLD_AGGREGATE_INVENTORY: bool = true;
            const ALLOWED_PIECES: u16 = 0xcf;
            const ALLOW_PROMOTED_SFEN: bool = false;
            const ONLY_KINGS_DRAW: bool = true;
            const ALLOW_DFPN: bool = false;
            const ALLOW_STANDARD_MODELS: bool = false;
            const PROMOTIONS: [Option<crate::Piece>; crate::Piece::NUM] = {
                use crate::Piece::*;
                [
                    Some(Gold),
                    Some(Gold),
                    Some(Gold),
                    Some(Gold),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ]
            };
        }
    };
}
heian_rule!(
    Heian8x8,
    "heian-8x8",
    12,
    8,
    8,
    "lnsgksnl/8/pppppppp/8/8/PPPPPPPP/8/LNSKGSNL b - 1",
    [16, 4, 4, 4, 0, 0, 2]
);
heian_rule!(
    Heian9x8,
    "heian-9x8",
    13,
    9,
    8,
    "lnsgkgsnl/9/ppppppppp/9/9/PPPPPPPPP/9/LNSGKGSNL b - 1",
    [18, 4, 4, 4, 0, 0, 4]
);
heian_rule!(
    Heian9x9,
    "heian-9x9",
    14,
    9,
    9,
    "lnsgkgsnl/9/ppppppppp/9/9/9/PPPPPPPPP/9/LNSGKGSNL b - 1",
    [18, 4, 4, 4, 0, 0, 4]
);
#[cfg(feature = "heian-8x8")]
pub type SelectedRule = Heian8x8;
#[cfg(feature = "heian-9x8")]
pub type SelectedRule = Heian9x8;
#[cfg(feature = "heian-9x9")]
pub type SelectedRule = Heian9x9;

#[cfg(feature = "minishogi")]
pub type SelectedRule = Minishogi;
#[cfg(feature = "judkins")]
pub type SelectedRule = Judkins;
#[cfg(feature = "gorogoro")]
pub type SelectedRule = Gorogoro;
#[cfg(feature = "gorogoro-plus")]
pub type SelectedRule = GorogoroPlus;
#[cfg(feature = "minishogi-kingin")]
pub type SelectedRule = MinishogiKingin;
#[cfg(not(feature = "variant-board"))]
pub type SelectedRule = ConfiguredShogi;
pub type SelectedGeometry = <SelectedRule as RuleSpec>::Geometry;
pub const BOARD_WIDTH: usize = SelectedGeometry::WIDTH;
pub const BOARD_HEIGHT: usize = SelectedGeometry::HEIGHT;

#[cfg(test)]
mod tests;

//! Shogitter's remaining 「５五系」: identities and physical front/back pairs.
//! Sources: kyotoginkakushogi, kinkeimaishogi, microshogi,
//! kyotoshogi-vs-gogoshogi rule definitions (not ordinary promoted-piece aliases).
use super::*;
const KYOTO: &[PieceDef] = &[
    piece!("ab", &[], &[(0, -1)], Some(PieceId(1)), 0, false, 0, 300),
    piece!("ai", GOLD, &[], Some(PieceId(0)), 0, false, 0, 550),
    piece!("ad", SILVER, &[], Some(PieceId(3)), 1, false, 0, 450),
    piece!("af", &[], DIAG, Some(PieceId(2)), 1, false, 0, 750),
    piece!("ae", GOLD, &[], Some(PieceId(5)), 2, false, 0, 550),
    piece!(
        "ac",
        &[(-1, -2), (1, -2)],
        &[],
        Some(PieceId(4)),
        2,
        false,
        0,
        300
    ),
    piece!("ag", &[], ORTH, Some(PieceId(7)), 3, false, 0, 900),
    piece!("aa", &[(0, -1)], &[], Some(PieceId(6)), 3, false, 0, 100),
    piece!("ah", KING, &[], None, 4, true, 0, 2000),
];
const KINKEI: &[PieceDef] = &[
    piece!("ae", GOLD, &[], Some(PieceId(1)), 0, false, 0, 550),
    piece!(
        "ac",
        &[(-1, -2), (1, -2)],
        &[],
        Some(PieceId(0)),
        0,
        false,
        0,
        300
    ),
    piece!("ah", KING, &[], None, 1, true, 0, 2000),
];
const MICRO: &[PieceDef] = &[
    piece!("aa", &[(0, -1)], &[], Some(PieceId(1)), 0, false, 1, 100),
    piece!(
        "ac",
        &[(-1, -2), (1, -2)],
        &[],
        Some(PieceId(0)),
        0,
        false,
        2,
        300
    ),
    piece!("ad", SILVER, &[], Some(PieceId(3)), 1, false, 0, 450),
    piece!("ab", &[], &[(0, -1)], Some(PieceId(2)), 1, false, 1, 300),
    piece!("ae", GOLD, &[], Some(PieceId(5)), 2, false, 0, 550),
    piece!("ag", &[], ORTH, Some(PieceId(4)), 2, false, 0, 900),
    piece!("af", &[], DIAG, Some(PieceId(7)), 3, false, 0, 750),
    piece!("ai", GOLD, &[], Some(PieceId(6)), 3, false, 0, 550),
    piece!("ah", KING, &[], None, 4, true, 0, 2000),
];
const MIXED: &[PieceDef] = &[
    piece!("aa", &[(0, -1)], &[], Some(PieceId(1)), 0, false, 1, 100),
    piece!("ai", GOLD, &[], None, 0, false, 0, 550),
    piece!("ad", SILVER, &[], Some(PieceId(3)), 1, false, 0, 450),
    piece!("al", GOLD, &[], None, 1, false, 0, 550),
    piece!("ae", GOLD, &[], None, 2, false, 0, 550),
    piece!("af", &[], DIAG, Some(PieceId(6)), 3, false, 0, 750),
    piece!("am", ORTH, DIAG, None, 3, false, 0, 1000),
    piece!("ag", &[], ORTH, Some(PieceId(8)), 4, false, 0, 900),
    piece!("an", DIAG, ORTH, None, 4, false, 0, 1150),
    piece!("ah", KING, &[], None, 9, true, 0, 2000),
    piece!("0m", &[], &[(0, -1)], Some(PieceId(11)), 5, false, 0, 300),
    piece!("0s", GOLD, &[], Some(PieceId(10)), 5, false, 0, 550),
    piece!("0o", SILVER, &[], Some(PieceId(13)), 6, false, 0, 450),
    piece!("0q", &[], DIAG, Some(PieceId(12)), 6, false, 0, 750),
    piece!("0p", GOLD, &[], Some(PieceId(15)), 7, false, 0, 550),
    piece!(
        "0n",
        &[(-1, -2), (1, -2)],
        &[],
        Some(PieceId(14)),
        7,
        false,
        0,
        300
    ),
    piece!("0r", &[], ORTH, Some(PieceId(17)), 8, false, 0, 900),
    piece!("0l", &[(0, -1)], &[], Some(PieceId(16)), 8, false, 0, 100),
];

macro_rules! flip_rule {
    ($rule:ident, $id:literal, $name:literal, $w:literal, $pieces:ident, $inventory:expr, $multiplier:literal, $hands:literal, $initial:expr, $uchi:expr) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        pub struct $rule;
        impl CaptureRoyalRule for $rule {
            type Geometry = Rect<$w, 5>;
            type State = ClassicalState;
            type Hands = Hands<$hands>;
            type Placement = ClassicalPiece;
            const ID: u32 = $id;
            const PIECES: &'static [PieceDef] = $pieces;
            const INVENTORY: &'static [usize] = $inventory;
            const INVENTORY_MULTIPLIER: usize = $multiplier;
            const PROMOTION_RANKS: usize = 1;
            const ALLOW_DEAD_BOARD_PIECES: bool = Self::ID == 98;
            fn capture_return(id: PieceId) -> Option<PieceId> {
                let base = Self::PIECES[id.0 as usize].base;
                Some(PieceId(
                    Self::PIECES.iter().position(|p| p.base == base).unwrap() as u16,
                ))
            }
            fn file_limit(id: PieceId) -> usize {
                usize::from(Self::ID == 125 && id == PieceId(0))
            }
            fn initial() -> Vec<ClassicalPiece> {
                let entries: &[([u16; 2], &str, u8)] = $initial;
                entries
                    .iter()
                    .map(|&(at, id, color)| ClassicalPiece::make(at, id, color, None))
                    .collect()
            }
        }
        impl ClassicalRule for $rule {
            const NAME: &'static str = $name;
            const FORCED: bool = false;
            const CHECK: bool = true;
            // Shogitter WinKoma ends on checkmate, not a non-check stalemate.
            const STALEMATE: bool = false;
            const ONLY_KINGS: bool = false;
            const LION: bool = false;
            const UCHIFU: Option<PieceId> = $uchi;
            fn continuation(
                _id: PieceId,
                _delta: (isize, isize),
            ) -> Option<&'static [(isize, isize)]> {
                None
            }
            fn drop_hand(id: PieceId) -> PieceId {
                Self::capture_return(id).unwrap()
            }
            fn droppable(id: PieceId) -> bool {
                !Self::PIECES[id.0 as usize].royal
                    && (Self::ID != 125 || id.0 >= 10 || Self::drop_hand(id) == id)
            }
            fn promotion_options(
                id: PieceId,
                from: usize,
                to: usize,
                captured: bool,
            ) -> (bool, bool) {
                if Self::PIECES[id.0 as usize].promotion.is_none() {
                    return (false, false);
                }
                if Self::ID == 98 {
                    return (captured, captured);
                }
                if Self::ID == 125 && id.0 < 10 {
                    let can = from == 0 || to == 0;
                    return (can, can && to < Self::PIECES[id.0 as usize].dead_ranks);
                }
                (true, true)
            }
        }
    };
}

flip_rule!(
    Kyoto,
    19,
    "kyoto",
    5,
    KYOTO,
    &[1, 1, 1, 1, 1],
    2,
    9,
    &[
        ([1, 1], "ai", 1),
        ([2, 1], "ad", 1),
        ([3, 1], "ah", 1),
        ([4, 1], "ae", 1),
        ([5, 1], "aa", 1),
        ([1, 5], "aa", 0),
        ([2, 5], "ae", 0),
        ([3, 5], "ah", 0),
        ([4, 5], "ad", 0),
        ([5, 5], "ai", 0)
    ],
    Some(PieceId(7))
);

flip_rule!(
    KinkeiMai,
    123,
    "kinkei-mai",
    5,
    KINKEI,
    &[4, 1],
    2,
    3,
    &[
        ([1, 5], "ae", 0),
        ([2, 5], "ae", 0),
        ([3, 5], "ah", 0),
        ([4, 5], "ae", 0),
        ([5, 5], "ae", 0),
        ([1, 1], "ae", 1),
        ([2, 1], "ae", 1),
        ([3, 1], "ah", 1),
        ([4, 1], "ae", 1),
        ([5, 1], "ae", 1)
    ],
    None
);

flip_rule!(
    Microshogi,
    98,
    "microshogi",
    4,
    MICRO,
    &[1, 1, 1, 1, 1],
    2,
    9,
    &[
        ([1, 1], "ad", 1),
        ([2, 1], "ae", 1),
        ([3, 1], "af", 1),
        ([4, 1], "ah", 1),
        ([4, 2], "aa", 1),
        ([1, 4], "aa", 0),
        ([1, 5], "ah", 0),
        ([2, 5], "af", 0),
        ([3, 5], "ae", 0),
        ([4, 5], "ad", 0)
    ],
    None
);

flip_rule!(
    KyotoMinishogi,
    125,
    "kyoto-minishogi",
    5,
    MIXED,
    &[1, 1, 1, 1, 1, 1, 1, 1, 1, 2],
    1,
    18,
    &[
        ([1, 1], "ah", 1),
        ([1, 2], "aa", 1),
        ([2, 1], "ae", 1),
        ([3, 1], "ad", 1),
        ([4, 1], "af", 1),
        ([5, 1], "ag", 1),
        ([1, 5], "0l", 0),
        ([2, 5], "0p", 0),
        ([3, 5], "ah", 0),
        ([4, 5], "0o", 0),
        ([5, 5], "0s", 0)
    ],
    Some(PieceId(0))
);

#[cfg(test)]
mod tests;

//! Orthodox movement rules share geometry, reserves and completed-turn search.
use super::*;
macro_rules! ordinary_rule {
    ($rule:ident,$id:literal,$name:literal,$w:literal,$h:literal,$pieces:ident,$initial:ident,$inventory:expr,$hands:ty,$dan:literal) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        pub struct $rule;
        impl CaptureRoyalRule for $rule {
            type Geometry = Rect<$w, $h>;
            type State = ClassicalState;
            type Hands = $hands;
            type Placement = ClassicalPiece;
            const ID: u32 = $id;
            const PIECES: &'static [PieceDef] = $pieces;
            const INVENTORY: &'static [usize] = $inventory;
            const INVENTORY_MULTIPLIER: usize = 1;
            const CONVERTS_INVENTORY: bool = Self::ID == 20;
            const CACHE_ROYALS: bool = true;
            const CAPTURE_SUPPORT: i8 = match Self::ID {
                112 => 1,
                113 => -1,
                _ => 0,
            };
            fn captured_reserve(piece: PieceId, color: u8) -> Option<PieceId> {
                let base = Self::capture_return(piece)?;
                let species = Self::PIECES[base.0 as usize].id;
                let target = match (Self::ID, species, color) {
                    (20, "ac", 0) => "be",
                    (20, "be", 1) => "ac",
                    _ => return Some(base),
                };
                Self::PIECES
                    .iter()
                    .position(|p| p.id == target)
                    .map(|i| PieceId(i as u16))
            }
            const PROMOTION_RANKS: usize = $dan;
            // Core permits immobile moves when promotion is disabled; dead-rank drops remain forbidden.
            const ALLOW_DEAD_BOARD_PIECES: bool = Self::ID == 111;
            fn capture_return(id: PieceId) -> Option<PieceId> {
                let base = Self::PIECES[id.0 as usize].base;
                Some(PieceId(
                    Self::PIECES.iter().position(|p| p.base == base).unwrap() as u16,
                ))
            }
            fn file_limit(id: PieceId) -> usize {
                usize::from(
                    Self::ID != 138 && matches!(Self::PIECES[id.0 as usize].id, "aa" | "cq"),
                )
            }
            fn capture_allowed(mover: PieceId, victim: PieceId, color: u8) -> bool {
                let from = Self::PIECES[mover.0 as usize].id;
                let to = Self::PIECES[victim.0 as usize].id;
                match Self::ID {
                    30 => !(color == 1 && to == "ad"),
                    31 => to != "ad",
                    153 | 165 => from != "1e",
                    _ => true,
                }
            }
            fn initial() -> Vec<ClassicalPiece> {
                $initial
                    .iter()
                    .map(|&(at, id, color)| ClassicalPiece::make(at, id, color, None))
                    .collect()
            }
        }
        impl ClassicalRule for $rule {
            const NAME: &'static str = $name;
            const FORCED: bool = false;
            const CHECK: bool = !matches!(Self::ID, 17 | 20 | 21 | 153 | 165);
            const CHECKMATE: bool = Self::CHECK || matches!(Self::ID, 20 | 21);
            const STALEMATE: bool = false;
            const ONLY_KINGS: bool = false;
            const TRY: bool = Self::ID == 17;
            const FORCE_CAPTURE: bool = Self::ID == 28;
            const LION: bool = matches!(Self::ID, 22 | 117);
            const LION_CAPTURE_RULES: bool = false;
            const UCHIFU: Option<PieceId> = if matches!(Self::ID, 17 | 65 | 92 | 103 | 153 | 165) {
                None
            } else {
                Some(PieceId(0))
            };
            const INITIAL_SIDE: u8 = if matches!(Self::ID, 20 | 22 | 30) {
                1
            } else {
                0
            };
            fn droppable(id: PieceId) -> bool {
                !Self::PIECES[id.0 as usize].royal && Self::capture_return(id) == Some(id)
            }
            fn continuation(
                id: PieceId,
                delta: (isize, isize),
            ) -> Option<&'static [(isize, isize)]> {
                if Self::LION
                    && Self::PIECES[id.0 as usize].id == "bl"
                    && delta.0.abs().max(delta.1.abs()) == 1
                {
                    Some(&[
                        (1, -1),
                        (0, -1),
                        (-1, -1),
                        (1, 0),
                        (-1, 0),
                        (1, 1),
                        (0, 1),
                        (-1, 1),
                    ])
                } else {
                    None
                }
            }
        }
    };
}
include!("ordinary_defs.rs");

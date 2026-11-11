//! Orthodox movement rules share geometry, reserves and completed-turn search.
use super::*;
macro_rules! ordinary_rule {
    ($rule:ident,$id:literal,$name:literal,$w:literal,$h:literal,$pieces:ident,$initial:ident,$inventory:expr,$hands:literal,$dan:literal) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        pub struct $rule;
        impl CaptureRoyalRule for $rule {
            type Geometry = Rect<$w, $h>;
            type State = ClassicalState;
            type Hands = Hands<$hands>;
            type Placement = ClassicalPiece;
            const ID: u32 = $id;
            const PIECES: &'static [PieceDef] = $pieces;
            const INVENTORY: &'static [usize] = $inventory;
            const INVENTORY_MULTIPLIER: usize = 1;
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
            const CHECK: bool = !matches!(Self::ID, 17 | 153 | 165);
            const STALEMATE: bool = false;
            const ONLY_KINGS: bool = false;
            const TRY: bool = Self::ID == 17;
            const FORCE_CAPTURE: bool = Self::ID == 28;
            const LION: bool = false;
            const UCHIFU: Option<PieceId> = if matches!(Self::ID, 17 | 65 | 153 | 165) {
                None
            } else {
                Some(PieceId(0))
            };
            const INITIAL_SIDE: u8 = if Self::ID == 30 { 1 } else { 0 };
            fn droppable(id: PieceId) -> bool {
                !Self::PIECES[id.0 as usize].royal && Self::capture_return(id) == Some(id)
            }
            fn continuation(
                _id: PieceId,
                _delta: (isize, isize),
            ) -> Option<&'static [(isize, isize)]> {
                None
            }
        }
    };
}
include!("ordinary_defs.rs");

//! Orthogonal capture, inventory, and hand-based outcomes.
use crate::*;

pub const fn allows_ignored_check() -> bool {
    cfg!(any(feature = "nakamaware", feature = "suika"))
}

impl Board {
    pub(crate) fn receive_capture(&mut self, color: Color, piece: Piece) {
        if cfg!(feature = "fushiyo") {
            return;
        }
        let copies = if cfg!(feature = "zoshoku") { 2 } else { 1 };
        for _ in 0..copies {
            self.inner.take_in_hand(color, piece);
        }
        if cfg!(feature = "share") {
            self.inner.take_in_hand(!color, piece);
        }
        if cfg!(feature = "babanuki") && self.num_in_hand(color, piece) >= 2 {
            self.inner
                .unchecked_set_hand(color, piece, self.num_in_hand(color, piece) % 2);
        }
        if cfg!(feature = "suika") {
            let order = [
                Piece::Pawn,
                Piece::Lance,
                Piece::Knight,
                Piece::Silver,
                Piece::Gold,
                Piece::Bishop,
                Piece::Rook,
                Piece::King,
            ];
            for pair in order.windows(2) {
                let count = self.num_in_hand(color, pair[0]);
                if count >= 2 {
                    self.inner.unchecked_set_hand(color, pair[0], count % 2);
                    let next = self
                        .num_in_hand(color, pair[1])
                        .checked_add(count / 2)
                        .expect("hand inventory overflow");
                    self.inner.unchecked_set_hand(color, pair[1], next);
                }
            }
        }
    }

    // Nakamaware allows even friendly King captures; losing one's own physical
    // King takes precedence over any winning inventory outcome.
    pub(crate) fn captured_king_winner(&self) -> Option<Color> {
        if cfg!(feature = "nakamaware") {
            for color in Color::ALL {
                if !self.has(color, Piece::King) {
                    return Some(!color);
                }
            }
        }
        None
    }

    pub(crate) fn hand_rule_winner(&self) -> Option<Color> {
        let previous = !self.side_to_move();
        if cfg!(feature = "suika") && self.num_in_hand(previous, Piece::King) > 0 {
            return Some(previous);
        }
        if cfg!(feature = "tensuu") {
            let score: u64 = [
                (Piece::Pawn, 1),
                (Piece::Lance, 2),
                (Piece::Knight, 3),
                (Piece::Silver, 4),
                (Piece::Gold, 5),
            ]
            .into_iter()
            .map(|(p, v)| u64::from(self.num_in_hand(previous, p)) * v)
            .sum();
            if score >= 15 {
                return Some(previous);
            }
        }
        None
    }
}

/// Default public identity for a single hand-related rule. Composite profiles
/// intentionally have no inferred ID.
pub fn single_rule_id(profile: &str) -> Option<u16> {
    match profile {
        "standard+nakamaware" => Some(24),
        "standard+fushiyo" => Some(54),
        "standard+share" => Some(57),
        "standard+zoshoku" => Some(93),
        "standard+babanuki" => Some(143),
        "standard+tensuu" => Some(146),
        "standard+suika" => Some(155),
        _ => None,
    }
}

//! Ordered game/search history and history-sensitive adjudication.

use crate::{Board, Color};

/// Frozen Anhoku history/adjudication rules used by search and game records.
pub const ANHOKU_HISTORY_RULES_VERSION: &str = "anhoku-history-adjudication-v1";

/// A result which can be determined from the ordered position history alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryAdjudication {
    Ongoing,
    RepetitionDraw,
    PerpetualCheckLoss(Color),
}

impl HistoryAdjudication {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ongoing => "ongoing",
            Self::RepetitionDraw => "repetition-draw",
            Self::PerpetualCheckLoss(Color::Black) => "perpetual-check-loss:black",
            Self::PerpetualCheckLoss(Color::White) => "perpetual-check-loss:white",
        }
    }
}

/// An ordered, inclusive sequence of positions. The final position is current.
#[derive(Debug, Clone)]
pub struct PositionHistory {
    positions: Vec<Board>,
}

impl PositionHistory {
    pub fn new(root: Board) -> Self {
        Self {
            positions: vec![root],
        }
    }

    pub fn from_positions(positions: Vec<Board>) -> Result<Self, &'static str> {
        if positions.is_empty() {
            return Err("position history must not be empty");
        }
        Ok(Self { positions })
    }

    pub fn positions(&self) -> &[Board] {
        &self.positions
    }

    pub fn current(&self) -> &Board {
        self.positions
            .last()
            .expect("PositionHistory is always non-empty")
    }

    pub fn len(&self) -> usize {
        self.positions.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn push(&mut self, board: Board) {
        self.positions.push(board);
    }

    pub fn pop(&mut self) -> Option<Board> {
        (self.positions.len() > 1)
            .then(|| self.positions.pop())
            .flatten()
    }

    pub fn matches_current(&self, board: &Board) -> bool {
        self.current().same_position(board)
    }

    /// Apply the frozen fourfold/perpetual-check rule to the current position.
    pub fn adjudication(&self) -> HistoryAdjudication {
        let current = self.current();
        let occurrences = self
            .positions
            .iter()
            .enumerate()
            .filter_map(|(index, board)| board.same_position(current).then_some(index))
            .collect::<Vec<_>>();
        if occurrences.len() < 4 {
            return HistoryAdjudication::Ongoing;
        }

        // Only the interval establishing the current (fourth) occurrence is relevant.
        let start = occurrences[occurrences.len() - 4];
        let end = *occurrences.last().expect("four occurrences exist");
        let mut black_moved = false;
        let mut white_moved = false;
        let mut black_all_checks = true;
        let mut white_all_checks = true;
        for index in (start + 1)..=end {
            let mover = self.positions[index - 1].side_to_move();
            let gave_check = !self.positions[index].checkers().is_empty();
            match mover {
                Color::Black => {
                    black_moved = true;
                    black_all_checks &= gave_check;
                }
                Color::White => {
                    white_moved = true;
                    white_all_checks &= gave_check;
                }
            }
        }

        let black_continuous = black_moved && black_all_checks;
        let white_continuous = white_moved && white_all_checks;
        match (black_continuous, white_continuous) {
            (true, false) => HistoryAdjudication::PerpetualCheckLoss(Color::Black),
            (false, true) => HistoryAdjudication::PerpetualCheckLoss(Color::White),
            _ => HistoryAdjudication::RepetitionDraw,
        }
    }

    /// Contextual TT key. Entire ordered history is mixed deliberately: this is
    /// conservative, deterministic, and prevents unsafe sharing across histories.
    pub fn tt_key(&self) -> u64 {
        const SEED: u64 = 0x6a09_e667_f3bc_c909;
        const STEP: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut key = SEED ^ (self.positions.len() as u64).wrapping_mul(STEP);
        for board in &self.positions {
            let checked = u64::from(!board.checkers().is_empty());
            let side = u64::from(board.side_to_move() == Color::White);
            let value = board.hash() ^ checked.rotate_left(17) ^ side.rotate_left(41);
            key ^= value
                .wrapping_add(STEP)
                .wrapping_add(key << 6)
                .wrapping_add(key >> 2);
            key = key.rotate_left(23).wrapping_mul(0x94d0_49bb_1331_11eb);
        }
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Move;
    use core::str::FromStr;

    fn history(sfen: &str, moves: &[&str]) -> PositionHistory {
        let mut board = Board::from_sfen(sfen).unwrap();
        let mut history = PositionHistory::new(board.clone());
        for text in moves {
            board.try_play(Move::from_str(text).unwrap()).unwrap();
            history.push(board.clone());
        }
        history
    }

    #[test]
    fn ordinary_fourfold_is_draw() {
        let cycle = ["5i4i", "5a4a", "4i5i", "4a5a"];
        let moves = cycle.repeat(3);
        let history = history("4k4/9/9/9/9/9/9/9/4K4 b - 1", &moves);
        assert_eq!(history.adjudication(), HistoryAdjudication::RepetitionDraw);
    }

    #[test]
    fn black_perpetual_checker_loses() {
        let cycle = ["4c5c", "5a4a", "5c4c", "4a5a"];
        let moves = cycle.repeat(3);
        let history = history("4k4/9/5R3/9/9/9/9/9/K8 b - 1", &moves);
        assert_eq!(
            history.adjudication(),
            HistoryAdjudication::PerpetualCheckLoss(Color::Black)
        );
    }

    #[test]
    fn white_perpetual_checker_loses() {
        let cycle = ["4g5g", "5i4i", "5g4g", "4i5i"];
        let moves = cycle.repeat(3);
        let history = history("k8/9/9/9/9/9/5r3/9/4K4 w - 1", &moves);
        assert_eq!(
            history.adjudication(),
            HistoryAdjudication::PerpetualCheckLoss(Color::White)
        );
    }

    #[test]
    fn same_board_different_history_has_different_tt_key() {
        let sfen = "4k4/9/9/9/9/9/9/9/4K4 b - 1";
        let fresh = history(sfen, &[]);
        let cycle = ["5i4i", "5a4a", "4i5i", "4a5a"];
        let repeated = history(sfen, &cycle.repeat(2));
        assert!(fresh.current().same_position(repeated.current()));
        assert_ne!(fresh.tt_key(), repeated.tt_key());
    }
}

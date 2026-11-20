//! Counterfactual features only: captured pieces remain usable by the evaluator,
//! but the real Board, move generation, terminal rules and TT history do not change.
use super::*;
use haitaka::PositionHistory;
const STANDARD_INVENTORY: [u8; 7] = [18, 4, 4, 4, 2, 2, 4];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GhostHands {
    origins: [u8; 81],
    hands: [[u8; 7]; 2],
}
impl GhostHands {
    fn new(board: &Board) -> Self {
        let mut origins = [0; 81];
        for square in Square::ALL {
            if let Some(piece) = board.piece_on(square) {
                origins[square as usize] = piece.unpromote() as u8 + 1;
            }
        }
        Self {
            origins,
            hands: [[0; 7]; 2],
        }
    }
    pub(super) fn play(&mut self, parent: &Board, mv: Move) {
        let Move::BoardMove { from, to, .. } = mv else {
            panic!("no drops in these diagnostic rules")
        };
        if parent.piece_on(to).is_some() {
            let original = self.origins[to as usize]
                .checked_sub(1)
                .expect("captured origin required") as usize;
            assert!(original < 7, "king captures have no static neural state");
            let count = &mut self.hands[parent.side_to_move() as usize][original];
            *count += 1;
            assert!(
                *count <= STANDARD_INVENTORY[original],
                "counterfactual hand exceeds model encoding"
            );
        }
        self.origins[to as usize] = self.origins[from as usize];
        self.origins[from as usize] = 0;
    }
    pub(super) fn feature_board(&self, board: &Board) -> Board {
        let mut features = board.clone();
        for color in Color::ALL {
            assert!(board.is_hand_empty(color));
            for piece in 0..7 {
                features.unchecked_set_hand(
                    color,
                    Piece::index_const(piece),
                    u32::from(self.hands[color as usize][piece]),
                );
            }
        }
        features
    }
    fn from_history(history: &PositionHistory) -> Self {
        let mut ghost = Self::new(&history.positions()[0]);
        for pair in history.positions().windows(2) {
            let color = pair[0].side_to_move();
            let from = pair[0].colors(color) & !pair[1].colors(color);
            let to = pair[1].colors(color) & !pair[0].colors(color);
            if from.is_empty() && to.is_empty() {
                continue;
            } // search null move
            assert_eq!(from.len(), 1);
            assert_eq!(to.len(), 1);
            ghost.play(
                &pair[0],
                Move::BoardMove {
                    from: from.next_square().unwrap(),
                    to: to.next_square().unwrap(),
                    promotion: false,
                },
            );
        }
        ghost
    }
}

impl SfnnModel {
    pub fn diagnostic_enable_ghost_hands(&mut self) -> Result<(), String> {
        if !cfg!(any(feature = "fushiyo", feature = "heian-9x9")) {
            return Err("ghost-hand experiment is restricted to Fushiyo and Heian 9x9".into());
        }
        self.diagnostic_ghost_hands = true;
        Ok(())
    }
    pub(crate) fn diagnostic_seed_history(
        &self,
        board: &Board,
        history: &PositionHistory,
        state: SfnnPositionState,
    ) -> SfnnPositionState {
        if !self.diagnostic_ghost_hands {
            return state;
        }
        let ghost = GhostHands::from_history(history);
        let mut state = self.build_position_state_full(&ghost.feature_board(board));
        state.ghost_hands = Some(ghost);
        state
    }
}

#[cfg(all(test, feature = "fushiyo"))]
mod tests {
    use super::*;
    #[test]
    fn ghost_capture_ledger_preserves_bishop_origins_without_real_hands_or_drops() {
        let mut board = Board::startpos();
        let mut history = PositionHistory::new(board.clone());
        for text in ["7g7f", "3c3d", "8h2b+", "3a2b"] {
            board.try_play(text.parse().unwrap()).unwrap();
            history.push(board.clone());
        }
        assert!(Color::ALL.iter().all(|&c| board.is_hand_empty(c)));
        let ghost = GhostHands::from_history(&history);
        assert_eq!(
            ghost.hands[Color::Black as usize][Piece::Bishop as usize],
            1
        );
        assert_eq!(
            ghost.hands[Color::White as usize][Piece::Bishop as usize],
            1
        );
        let feature_board = ghost.feature_board(&board);
        assert_eq!(feature_board.occupied(), board.occupied());
        for color in Color::ALL {
            assert_eq!(feature_board.num_in_hand(color, Piece::Bishop), 1);
        }
        let mut drops = 0;
        board.generate_drops(|g| {
            drops += g.len();
            false
        });
        assert_eq!(drops, 0);
    }
}
#[cfg(all(test, feature = "heian-9x9"))]
mod heian_tests {
    use super::*;
    #[test]
    fn ghost_capture_ledger_preserves_physical_pawn_through_canonical_gold_promotion() {
        let mut board = Board::startpos();
        let mut history = PositionHistory::new(board.clone());
        for text in [
            "7g7f", "3c3d", "7f7e", "3d3e", "7e7d", "3e3f", "7d7c+", "3f3g+",
        ] {
            board.try_play(text.parse().unwrap()).unwrap();
            history.push(board.clone());
        }
        let ghost = GhostHands::from_history(&history);
        assert_eq!(board.piece_on("7c".parse().unwrap()), Some(Piece::Gold));
        assert_eq!(
            ghost.origins["7c".parse::<Square>().unwrap() as usize],
            Piece::Pawn as u8 + 1
        );
        for color in Color::ALL {
            assert!(board.is_hand_empty(color));
            assert_eq!(ghost.hands[color as usize][Piece::Pawn as usize], 1);
        }
    }
}

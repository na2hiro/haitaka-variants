use std::cmp::{Ordering, Reverse};

use haitaka::{Board, Color, Move, Piece, Square};

const KILLER_SLOTS: usize = 2;
const MAX_SEARCH_PLY: usize = 128;
const BOARD_HISTORY_PER_SIDE: usize = Square::NUM * Square::NUM * 2;
const DROP_HISTORY_PER_SIDE: usize = Piece::HAND_NUM * Square::NUM;
const HISTORY_PER_SIDE: usize = BOARD_HISTORY_PER_SIDE + DROP_HISTORY_PER_SIDE;
const HISTORY_SIZE: usize = 2 * HISTORY_PER_SIDE;
const HISTORY_LIMIT: i32 = 1 << 20;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchOrderingStats {
    pub beta_cutoffs: u64,
    pub first_move_cutoffs: u64,
    pub hash_move_tries: u64,
    pub hash_move_cutoffs: u64,
    pub killer_move_tries: u64,
    pub killer_move_cutoffs: u64,
    pub history_move_tries: u64,
    pub history_move_cutoffs: u64,
}

impl SearchOrderingStats {
    pub fn add_iteration(&mut self, iteration: Self) {
        self.beta_cutoffs += iteration.beta_cutoffs;
        self.first_move_cutoffs += iteration.first_move_cutoffs;
        self.hash_move_tries += iteration.hash_move_tries;
        self.hash_move_cutoffs += iteration.hash_move_cutoffs;
        self.killer_move_tries += iteration.killer_move_tries;
        self.killer_move_cutoffs += iteration.killer_move_cutoffs;
        self.history_move_tries += iteration.history_move_tries;
        self.history_move_cutoffs += iteration.history_move_cutoffs;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveSource {
    Hash,
    Tactical,
    Killer,
    History,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickedMove {
    pub mv: Move,
    pub source: MoveSource,
}

pub struct SearchOrdering {
    killers: [[Option<Move>; KILLER_SLOTS]; MAX_SEARCH_PLY],
    history: Vec<i32>,
}

impl SearchOrdering {
    pub fn new() -> Self {
        Self {
            killers: [[None; KILLER_SLOTS]; MAX_SEARCH_PLY],
            history: vec![0; HISTORY_SIZE],
        }
    }

    pub fn killers_for_ply(&self, ply: usize) -> [Option<Move>; KILLER_SLOTS] {
        self.killers
            .get(ply)
            .copied()
            .unwrap_or([None; KILLER_SLOTS])
    }

    pub fn record_beta_cutoff(&mut self, side: Color, mv: Move, depth: u8, ply: usize) {
        if !Self::is_quiet(mv) {
            return;
        }

        if let Some(killers) = self.killers.get_mut(ply) {
            if killers[0] != Some(mv) {
                killers[1] = killers[0];
                killers[0] = Some(mv);
            }
        }

        let key = history_key(side, mv);
        let bonus = i32::from(depth.max(1)).pow(2);
        self.history[key] = self.history[key].saturating_add(bonus);
        if self.history[key] > HISTORY_LIMIT {
            for score in &mut self.history {
                *score /= 2;
            }
        }
    }

    fn history_score(&self, side: Color, mv: Move) -> i32 {
        self.history[history_key(side, mv)]
    }

    fn is_quiet(mv: Move) -> bool {
        !mv.is_promotion()
    }
}

impl Default for SearchOrdering {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MovePicker {
    hash: MoveStage,
    winning_tactical: MoveStage,
    equal_tactical: MoveStage,
    killer: MoveStage,
    history: MoveStage,
    losing_tactical: MoveStage,
    stage: PickStage,
}

impl MovePicker {
    pub fn new(
        board: &Board,
        tt_move: Option<Move>,
        ordering: &SearchOrdering,
        ply: usize,
    ) -> Self {
        let side = board.side_to_move();
        let killers = ordering.killers_for_ply(ply);
        let mut picker = Self {
            hash: MoveStage::default(),
            winning_tactical: MoveStage::default(),
            equal_tactical: MoveStage::default(),
            killer: MoveStage::default(),
            history: MoveStage::default(),
            losing_tactical: MoveStage::default(),
            stage: PickStage::Hash,
        };

        board.generate_moves(|piece_moves| {
            for mv in piece_moves {
                picker.push_scored(ScoredMove::new(
                    board,
                    side,
                    mv,
                    tt_move,
                    killers,
                    ordering.history_score(side, mv),
                ));
            }
            false
        });
        picker.sort_stages();
        picker
    }

    fn push_scored(&mut self, scored: ScoredMove) {
        if scored.is_hash {
            self.hash.push(scored);
        } else if scored.is_tactical() && scored.gain > 0 {
            self.winning_tactical.push(scored);
        } else if scored.is_tactical() && scored.gain == 0 {
            self.equal_tactical.push(scored);
        } else if !scored.is_tactical() && scored.killer_slot.is_some() {
            self.killer.push(scored);
        } else if !scored.is_tactical() {
            self.history.push(scored);
        } else {
            self.losing_tactical.push(scored);
        }
    }

    fn sort_stages(&mut self) {
        self.hash.sort();
        self.winning_tactical.sort();
        self.equal_tactical.sort();
        self.killer.sort();
        self.history.sort();
        self.losing_tactical.sort();
    }

    pub fn is_empty(&self) -> bool {
        self.hash.is_empty()
            && self.winning_tactical.is_empty()
            && self.equal_tactical.is_empty()
            && self.killer.is_empty()
            && self.history.is_empty()
            && self.losing_tactical.is_empty()
    }

    pub fn next(&mut self) -> Option<PickedMove> {
        loop {
            let selected = match self.stage {
                PickStage::Hash => self.hash.next(MoveSource::Hash),
                PickStage::WinningTactical => self.winning_tactical.next(MoveSource::Tactical),
                PickStage::EqualTactical => self.equal_tactical.next(MoveSource::Tactical),
                PickStage::Killer => self.killer.next(MoveSource::Killer),
                PickStage::History => self.history.next(MoveSource::History),
                PickStage::LosingTactical => self.losing_tactical.next(MoveSource::Tactical),
                PickStage::Done => return None,
            };

            if let Some(mv) = selected {
                return Some(mv);
            }
            self.stage = self.stage.next();
        }
    }
}

#[derive(Default)]
struct MoveStage {
    moves: Vec<ScoredMove>,
    next: usize,
}

impl MoveStage {
    fn push(&mut self, scored: ScoredMove) {
        self.moves.push(scored);
    }

    fn sort(&mut self) {
        self.moves.sort_unstable_by(ScoredMove::cmp_for_stage);
    }

    fn is_empty(&self) -> bool {
        self.next >= self.moves.len()
    }

    fn next(&mut self, source: MoveSource) -> Option<PickedMove> {
        let scored = *self.moves.get(self.next)?;
        self.next += 1;
        Some(PickedMove {
            mv: scored.mv,
            source,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickStage {
    Hash,
    WinningTactical,
    EqualTactical,
    Killer,
    History,
    LosingTactical,
    Done,
}

impl PickStage {
    const fn next(self) -> Self {
        match self {
            Self::Hash => Self::WinningTactical,
            Self::WinningTactical => Self::EqualTactical,
            Self::EqualTactical => Self::Killer,
            Self::Killer => Self::History,
            Self::History => Self::LosingTactical,
            Self::LosingTactical => Self::Done,
            Self::Done => Self::Done,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScoredMove {
    mv: Move,
    is_hash: bool,
    is_tactical: bool,
    gain: i32,
    history: i32,
    killer_slot: Option<usize>,
    fallback: FallbackKey,
}

impl ScoredMove {
    fn new(
        board: &Board,
        side: Color,
        mv: Move,
        tt_move: Option<Move>,
        killers: [Option<Move>; KILLER_SLOTS],
        history: i32,
    ) -> Self {
        let is_hash = tt_move == Some(mv);
        let capture = capture_value(board, side, mv);
        let promotion = promotion_gain(board, mv);
        let attacker = attacker_value(board, mv);
        let is_tactical = capture > 0 || promotion > 0;
        let gain = if capture > 0 {
            capture + promotion - attacker
        } else if promotion > 0 {
            promotion
        } else {
            0
        };
        let killer_slot = killers.iter().position(|killer| *killer == Some(mv));
        Self {
            mv,
            is_hash,
            is_tactical,
            gain,
            history,
            killer_slot,
            fallback: FallbackKey::new(mv),
        }
    }

    const fn is_tactical(self) -> bool {
        self.is_tactical
    }

    fn cmp_for_stage(&self, other: &Self) -> Ordering {
        (
            Reverse(self.gain),
            Reverse(self.history),
            self.killer_slot.unwrap_or(KILLER_SLOTS),
            self.fallback,
        )
            .cmp(&(
                Reverse(other.gain),
                Reverse(other.history),
                other.killer_slot.unwrap_or(KILLER_SLOTS),
                other.fallback,
            ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FallbackKey {
    to: u8,
    from_or_piece: u8,
    drop: u8,
    promotion: u8,
}

impl FallbackKey {
    fn new(mv: Move) -> Self {
        Self {
            to: mv.to() as u8,
            from_or_piece: move_from_or_piece_index(mv),
            drop: mv.is_drop() as u8,
            promotion: mv.is_promotion() as u8,
        }
    }
}

fn history_key(side: Color, mv: Move) -> usize {
    let side_offset = side as usize * HISTORY_PER_SIDE;
    match mv {
        Move::BoardMove {
            from,
            to,
            promotion,
        } => {
            side_offset + ((from as usize * Square::NUM + to as usize) * 2 + usize::from(promotion))
        }
        Move::Drop { piece, to } => {
            side_offset + BOARD_HISTORY_PER_SIDE + piece as usize * Square::NUM + to as usize
        }
    }
}

const fn move_from_or_piece_index(mv: Move) -> u8 {
    match mv {
        Move::BoardMove { from, .. } => from as u8,
        Move::Drop { piece, .. } => piece as u8,
    }
}

fn capture_value(board: &Board, side: Color, mv: Move) -> i32 {
    match mv {
        Move::BoardMove { to, .. } => board
            .color_on(to)
            .filter(|color| *color != side)
            .and_then(|_| board.piece_on(to))
            .map(piece_value)
            .unwrap_or(0),
        Move::Drop { .. } => 0,
    }
}

fn attacker_value(board: &Board, mv: Move) -> i32 {
    match mv {
        Move::BoardMove { from, .. } => board.piece_on(from).map(piece_value).unwrap_or(0),
        Move::Drop { piece, .. } => piece_value(piece),
    }
}

fn promotion_gain(board: &Board, mv: Move) -> i32 {
    match mv {
        Move::BoardMove {
            from,
            promotion: true,
            ..
        } => board
            .piece_on(from)
            .map(|piece| piece_value(piece.promote()) - piece_value(piece))
            .unwrap_or(0),
        _ => 0,
    }
}

fn piece_value(piece: Piece) -> i32 {
    match piece {
        Piece::Pawn => 100,
        Piece::Lance => 300,
        Piece::Knight => 300,
        Piece::Silver => 400,
        Piece::Gold => 500,
        Piece::Bishop => 700,
        Piece::Rook => 800,
        Piece::King => 0,
        Piece::Tokin | Piece::PLance | Piece::PKnight | Piece::PSilver => 550,
        Piece::PBishop => 900,
        Piece::PRook => 1000,
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn collect_picker_moves(
        board: &Board,
        tt_move: Option<Move>,
        ordering: &SearchOrdering,
        ply: usize,
    ) -> Vec<PickedMove> {
        let mut picker = MovePicker::new(board, tt_move, ordering, ply);
        let mut moves = Vec::new();
        while let Some(picked) = picker.next() {
            moves.push(picked);
        }
        moves
    }

    #[test]
    fn tt_move_is_first_only_when_legal() {
        let board = Board::from_sfen(haitaka::SFEN_STARTPOS).unwrap();
        let ordering = SearchOrdering::new();
        let legal = Move::from_str("7g7f").unwrap();
        let illegal = Move::from_str("1a1b").unwrap();

        let moves = collect_picker_moves(&board, Some(legal), &ordering, 0);
        assert_eq!(moves.first().map(|picked| picked.mv), Some(legal));
        assert_eq!(
            moves.first().map(|picked| picked.source),
            Some(MoveSource::Hash)
        );

        let moves = collect_picker_moves(&board, Some(illegal), &ordering, 0);
        assert_ne!(moves.first().map(|picked| picked.mv), Some(illegal));
        assert!(moves.iter().all(|picked| picked.mv != illegal));
    }

    #[test]
    fn tactical_capture_precedes_killer_and_quiet() {
        let board = Board::from_sfen("9/9/k8/9/4Rr3/9/9/9/4K4 b P 1").unwrap();
        let mut ordering = SearchOrdering::new();
        let quiet_drop = Move::from_str("P*5f").unwrap();
        ordering.record_beta_cutoff(board.side_to_move(), quiet_drop, 4, 0);

        let moves = collect_picker_moves(&board, None, &ordering, 0);
        let capture = Move::from_str("5e4e").unwrap();
        let capture_index = moves
            .iter()
            .position(|picked| picked.mv == capture)
            .expect("capture should be legal");
        let quiet_index = moves
            .iter()
            .position(|picked| picked.mv == quiet_drop)
            .expect("quiet drop should be legal");
        assert_eq!(moves[capture_index].source, MoveSource::Tactical);
        assert!(capture_index < quiet_index);
    }

    #[test]
    fn non_capture_major_promotion_is_winning_tactical() {
        let board = Board::from_sfen("4k4/9/4B4/9/9/9/9/9/4K4 b - 1").unwrap();
        let ordering = SearchOrdering::new();
        let promotion = Move::from_str("5c4b+").unwrap();
        let quiet = Move::from_str("5i5h").unwrap();

        let scored = ScoredMove::new(
            &board,
            board.side_to_move(),
            promotion,
            None,
            [None; KILLER_SLOTS],
            0,
        );
        assert!(scored.is_tactical());
        assert_eq!(scored.gain, 200);

        let moves = collect_picker_moves(&board, None, &ordering, 0);
        let promotion_index = moves
            .iter()
            .position(|picked| picked.mv == promotion)
            .expect("promotion should be legal");
        let quiet_index = moves
            .iter()
            .position(|picked| picked.mv == quiet)
            .expect("quiet king move should be legal");
        assert_eq!(moves[promotion_index].source, MoveSource::Tactical);
        assert!(promotion_index < quiet_index);
    }

    #[test]
    fn quiet_drops_and_board_moves_use_distinct_history_keys() {
        let board_move = Move::from_str("7g7f").unwrap();
        let drop = Move::from_str("P*7f").unwrap();
        assert_ne!(
            history_key(Color::Black, board_move),
            history_key(Color::Black, drop)
        );
        assert_ne!(
            history_key(Color::Black, board_move),
            history_key(Color::White, board_move)
        );
    }

    #[test]
    fn killer_insertion_avoids_duplicates_and_keeps_two_slots() {
        let mut ordering = SearchOrdering::new();
        let first = Move::from_str("7g7f").unwrap();
        let second = Move::from_str("2g2f").unwrap();

        ordering.record_beta_cutoff(Color::Black, first, 3, 5);
        ordering.record_beta_cutoff(Color::Black, first, 3, 5);
        assert_eq!(ordering.killers_for_ply(5), [Some(first), None]);

        ordering.record_beta_cutoff(Color::Black, second, 3, 5);
        assert_eq!(ordering.killers_for_ply(5), [Some(second), Some(first)]);
    }
}

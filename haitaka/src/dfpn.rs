use core::mem::size_of;

use instant::Instant;

use crate::*;

const INF_PN: u32 = u32::MAX / 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DfpnStatus {
    Mate,
    NoMate,
    Unknown,
}

impl DfpnStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mate => "mate",
            Self::NoMate => "no_mate",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DfpnOptions {
    pub max_nodes: Option<u64>,
    pub max_time_ms: Option<u64>,
    pub tt_megabytes: usize,
    pub max_pv_moves: usize,
}

impl Default for DfpnOptions {
    fn default() -> Self {
        Self {
            max_nodes: None,
            max_time_ms: None,
            tt_megabytes: 16,
            max_pv_moves: 256,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DfpnStats {
    pub nodes: u64,
    pub tt_hits: u64,
    pub tt_stores: u64,
    pub tt_collisions: u64,
    pub elapsed_ms: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DfpnResult {
    pub status: DfpnStatus,
    pub pv: Vec<Move>,
    pub stats: DfpnStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeKind {
    Attacker,
    Defender,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProofNumbers {
    pn: u32,
    dn: u32,
}

impl ProofNumbers {
    const UNKNOWN: Self = Self { pn: 1, dn: 1 };
    const MATE: Self = Self { pn: 0, dn: INF_PN };
    const NO_MATE: Self = Self { pn: INF_PN, dn: 0 };

    const fn is_resolved(self) -> bool {
        self.pn == 0 || self.dn == 0 || self.pn >= INF_PN || self.dn >= INF_PN
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NodeEvaluation {
    numbers: ProofNumbers,
    best_move: Option<Move>,
}

#[derive(Debug, Clone)]
struct Child {
    mv: Move,
    board: Board,
    numbers: ProofNumbers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TtEntry {
    hash: u64,
    numbers: ProofNumbers,
    best_move: Option<Move>,
    generation: u8,
    occupied: bool,
}

impl Default for TtEntry {
    fn default() -> Self {
        Self {
            hash: 0,
            numbers: ProofNumbers::UNKNOWN,
            best_move: None,
            generation: 0,
            occupied: false,
        }
    }
}

impl TtEntry {
    const fn evaluation(self) -> NodeEvaluation {
        NodeEvaluation {
            numbers: self.numbers,
            best_move: self.best_move,
        }
    }
}

struct DfpnSolver {
    attacker: Color,
    options: DfpnOptions,
    started_at: Instant,
    stats: DfpnStats,
    budget_hit: bool,
    generation: u8,
    tt: Vec<TtEntry>,
}

impl DfpnSolver {
    fn new(attacker: Color, options: DfpnOptions) -> Self {
        let tt_size = tt_capacity(options.tt_megabytes.max(1));
        Self {
            attacker,
            options,
            started_at: Instant::now(),
            stats: DfpnStats {
                nodes: 0,
                tt_hits: 0,
                tt_stores: 0,
                tt_collisions: 0,
                elapsed_ms: 0.0,
            },
            budget_hit: false,
            generation: 1,
            tt: vec![TtEntry::default(); tt_size],
        }
    }

    fn solve(mut self, board: &Board) -> DfpnResult {
        let mut path = Vec::new();
        let evaluation = self.search(board, INF_PN, INF_PN, &mut path);
        self.stats.elapsed_ms = self.started_at.elapsed().as_secs_f64() * 1_000.0;

        let status = if evaluation.numbers.pn == 0 {
            DfpnStatus::Mate
        } else if evaluation.numbers.dn == 0 {
            DfpnStatus::NoMate
        } else {
            DfpnStatus::Unknown
        };

        let pv = if status == DfpnStatus::Mate {
            self.reconstruct_pv(board)
        } else {
            Vec::new()
        };

        DfpnResult {
            status,
            pv,
            stats: self.stats,
        }
    }

    fn search(
        &mut self,
        board: &Board,
        phi: u32,
        delta: u32,
        path: &mut Vec<u64>,
    ) -> NodeEvaluation {
        self.stats.nodes += 1;
        if self.over_budget() {
            return NodeEvaluation {
                numbers: ProofNumbers::UNKNOWN,
                best_move: None,
            };
        }

        let hash = board.hash();
        if path.contains(&hash) {
            return NodeEvaluation {
                numbers: ProofNumbers::UNKNOWN,
                best_move: None,
            };
        }

        if let Some(entry) = self.probe(hash) {
            if entry.numbers.is_resolved() || entry.numbers.pn >= phi || entry.numbers.dn >= delta {
                return entry.evaluation();
            }
        }

        let kind = self.node_kind(board);
        let tt_move = self.entry(hash).and_then(|entry| entry.best_move);
        let moves = collect_candidate_moves(board, kind, tt_move);
        if moves.is_empty() {
            let numbers = terminal_numbers(kind);
            self.store(hash, numbers, None);
            return NodeEvaluation {
                numbers,
                best_move: None,
            };
        }

        let mut children = Vec::with_capacity(moves.len());
        for mv in moves {
            let mut child = board.clone();
            child.play_unchecked(mv);
            children.push(Child {
                mv,
                numbers: self.initial_child_numbers(&child, path),
                board: child,
            });
        }

        let mut evaluation = summarize(kind, &children);
        while evaluation.numbers.pn < phi && evaluation.numbers.dn < delta {
            let (best_index, second_numbers) = select_most_proving_child(kind, &children);
            let best_child = &children[best_index];
            let (child_phi_target, child_delta_target) =
                child_thresholds(kind, evaluation.numbers, best_child.numbers, second_numbers, phi, delta);
            let child_phi = mid(best_child.numbers.pn, child_phi_target);
            let child_delta = mid(best_child.numbers.dn, child_delta_target);

            let previous_node_numbers = evaluation.numbers;
            let previous_child_numbers = best_child.numbers;

            path.push(hash);
            let child_evaluation =
                self.search(&best_child.board, child_phi, child_delta, path);
            path.pop();

            children[best_index].numbers = child_evaluation.numbers;
            evaluation = summarize(kind, &children);

            if evaluation.numbers == previous_node_numbers
                && children[best_index].numbers == previous_child_numbers
            {
                break;
            }
        }

        self.store(hash, evaluation.numbers, evaluation.best_move);
        evaluation
    }

    fn node_kind(&self, board: &Board) -> NodeKind {
        if board.side_to_move() == self.attacker {
            NodeKind::Attacker
        } else {
            NodeKind::Defender
        }
    }

    fn over_budget(&mut self) -> bool {
        if self
            .options
            .max_nodes
            .is_some_and(|max_nodes| self.stats.nodes > max_nodes)
        {
            self.budget_hit = true;
            return true;
        }

        if self.options.max_time_ms.is_some_and(|max_time_ms| {
            self.started_at.elapsed().as_millis() >= max_time_ms as u128
        }) {
            self.budget_hit = true;
            return true;
        }

        false
    }

    fn initial_child_numbers(&mut self, board: &Board, path: &[u64]) -> ProofNumbers {
        let hash = board.hash();
        if path.contains(&hash) {
            return ProofNumbers::UNKNOWN;
        }
        self.probe(hash)
            .map(|entry| entry.numbers)
            .unwrap_or(ProofNumbers::UNKNOWN)
    }

    fn entry(&self, hash: u64) -> Option<TtEntry> {
        let entry = self.tt[tt_index(hash, self.tt.len())];
        (entry.occupied && entry.hash == hash).then_some(entry)
    }

    fn probe(&mut self, hash: u64) -> Option<TtEntry> {
        let entry = self.entry(hash)?;
        self.stats.tt_hits += 1;
        Some(entry)
    }

    fn store(&mut self, hash: u64, numbers: ProofNumbers, best_move: Option<Move>) {
        let index = tt_index(hash, self.tt.len());
        if self.tt[index].occupied && self.tt[index].hash != hash {
            self.stats.tt_collisions += 1;
        }
        self.tt[index] = TtEntry {
            hash,
            numbers,
            best_move,
            generation: self.generation,
            occupied: true,
        };
        self.stats.tt_stores += 1;
    }

    fn reconstruct_pv(&self, root: &Board) -> Vec<Move> {
        let mut board = root.clone();
        let mut pv = Vec::new();
        let mut path = Vec::new();

        while pv.len() < self.options.max_pv_moves {
            let hash = board.hash();
            if path.contains(&hash) {
                break;
            }
            path.push(hash);

            let Some(entry) = self.entry(hash) else {
                break;
            };
            let Some(mv) = entry.best_move else {
                break;
            };

            pv.push(mv);
            board.play_unchecked(mv);

            let next_kind = if board.side_to_move() == self.attacker {
                NodeKind::Attacker
            } else {
                NodeKind::Defender
            };
            if !has_candidates(&board, next_kind) {
                break;
            }
        }

        pv
    }
}

impl Board {
    pub fn dfpn(&self, options: &DfpnOptions) -> DfpnResult {
        DfpnSolver::new(self.side_to_move(), *options).solve(self)
    }
}

fn tt_capacity(tt_megabytes: usize) -> usize {
    let bytes = tt_megabytes.saturating_mul(1024 * 1024);
    let entries = (bytes / size_of::<TtEntry>()).max(1);
    entries.next_power_of_two()
}

fn tt_index(hash: u64, capacity: usize) -> usize {
    debug_assert!(capacity > 0);
    (hash as usize) & (capacity - 1)
}

fn collect_candidate_moves(board: &Board, kind: NodeKind, tt_move: Option<Move>) -> Vec<Move> {
    let mut moves = Vec::new();
    match kind {
        NodeKind::Attacker => board.generate_checks(|piece_moves| {
            moves.extend(piece_moves);
            false
        }),
        NodeKind::Defender => board.generate_moves(|piece_moves| {
            moves.extend(piece_moves);
            false
        }),
    };

    if let Some(tt_move) = tt_move
        && let Some(index) = moves.iter().position(|mv| *mv == tt_move)
    {
        moves.swap(0, index);
    }

    moves
}

fn has_candidates(board: &Board, kind: NodeKind) -> bool {
    match kind {
        NodeKind::Attacker => board.generate_checks(|_| true),
        NodeKind::Defender => board.generate_moves(|_| true),
    }
}

const fn terminal_numbers(kind: NodeKind) -> ProofNumbers {
    match kind {
        NodeKind::Attacker => ProofNumbers::NO_MATE,
        NodeKind::Defender => ProofNumbers::MATE,
    }
}

fn summarize(kind: NodeKind, children: &[Child]) -> NodeEvaluation {
    debug_assert!(!children.is_empty());
    let mut best_index = 0;

    match kind {
        NodeKind::Attacker => {
            let mut pn = INF_PN;
            let mut dn = 0;
            for (index, child) in children.iter().enumerate() {
                if child.numbers.pn < children[best_index].numbers.pn
                    || (child.numbers.pn == children[best_index].numbers.pn
                        && child.numbers.dn > children[best_index].numbers.dn)
                {
                    best_index = index;
                }
                pn = pn.min(child.numbers.pn);
                dn = add_pn(dn, child.numbers.dn);
            }
            NodeEvaluation {
                numbers: ProofNumbers { pn, dn },
                best_move: Some(children[best_index].mv),
            }
        }
        NodeKind::Defender => {
            let mut pn = 0;
            let mut dn = INF_PN;
            for (index, child) in children.iter().enumerate() {
                if child.numbers.dn < children[best_index].numbers.dn
                    || (child.numbers.dn == children[best_index].numbers.dn
                        && child.numbers.pn > children[best_index].numbers.pn)
                {
                    best_index = index;
                }
                pn = add_pn(pn, child.numbers.pn);
                dn = dn.min(child.numbers.dn);
            }
            NodeEvaluation {
                numbers: ProofNumbers { pn, dn },
                best_move: Some(children[best_index].mv),
            }
        }
    }
}

fn select_most_proving_child(kind: NodeKind, children: &[Child]) -> (usize, ProofNumbers) {
    debug_assert!(!children.is_empty());
    let mut best_index = 0;
    let mut second_index: Option<usize> = None;

    for index in 1..children.len() {
        let is_better = match kind {
            NodeKind::Attacker => {
                children[index].numbers.pn < children[best_index].numbers.pn
                    || (children[index].numbers.pn == children[best_index].numbers.pn
                        && children[index].numbers.dn > children[best_index].numbers.dn)
            }
            NodeKind::Defender => {
                children[index].numbers.dn < children[best_index].numbers.dn
                    || (children[index].numbers.dn == children[best_index].numbers.dn
                        && children[index].numbers.pn > children[best_index].numbers.pn)
            }
        };

        if is_better {
            second_index = Some(best_index);
            best_index = index;
        } else if second_index.is_none_or(|candidate| {
            match kind {
                NodeKind::Attacker => {
                    children[index].numbers.pn < children[candidate].numbers.pn
                        || (children[index].numbers.pn == children[candidate].numbers.pn
                            && children[index].numbers.dn > children[candidate].numbers.dn)
                }
                NodeKind::Defender => {
                    children[index].numbers.dn < children[candidate].numbers.dn
                        || (children[index].numbers.dn == children[candidate].numbers.dn
                            && children[index].numbers.pn > children[candidate].numbers.pn)
                }
            }
        }) {
            second_index = Some(index);
        }
    }

    let second_numbers = second_index
        .map(|index| children[index].numbers)
        .unwrap_or(ProofNumbers {
            pn: INF_PN,
            dn: INF_PN,
        });

    (best_index, second_numbers)
}

fn child_thresholds(
    kind: NodeKind,
    parent: ProofNumbers,
    best_child: ProofNumbers,
    second_best: ProofNumbers,
    phi: u32,
    delta: u32,
) -> (u32, u32) {
    match kind {
        NodeKind::Attacker => (
            phi.min(second_best.pn.saturating_add(1)),
            delta_target(delta, parent.dn, best_child.dn),
        ),
        NodeKind::Defender => (
            delta_target(phi, parent.pn, best_child.pn),
            delta.min(second_best.dn.saturating_add(1)),
        ),
    }
}

fn delta_target(target: u32, total: u32, current: u32) -> u32 {
    if target >= INF_PN {
        INF_PN
    } else {
        target.saturating_sub(total.saturating_sub(current)).min(INF_PN)
    }
}

fn add_pn(lhs: u32, rhs: u32) -> u32 {
    lhs.saturating_add(rhs).min(INF_PN)
}

fn mid(current: u32, target: u32) -> u32 {
    if target <= current {
        return target;
    }
    if target >= INF_PN {
        if current >= INF_PN {
            return INF_PN;
        }
        return current
            .saturating_mul(2)
            .saturating_add(1)
            .max(current.saturating_add(1))
            .min(INF_PN);
    }

    let gap = target - current;
    current + (gap / 2).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "annan"))]
    const TSUME_SFEN: &str = "lpg6/3s2R2/1kpppp3/p8/9/P8/2N6/9/9 b BGN 1";
    const ONE_PLY_MATE_SFEN: &str = "8k/6G2/7B1/9/9/9/9/9/K8 b R 1";
    const ONE_PLY_MATE_WHITE_SFEN: &str = "k8/9/9/9/9/9/7b1/6g2/8K w r 1";
    const NO_MATE_SFEN: &str = "4k4/9/9/9/9/9/9/9/4K4 b - 1";
    #[cfg(feature = "annan")]
    const ANNAN_PROBLEM_SFEN: &str = "7p1/8k/5+R3/6P2/7G1/9/9/9/9 b N 1";

    fn assert_mating_line(board: &Board, pv: &[Move]) {
        let attacker = board.side_to_move();
        let mut board = board.clone();
        for &mv in pv {
            if board.side_to_move() == attacker {
                let mut checks = Vec::new();
                board.generate_checks(|moves| {
                    checks.extend(moves);
                    false
                });
                assert!(checks.contains(&mv), "{mv} should be a legal checking move");
            } else {
                assert!(board.is_legal(mv), "{mv} should be a legal defense move");
            }
            board.play_unchecked(mv);
        }

        assert_ne!(board.side_to_move(), attacker, "line should end on defender turn");
        assert!(
            !board.generate_moves(|_| true),
            "defender should have no legal replies at the end of the PV"
        );
    }

    fn parse_problem_board(sfen: &str) -> Board {
        Board::from_sfen(sfen)
            .or_else(|_| Board::tsume(sfen))
            .unwrap()
    }

    #[test]
    #[cfg(not(feature = "annan"))]
    fn solves_existing_tsume_position() {
        let board = Board::tsume(TSUME_SFEN).unwrap();
        let result = board.dfpn(&DfpnOptions::default());
        assert_eq!(result.status, DfpnStatus::Mate);
        assert_eq!(result.pv.first().copied(), Some("N*7e".parse().unwrap()));
        assert_mating_line(&board, &result.pv);
    }

    #[test]
    fn finds_one_ply_mate() {
        let board = Board::from_sfen(ONE_PLY_MATE_SFEN).unwrap();
        let result = board.dfpn(&DfpnOptions::default());
        assert_eq!(result.status, DfpnStatus::Mate);
        assert_mating_line(&board, &result.pv);
    }

    #[test]
    fn returns_no_mate_when_no_checks_exist() {
        let board = Board::from_sfen(NO_MATE_SFEN).unwrap();
        let result = board.dfpn(&DfpnOptions::default());
        assert_eq!(result.status, DfpnStatus::NoMate);
        assert!(result.pv.is_empty());
    }

    #[test]
    fn returns_unknown_when_budget_is_too_small() {
        let board = Board::from_sfen(ONE_PLY_MATE_SFEN).unwrap();
        let options = DfpnOptions {
            max_nodes: Some(1),
            ..DfpnOptions::default()
        };
        let result = board.dfpn(&options);
        assert_eq!(result.status, DfpnStatus::Unknown);
    }

    #[test]
    fn solves_white_to_move_mate() {
        let board = Board::from_sfen(ONE_PLY_MATE_WHITE_SFEN).unwrap();
        let result = board.dfpn(&DfpnOptions::default());
        assert_eq!(result.status, DfpnStatus::Mate);
        assert_mating_line(&board, &result.pv);
    }

    #[test]
    fn reports_statistics() {
        let board = Board::from_sfen(ONE_PLY_MATE_SFEN).unwrap();
        let result = board.dfpn(&DfpnOptions::default());
        assert!(result.stats.nodes > 0);
        assert!(result.stats.elapsed_ms >= 0.0);
    }

    #[test]
    #[cfg(feature = "annan")]
    fn solves_annan_mate_position() {
        let board = Board::from_sfen(ONE_PLY_MATE_SFEN).unwrap();
        let result = board.dfpn(&DfpnOptions::default());
        assert_eq!(result.status, DfpnStatus::Mate);
        assert_mating_line(&board, &result.pv);
    }

    #[test]
    #[cfg(feature = "annan")]
    fn solves_specific_annan_problem() {
        let board = parse_problem_board(ANNAN_PROBLEM_SFEN);
        let result = board.dfpn(&DfpnOptions::default());
        assert_eq!(result.status, DfpnStatus::Mate);
        assert_eq!(result.pv.first().copied(), Some("4c1c".parse().unwrap()));
        assert_mating_line(&board, &result.pv);
    }

    #[test]
    #[cfg(feature = "annan")]
    fn solves_annan_no_mate_position() {
        let board = Board::from_sfen(NO_MATE_SFEN).unwrap();
        let result = board.dfpn(&DfpnOptions::default());
        assert_eq!(result.status, DfpnStatus::NoMate);
    }

    #[test]
    #[cfg(feature = "annan")]
    fn returns_unknown_for_annan_when_budget_is_too_small() {
        let board = Board::from_sfen(ONE_PLY_MATE_SFEN).unwrap();
        let options = DfpnOptions {
            max_nodes: Some(1),
            ..DfpnOptions::default()
        };
        let result = board.dfpn(&options);
        assert_eq!(result.status, DfpnStatus::Unknown);
    }
}

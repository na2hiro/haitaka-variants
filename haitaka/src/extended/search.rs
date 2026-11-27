//! Search over completed turns, shared by the JSON v1–v5 rule backends.
use super::{Game, JsonMove, MATE, Outcome, Position, Shoshogi};
use crate::search::{
    Bound, ChildResult, ChildSearchPhase, ChildTrial, QuiescenceWindow, search_child,
    search_ordered_moves, select_root_move,
};
use core::sync::atomic::{AtomicBool, Ordering};
use instant::Instant;
use serde::Serialize;
use std::collections::HashMap;

pub(super) trait SearchGame {
    type Move: Copy + Eq + Serialize;
    const SCHEMA: u8;
    const REDUCE_QUIET_MOVES: bool = false;
    fn moves(&self, limit: usize, stop: &mut dyn FnMut() -> bool) -> Option<Vec<Self::Move>>;
    fn outcome(&self, stop: &mut dyn FnMut() -> bool) -> Option<Outcome>;
    fn side(&self) -> u8;
    fn evaluate(&self) -> i32;
    fn push(&mut self, mv: Self::Move);
    fn pop(&mut self);
    fn key(&self) -> u64;
    fn capture_value(&self, mv: Self::Move) -> i32;
    fn promotion(mv: Self::Move) -> bool;
    fn quiet_for_reduction(&self, _mv: Self::Move) -> bool {
        false
    }
    fn royal_threatened(&self, _side: u8) -> bool {
        true
    }
}
impl SearchGame for Game<Shoshogi> {
    type Move = JsonMove;
    const SCHEMA: u8 = 1;
    const REDUCE_QUIET_MOVES: bool = true;
    fn moves(&self, limit: usize, stop: &mut dyn FnMut() -> bool) -> Option<Vec<JsonMove>> {
        if stop() {
            return None;
        }
        let mut moves = self.current().moves();
        moves.truncate(limit);
        Some(moves)
    }
    fn outcome(&self, stop: &mut dyn FnMut() -> bool) -> Option<Outcome> {
        if stop() { None } else { Some(self.outcome()) }
    }
    fn side(&self) -> u8 {
        self.current().side
    }
    fn evaluate(&self) -> i32 {
        self.current().evaluate()
    }
    fn push(&mut self, mv: JsonMove) {
        Game::push(self, mv);
    }
    fn pop(&mut self) {
        self.history.pop();
    }
    fn key(&self) -> u64 {
        self.context_key()
    }
    fn capture_value(&self, mv: JsonMove) -> i32 {
        self.current().cells[Position::<Shoshogi>::index(mv.to).unwrap()]
            .map_or(0, |c| Position::<Shoshogi>::def(c).value)
    }
    fn promotion(mv: JsonMove) -> bool {
        mv.promotion
    }
    fn quiet_for_reduction(&self, mv: JsonMove) -> bool {
        let position = self.current();
        !mv.promotion && position.cells[Position::<Shoshogi>::index(mv.to).unwrap()].is_none()
    }
    fn royal_threatened(&self, side: u8) -> bool {
        self.current().attacked(side)
    }
}
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult<M = JsonMove> {
    pub schema_version: u8,
    pub best_move: Option<M>,
    pub completed_depth: u8,
    pub timed_out: bool,
    pub node_limit_reached: bool,
    pub elapsed_ms: f64,
    pub states: u64,
    pub nps: u64,
    pub score: i32,
    pub outcome: Outcome,
    #[serde(skip)]
    pub stats: SearchStats,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SearchStats {
    pub pvs_probes: u64,
    pub pvs_researches: u64,
    pub lmr_reductions: u64,
    pub lmr_researches: u64,
}

const PVS: bool = cfg!(all(
    feature = "search-pvs",
    feature = "search-variant-experiments"
));
const LMR: bool = PVS && cfg!(feature = "search-lmr");

fn is_mate_score(score: i32) -> bool {
    score.abs() > MATE - 1000
}

impl<M: Serialize> SearchResult<M> {
    pub(super) fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|error| error.to_string())
    }
}

#[derive(Clone, Copy)]
struct Entry<M> {
    depth: u8,
    score: i32,
    bound: Bound,
    best: Option<M>,
}
struct Search<'a, M> {
    start: Instant,
    timeout_ms: u32,
    limit: u64,
    nodes: u64,
    stopped: bool,
    cancel: Option<&'a AtomicBool>,
    stats: SearchStats,
    tt: HashMap<u64, Entry<M>>,
}
impl<M: Copy + Eq + Serialize> Search<'_, M> {
    fn eval<G: SearchGame<Move = M>>(&self, g: &G) -> i32 {
        g.evaluate()
    }

    fn expired(&mut self) -> bool {
        if self.nodes >= self.limit
            || self.start.elapsed().as_millis() >= u128::from(self.timeout_ms)
            || self.cancel.is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            self.stopped = true;
        }
        self.stopped
    }
    fn ordered<G: SearchGame<Move = M>>(&mut self, g: &G, best: Option<M>) -> Option<Vec<M>> {
        let mut moves = g.moves(usize::MAX, &mut || self.expired())?;
        moves.sort_by_cached_key(|&m| {
            -(if Some(m) == best { 100000 } else { 0 }
                + g.capture_value(m) * 10
                + if G::promotion(m) { 500 } else { 0 })
        });
        Some(moves)
    }
    fn terminal<G: SearchGame<Move = M>>(&mut self, g: &G, ply: i32) -> Option<i32> {
        let outcome = g.outcome(&mut || self.expired())?;
        if outcome == Outcome::ONGOING {
            None
        } else {
            Some(outcome.winner.map_or(0, |w| {
                if w == g.side() {
                    MATE - ply
                } else {
                    -MATE + ply
                }
            }))
        }
    }
    fn qsearch<G: SearchGame<Move = M>>(
        &mut self,
        g: &mut G,
        alpha: i32,
        beta: i32,
        left: u8,
        ply: i32,
    ) -> i32 {
        if self.expired() {
            return 0;
        }
        self.nodes += 1;
        if let Some(v) = self.terminal(g, ply) {
            return v;
        }
        if self.stopped {
            return 0;
        }
        let mut window = QuiescenceWindow::new(alpha, beta, false);
        if let Some(score) = window.stand_pat(self.eval(g)) {
            return score;
        }
        if left == 0 {
            return window.alpha();
        }
        let Some(moves) = self.ordered(g, None) else {
            return 0;
        };
        for mv in moves {
            if g.capture_value(mv) == 0 && !G::promotion(mv) {
                continue;
            }
            g.push(mv);
            let (child_alpha, child_beta) = window.child_window();
            let v = -self.qsearch(g, child_alpha, child_beta, left - 1, ply + 1);
            g.pop();
            if self.stopped {
                return 0;
            }
            if let Some(score) = window.record_child(v, false).result {
                return score;
            }
        }
        window.alpha()
    }
    fn negamax<G: SearchGame<Move = M>>(
        &mut self,
        g: &mut G,
        depth: u8,
        alpha: i32,
        beta: i32,
        ply: i32,
    ) -> (i32, Option<M>) {
        if self.expired() {
            return (0, None);
        }
        self.nodes += 1;
        if let Some(v) = self.terminal(g, ply) {
            return (v, None);
        }
        if self.stopped {
            return (0, None);
        }
        if depth == 0 {
            return (self.qsearch(g, alpha, beta, 4, ply), None);
        }
        let key = g.key();
        let entry = self.tt.get(&key).copied();
        if let Some(e) = entry
            && e.depth >= depth
            && e.bound.can_cutoff(e.score, (alpha, beta))
        {
            return (e.score, e.best);
        }
        let Some(moves) = self.ordered(g, entry.and_then(|e| e.best)) else {
            return (0, None);
        };
        if moves.is_empty() {
            return (g.evaluate(), None);
        }
        let side = g.side();
        let can_reduce = LMR
            && G::REDUCE_QUIET_MOVES
            && ply > 0
            && depth >= 3
            && moves.len() >= 5
            && beta == alpha + 1
            && !is_mate_score(alpha)
            && !is_mate_score(beta)
            && !g.royal_threatened(side);
        let result = search_ordered_moves(
            self,
            moves,
            (alpha, beta),
            -MATE - 1,
            |search, mv, node| {
                let quiet_late = can_reduce
                    && node.number >= 5
                    && !is_mate_score(node.best_score)
                    && entry.is_none_or(|entry| entry.best != Some(mv))
                    && g.quiet_for_reduction(mv);
                g.push(mv);
                let trial = if quiet_late && !g.royal_threatened(side ^ 1) {
                    ChildTrial::Lmr
                } else {
                    ChildTrial::pvs(PVS, depth, node.window, node.number == 1)
                };
                let result = search_child(
                    depth - 1,
                    node.window,
                    trial,
                    is_mate_score,
                    |depth, (alpha, beta), phase| {
                        match (phase, trial) {
                            (ChildSearchPhase::Trial, ChildTrial::Pvs) => {
                                search.stats.pvs_probes += 1
                            }
                            (ChildSearchPhase::Research, ChildTrial::Pvs) => {
                                search.stats.pvs_researches += 1
                            }
                            (ChildSearchPhase::Trial, ChildTrial::Lmr) => {
                                search.stats.lmr_reductions += 1
                            }
                            (ChildSearchPhase::Research, ChildTrial::Lmr) => {
                                search.stats.lmr_researches += 1
                            }
                            _ => {}
                        }
                        let (value, _) = search.negamax(g, depth, alpha, beta, ply + 1);
                        if search.stopped {
                            Err(())
                        } else {
                            Ok(ChildResult::Score(value))
                        }
                    },
                );
                g.pop();
                result
            },
            |_, _, _| Ok(()),
        );
        let Ok(result) = result else {
            return (0, None);
        };
        if result.may_store {
            if self.tt.len() >= 65536 {
                self.tt.clear();
            }
            self.tt.insert(
                key,
                Entry {
                    depth,
                    score: result.score,
                    bound: Bound::for_score(result.score, (alpha, beta)),
                    best: result.best_move,
                },
            );
        }
        (result.score, result.best_move)
    }
}
pub(super) fn search_json<G: SearchGame>(
    game: &mut G,
    max_depth: u8,
    timeout_ms: u32,
    node_budget: u32,
) -> Result<String, String> {
    search_position(game, max_depth, timeout_ms, node_budget, None)?.to_json()
}

pub(super) fn search_position<G: SearchGame>(
    game: &mut G,
    max_depth: u8,
    timeout_ms: u32,
    node_budget: u32,
    cancel: Option<&AtomicBool>,
) -> Result<SearchResult<G::Move>, String> {
    if max_depth == 0 || max_depth > 100 || timeout_ms == 0 {
        return Err("depth must be 1..100 and timeoutMs must be positive".into());
    }
    let start = Instant::now();
    let outcome = game.outcome(&mut || false).unwrap();
    let fallback = if outcome == Outcome::ONGOING {
        game.moves(1, &mut || false).unwrap().first().copied()
    } else {
        None
    };
    let mut search = Search {
        start,
        timeout_ms,
        limit: if node_budget == 0 {
            u64::MAX
        } else {
            u64::from(node_budget)
        },
        nodes: 0,
        stopped: false,
        cancel,
        stats: SearchStats::default(),
        tt: HashMap::new(),
    };
    let mut completed_move = None;
    let mut completed_depth = 0;
    let mut score = search.terminal(game, 0).unwrap_or_else(|| game.evaluate());
    if outcome == Outcome::ONGOING && fallback.is_some() {
        for depth in 1..=max_depth {
            let (v, mv) = search.negamax(game, depth, -MATE - 1, MATE + 1, 0);
            if search.stopped {
                break;
            }
            score = v;
            completed_move = mv;
            completed_depth = depth;
            if score.abs() >= MATE - i32::from(depth) {
                break;
            }
        }
    }
    let elapsed_ms = search.start.elapsed().as_secs_f64() * 1000.;
    let (best_move, _) = select_root_move(None, completed_move, || fallback);
    Ok(SearchResult {
        schema_version: G::SCHEMA,
        best_move,
        completed_depth,
        timed_out: search.stopped && elapsed_ms >= f64::from(timeout_ms),
        node_limit_reached: search.stopped && search.nodes >= search.limit,
        elapsed_ms,
        states: search.nodes,
        nps: (search.nodes as f64 * 1000. / elapsed_ms.max(0.001)) as u64,
        score,
        outcome,
        stats: search.stats,
    })
}

#[cfg(all(test, feature = "chushogi"))]
#[test]
fn chu_node_budget_restores_completed_turn_history() {
    use super::chu::{ChuGame, start_position_json};

    let mut game = ChuGame::parse(&start_position_json()).unwrap();
    let before = game.to_json();
    let mut search = Search {
        start: Instant::now(),
        timeout_ms: 10_000,
        // Enter the first child, then interrupt before it settles.
        limit: 2,
        nodes: 0,
        stopped: false,
        cancel: None,
        stats: SearchStats::default(),
        tt: HashMap::new(),
    };
    let result = search.negamax(&mut game, 2, -MATE - 1, MATE + 1, 0);
    assert!(search.stopped);
    assert_eq!(search.nodes, 2);
    assert_eq!(result, (0, None));
    assert!(search.tt.is_empty());
    assert_eq!(game.to_json(), before);
}

#[cfg(all(
    test,
    feature = "chushogi",
    feature = "search-variant-experiments",
    feature = "search-lmr"
))]
#[test]
fn chu_late_quiet_moves_are_reduced_and_the_fail_low_is_not_cached() {
    use super::chu::{ChuGame, ChuPosition, ChuState};
    use super::{PlacedPiece, WirePosition};
    let position = ChuPosition::from_wire(&WirePosition {
        side_to_move: 0,
        pieces: [
            ([1, 12], "ah", 0),
            ([12, 1], "ah", 1),
            ([3, 8], "ae", 0),
            ([9, 8], "ae", 0),
            ([3, 4], "ae", 1),
            ([9, 4], "ae", 1),
        ]
        .map(|(at, piece, color)| PlacedPiece {
            at,
            piece: piece.to_owned(),
            color,
        })
        .to_vec(),
        hands: Default::default(),
        state: ChuState::default(),
    })
    .unwrap();
    let mut game = ChuGame::from_position(position);
    let before = game.to_json();
    let key = game.key();
    let mut search = Search {
        start: Instant::now(),
        timeout_ms: 10_000,
        limit: 100_000,
        nodes: 0,
        stopped: false,
        cancel: None,
        stats: SearchStats::default(),
        tt: HashMap::new(),
    };
    assert_eq!(search.negamax(&mut game, 3, 10_000, 10_001, 1).0, 10_000);
    assert!(!search.stopped);
    assert!(search.stats.lmr_reductions > 0);
    assert_eq!(search.stats.lmr_researches, 0);
    assert!(!search.tt.contains_key(&key));
    assert_eq!(game.to_json(), before);
}

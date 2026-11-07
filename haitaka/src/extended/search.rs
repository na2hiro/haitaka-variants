//! Search over completed turns, shared by the v1, v2 and v3 rule backends.
use super::{Game, JsonMove, MATE, Outcome, Position, Shoshogi};
use instant::Instant;
use serde::Serialize;
use std::collections::HashMap;

pub(super) trait SearchGame {
    type Move: Copy + Eq + Serialize;
    const SCHEMA: u8;
    fn moves(&self, limit: usize, stop: &mut dyn FnMut() -> bool) -> Option<Vec<Self::Move>>;
    fn outcome(&self, stop: &mut dyn FnMut() -> bool) -> Option<Outcome>;
    fn side(&self) -> u8;
    fn evaluate(&self) -> i32;
    fn push(&mut self, mv: Self::Move);
    fn pop(&mut self);
    fn key(&self) -> u64;
    fn capture_value(&self, mv: Self::Move) -> i32;
    fn promotion(mv: Self::Move) -> bool;
}
impl SearchGame for Game<Shoshogi> {
    type Move = JsonMove;
    const SCHEMA: u8 = 1;
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
}
#[derive(Clone, Copy)]
struct Entry<M> {
    depth: u8,
    score: i32,
    bound: u8,
    best: Option<M>,
}
struct Search<M> {
    start: Instant,
    timeout_ms: u32,
    limit: u64,
    nodes: u64,
    stopped: bool,
    tt: HashMap<u64, Entry<M>>,
}
impl<M: Copy + Eq + Serialize> Search<M> {
    fn expired(&mut self) -> bool {
        if self.nodes >= self.limit
            || self.start.elapsed().as_millis() >= u128::from(self.timeout_ms)
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
        mut alpha: i32,
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
        let stand = g.evaluate();
        if stand >= beta {
            return stand;
        }
        alpha = alpha.max(stand);
        if left == 0 {
            return alpha;
        }
        let Some(moves) = self.ordered(g, None) else {
            return 0;
        };
        for mv in moves {
            if g.capture_value(mv) == 0 && !G::promotion(mv) {
                continue;
            }
            g.push(mv);
            let v = -self.qsearch(g, -beta, -alpha, left - 1, ply + 1);
            g.pop();
            if self.stopped {
                return 0;
            }
            if v >= beta {
                return v;
            }
            alpha = alpha.max(v);
        }
        alpha
    }
    fn negamax<G: SearchGame<Move = M>>(
        &mut self,
        g: &mut G,
        depth: u8,
        mut alpha: i32,
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
        let original = alpha;
        if let Some(e) = entry
            && e.depth >= depth
            && (e.bound == 0 || e.bound == 1 && e.score >= beta || e.bound == 2 && e.score <= alpha)
        {
            return (e.score, e.best);
        }
        let Some(moves) = self.ordered(g, entry.and_then(|e| e.best)) else {
            return (0, None);
        };
        if moves.is_empty() {
            return (g.evaluate(), None);
        }
        let mut score = -MATE - 1;
        let mut best = None;
        for mv in moves {
            g.push(mv);
            let value = -self.negamax(g, depth - 1, -beta, -alpha, ply + 1).0;
            g.pop();
            if self.stopped {
                return (0, None);
            }
            if value > score {
                score = value;
                best = Some(mv);
            }
            alpha = alpha.max(value);
            if alpha >= beta {
                break;
            }
        }
        if self.tt.len() >= 65536 {
            self.tt.clear();
        }
        self.tt.insert(
            key,
            Entry {
                depth,
                score,
                bound: if score <= original {
                    2
                } else if score >= beta {
                    1
                } else {
                    0
                },
                best,
            },
        );
        (score, best)
    }
}
pub(super) fn search_json<G: SearchGame>(
    game: &mut G,
    max_depth: u8,
    timeout_ms: u32,
    node_budget: u32,
) -> Result<String, String> {
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
        tt: HashMap::new(),
    };
    let mut best_move = fallback;
    let mut completed_depth = 0;
    let mut score = search.terminal(game, 0).unwrap_or_else(|| game.evaluate());
    if outcome == Outcome::ONGOING && fallback.is_some() {
        for depth in 1..=max_depth {
            let (v, mv) = search.negamax(game, depth, -MATE - 1, MATE + 1, 0);
            if search.stopped {
                break;
            }
            score = v;
            best_move = mv;
            completed_depth = depth;
            if score.abs() >= MATE - i32::from(depth) {
                break;
            }
        }
    }
    let elapsed_ms = search.start.elapsed().as_secs_f64() * 1000.;
    serde_json::to_string(&SearchResult {
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
    })
    .map_err(|e| e.to_string())
}

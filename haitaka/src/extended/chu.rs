pub use super::path::{LionState as ChuState, Path, Prefix};
// Chu shogi: 144-square bitboards, completed paths, and explicit capture state.
use super::search::SearchGame;
use super::*;
include!("chu_defs.rs");
pub const PROTOCOL: &str = "haitaka-json-v2";
pub const NAME: &str = "chushogi";
pub const RULE_ID: u32 = 48;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chushogi;
impl CaptureRoyalRule for Chushogi {
    type Geometry = Rect<12, 12>;
    type State = ChuState;
    type Hands = NoHands;
    type Placement = PlacedPiece;
    const ID: u32 = 48;
    const PIECES: &'static [PieceDef] = CHU_PIECES;
    const INVENTORY: &'static [usize] = CHU_INVENTORY;
    const PROMOTION_RANKS: usize = 4;
    fn initial() -> Vec<PlacedPiece> {
        INITIAL
            .iter()
            .map(|&(at, id, color)| PlacedPiece {
                at,
                piece: CHU_PIECES[id as usize].id.into(),
                color,
            })
            .collect()
    }
}
pub type ChuPosition = Position<Chushogi>;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathMove {
    pub schema_version: u8,
    pub from: [u16; 2],
    pub path: Path,
    pub promotion: bool,
}
impl PathMove {
    fn new(from: usize, path: Path, promotion: bool) -> Self {
        Self {
            schema_version: 2,
            from: ChuPosition::at(from),
            path,
            promotion,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct History {
    pub initial: WirePosition<ChuState>,
    pub moves: Vec<PathMove>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub schema_version: u8,
    pub rule_id: u32,
    pub position: WirePosition<ChuState>,
    pub history: Option<History>,
    pub pending: Option<Prefix>,
}
fn lion(cell: Cell) -> bool {
    matches!(ChuPosition::def(cell).id, "bl" | "dh")
}
fn adjacent(a: usize, b: usize) -> bool {
    let (x, y) = Rect::<12, 12>::coordinates(a).unwrap();
    let (u, v) = Rect::<12, 12>::coordinates(b).unwrap();
    x.abs_diff(u).max(y.abs_diff(v)) == 1
}
impl ChuPosition {
    fn protected_lion(&self, target: usize, capturing: u8, remove: usize) -> bool {
        let mut position = self.clone();
        position.set_cell(remove, None);
        position.cells.iter().enumerate().any(|(i, c)| {
            c.is_some_and(|c| {
                c.color != capturing && position.geometric_targets(i, c, true).contains(&target)
            })
        })
    }
    fn capture_allowed(
        &self,
        original: &Self,
        from: usize,
        to: usize,
        first: Option<(usize, Option<Cell>)>,
    ) -> bool {
        let Some(victim) = self.cells[to] else {
            return true;
        };
        let mover = self.cells[from].unwrap();
        if victim.color == mover.color {
            return false;
        }
        if !lion(victim) {
            return true;
        }
        if !lion(mover) {
            return !self.state.senjishi;
        }
        if let Some((source, first_victim)) = first {
            if let Some(first_victim) = first_victim {
                if !matches!(Self::def(first_victim).id, "aa" | "cq") {
                    return true;
                }
                return !original.protected_lion(to, mover.color, source);
            }
            return !self.protected_lion(to, mover.color, from);
        }
        adjacent(from, to) || !self.protected_lion(to, mover.color, from)
    }
    fn hop(&mut self, from: usize, to: usize) {
        let mover = self.cells[from].unwrap();
        let victim = self.cells[to];
        self.state.senjishi = victim.is_some_and(lion) && !lion(mover);
        self.set_cell(from, None);
        self.set_cell(to, Some(mover));
    }
    fn can_promote(&self, from: usize, to: usize) -> bool {
        let c = self.cells[from].unwrap();
        let def = Self::def(c);
        if def.promotion.is_none() {
            return false;
        }
        let a = self.relative_rank(from, c.color);
        let b = self.relative_rank(to, c.color);
        a >= 4 && b < 4
            || matches!(def.id, "aa" | "ab") && b == 0
            || self.cells[to].is_some() && a < 4
    }
    fn append(
        &self,
        out: &mut Vec<PathMove>,
        from: usize,
        path: Path,
        can_promote: bool,
        limit: usize,
    ) -> bool {
        out.push(PathMove::new(from, path, false));
        if out.len() >= limit {
            return true;
        }
        if can_promote {
            out.push(PathMove::new(from, path, true));
        }
        out.len() >= limit
    }
    pub fn legal_paths(
        &self,
        limit: usize,
        stop: &mut dyn FnMut() -> bool,
    ) -> Option<Vec<PathMove>> {
        let mut out = vec![];
        if self.capture_outcome() != Outcome::ONGOING {
            return Some(out);
        }
        let mut pieces = self.occupied[self.side as usize];
        while let Some(from) = pieces.first() {
            if stop() {
                return None;
            }
            pieces = pieces.without(from);
            let cell = self.cells[from].unwrap();
            let def = Self::def(cell);
            let sign = if cell.color == 0 { 1 } else { -1 };
            let (a, b) = Rect::<12, 12>::coordinates(from).unwrap();
            for to in self.targets(from, cell) {
                if stop() {
                    return None;
                }
                if !self.capture_allowed(self, from, to, None) {
                    continue;
                }
                let (x, y) = Rect::<12, 12>::coordinates(to).unwrap();
                let delta = (
                    (x as isize - a as isize) * sign,
                    (y as isize - b as isize) * sign,
                );
                let follow = continuation(def.id, delta);
                let promotion = follow.is_none() && self.can_promote(from, to);
                if self.append(&mut out, from, Path::one(Self::at(to)), promotion, limit) {
                    return Some(out);
                }
                if let Some(follow) = follow {
                    let first_victim = self.cells[to];
                    let mut middle = self.clone();
                    middle.hop(from, to);
                    if middle.capture_outcome() != Outcome::ONGOING {
                        continue;
                    }
                    for &(dx, dy) in follow {
                        if stop() {
                            return None;
                        }
                        let Some(end) = Rect::<12, 12>::offset(to, dx * sign, dy * sign) else {
                            continue;
                        };
                        if !middle.capture_allowed(self, to, end, Some((from, first_victim))) {
                            continue;
                        }
                        if self.append(
                            &mut out,
                            from,
                            Path::two(Self::at(to), Self::at(end)),
                            false,
                            limit,
                        ) {
                            return Some(out);
                        }
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        Some(out)
    }
    fn play_path_unchecked(&self, mv: PathMove) -> Self {
        let mut next = self.clone();
        let mut from = Self::index(mv.from).unwrap();
        for &to in mv.path.iter() {
            let to = Self::index(to).unwrap();
            next.hop(from, to);
            from = to;
        }
        if mv.promotion {
            let mut c = next.cells[from].unwrap();
            c.piece = Self::def(c).promotion.unwrap();
            next.set_cell(from, Some(c));
        }
        next.side ^= 1;
        next
    }
}
#[derive(Clone)]
pub struct ChuGame {
    pub history: Vec<ChuPosition>,
    pub played: Vec<PathMove>,
    pub pending: Option<Prefix>,
    pending_ply: usize,
}
pub type CompiledGame = ChuGame;
impl ChuGame {
    pub fn parse(json: &str) -> Result<Self, String> {
        let req: Request = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if req.schema_version != 2 || req.rule_id != 48 {
            return Err("unsupported schemaVersion or ruleId".into());
        }
        let current = ChuPosition::from_wire(&req.position)?;
        let mut game = if let Some(history) = req.history {
            let mut game = Self {
                history: vec![ChuPosition::from_wire(&history.initial)?],
                played: vec![],
                pending: None,
                pending_ply: 0,
            };
            for mv in history.moves {
                game.play(mv)?;
            }
            game
        } else {
            Self {
                history: vec![current.clone()],
                played: vec![],
                pending: None,
                pending_ply: 0,
            }
        };
        if game.current() != &current {
            return Err("history must end at position, including rule state".into());
        }
        game.pending_ply = game.played.len();
        game.pending = req.pending;
        if let Some(prefix) = game.pending {
            let from = ChuPosition::index(prefix.from)?;
            let via = ChuPosition::index(prefix.via)?;
            let mover = game.current().cells[from].ok_or("pending path has no moving piece")?;
            let (x, y) = Rect::<12, 12>::coordinates(from).unwrap();
            let (u, v) = Rect::<12, 12>::coordinates(via).unwrap();
            let sign = if mover.color == 0 { 1 } else { -1 };
            if continuation(
                ChuPosition::def(mover).id,
                (
                    (u as isize - x as isize) * sign,
                    (v as isize - y as isize) * sign,
                ),
            )
            .is_none()
                || game
                    .current()
                    .play_path_unchecked(PathMove {
                        schema_version: 2,
                        from: prefix.from,
                        path: Path::one(prefix.via),
                        promotion: false,
                    })
                    .capture_outcome()
                    != Outcome::ONGOING
                || game.moves().is_empty()
            {
                return Err("invalid or already completed pending path".into());
            }
        }
        // Canonical output always carries the initial position and exact paths.
        game.history.shrink_to_fit();
        Ok(game)
    }
    pub fn current(&self) -> &ChuPosition {
        self.history.last().unwrap()
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(&Request {
            schema_version: 2,
            rule_id: 48,
            position: self.current().to_wire(),
            pending: self.active_pending(),
            history: Some(History {
                initial: self.history[0].to_wire(),
                moves: self.played.clone(),
            }),
        })
        .unwrap()
    }
    fn active_pending(&self) -> Option<Prefix> {
        if self.played.len() == self.pending_ply {
            self.pending
        } else {
            None
        }
    }
    fn allowed_prefix(&self, mv: PathMove) -> bool {
        self.active_pending()
            .is_none_or(|p| mv.from == p.from && mv.path[0] == p.via)
    }
    fn paths_cancel(&self, limit: usize, stop: &mut dyn FnMut() -> bool) -> Option<Vec<PathMove>> {
        let mut moves = self.current().legal_paths(
            if self.active_pending().is_some() {
                usize::MAX
            } else {
                limit
            },
            stop,
        )?;
        moves.retain(|m| self.allowed_prefix(*m));
        moves.truncate(limit);
        Some(moves)
    }
    pub fn position_hash(&self) -> u64 {
        let mut hash = self.current().hash();
        if let Some(p) = self.active_pending() {
            for n in p.from.into_iter().chain(p.via) {
                hash = hash.rotate_left(7) ^ u64::from(n).wrapping_mul(0x9e3779b97f4a7c15);
            }
        }
        hash
    }
    fn basic_outcome(&self) -> Outcome {
        let p = self.current();
        let captured = p.capture_outcome();
        if captured != Outcome::ONGOING {
            return captured;
        }
        let matches: Vec<_> = self
            .history
            .iter()
            .enumerate()
            .filter(|(_, q)| *q == p)
            .map(|(i, _)| i)
            .collect();
        if matches.len() < 4 {
            return Outcome::ONGOING;
        }
        for victim in [p.side, p.side ^ 1] {
            if self.history[matches[0] + 1..]
                .iter()
                .filter(|p| p.side == victim)
                .all(|p| p.attacked(victim))
            {
                return Outcome::win(victim);
            }
        }
        Outcome::DRAW
    }
    fn outcome_cancel(&self, stop: &mut dyn FnMut() -> bool) -> Option<Outcome> {
        if stop() {
            return None;
        }
        let basic = self.basic_outcome();
        if basic != Outcome::ONGOING {
            return Some(basic);
        }
        if self.paths_cancel(1, stop)?.is_empty() {
            Some(Outcome::win(self.current().side ^ 1))
        } else {
            Some(basic)
        }
    }
    pub fn outcome(&self) -> Outcome {
        self.outcome_cancel(&mut || false).unwrap()
    }
    pub fn moves(&self) -> Vec<PathMove> {
        if self.basic_outcome() != Outcome::ONGOING {
            vec![]
        } else {
            self.paths_cancel(usize::MAX, &mut || false).unwrap()
        }
    }
    pub fn play(&mut self, mv: PathMove) -> Result<(), String> {
        if mv.schema_version != 2 || !self.moves().contains(&mv) {
            return Err("illegal completed path or unsupported schemaVersion".into());
        }
        self.push(mv);
        Ok(())
    }
    fn push(&mut self, mv: PathMove) {
        self.history.push(self.current().play_path_unchecked(mv));
        self.played.push(mv);
    }
    fn context_key(&self) -> u64 {
        self.history.iter().fold(0x6a09e667f3bcc909u64, |h, p| {
            h.rotate_left(7) ^ p.hash().wrapping_mul(0x9e3779b97f4a7c15)
        }) ^ self.position_hash().rotate_left(13)
    }
    pub fn perft(&mut self, depth: u8) -> u64 {
        if depth == 0 {
            return 1;
        }
        let mut n = 0;
        for mv in self.moves() {
            self.push(mv);
            n += self.perft(depth - 1);
            self.history.pop();
            self.played.pop();
        }
        n
    }
}
impl SearchGame for ChuGame {
    type Move = PathMove;
    const SCHEMA: u8 = 2;
    fn moves(&self, limit: usize, stop: &mut dyn FnMut() -> bool) -> Option<Vec<PathMove>> {
        self.paths_cancel(limit, stop)
    }
    fn outcome(&self, stop: &mut dyn FnMut() -> bool) -> Option<Outcome> {
        self.outcome_cancel(stop)
    }
    fn side(&self) -> u8 {
        self.current().side
    }
    fn evaluate(&self) -> i32 {
        self.current().evaluate()
    }
    fn push(&mut self, mv: PathMove) {
        ChuGame::push(self, mv);
    }
    fn pop(&mut self) {
        self.history.pop();
        self.played.pop();
    }
    fn key(&self) -> u64 {
        self.context_key()
    }
    fn capture_value(&self, mv: PathMove) -> i32 {
        let source = ChuPosition::index(mv.from).unwrap();
        let mut taken = vec![];
        let mut value = 0;
        for &at in mv.path.iter() {
            let i = ChuPosition::index(at).unwrap();
            if i != source && !taken.contains(&i) {
                if let Some(c) = self.current().cells[i] {
                    value += ChuPosition::def(c).value;
                }
                taken.push(i);
            }
        }
        value
    }
    fn promotion(mv: PathMove) -> bool {
        mv.promotion
    }
}
pub fn start_position_json() -> String {
    let p = ChuPosition::from_wire(&WirePosition {
        side_to_move: 0,
        pieces: Chushogi::initial(),
        hands: Default::default(),
        state: ChuState::default(),
    })
    .unwrap();
    ChuGame {
        history: vec![p],
        played: vec![],
        pending: None,
        pending_ply: 0,
    }
    .to_json()
}
pub fn validate_position_json(json: &str) -> Result<String, String> {
    Ok(ChuGame::parse(json)?.to_json())
}
pub fn legal_moves_json(json: &str) -> Result<String, String> {
    serde_json::to_string(&ChuGame::parse(json)?.moves()).map_err(|e| e.to_string())
}
pub fn apply_move_json(json: &str, mv: &str) -> Result<String, String> {
    let mut game = ChuGame::parse(json)?;
    let mv = serde_json::from_str(mv).map_err(|e| e.to_string())?;
    game.play(mv)?;
    Ok(game.to_json())
}
pub fn inspect_json(json: &str) -> Result<String, String> {
    let g = ChuGame::parse(json)?;
    Ok(serde_json::json!({"schemaVersion":2,"positionHash":format!("{:016x}",g.position_hash()),"contextHash":format!("{:016x}",g.context_key()),"outcome":g.outcome()}).to_string())
}
pub fn perft_json(json: &str, depth: u8) -> Result<String, String> {
    if depth > 4 {
        return Err("Chu perft depth must be <= 4".into());
    }
    let mut g = ChuGame::parse(json)?;
    let start = Instant::now();
    let nodes = g.perft(depth);
    let elapsed = start.elapsed().as_secs_f64() * 1000.;
    Ok(serde_json::json!({"nodes":nodes,"elapsedMs":elapsed,"nps":(nodes as f64*1000./elapsed.max(0.001))as u64}).to_string())
}
pub fn search_json(
    json: &str,
    max_depth: u8,
    timeout_ms: u32,
    node_budget: u32,
) -> Result<String, String> {
    let mut game = ChuGame::parse(json)?;
    search::search_json(&mut game, max_depth, timeout_ms, node_budget)
}
#[cfg(test)]
mod tests;

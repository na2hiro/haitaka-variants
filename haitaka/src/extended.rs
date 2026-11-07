//! Rule-local pieces and capture-royal games. The ordinary 14-piece Board is unchanged.
//! Wire coordinates are one-based, file-major; no invented SFEN/USI piece letters.
use haitaka_types::geometry::{BoardGeometry, Rect};
use instant::Instant;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    marker::PhantomData,
};

pub const PROTOCOL: &str = "haitaka-json-v1";
const MATE: i32 = 30_000;
const KING: &[(isize, isize)] = &[
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];
const GOLD: &[(isize, isize)] = &[(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (0, 1)];
const SILVER: &[(isize, isize)] = &[(-1, -1), (0, -1), (1, -1), (-1, 1), (1, 1)];
const DIAG: &[(isize, isize)] = &[(-1, -1), (1, -1), (-1, 1), (1, 1)];
const ORTH: &[(isize, isize)] = &[(0, -1), (-1, 0), (1, 0), (0, 1)];
const ELEPHANT: &[(isize, isize)] = &[(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (1, 1)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PieceId(pub u16);
/// Identity, movement, promotion, inventory origin and royal role are rule-local.
pub struct PieceDef {
    pub id: &'static str,
    pub steps: &'static [(isize, isize)],
    pub rays: &'static [(isize, isize)],
    pub promotion: Option<PieceId>,
    pub base: usize,
    pub royal: bool,
    pub dead_ranks: usize,
    pub value: i32,
}
macro_rules! piece {
    ($id:literal,$steps:expr,$rays:expr,$promotion:expr,$base:expr,$royal:expr,$dead:expr,$value:expr) => {
        PieceDef {
            id: $id,
            steps: $steps,
            rays: $rays,
            promotion: $promotion,
            base: $base,
            royal: $royal,
            dead_ranks: $dead,
            value: $value,
        }
    };
}
const SHO_PIECES: &[PieceDef] = &[
    piece!("aa", &[(0, -1)], &[], Some(PieceId(8)), 0, false, 1, 100),
    piece!("ab", &[], &[(0, -1)], Some(PieceId(9)), 1, false, 1, 300),
    piece!(
        "ac",
        &[(-1, -2), (1, -2)],
        &[],
        Some(PieceId(10)),
        2,
        false,
        2,
        300
    ),
    piece!("ad", SILVER, &[], Some(PieceId(11)), 3, false, 0, 450),
    piece!("af", &[], DIAG, Some(PieceId(12)), 4, false, 0, 750),
    piece!("ag", &[], ORTH, Some(PieceId(13)), 5, false, 0, 900),
    piece!("ae", GOLD, &[], None, 6, false, 0, 550),
    piece!("ah", KING, &[], None, 7, true, 0, 2000),
    piece!("ai", GOLD, &[], None, 0, false, 0, 550),
    piece!("aj", GOLD, &[], None, 1, false, 0, 550),
    piece!("ak", GOLD, &[], None, 2, false, 0, 550),
    piece!("al", GOLD, &[], None, 3, false, 0, 550),
    piece!("am", ORTH, DIAG, None, 4, false, 0, 1000),
    piece!("an", DIAG, ORTH, None, 5, false, 0, 1150),
    piece!("cf", ELEPHANT, &[], Some(PieceId(15)), 8, false, 0, 650),
    piece!("cg", KING, &[], None, 8, true, 0, 2100),
];

/// This backend currently implements discard captures, ordinary promotion, and loss
/// when every royal is captured. Check exposure is legal. Other policies must get
/// explicit implementations, rather than silently using these semantics.
pub trait CaptureRoyalRule: Clone + PartialEq + Eq {
    type Geometry: BoardGeometry;
    const ID: u32;
    const PIECES: &'static [PieceDef];
    const INVENTORY: &'static [usize];
    const PROMOTION_RANKS: usize;
    fn initial() -> Vec<PlacedPiece>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shoshogi;
impl CaptureRoyalRule for Shoshogi {
    type Geometry = Rect<9, 9>;
    const ID: u32 = 43;
    const PIECES: &'static [PieceDef] = SHO_PIECES;
    const INVENTORY: &'static [usize] = &[9, 2, 2, 2, 1, 1, 2, 1, 1];
    const PROMOTION_RANKS: usize = 3;
    fn initial() -> Vec<PlacedPiece> {
        let mut out = vec![];
        for color in 0..2 {
            let back = if color == 0 { 9 } else { 1 };
            let pawn = if color == 0 { 7 } else { 3 };
            let middle = if color == 0 { 8 } else { 2 };
            for (x, id) in ["ab", "ac", "ad", "ae", "ah", "ae", "ad", "ac", "ab"]
                .iter()
                .enumerate()
            {
                out.push(PlacedPiece {
                    at: [x as u16 + 1, back],
                    piece: id.to_string(),
                    color,
                });
                out.push(PlacedPiece {
                    at: [x as u16 + 1, pawn],
                    piece: "aa".into(),
                    color,
                });
            }
            for (x, id) in if color == 0 {
                [(2, "ag"), (5, "cf"), (8, "af")]
            } else {
                [(2, "af"), (5, "cf"), (8, "ag")]
            } {
                out.push(PlacedPiece {
                    at: [x, middle],
                    piece: id.into(),
                    color,
                });
            }
        }
        out
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlacedPiece {
    pub at: [u16; 2],
    pub piece: String,
    pub color: u8,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuleState {}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WirePosition {
    pub side_to_move: u8,
    pub pieces: Vec<PlacedPiece>,
    pub hands: [BTreeMap<String, u16>; 2],
    pub state: RuleState,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PositionRequest {
    pub schema_version: u8,
    pub rule_id: u32,
    pub position: WirePosition,
    /// Inclusive timeline ending at position. Empty means a fresh analysis root.
    pub history: Vec<WirePosition>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JsonMove {
    pub schema_version: u8,
    pub from: [u16; 2],
    pub to: [u16; 2],
    pub promotion: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cell {
    piece: PieceId,
    color: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Position<R: CaptureRoyalRule> {
    cells: Vec<Option<Cell>>,
    side: u8,
    rule: PhantomData<R>,
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub kind: &'static str,
    pub winner: Option<u8>,
}
impl Outcome {
    const ONGOING: Self = Self {
        kind: "ongoing",
        winner: None,
    };
    const DRAW: Self = Self {
        kind: "draw",
        winner: None,
    };
    fn win(winner: u8) -> Self {
        Self {
            kind: "win",
            winner: Some(winner),
        }
    }
}
impl<R: CaptureRoyalRule> Position<R> {
    fn index(at: [u16; 2]) -> Result<usize, String> {
        let [x, y] = at;
        R::Geometry::index(
            x.checked_sub(1).ok_or("coordinate must be positive")? as usize,
            y.checked_sub(1).ok_or("coordinate must be positive")? as usize,
        )
        .ok_or("coordinate outside board".into())
    }
    fn at(i: usize) -> [u16; 2] {
        let (x, y) = R::Geometry::coordinates(i).unwrap();
        [x as u16 + 1, y as u16 + 1]
    }
    fn def(cell: Cell) -> &'static PieceDef {
        &R::PIECES[cell.piece.0 as usize]
    }
    fn relative_rank(&self, i: usize, color: u8) -> usize {
        let (_, y) = R::Geometry::coordinates(i).unwrap();
        if color == 0 {
            y
        } else {
            R::Geometry::HEIGHT - 1 - y
        }
    }
    pub fn from_wire(wire: &WirePosition) -> Result<Self, String> {
        if wire.side_to_move > 1 {
            return Err("sideToMove must be 0 or 1".into());
        }
        if wire.hands.iter().any(|h| !h.is_empty()) {
            return Err("this rule has no hands; hands must be [{},{}]".into());
        }
        let mut pos = Self {
            cells: vec![None; R::Geometry::SQUARES],
            side: wire.side_to_move,
            rule: PhantomData,
        };
        let mut count = vec![vec![0; R::INVENTORY.len()]; 2];
        for p in &wire.pieces {
            if p.color > 1 {
                return Err("piece color must be 0 or 1".into());
            }
            let id = R::PIECES
                .iter()
                .position(|d| d.id == p.piece)
                .ok_or_else(|| format!("unknown piece ID: {}", p.piece))?;
            let i = Self::index(p.at)?;
            if pos.cells[i].is_some() {
                return Err("duplicate square".into());
            }
            let def = &R::PIECES[id];
            if pos.relative_rank(i, p.color) < def.dead_ranks {
                return Err("unpromoted piece on a dead rank".into());
            }
            count[p.color as usize][def.base] += 1;
            if count[p.color as usize][def.base] > R::INVENTORY[def.base] {
                return Err(format!("piece inventory exceeded: {}", p.piece));
            }
            pos.cells[i] = Some(Cell {
                piece: PieceId(id as u16),
                color: p.color,
            });
        }
        if pos.royals(0) == 0 && pos.royals(1) == 0 {
            return Err("both sides lack every royal".into());
        }
        Ok(pos)
    }
    pub fn to_wire(&self) -> WirePosition {
        WirePosition {
            side_to_move: self.side,
            pieces: self
                .cells
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    c.map(|c| PlacedPiece {
                        at: Self::at(i),
                        piece: Self::def(c).id.into(),
                        color: c.color,
                    })
                })
                .collect(),
            hands: Default::default(),
            state: RuleState {},
        }
    }
    fn royals(&self, color: u8) -> usize {
        self.cells
            .iter()
            .flatten()
            .filter(|c| c.color == color && Self::def(**c).royal)
            .count()
    }
    fn capture_outcome(&self) -> Outcome {
        if self.royals(0) == 0 {
            Outcome::win(1)
        } else if self.royals(1) == 0 {
            Outcome::win(0)
        } else {
            Outcome::ONGOING
        }
    }
    /// Stable FNV-1a fingerprint of rule, geometry, side and actual piece identities.
    /// Royalties are derived from those identities; v1 state/hands are empty.
    pub fn hash(&self) -> u64 {
        let mut hash = 0xcbf29ce484222325u64;
        let mut add = |b: u8| {
            hash = (hash ^ u64::from(b)).wrapping_mul(0x100000001b3);
        };
        for b in R::ID
            .to_le_bytes()
            .into_iter()
            .chain((R::Geometry::WIDTH as u16).to_le_bytes())
            .chain((R::Geometry::HEIGHT as u16).to_le_bytes())
        {
            add(b);
        }
        add(self.side);
        for c in &self.cells {
            match c {
                None => add(0),
                Some(c) => {
                    add(c.color + 1);
                    for b in Self::def(*c).id.bytes() {
                        add(b);
                    }
                    add(0);
                }
            }
        }
        hash
    }
    fn targets(&self, from: usize, cell: Cell) -> Vec<usize> {
        let sign = if cell.color == 0 { 1 } else { -1 };
        let def = Self::def(cell);
        let mut out = vec![];
        for &(x, y) in def.steps {
            if let Some(to) = R::Geometry::offset(from, x * sign, y * sign)
                && self.cells[to].is_none_or(|c| c.color != cell.color)
            {
                out.push(to);
            }
        }
        for &(x, y) in def.rays {
            let mut p = from;
            while let Some(to) = R::Geometry::offset(p, x * sign, y * sign) {
                if let Some(c) = self.cells[to] {
                    if c.color != cell.color {
                        out.push(to);
                    }
                    break;
                }
                out.push(to);
                p = to;
            }
        }
        out
    }
    /// Any royal attacked, as in Shogitter's checkOuteByDirection. This is used
    /// only for continuous-check repetition, never to restrict Sho legal moves.
    fn attacked(&self, color: u8) -> bool {
        self.cells.iter().enumerate().any(|(i, c)| {
            c.is_some_and(|c| {
                c.color != color
                    && self.targets(i, c).iter().any(|&j| {
                        self.cells[j].is_some_and(|v| v.color == color && Self::def(v).royal)
                    })
            })
        })
    }
    pub fn moves(&self) -> Vec<JsonMove> {
        if self.capture_outcome() != Outcome::ONGOING {
            return vec![];
        }
        let mut out = vec![];
        for (from, cell) in self.cells.iter().enumerate() {
            let Some(cell) = cell else { continue };
            if cell.color != self.side {
                continue;
            }
            let def = Self::def(*cell);
            for to in self.targets(from, *cell) {
                let zone = self.relative_rank(from, cell.color) < R::PROMOTION_RANKS
                    || self.relative_rank(to, cell.color) < R::PROMOTION_RANKS;
                let can = zone && def.promotion.is_some();
                let must = self.relative_rank(to, cell.color) < def.dead_ranks;
                if !must {
                    out.push(JsonMove {
                        schema_version: 1,
                        from: Self::at(from),
                        to: Self::at(to),
                        promotion: false,
                    });
                }
                if can {
                    out.push(JsonMove {
                        schema_version: 1,
                        from: Self::at(from),
                        to: Self::at(to),
                        promotion: true,
                    });
                }
            }
        }
        out.sort();
        out
    }
    fn play_unchecked(&self, mv: JsonMove) -> Self {
        let mut next = self.clone();
        let from = Self::index(mv.from).unwrap();
        let to = Self::index(mv.to).unwrap();
        let mut cell = next.cells[from].take().unwrap();
        if mv.promotion {
            cell.piece = Self::def(cell).promotion.unwrap();
        }
        next.cells[to] = Some(cell);
        next.side ^= 1;
        next
    }
    fn evaluate(&self) -> i32 {
        let sum: i32 = self
            .cells
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                c.map(|c| {
                    let d = Self::def(c);
                    let advance = if d.royal {
                        0
                    } else {
                        (R::Geometry::HEIGHT - 1 - self.relative_rank(i, c.color)) as i32 * 2
                    };
                    (d.value + advance) * if c.color == self.side { 1 } else { -1 }
                })
            })
            .sum();
        sum.clamp(-MATE + 1000, MATE - 1000)
    }
}

#[derive(Clone)]
pub struct Game<R: CaptureRoyalRule> {
    pub history: Vec<Position<R>>,
}
impl<R: CaptureRoyalRule> Game<R> {
    pub fn parse(json: &str) -> Result<Self, String> {
        let req: PositionRequest = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if req.schema_version != 1 || req.rule_id != R::ID {
            return Err("unsupported schemaVersion or ruleId".into());
        }
        let current = Position::<R>::from_wire(&req.position)?;
        let mut game = Self { history: vec![] };
        for wire in &req.history {
            let next = Position::<R>::from_wire(wire)?;
            if let Some(prev) = game.history.last() {
                if game.outcome() != Outcome::ONGOING {
                    return Err("history continues after game end".into());
                }
                // Infer the sole ordinary move from its two changed cells.
                let changed: Vec<_> = prev
                    .cells
                    .iter()
                    .zip(&next.cells)
                    .enumerate()
                    .filter(|(_, (a, b))| a != b)
                    .map(|(i, _)| i)
                    .collect();
                if changed.len() != 2 {
                    return Err("history is not an ordinary move".into());
                }
                let from = changed
                    .iter()
                    .copied()
                    .find(|&i| next.cells[i].is_none())
                    .ok_or("history has no vacated square")?;
                let to = changed.iter().copied().find(|&i| i != from).unwrap();
                let mv = JsonMove {
                    schema_version: 1,
                    from: Position::<R>::at(from),
                    to: Position::<R>::at(to),
                    promotion: prev.cells[from].map(|c| c.piece) != next.cells[to].map(|c| c.piece),
                };
                if !prev.moves().contains(&mv) || prev.play_unchecked(mv) != next {
                    return Err("illegal history transition".into());
                }
            }
            game.history.push(next);
        }
        if game.history.is_empty() {
            game.history.push(current);
        } else if game.current() != &current {
            return Err("history must end at position".into());
        }
        Ok(game)
    }
    pub fn current(&self) -> &Position<R> {
        self.history.last().unwrap()
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(&PositionRequest {
            schema_version: 1,
            rule_id: R::ID,
            position: self.current().to_wire(),
            history: self.history.iter().map(Position::to_wire).collect(),
        })
        .unwrap()
    }
    pub fn outcome(&self) -> Outcome {
        let current = self.current();
        let captured = current.capture_outcome();
        if captured != Outcome::ONGOING {
            return captured;
        }
        let matches: Vec<_> = self
            .history
            .iter()
            .enumerate()
            .filter(|(_, p)| *p == current)
            .map(|(i, _)| i)
            .collect();
        if matches.len() < 4 {
            return Outcome::ONGOING;
        }
        // The reference walks backwards and, if both players attack continuously,
        // first considers the current side as victim (the latest mover loses).
        let first = matches[0];
        for victim in [current.side, current.side ^ 1] {
            if self.history[first + 1..]
                .iter()
                .filter(|p| p.side == victim)
                .all(|p| p.attacked(victim))
            {
                return Outcome::win(victim);
            }
        }
        Outcome::DRAW
    }
    pub fn moves(&self) -> Vec<JsonMove> {
        if self.outcome() == Outcome::ONGOING {
            self.current().moves()
        } else {
            vec![]
        }
    }
    pub fn play(&mut self, mv: JsonMove) -> Result<(), String> {
        if mv.schema_version != 1 || !self.moves().contains(&mv) {
            return Err("illegal move or unsupported schemaVersion".into());
        }
        self.push(mv);
        Ok(())
    }
    fn push(&mut self, mv: JsonMove) {
        let next = self.current().play_unchecked(mv);
        self.history.push(next);
    }
    fn context_key(&self) -> u64 {
        self.history.iter().fold(0x6a09e667f3bcc909, |h, p| {
            h.rotate_left(7) ^ p.hash().wrapping_mul(0x9e3779b97f4a7c15)
        })
    }
    pub fn perft(&mut self, depth: u8) -> u64 {
        if depth == 0 {
            return 1;
        }
        let moves = self.moves();
        let mut n = 0;
        for mv in moves {
            self.push(mv);
            n += self.perft(depth - 1);
            self.history.pop();
        }
        n
    }
}
pub type ShoGame = Game<Shoshogi>;
pub fn start_position_json() -> String {
    let position = WirePosition {
        side_to_move: 0,
        pieces: Shoshogi::initial(),
        hands: Default::default(),
        state: RuleState {},
    };
    let p = Position::<Shoshogi>::from_wire(&position).unwrap();
    Game { history: vec![p] }.to_json()
}
pub fn validate_position_json(json: &str) -> Result<String, String> {
    Ok(ShoGame::parse(json)?.to_json())
}
pub fn legal_moves_json(json: &str) -> Result<String, String> {
    serde_json::to_string(&ShoGame::parse(json)?.moves()).map_err(|e| e.to_string())
}
pub fn apply_move_json(json: &str, mv: &str) -> Result<String, String> {
    let mut game = ShoGame::parse(json)?;
    let mv = serde_json::from_str(mv).map_err(|e| e.to_string())?;
    game.play(mv)?;
    Ok(game.to_json())
}
pub fn inspect_json(json: &str) -> Result<String, String> {
    let g = ShoGame::parse(json)?;
    Ok(serde_json::json!({"schemaVersion":1,"positionHash":format!("{:016x}",g.current().hash()),"contextHash":format!("{:016x}",g.context_key()),"outcome":g.outcome()}).to_string())
}
pub fn perft_json(json: &str, depth: u8) -> Result<String, String> {
    if depth > 6 {
        return Err("perft depth must be <= 6".into());
    }
    let mut g = ShoGame::parse(json)?;
    let start = Instant::now();
    let nodes = g.perft(depth);
    let elapsed = start.elapsed().as_secs_f64() * 1000.;
    Ok(serde_json::json!({"nodes":nodes,"elapsedMs":elapsed,"nps":(nodes as f64*1000./elapsed.max(0.001)) as u64}).to_string())
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub schema_version: u8,
    pub best_move: Option<JsonMove>,
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
struct Entry {
    depth: u8,
    score: i32,
    bound: u8,
    best: Option<JsonMove>,
}
struct Search {
    start: Instant,
    timeout_ms: u32,
    limit: u64,
    nodes: u64,
    stopped: bool,
    tt: HashMap<u64, Entry>,
}
impl Search {
    fn expired(&mut self) -> bool {
        if self.nodes >= self.limit
            || self.start.elapsed().as_millis() >= u128::from(self.timeout_ms)
        {
            self.stopped = true;
        }
        self.stopped
    }
    fn ordered<R: CaptureRoyalRule>(&self, g: &Game<R>, best: Option<JsonMove>) -> Vec<JsonMove> {
        let p = g.current();
        let mut moves = p.moves();
        moves.sort_by_cached_key(|m| {
            let victim = p.cells[Position::<R>::index(m.to).unwrap()]
                .map_or(0, |c| Position::<R>::def(c).value);
            -(if Some(*m) == best { 100_000 } else { 0 }
                + victim * 10
                + if m.promotion { 500 } else { 0 })
        });
        moves
    }
    fn terminal<R: CaptureRoyalRule>(&self, g: &Game<R>, ply: i32) -> Option<i32> {
        let outcome = g.outcome();
        if outcome == Outcome::ONGOING {
            None
        } else {
            Some(outcome.winner.map_or(0, |w| {
                if w == g.current().side {
                    MATE - ply
                } else {
                    -MATE + ply
                }
            }))
        }
    }
    fn qsearch<R: CaptureRoyalRule>(
        &mut self,
        g: &mut Game<R>,
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
        let stand = g.current().evaluate();
        if stand >= beta {
            return stand;
        }
        alpha = alpha.max(stand);
        if left == 0 {
            return alpha;
        }
        for mv in self.ordered(g, None) {
            if g.current().cells[Position::<R>::index(mv.to).unwrap()].is_none() && !mv.promotion {
                continue;
            }
            g.push(mv);
            let v = -self.qsearch(g, -beta, -alpha, left - 1, ply + 1);
            g.history.pop();
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
    fn negamax<R: CaptureRoyalRule>(
        &mut self,
        g: &mut Game<R>,
        depth: u8,
        mut alpha: i32,
        beta: i32,
        ply: i32,
    ) -> (i32, Option<JsonMove>) {
        if self.expired() {
            return (0, None);
        }
        self.nodes += 1;
        if let Some(v) = self.terminal(g, ply) {
            return (v, None);
        }
        if depth == 0 {
            return (self.qsearch(g, alpha, beta, 4, ply), None);
        }
        let key = g.context_key();
        let entry = self.tt.get(&key).copied();
        let original = alpha;
        if let Some(e) = entry
            && e.depth >= depth
            && (e.bound == 0 || e.bound == 1 && e.score >= beta || e.bound == 2 && e.score <= alpha)
        {
            return (e.score, e.best);
        }
        let moves = self.ordered(g, entry.and_then(|e| e.best));
        if moves.is_empty() {
            return (g.current().evaluate(), None);
        }
        let mut score = -MATE - 1;
        let mut best = None;
        for mv in moves {
            g.push(mv);
            let value = -self.negamax(g, depth - 1, -beta, -alpha, ply + 1).0;
            g.history.pop();
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
        if self.tt.len() >= 65_536 {
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
pub fn search_json(
    json: &str,
    max_depth: u8,
    timeout_ms: u32,
    node_budget: u32,
) -> Result<String, String> {
    if max_depth == 0 || max_depth > 100 || timeout_ms == 0 {
        return Err("depth must be 1..100 and timeoutMs must be positive".into());
    }
    let mut game = ShoGame::parse(json)?;
    let outcome = game.outcome();
    let fallback = game.moves().first().copied();
    let mut search = Search {
        start: Instant::now(),
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
    let mut score = search
        .terminal(&game, 0)
        .unwrap_or_else(|| game.current().evaluate());
    if outcome == Outcome::ONGOING && fallback.is_some() {
        for depth in 1..=max_depth {
            let (v, mv) = search.negamax(&mut game, depth, -MATE - 1, MATE + 1, 0);
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
        schema_version: 1,
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

#[cfg(test)]
mod tests;

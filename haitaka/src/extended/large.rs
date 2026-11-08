//! Effectful large-board rules. Search applies reversible square/state deltas.
use super::classical::ClassicalPiece;
use super::path::BoundedPath;
use super::search::SearchGame;
use super::*;
use std::collections::BTreeSet;

pub const PROTOCOL: &str = "haitaka-json-v4";
pub const NAME: &str = "tenjiku-dai";
pub const RULE_ID: u32 = 59;
type Geometry = Rect<16, 16>;
type Pos = Position<TenjikuDai>;
pub type TurnPath = BoundedPath<3>;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resident {
    pub at: [u16; 2],
    pub color: u8,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TenjikuState {
    pub senjishi: bool,
    pub shitenno: Vec<Resident>,
}
impl PositionState for TenjikuState {
    fn hash_bytes(&self, add: &mut impl FnMut(u8)) {
        add(self.senjishi as u8);
        for p in &self.shitenno {
            for n in p.at {
                for b in n.to_le_bytes() {
                    add(b);
                }
            }
            add(p.color);
        }
        add(255);
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TenjikuDai;
#[derive(Clone, Copy)]
struct Primitive {
    delta: (isize, isize),
    kind: u8,
    limit: u8,
    skip: u8,
    next: i8,
    enemy: bool,
}
struct Movement {
    root: &'static [Primitive],
    states: &'static [&'static [Primitive]],
    exceptions: &'static [&'static str],
    stop_capture: bool,
    no_stop: bool,
    file_limit: usize,
}
include!("large/tenjiku_defs.rs");
impl CaptureRoyalRule for TenjikuDai {
    type Geometry = Geometry;
    type State = TenjikuState;
    type Hands = NoHands;
    type Placement = ClassicalPiece;
    const ID: u32 = 59;
    const ALLOW_NO_ROYALS: bool = true;
    const PIECES: &'static [PieceDef] = PIECES;
    const INVENTORY: &'static [usize] = INVENTORY;
    const PROMOTION_RANKS: usize = 5;
    fn initial() -> Vec<ClassicalPiece> {
        INITIAL
            .iter()
            .map(|&(at, id, color)| ClassicalPiece::make(at, PIECES[id as usize].id, color, None))
            .collect()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LargeMove {
    pub schema_version: u8,
    pub from: [u16; 2],
    pub path: TurnPath,
    pub promotion: bool,
}
impl LargeMove {
    fn new(from: usize, path: &[[u16; 2]], promotion: bool) -> Self {
        Self {
            schema_version: 4,
            from: Pos::at(from),
            path: TurnPath::from_steps(path).unwrap(),
            promotion,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pending {
    pub from: [u16; 2],
    pub path: BoundedPath<2>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct History {
    pub initial: WirePosition<TenjikuState, ClassicalPiece>,
    pub moves: Vec<LargeMove>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub schema_version: u8,
    pub rule_id: u32,
    pub position: WirePosition<TenjikuState, ClassicalPiece>,
    pub history: Option<History>,
    pub pending: Option<Pending>,
}
#[derive(Clone, Debug)]
struct Delta {
    squares: Vec<(usize, Option<Cell>)>,
    state: TenjikuState,
    side: u8,
}
impl Delta {
    fn new(p: &Pos) -> Self {
        Self {
            squares: vec![],
            state: p.state.clone(),
            side: p.side,
        }
    }
    fn set(&mut self, p: &mut Pos, i: usize, c: Option<Cell>) {
        if p.cells[i] != c {
            if !self.squares.iter().any(|&(j, _)| i == j) {
                self.squares.push((i, p.cells[i]));
            }
            p.set_cell(i, c);
        }
    }
    fn restore(self, p: &mut Pos) {
        for (i, c) in self.squares {
            p.set_cell(i, c);
        }
        p.state = self.state;
        p.side = self.side;
    }
}
#[derive(Clone, Copy, Debug)]
struct Target {
    to: usize,
    next: Option<u8>,
}
fn adjacent(a: usize, b: usize) -> bool {
    let (x, y) = Geometry::coordinates(a).unwrap();
    let (u, v) = Geometry::coordinates(b).unwrap();
    a != b && x.abs_diff(u).max(y.abs_diff(v)) == 1
}
fn fire(c: Cell) -> bool {
    matches!(Pos::def(c).id, "fe" | "zg")
}
fn lion(c: Cell) -> bool {
    matches!(Pos::def(c).id, "bl" | "dh")
}
fn at_cell(p: &Pos, i: usize, ignore: Option<usize>) -> Option<Cell> {
    if Some(i) == ignore { None } else { p.cells[i] }
}
fn order(p: &Pos, previous: &[Resident], ignore: Option<usize>) -> Vec<Resident> {
    let current: Vec<_> = p
        .cells
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            c.filter(|c| Some(i) != ignore && Pos::def(*c).id == "fp")
                .map(|c| Resident {
                    at: Pos::at(i),
                    color: c.color,
                })
        })
        .collect();
    let mut out: Vec<_> = previous
        .iter()
        .filter(|r| current.contains(r))
        .cloned()
        .collect();
    for c in current {
        if !out.iter().any(|r| r.at == c.at) {
            out.push(c);
        }
    }
    out
}
fn active(p: &Pos, ignore: Option<usize>) -> Vec<Resident> {
    let mut out: Vec<Resident> = vec![];
    for r in order(p, &p.state.shitenno, ignore) {
        let i = Pos::index(r.at).unwrap();
        if !out
            .iter()
            .any(|q| q.color != r.color && adjacent(Pos::index(q.at).unwrap(), i))
        {
            out.push(r);
        }
    }
    out
}
fn frozen(i: usize, color: u8, actives: &[Resident]) -> bool {
    actives
        .iter()
        .any(|p| p.color != color && adjacent(Pos::index(p.at).unwrap(), i))
}
fn fires(p: &Pos) -> Vec<Resident> {
    let a = active(p, None);
    p.cells
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            c.filter(|c| fire(*c) && !frozen(i, c.color, &a))
                .map(|c| Resident {
                    at: Pos::at(i),
                    color: c.color,
                })
        })
        .collect()
}
fn validate_state(p: &Pos) -> Result<(), String> {
    let normalized = order(p, &p.state.shitenno, None);
    if normalized != p.state.shitenno
        || p.state
            .shitenno
            .iter()
            .map(|r| r.at)
            .collect::<BTreeSet<_>>()
            .len()
            != p.state.shitenno.len()
    {
        return Err(
            "shitenno must list every four heavenly king exactly once, in resident order".into(),
        );
    }
    Ok(())
}
fn capture_outcome(p: &Pos) -> Outcome {
    match (p.royals(0), p.royals(1)) {
        (0, 0) => Outcome::DRAW,
        (0, _) => Outcome::win(1),
        (_, 0) => Outcome::win(0),
        _ => Outcome::ONGOING,
    }
}
fn targets(p: &Pos, from: usize, status: Option<u8>, ignore: Option<usize>) -> Vec<Target> {
    let Some(c) = at_cell(p, from, ignore) else {
        return vec![];
    };
    let m = &MOVEMENT[c.piece.0 as usize];
    if status.is_none() && frozen(from, c.color, &active(p, ignore)) {
        return vec![];
    }
    let prims = if let Some(s) = status {
        m.states.get(s as usize).copied().unwrap_or(&[])
    } else {
        m.root
    };
    let mut raw: Vec<(usize, i8)> = vec![];
    let mut prohibited = BTreeSet::new();
    let sign = if c.color == 0 { 1 } else { -1 };
    for prim in prims {
        let (dx, dy) = (prim.delta.0 * sign, prim.delta.1 * sign);
        let mut now = from;
        for _ in 0..=prim.skip {
            if let Some(next) = Geometry::offset(now, dx, dy) {
                now = next;
            } else {
                now = usize::MAX;
                break;
            }
        }
        if now == usize::MAX {
            continue;
        }
        let ray = matches!(prim.kind, 2 | 6) || prim.limit > 0;
        let max = if prim.limit > 0 {
            usize::from(prim.limit)
        } else if ray {
            Geometry::SQUARES
        } else {
            1
        };
        for _ in 0..max {
            let v = at_cell(p, now, ignore);
            if v.is_some_and(|v| m.exceptions.contains(&Pos::def(v).id)) {
                break;
            }
            if prim.enemy && !v.is_some_and(|v| v.color != c.color) {
                prohibited.insert(now);
                break;
            }
            if let Some(t) = raw.iter_mut().find(|t| t.0 == now) {
                if t.1 == -2 && prim.next != -2 {
                    t.1 = prim.next;
                }
            } else {
                raw.push((now, prim.next));
            }
            if !ray || (prim.kind != 6 && v.is_some()) {
                break;
            }
            let Some(next) = Geometry::offset(now, dx, dy) else {
                break;
            };
            now = next;
        }
    }
    if status.is_none() && Pos::def(c).id == "fp" {
        let mut frontier = BTreeSet::from([from]);
        for step in 1..=3 {
            let mut next = BTreeSet::new();
            for via in frontier {
                for &(dx, dy) in KING {
                    let Some(to) = Geometry::offset(via, dx, dy) else {
                        continue;
                    };
                    if to == from {
                        continue;
                    }
                    let v = at_cell(p, to, ignore);
                    if step == 1 {
                        next.insert(to);
                    } else if !v.is_some_and(|v| v.color == c.color) {
                        if !adjacent(from, to) && !raw.iter().any(|r| r.0 == to) {
                            raw.push((to, -2));
                        }
                        if v.is_none() {
                            next.insert(to);
                        }
                    }
                }
            }
            frontier = next;
        }
    }
    raw.into_iter()
        .filter(|(to, _)| !prohibited.contains(to))
        .map(|(to, n)| Target {
            to,
            next: if n >= 0 && !(m.stop_capture && at_cell(p, to, ignore).is_some()) {
                Some(n as u8)
            } else {
                None
            },
        })
        .collect()
}
fn protected(p: &Pos, to: usize, capturing: u8, remove: usize) -> bool {
    p.cells.iter().enumerate().any(|(i, c)| {
        i != remove
            && c.is_some_and(|c| {
                c.color != capturing && targets(p, i, None, Some(remove)).iter().any(|t| t.to == to)
            })
    })
}
fn capture_allowed(
    p: &Pos,
    original: &Pos,
    from: usize,
    to: usize,
    first: Option<(usize, Option<Cell>)>,
) -> bool {
    let Some(victim) = p.cells[to] else {
        return true;
    };
    let mover = p.cells[from].unwrap();
    if victim.color == mover.color {
        return false;
    }
    if !lion(victim) {
        return true;
    }
    if !lion(mover) {
        return !p.state.senjishi;
    }
    if let Some((source, v)) = first {
        if let Some(v) = v {
            if Pos::def(v).id != "aa" {
                return true;
            }
            return !protected(original, to, mover.color, source);
        }
        return !protected(p, to, mover.color, from);
    }
    adjacent(from, to) || !protected(p, to, mover.color, from)
}
fn hop(p: &mut Pos, from: usize, to: usize, d: &mut Delta) {
    let mover = p.cells[from].unwrap();
    p.state.senjishi = p.cells[to].is_some_and(lion) && !lion(mover);
    d.set(p, from, None);
    d.set(p, to, Some(mover));
}
fn finish(p: &mut Pos, to: usize, resident_fires: &[Resident], d: &mut Delta) {
    if let Some(moved) = p.cells[to] {
        if resident_fires.iter().any(|r| {
            r.color != moved.color
                && adjacent(Pos::index(r.at).unwrap(), to)
                && p.cells[Pos::index(r.at).unwrap()].is_some_and(fire)
        }) {
            d.set(p, to, None);
        }
    }
    p.state.shitenno = order(p, &p.state.shitenno, None);
    let actives = active(p, None);
    let mut burnt = BTreeSet::new();
    for (i, c) in p.cells.iter().enumerate() {
        let Some(c) = c else {
            continue;
        };
        if !fire(*c) || frozen(i, c.color, &actives) {
            continue;
        }
        for &(dx, dy) in KING {
            if let Some(j) = Geometry::offset(i, dx, dy) {
                if p.cells[j].is_some_and(|v| v.color != c.color) {
                    burnt.insert(j);
                }
            }
        }
    }
    for i in burnt {
        d.set(p, i, None);
    }
    p.state.shitenno = order(p, &p.state.shitenno, None);
}
fn file_allowed(p: &Pos, to: usize) -> bool {
    let Some(c) = p.cells[to] else {
        return true;
    };
    let limit = MOVEMENT[c.piece.0 as usize].file_limit;
    if limit == 0 {
        return true;
    }
    let (x, _) = Geometry::coordinates(to).unwrap();
    p.cells[x * 16..(x + 1) * 16]
        .iter()
        .flatten()
        .filter(|v| v.color == c.color && v.piece == c.piece)
        .count()
        <= limit
}
fn attacked(p: &Pos, color: u8) -> bool {
    p.cells.iter().enumerate().any(|(i, c)| {
        c.is_some_and(|c| {
            c.color != color
                && targets(p, i, None, None)
                    .iter()
                    .any(|t| p.cells[t.to].is_some_and(|v| v.color == color && Pos::def(v).royal))
        })
    })
}
struct Enumeration<'a> {
    original: &'a Pos,
    resident_fires: Vec<Resident>,
    pending: Option<Pending>,
    wanted: Option<LargeMove>,
    stop: &'a mut dyn FnMut() -> bool,
    limit: usize,
    out: Vec<LargeMove>,
}
impl Enumeration<'_> {
    fn emit(
        &mut self,
        p: &mut Pos,
        from: usize,
        path: &[[u16; 2]],
        can: bool,
        early: bool,
    ) -> Option<bool> {
        let to = Pos::index(*path.last().unwrap()).unwrap();
        let c = p.cells[to].unwrap();
        let must = can && p.relative_rank(to, c.color) < Pos::def(c).dead_ranks;
        for promotion in [false, true] {
            if promotion && (!can || early) || !promotion && must {
                continue;
            }
            let mv = LargeMove::new(from, path, promotion);
            if self.wanted.is_some_and(|w| w != mv)
                || self
                    .pending
                    .is_some_and(|v| mv.from != v.from || !mv.path.starts_with(&v.path))
            {
                continue;
            }
            if (self.stop)() {
                return None;
            }
            let mut undo = Delta::new(p);
            if promotion {
                let mut c = c;
                c.piece = Pos::def(c).promotion.unwrap();
                undo.set(p, to, Some(c));
            }
            if !early {
                finish(p, to, &self.resident_fires, &mut undo);
            } else {
                p.state.shitenno = order(p, &p.state.shitenno, None);
            }
            let legal = file_allowed(p, to);
            undo.restore(p);
            if legal {
                self.out.push(mv);
                if self.out.len() >= self.limit {
                    return Some(true);
                }
            }
        }
        Some(false)
    }
    fn visit(
        &mut self,
        p: &mut Pos,
        root: usize,
        from: usize,
        status: Option<u8>,
        path: &mut Vec<[u16; 2]>,
        can: bool,
        first: Option<(usize, Option<Cell>)>,
    ) -> Option<bool> {
        for t in targets(p, from, status, None) {
            if (self.stop)() {
                return None;
            }
            if let Some(w) = self.wanted {
                if w.path.len() <= path.len() || w.path[path.len()] != Pos::at(t.to) {
                    continue;
                }
            }
            if let Some(v) = self.pending {
                if path.len() < v.path.len() && v.path[path.len()] != Pos::at(t.to) {
                    continue;
                }
            }
            if !capture_allowed(p, self.original, from, t.to, first) {
                continue;
            }
            let c = p.cells[from].unwrap();
            let captured = p.cells[t.to];
            let eligible = can
                || (Pos::def(c).promotion.is_some()
                    && (p.relative_rank(from, c.color) < 5 || p.relative_rank(t.to, c.color) < 5));
            let mut undo = Delta::new(p);
            hop(p, from, t.to, &mut undo);
            path.push(Pos::at(t.to));
            let early = t.next.is_some() && capture_outcome(p) != Outcome::ONGOING;
            let can_stop = early || t.next.is_none() || !MOVEMENT[c.piece.0 as usize].no_stop;
            let mut result = Some(false);
            if can_stop {
                result = self.emit(p, root, path, eligible, early);
            }
            if result == Some(false) && !early && t.next.is_some() && path.len() < 3 {
                result = self.visit(
                    p,
                    root,
                    t.to,
                    t.next,
                    path,
                    eligible,
                    first.or(Some((root, captured))),
                );
            }
            path.pop();
            undo.restore(p);
            if result != Some(false) {
                return result;
            }
        }
        Some(false)
    }
}
fn enumerate(
    p: &Pos,
    pending: Option<Pending>,
    wanted: Option<LargeMove>,
    limit: usize,
    stop: &mut dyn FnMut() -> bool,
) -> Option<Vec<LargeMove>> {
    if capture_outcome(p) != Outcome::ONGOING || limit == 0 {
        return Some(vec![]);
    }
    let mut scratch = p.clone();
    let mut e = Enumeration {
        original: p,
        resident_fires: fires(p),
        pending,
        wanted,
        stop,
        limit,
        out: vec![],
    };
    let mut occupied = p.occupied[p.side as usize];
    while let Some(from) = occupied.first() {
        occupied = occupied.without(from);
        if wanted.is_some_and(|m| m.from != Pos::at(from))
            || pending.is_some_and(|m| m.from != Pos::at(from))
        {
            continue;
        }
        if e.visit(&mut scratch, from, from, None, &mut vec![], false, None)? {
            break;
        }
    }
    e.out.sort();
    e.out.dedup();
    Some(e.out)
}
#[derive(Clone)]
pub struct LargeGame {
    position: Pos,
    initial: WirePosition<TenjikuState, ClassicalPiece>,
    played: Vec<LargeMove>,
    undo: Vec<Delta>,
    timeline: Vec<(u64, u8)>,
    pending: Option<Pending>,
    pending_ply: usize,
}
impl LargeGame {
    pub fn root(p: Pos) -> Result<Self, String> {
        validate_state(&p)?;
        Ok(Self {
            initial: p.to_wire(),
            timeline: vec![(p.hash(), p.side)],
            position: p,
            played: vec![],
            undo: vec![],
            pending: None,
            pending_ply: 0,
        })
    }
    pub fn parse(json: &str) -> Result<Self, String> {
        let q: Request = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if q.schema_version != 4 || q.rule_id != 59 {
            return Err("unsupported schemaVersion or ruleId".into());
        }
        let current = Pos::from_wire(&q.position)?;
        validate_state(&current)?;
        let mut g = if let Some(h) = q.history {
            let mut g = Self::root(Pos::from_wire(&h.initial)?)?;
            for m in h.moves {
                g.play(m)?;
            }
            g
        } else {
            Self::root(current.clone())?
        };
        if g.position != current {
            return Err("history must end at position including ordered effect state".into());
        }
        g.pending_ply = g.played.len();
        g.pending = q.pending;
        if let Some(pending) = g.pending {
            for &at in pending.path.iter() {
                Pos::index(at)?;
            }
            Pos::index(pending.from)?;
            if g.moves().is_empty() {
                return Err("invalid or completed pending path".into());
            }
            let mut p = g.position.clone();
            let mut from = Pos::index(pending.from)?;
            let mut status = None;
            for &at in pending.path.iter() {
                let to = Pos::index(at)?;
                let t = targets(&p, from, status, None)
                    .into_iter()
                    .find(|t| t.to == to)
                    .ok_or("invalid pending hop")?;
                let mut d = Delta::new(&p);
                hop(&mut p, from, to, &mut d);
                status = t.next;
                from = to;
            }
            if status.is_none() || capture_outcome(&p) != Outcome::ONGOING {
                return Err("pending path is already complete".into());
            }
        }
        Ok(g)
    }
    pub fn current(&self) -> &Pos {
        &self.position
    }
    fn prefix(&self) -> Option<Pending> {
        if self.played.len() == self.pending_ply {
            self.pending
        } else {
            None
        }
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(&Request {
            schema_version: 4,
            rule_id: 59,
            position: self.position.to_wire(),
            history: Some(History {
                initial: self.initial.clone(),
                moves: self.played.clone(),
            }),
            pending: self.prefix(),
        })
        .unwrap()
    }
    fn basic_outcome(&self) -> Outcome {
        let captured = capture_outcome(&self.position);
        if captured != Outcome::ONGOING {
            return captured;
        }
        let h = self.position.hash();
        let matches: Vec<_> = self
            .timeline
            .iter()
            .enumerate()
            .filter(|(_, p)| p.0 == h && p.1 == self.position.side)
            .map(|(i, _)| i)
            .collect();
        if matches.len() < 4 {
            return Outcome::ONGOING;
        }
        // Check attacks only when fourfold repetition actually occurs. Replaying
        // every royal's geometric attacks on every search push is costly here.
        let mut checked = [true, true];
        let mut p = self.position.clone();
        for i in (matches[0] + 1..self.timeline.len()).rev() {
            let color = p.side as usize;
            checked[color] &= attacked(&p, p.side);
            self.undo[i - 1].clone().restore(&mut p);
        }
        for victim in [self.position.side, self.position.side ^ 1] {
            if checked[victim as usize] {
                return Outcome::win(victim);
            }
        }
        Outcome::DRAW
    }
    fn outcome_cancel(&self, stop: &mut dyn FnMut() -> bool) -> Option<Outcome> {
        if stop() {
            return None;
        }
        let o = self.basic_outcome();
        if o != Outcome::ONGOING {
            return Some(o);
        }
        if enumerate(&self.position, self.prefix(), None, 1, stop)?.is_empty()
            && !attacked(&self.position, self.position.side)
        {
            Some(Outcome::win(self.position.side ^ 1))
        } else {
            Some(o)
        }
    }
    pub fn outcome(&self) -> Outcome {
        self.outcome_cancel(&mut || false).unwrap()
    }
    pub fn moves(&self) -> Vec<LargeMove> {
        if self.basic_outcome() != Outcome::ONGOING {
            vec![]
        } else {
            enumerate(&self.position, self.prefix(), None, usize::MAX, &mut || {
                false
            })
            .unwrap()
        }
    }
    pub fn play(&mut self, m: LargeMove) -> Result<(), String> {
        if m.schema_version != 4
            || self.basic_outcome() != Outcome::ONGOING
            || enumerate(&self.position, self.prefix(), Some(m), 1, &mut || false)
                .unwrap()
                .is_empty()
        {
            return Err("illegal completed move".into());
        }
        self.push(m);
        Ok(())
    }
    fn push(&mut self, m: LargeMove) {
        let resident = fires(&self.position);
        let mut delta = Delta::new(&self.position);
        let mut from = Pos::index(m.from).unwrap();
        let mut status = None;
        for &at in m.path.iter() {
            let to = Pos::index(at).unwrap();
            status = targets(&self.position, from, status, None)
                .into_iter()
                .find(|t| t.to == to)
                .unwrap()
                .next;
            hop(&mut self.position, from, to, &mut delta);
            from = to;
        }
        let early = status.is_some() && capture_outcome(&self.position) != Outcome::ONGOING;
        if m.promotion {
            let mut c = self.position.cells[from].unwrap();
            c.piece = Pos::def(c).promotion.unwrap();
            delta.set(&mut self.position, from, Some(c));
        }
        if !early {
            finish(&mut self.position, from, &resident, &mut delta);
        } else {
            self.position.state.shitenno =
                order(&self.position, &self.position.state.shitenno, None);
        }
        self.position.side ^= 1;
        self.timeline
            .push((self.position.hash(), self.position.side));
        self.undo.push(delta);
        self.played.push(m);
    }
    fn pop(&mut self) {
        self.undo.pop().unwrap().restore(&mut self.position);
        self.played.pop();
        self.timeline.pop();
    }
    pub fn position_hash(&self) -> u64 {
        let mut h = self.position.hash();
        if let Some(p) = self.prefix() {
            for n in p.from.into_iter().chain(p.path.iter().flatten().copied()) {
                h = h.rotate_left(7) ^ u64::from(n).wrapping_mul(0x9e3779b97f4a7c15);
            }
        }
        h
    }
    fn context_key(&self) -> u64 {
        self.timeline.iter().fold(0x6a09e667f3bcc909u64, |h, p| {
            h.rotate_left(7) ^ p.0.wrapping_mul(0x9e3779b97f4a7c15)
        }) ^ self.position_hash().rotate_left(13)
    }
    pub fn perft(&mut self, depth: u8) -> u64 {
        if depth == 0 {
            return 1;
        }
        let mut nodes = 0;
        for m in self.moves() {
            self.push(m);
            nodes += self.perft(depth - 1);
            self.pop();
        }
        nodes
    }
}
impl SearchGame for LargeGame {
    type Move = LargeMove;
    const SCHEMA: u8 = 4;
    fn moves(&self, limit: usize, stop: &mut dyn FnMut() -> bool) -> Option<Vec<LargeMove>> {
        enumerate(&self.position, self.prefix(), None, limit, stop)
    }
    fn outcome(&self, stop: &mut dyn FnMut() -> bool) -> Option<Outcome> {
        self.outcome_cancel(stop)
    }
    fn side(&self) -> u8 {
        self.position.side
    }
    fn evaluate(&self) -> i32 {
        self.position.evaluate()
    }
    fn push(&mut self, m: LargeMove) {
        LargeGame::push(self, m)
    }
    fn pop(&mut self) {
        LargeGame::pop(self)
    }
    fn key(&self) -> u64 {
        self.context_key()
    }
    fn capture_value(&self, m: LargeMove) -> i32 {
        let p = &self.position;
        let from = Pos::index(m.from).unwrap();
        let c = p.cells[from].unwrap();
        let mut seen = BTreeSet::new();
        let mut value = 0;
        for &at in m.path.iter() {
            let i = Pos::index(at).unwrap();
            if i != from && seen.insert(i) {
                if let Some(v) = p.cells[i] {
                    value += Pos::def(v).value;
                }
            }
        }
        let end = Pos::index(*m.path.last().unwrap()).unwrap();
        let promoted = if m.promotion {
            Pos::def(c).promotion.map(|id| Cell {
                piece: id,
                color: c.color,
            })
        } else {
            None
        };
        if fire(c) || promoted.is_some_and(fire) {
            for &(dx, dy) in KING {
                if let Some(i) = Geometry::offset(end, dx, dy) {
                    if i != from && seen.insert(i) {
                        if let Some(v) = p.cells[i].filter(|v| v.color != c.color) {
                            value += Pos::def(v).value;
                        }
                    }
                }
            }
        }
        value
    }
    fn promotion(m: LargeMove) -> bool {
        m.promotion
    }
}
pub mod compiled {
    use super::*;
    pub type CompiledGame = LargeGame;
    pub const PROTOCOL: &str = super::PROTOCOL;
    pub const NAME: &str = super::NAME;
    pub const RULE_ID: u32 = super::RULE_ID;
    pub fn start_position_json() -> String {
        LargeGame::root(
            Pos::from_wire(&WirePosition {
                side_to_move: 0,
                pieces: TenjikuDai::initial(),
                hands: Default::default(),
                state: Default::default(),
            })
            .unwrap(),
        )
        .unwrap()
        .to_json()
    }
    pub fn validate_position_json(json: &str) -> Result<String, String> {
        Ok(LargeGame::parse(json)?.to_json())
    }
    pub fn legal_moves_json(json: &str) -> Result<String, String> {
        serde_json::to_string(&LargeGame::parse(json)?.moves()).map_err(|e| e.to_string())
    }
    pub fn apply_move_json(json: &str, mv: &str) -> Result<String, String> {
        let mut g = LargeGame::parse(json)?;
        g.play(serde_json::from_str(mv).map_err(|e| e.to_string())?)?;
        Ok(g.to_json())
    }
    pub fn inspect_json(json: &str) -> Result<String, String> {
        let g = LargeGame::parse(json)?;
        Ok(serde_json::json!({"schemaVersion":4,"positionHash":format!("{:016x}",g.position_hash()),"contextHash":format!("{:016x}",g.context_key()),"outcome":g.outcome()}).to_string())
    }
    pub fn perft_json(json: &str, depth: u8) -> Result<String, String> {
        if depth > 4 {
            return Err("large-rule perft depth must be <= 4".into());
        }
        let mut g = LargeGame::parse(json)?;
        let t = Instant::now();
        let nodes = g.perft(depth);
        let ms = t.elapsed().as_secs_f64() * 1000.;
        Ok(serde_json::json!({"nodes":nodes,"elapsedMs":ms,"nps":(nodes as f64*1000./ms.max(0.001))as u64}).to_string())
    }
    pub fn search_json(
        json: &str,
        depth: u8,
        timeout_ms: u32,
        nodes: u32,
    ) -> Result<String, String> {
        search::search_json(&mut LargeGame::parse(json)?, depth, timeout_ms, nodes)
    }
}
#[cfg(test)]
mod tests;

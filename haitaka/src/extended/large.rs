//! Effectful large-board rules. Search applies reversible square/state deltas.
use super::classical::ClassicalPiece;
use super::path::BoundedPath;
use super::search::SearchGame;
use super::*;
use std::collections::BTreeSet;

#[cfg(not(feature = "taikyoku"))]
pub const PROTOCOL: &str = "haitaka-json-v4";
#[cfg(feature = "taikyoku")]
pub const PROTOCOL: &str = "haitaka-json-v5";
const SCHEMA_VERSION: u8 = if cfg!(feature = "taikyoku") { 5 } else { 4 };
#[cfg(feature = "tenjiku-dai")]
pub const NAME: &str = "tenjiku-dai";
#[cfg(feature = "tenjiku-dai")]
pub const RULE_ID: u32 = 59;
#[cfg(feature = "dai-dai")]
pub const NAME: &str = "dai-dai";
#[cfg(feature = "dai-dai")]
pub const RULE_ID: u32 = 60;
#[cfg(feature = "maka-dai-dai")]
pub const NAME: &str = "maka-dai-dai";
#[cfg(feature = "maka-dai-dai")]
pub const RULE_ID: u32 = 61;
#[cfg(feature = "taishogi")]
pub const NAME: &str = "taishogi";
#[cfg(feature = "taishogi")]
pub const RULE_ID: u32 = 63;
#[cfg(feature = "taikyoku")]
pub const NAME: &str = "taikyoku";
#[cfg(feature = "taikyoku")]
pub const RULE_ID: u32 = 64;
const MAKA_PROMOTION: bool = cfg!(any(feature = "maka-dai-dai", feature = "taishogi"));
const CAPTURE_PROMOTION: bool = cfg!(any(
    feature = "dai-dai",
    feature = "maka-dai-dai",
    feature = "taishogi"
));
const BOARD_SIZE: usize = Geometry::HEIGHT;
const MAX_HOPS: usize = if cfg!(feature = "taikyoku") { 4 } else { 3 };
const ORDERED_EFFECTS: bool = cfg!(feature = "tenjiku-dai");
#[cfg(feature = "tenjiku-dai")]
type Geometry = Rect<16, 16>;
#[cfg(feature = "dai-dai")]
type Geometry = Rect<17, 17>;
#[cfg(feature = "maka-dai-dai")]
type Geometry = Rect<19, 19>;
#[cfg(feature = "taishogi")]
type Geometry = Rect<25, 25>;
#[cfg(feature = "taikyoku")]
type Geometry = Rect<36, 36>;
type Pos = Position<LargeRule>;
mod delta;
mod effects;
use delta::SquareChanges;
mod hash;
mod selected;
use effects::{EffectContext, settle_residents};
use hash::TimelineEntry;
use selected::target_one;
pub type TurnPath = BoundedPath<MAX_HOPS>;
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
pub struct LargeRule;
#[cfg(feature = "tenjiku-dai")]
pub type TenjikuDai = LargeRule;
#[cfg(feature = "dai-dai")]
pub type DaiDai = LargeRule;
#[derive(Clone, Copy)]
struct Requirement {
    delta: (isize, isize),
    kind: u8,
}
#[derive(Clone, Copy)]
struct Primitive {
    delta: (isize, isize),
    kind: u8,
    limit: u8,
    skip: u8,
    next: i8,
    occupied: &'static [Requirement],
    #[cfg(feature = "taikyoku")]
    empty: &'static [(isize, isize)],
    #[cfg(feature = "taikyoku")]
    jump_limit: u8,
}
struct Movement {
    root: &'static [Primitive],
    states: &'static [&'static [Primitive]],
    exceptions: &'static [&'static str],
    stop_capture: bool,
    no_stop: bool,
    file_limit: usize,
}
#[cfg(feature = "tenjiku-dai")]
include!("large/tenjiku_defs.rs");
#[cfg(feature = "dai-dai")]
include!("large/dai_dai_defs.rs");
#[cfg(feature = "maka-dai-dai")]
include!("large/maka_dai_dai_defs.rs");
#[cfg(feature = "taishogi")]
include!("large/taishogi_defs.rs");
#[cfg(feature = "taikyoku")]
include!("large/taikyoku_defs.rs");
impl CaptureRoyalRule for LargeRule {
    type Geometry = Geometry;
    type State = TenjikuState;
    type Hands = NoHands;
    type Placement = ClassicalPiece;
    const ID: u32 = RULE_ID;
    const CACHE_ROYALS: bool = cfg!(feature = "taikyoku");
    const ALLOW_NO_ROYALS: bool = RULE_ID != 60;
    const CONVERTS_INVENTORY: bool = MAKA_PROMOTION;
    fn origin(id: PieceId) -> Option<&'static str> {
        ORIGINS[id.0 as usize]
    }
    const PIECES: &'static [PieceDef] = PIECES;
    const INVENTORY: &'static [usize] = INVENTORY;
    const PROMOTION_RANKS: usize = if CAPTURE_PROMOTION {
        0
    } else if cfg!(feature = "taikyoku") {
        11
    } else {
        5
    };
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
            schema_version: SCHEMA_VERSION,
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
    pub path: BoundedPath<{ MAX_HOPS - 1 }>,
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
    squares: SquareChanges,
    state: TenjikuState,
    side: u8,
}
impl Delta {
    fn new(p: &Pos) -> Self {
        Self {
            squares: SquareChanges::default(),
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
        for &(i, c) in self.squares.iter() {
            p.set_cell(i, c);
        }
        p.state = self.state;
        p.side = self.side;
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Target {
    to: usize,
    through: bool,
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
    if !ORDERED_EFFECTS {
        return vec![];
    }
    active_order(order(p, &p.state.shitenno, ignore))
}
fn active_order(residents: impl IntoIterator<Item = Resident>) -> Vec<Resident> {
    let mut out: Vec<Resident> = vec![];
    for r in residents {
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
#[cfg(test)]
fn fires(p: &Pos) -> Vec<Resident> {
    if !ORDERED_EFFECTS {
        return vec![];
    }
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
    if !ORDERED_EFFECTS {
        return if p.state.shitenno.is_empty() {
            Ok(())
        } else {
            Err("this ruleset has no ordered resident effects".into())
        };
    }
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
    targets_to(p, from, status, ignore, None)
}
fn targets_to(
    p: &Pos,
    from: usize,
    status: Option<u8>,
    ignore: Option<usize>,
    wanted: Option<usize>,
) -> Vec<Target> {
    targets_with_active(p, from, status, ignore, wanted, None)
}
fn targets_with_active(
    p: &Pos,
    from: usize,
    status: Option<u8>,
    ignore: Option<usize>,
    wanted: Option<usize>,
    cached_active: Option<&[Resident]>,
) -> Vec<Target> {
    let result = match wanted {
        Some(to) => target_one(p, from, status, ignore, to, cached_active)
            .into_iter()
            .collect(),
        None => targets_selected(p, from, status, ignore, None, cached_active),
    };
    #[cfg(test)]
    if let Some(to) = wanted {
        let expected: Vec<_> =
            known_target_tests::reference(p, from, status, ignore, wanted, cached_active)
                .into_iter()
                .filter(|t| t.to == to)
                .collect();
        assert_eq!(
            result, expected,
            "known destination and continuation metadata"
        );
    }
    result
}
fn targets_selected(
    p: &Pos,
    from: usize,
    status: Option<u8>,
    ignore: Option<usize>,
    wanted: Option<usize>,
    cached_active: Option<&[Resident]>,
) -> Vec<Target> {
    let Some(c) = at_cell(p, from, ignore) else {
        return vec![];
    };
    if MAKA_PROMOTION && Pos::def(c).id == "ix" {
        if let Some(to) = wanted {
            if at_cell(p, to, ignore).is_none() {
                return vec![Target {
                    to,
                    next: None,
                    through: false,
                }];
            }
            if at_cell(p, to, ignore).is_some_and(|v| v.color == c.color) {
                return vec![];
            }
            let defended = p.cells.iter().enumerate().any(|(i, v)| {
                Some(i) != ignore
                    && v.is_some_and(|v| v.color != c.color && Pos::def(v).id != "ix")
                    && targets(p, i, None, ignore).iter().any(|t| t.to == to)
            });
            return if defended {
                vec![]
            } else {
                vec![Target {
                    to,
                    next: None,
                    through: false,
                }]
            };
        }
        let mut defended = BTreeSet::new();
        for (i, v) in p.cells.iter().enumerate() {
            if Some(i) != ignore && v.is_some_and(|v| v.color != c.color && Pos::def(v).id != "ix")
            {
                defended.extend(targets(p, i, None, ignore).into_iter().map(|t| t.to));
            }
        }
        return (0..Geometry::SQUARES)
            .filter(|&i| at_cell(p, i, ignore).is_none() || !defended.contains(&i))
            .map(|to| Target {
                to,
                next: None,
                through: false,
            })
            .collect();
    }
    let m = &MOVEMENT[c.piece.0 as usize];
    if status.is_none() {
        let computed;
        let actives = match cached_active {
            Some(a) => a,
            None => {
                computed = active(p, ignore);
                &computed
            }
        };
        if frozen(from, c.color, actives) {
            return vec![];
        }
    }
    let prims = if let Some(s) = status {
        m.states.get(s as usize).copied().unwrap_or(&[])
    } else {
        m.root
    };
    let mut raw: Vec<(usize, i8, bool)> = vec![];
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
        let mut blocked = false;
        #[cfg(feature = "taikyoku")]
        for &(ex, ey) in prim.empty {
            if Geometry::offset(
                from,
                dx + (if dx > 0 { 1 } else { -1 }) * ex,
                dy + (if dy > 0 { 1 } else { -1 }) * ey,
            )
            .is_some_and(|i| at_cell(p, i, ignore).is_some())
            {
                blocked = true;
                break;
            }
        }
        for r in prim.occupied {
            if let Some(i) = Geometry::offset(
                from,
                dx + (if dx > 0 { 1 } else { -1 }) * r.delta.0,
                dy + (if dy > 0 { 1 } else { -1 }) * r.delta.1,
            ) {
                if !at_cell(p, i, ignore)
                    .is_some_and(|v| r.kind == 3 || (r.kind == 1) == (v.color == c.color))
                {
                    blocked = true;
                    break;
                }
            }
        }
        if blocked {
            if wanted.is_none_or(|w| w == now) {
                prohibited.insert(now);
            }
            continue;
        }
        let ray = matches!(prim.kind, 2 | 6 | 100 | 102) || prim.limit > 0;
        let max = if prim.limit > 0 {
            usize::from(prim.limit)
        } else if ray {
            Geometry::SQUARES
        } else {
            1
        };
        #[cfg(feature = "taikyoku")]
        let mut jumps = 0;
        for _ in 0..max {
            #[cfg(feature = "taikyoku")]
            if jumps > prim.jump_limit {
                break;
            }
            let v = at_cell(p, now, ignore);
            if v.is_some_and(|v| m.exceptions.contains(&Pos::def(v).id)) {
                break;
            }
            if wanted.is_none_or(|w| w == now) {
                if let Some(t) = raw.iter_mut().find(|t| t.0 == now) {
                    t.2 |= prim.kind == 100;
                    if t.1 == -2 && prim.next != -2 {
                        t.1 = prim.next;
                    }
                } else {
                    raw.push((now, prim.next, prim.kind == 100));
                }
            }
            if prim.kind == 102 && v.is_none() {
                for (tx, ty) in [(-dy, dx), (dy, -dx)] {
                    let mut turn = now;
                    while let Some(j) = Geometry::offset(turn, tx, ty) {
                        if wanted.is_none_or(|w| w == j) && !raw.iter().any(|r| r.0 == j) {
                            raw.push((j, prim.next, false));
                        }
                        if at_cell(p, j, ignore).is_some() {
                            break;
                        }
                        turn = j;
                    }
                }
            }
            #[cfg(feature = "taikyoku")]
            if v.is_some() {
                jumps += 1;
            }
            if !ray || (!matches!(prim.kind, 6 | 100) && v.is_some()) {
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
                        if wanted.is_none_or(|w| w == to)
                            && !adjacent(from, to)
                            && !raw.iter().any(|r| r.0 == to)
                        {
                            raw.push((to, -2, false));
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
        .filter(|(to, _, _)| !prohibited.contains(to))
        .map(|(to, n, through)| Target {
            to,
            through,
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
            if Pos::def(v).id != "aa"
                && !((MAKA_PROMOTION || cfg!(feature = "taikyoku")) && Pos::def(v).id == "cq")
            {
                return true;
            }
            return !protected(original, to, mover.color, source);
        }
        return !protected(p, to, mover.color, from);
    }
    adjacent(from, to) || !protected(p, to, mover.color, from)
}
fn through_squares(from: usize, to: usize) -> Vec<usize> {
    let (x, y) = Geometry::coordinates(from).unwrap();
    let (u, v) = Geometry::coordinates(to).unwrap();
    let (dx, dy) = (
        (u as isize - x as isize).signum(),
        (v as isize - y as isize).signum(),
    );
    let mut out = vec![];
    let mut now = from;
    while let Some(i) = Geometry::offset(now, dx, dy) {
        if i == to {
            break;
        }
        out.push(i);
        now = i;
    }
    out
}
fn hop(p: &mut Pos, from: usize, to: usize, through: bool, d: &mut Delta) {
    let mover = p.cells[from].unwrap();
    p.state.senjishi = p.cells[to].is_some_and(lion) && !lion(mover);
    if through {
        for i in through_squares(from, to) {
            d.set(p, i, None);
        }
    }
    d.set(p, from, None);
    d.set(p, to, Some(mover));
}
fn finish(p: &mut Pos, to: usize, effects: &EffectContext, d: &mut Delta) {
    if !ORDERED_EFFECTS {
        return;
    }
    #[cfg(test)]
    let expected = {
        let mut copy = p.clone();
        let mut delta = Delta::new(&copy);
        finish_reference(&mut copy, to, &effects.resident_fires, &mut delta);
        copy
    };
    if let Some(moved) = p.cells[to] {
        if effects.resident_fires.iter().any(|r| {
            r.color != moved.color
                && adjacent(Pos::index(r.at).unwrap(), to)
                && p.cells[Pos::index(r.at).unwrap()].is_some_and(fire)
        }) {
            d.set(p, to, None);
        }
    }
    settle_residents(p, to);
    let actives = active_order(p.state.shitenno.iter().cloned());
    let mut burnt = BTreeSet::new();
    // Only the moving piece can introduce a new fire at a new square.
    for i in effects
        .fire_squares
        .iter()
        .copied()
        .chain((!effects.fire_squares.contains(&to)).then_some(to))
    {
        let Some(c) = p.cells[i] else {
            continue;
        };
        if !fire(c) || frozen(i, c.color, &actives) {
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
    settle_residents(p, to);
    #[cfg(test)]
    assert_eq!(*p, expected, "shared effects must equal full-board effects");
}

#[cfg(test)]
fn finish_reference(p: &mut Pos, to: usize, resident_fires: &[Resident], d: &mut Delta) {
    if !ORDERED_EFFECTS {
        return;
    }
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
    p.cells[x * BOARD_SIZE..(x + 1) * BOARD_SIZE]
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
fn next_promotion(p: &Pos, from: usize, to: usize, prior: Option<PieceId>) -> Option<PieceId> {
    let c = p.cells[from].unwrap();
    let def = Pos::def(c);
    let victim = p.cells[to];
    if MAKA_PROMOTION {
        let special = victim.and_then(|v| match Pos::def(v).id {
            "ih" | "iz" => Some("iz"),
            "ii" | "jb" => Some("jb"),
            _ => None,
        });
        if def.royal && special.is_some() {
            return None;
        }
        let royal_promotion = def.promotion.is_some_and(|id| PIECES[id.0 as usize].royal);
        if !def.royal && !royal_promotion {
            if let Some(id) = special {
                return Some(PieceId(
                    PIECES.iter().position(|p| p.id == id).unwrap() as u16
                ));
            }
        }
    }
    prior.or_else(|| {
        def.promotion.filter(|_| {
            if CAPTURE_PROMOTION {
                victim.is_some()
            } else {
                p.relative_rank(from, c.color) < LargeRule::PROMOTION_RANKS
                    || p.relative_rank(to, c.color) < LargeRule::PROMOTION_RANKS
            }
        })
    })
}
struct Enumeration<'a> {
    original: &'a Pos,
    effects: EffectContext,
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
        target: Option<PieceId>,
        early: bool,
    ) -> Option<bool> {
        let to = Pos::index(*path.last().unwrap()).unwrap();
        let c = p.cells[to].unwrap();
        let must = target.is_some()
            && !early
            && (CAPTURE_PROMOTION || p.relative_rank(to, c.color) < Pos::def(c).dead_ranks);
        for promotion in [false, true] {
            if promotion && (target.is_none() || early) || !promotion && must {
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
                c.piece = target.unwrap();
                undo.set(p, to, Some(c));
            }
            if !early {
                finish(p, to, &self.effects, &mut undo);
            } else {
                settle_residents(p, to);
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
        target: Option<PieceId>,
        first: Option<(usize, Option<Cell>)>,
    ) -> Option<bool> {
        let wanted = self
            .wanted
            .and_then(|w| w.path.get(path.len()).copied())
            .and_then(|at| Pos::index(at).ok());
        for t in targets_with_active(p, from, status, None, wanted, Some(&self.effects.actives)) {
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
            let eligible = next_promotion(p, from, t.to, target);
            let mut undo = Delta::new(p);
            hop(p, from, t.to, t.through, &mut undo);
            path.push(Pos::at(t.to));
            let early = t.next.is_some() && capture_outcome(p) != Outcome::ONGOING;
            let can_stop = early || t.next.is_none() || !MOVEMENT[c.piece.0 as usize].no_stop;
            let mut result = Some(false);
            if can_stop {
                result = self.emit(p, root, path, eligible, early);
            }
            if result == Some(false) && !early && t.next.is_some() && path.len() < MAX_HOPS {
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
        effects: EffectContext::new(p),
        pending,
        wanted,
        stop,
        limit,
        out: vec![],
    };
    let mut occupied = p.occupied[p.side as usize];
    while let Some(from) = occupied.pop_first() {
        if wanted.is_some_and(|m| m.from != Pos::at(from))
            || pending.is_some_and(|m| m.from != Pos::at(from))
        {
            continue;
        }
        if e.visit(&mut scratch, from, from, None, &mut vec![], None, None)? {
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
    timeline: Vec<TimelineEntry>,
    pending: Option<Pending>,
    pending_ply: usize,
}
impl LargeGame {
    pub fn root(p: Pos) -> Result<Self, String> {
        validate_state(&p)?;
        Ok(Self {
            initial: p.to_wire(),
            timeline: vec![TimelineEntry::root(&p)],
            position: p,
            played: vec![],
            undo: vec![],
            pending: None,
            pending_ply: 0,
        })
    }
    pub fn parse(json: &str) -> Result<Self, String> {
        let q: Request = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if q.schema_version != SCHEMA_VERSION || q.rule_id != RULE_ID {
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
                let t = targets_to(&p, from, status, None, Some(to))
                    .into_iter()
                    .find(|t| t.to == to)
                    .ok_or("invalid pending hop")?;
                let mut d = Delta::new(&p);
                hop(&mut p, from, to, t.through, &mut d);
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
            schema_version: SCHEMA_VERSION,
            rule_id: RULE_ID,
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
        let h = self.timeline.last().unwrap().key;
        let matches: Vec<_> = self
            .timeline
            .iter()
            .enumerate()
            .filter(|(_, p)| p.key == h && p.side == self.position.side)
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
        if m.schema_version != SCHEMA_VERSION
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
        let effects = EffectContext::new(&self.position);
        let mut delta = Delta::new(&self.position);
        let mut from = Pos::index(m.from).unwrap();
        let mut status = None;
        let mut target = None;
        for &at in m.path.iter() {
            let to = Pos::index(at).unwrap();
            let t = target_one(
                &self.position,
                from,
                status,
                None,
                to,
                Some(&effects.actives),
            )
            .unwrap();
            status = t.next;
            target = next_promotion(&self.position, from, to, target);
            hop(&mut self.position, from, to, t.through, &mut delta);
            from = to;
        }
        let early = status.is_some() && capture_outcome(&self.position) != Outcome::ONGOING;
        if m.promotion {
            let mut c = self.position.cells[from].unwrap();
            c.piece = target.unwrap();
            delta.set(&mut self.position, from, Some(c));
        }
        if !early {
            finish(&mut self.position, from, &effects, &mut delta);
        } else {
            settle_residents(&mut self.position, from);
        }
        self.position.side ^= 1;
        let previous = *self.timeline.last().unwrap();
        let key = hash::updated(previous.key, &self.position, &delta);
        self.timeline
            .push(TimelineEntry::next(previous, key, self.position.side));
        self.undo.push(delta);
        self.played.push(m);
        #[cfg(test)]
        hash_tests::assert_incremental(self);
    }
    fn pop(&mut self) {
        self.undo.pop().unwrap().restore(&mut self.position);
        self.played.pop();
        self.timeline.pop();
        #[cfg(test)]
        hash_tests::assert_incremental(self);
    }
    pub fn position_hash(&self) -> u64 {
        self.with_pending_hash(self.position.hash())
    }
    fn with_pending_hash(&self, mut h: u64) -> u64 {
        if let Some(p) = self.prefix() {
            for n in p.from.into_iter().chain(p.path.iter().flatten().copied()) {
                h = h.rotate_left(7) ^ u64::from(n).wrapping_mul(0x9e3779b97f4a7c15);
            }
        }
        h
    }
    /// Public FNV context stays byte-for-byte compatible. Reconstruct its
    /// historical FNV values only at this API boundary, never in search.
    fn context_hash(&self) -> u64 {
        let current = self.position.hash();
        let mut hashes = Vec::with_capacity(self.timeline.len());
        hashes.push(current);
        if !self.undo.is_empty() {
            let mut p = self.position.clone();
            for delta in self.undo.iter().rev() {
                delta.clone().restore(&mut p);
                hashes.push(p.hash());
            }
        }
        hashes
            .iter()
            .rev()
            .fold(hash::HISTORY_SEED, |h, &key| hash::history(h, key))
            ^ self.with_pending_hash(current).rotate_left(13)
    }
    fn context_key(&self) -> u64 {
        let current = self.timeline.last().unwrap();
        current.context ^ self.with_pending_hash(current.key).rotate_left(13)
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
    const SCHEMA: u8 = SCHEMA_VERSION;
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
        if m.path.len() == 1 {
            let (x, y) = Geometry::coordinates(from).unwrap();
            let (u, v) = Geometry::coordinates(end).unwrap();
            let sign = if c.color == 0 { 1 } else { -1 };
            let dir = (
                ((u as isize - x as isize).signum()) * sign,
                ((v as isize - y as isize).signum()) * sign,
            );
            if MOVEMENT[c.piece.0 as usize]
                .root
                .iter()
                .any(|p| p.kind == 100 && p.delta == dir)
            {
                for i in through_squares(from, end) {
                    if seen.insert(i) {
                        if let Some(v) = p.cells[i] {
                            value += if v.color == c.color {
                                -Pos::def(v).value
                            } else {
                                Pos::def(v).value
                            };
                        }
                    }
                }
            }
        }
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
                pieces: LargeRule::initial(),
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
        Ok(serde_json::json!({"schemaVersion":SCHEMA_VERSION,"positionHash":format!("{:016x}",g.position_hash()),"contextHash":format!("{:016x}",g.context_hash()),"outcome":g.outcome()}).to_string())
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
#[cfg(all(test, feature = "dai-dai"))]
#[path = "large/dai_dai_tests.rs"]
mod dai_dai_tests;
#[cfg(all(test, feature = "tenjiku-dai"))]
mod tests;

#[cfg(all(test, feature = "maka-dai-dai"))]
#[path = "large/maka_dai_dai_tests.rs"]
mod maka_dai_dai_tests;

#[cfg(all(test, feature = "taishogi"))]
#[path = "large/taishogi_tests.rs"]
mod taishogi_tests;

#[cfg(all(test, feature = "taikyoku"))]
#[path = "large/taikyoku_tests.rs"]
mod taikyoku_tests;

#[cfg(test)]
#[path = "large/hash_tests.rs"]
mod hash_tests;

#[cfg(test)]
#[path = "large/known_target_tests.rs"]
mod known_target_tests;
#[cfg(test)]
mod royal_cache_tests {
    use super::*;
    #[test]
    fn royal_counts_follow_every_piece_replacement_and_restore() {
        let mut p = LargeGame::parse(&compiled::start_position_json())
            .unwrap()
            .position;
        for i in 0..Geometry::SQUARES {
            p.set_cell(i, None);
        }
        assert_eq!((p.royals(0), p.royals(1)), (0, 0));
        for color in 0..2 {
            for id in 0..PIECES.len() {
                let c = Cell {
                    piece: PieceId(id as u16),
                    color,
                };
                p.set_cell(0, Some(c));
                p.set_cell(0, Some(c));
                assert_eq!(p.royals(color), usize::from(PIECES[id].royal));
                assert_eq!(p.royals(color ^ 1), 0);
                let before = p.clone();
                let mut d = Delta::new(&p);
                d.set(&mut p, 0, None);
                assert_eq!(p.royals(color), 0);
                d.restore(&mut p);
                assert_eq!(p, before);
                p.set_cell(0, None);
            }
        }
    }
}

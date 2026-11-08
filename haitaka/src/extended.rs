//! Rule-local pieces and capture-royal games. The ordinary 14-piece Board is unchanged.
//! Wire coordinates are one-based, file-major; no invented SFEN/USI piece letters.
use haitaka_types::fixed_bitboard::SquareSet;
use haitaka_types::geometry::{BoardGeometry, Rect};
use instant::Instant;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, marker::PhantomData};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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

/// Rule-local state that contributes to repetition and search identity.
pub trait PositionState:
    Clone + Default + core::fmt::Debug + PartialEq + Eq + Serialize + for<'de> Deserialize<'de>
{
    fn hash_bytes(&self, add: &mut impl FnMut(u8));
    fn senjishi(&self) -> bool {
        false
    }
    fn set_senjishi(&mut self, value: bool) {
        debug_assert!(!value);
    }
}
impl PositionState for RuleState {
    fn hash_bytes(&self, _add: &mut impl FnMut(u8)) {}
}
/// Rule-local reserve storage; discard games carry no hand allocation.
pub trait Reserve: Clone + Default + core::fmt::Debug + PartialEq + Eq {
    const ENABLED: bool;
    fn count(&self, color: u8, piece: PieceId) -> u16;
    fn set(&mut self, color: u8, piece: PieceId, count: u16);
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoHands;
impl Reserve for NoHands {
    const ENABLED: bool = false;
    fn count(&self, _color: u8, _piece: PieceId) -> u16 {
        0
    }
    fn set(&mut self, _color: u8, _piece: PieceId, count: u16) {
        assert_eq!(count, 0);
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hands<const N: usize>([[u16; N]; 2]);
impl<const N: usize> Default for Hands<N> {
    fn default() -> Self {
        Self([[0; N]; 2])
    }
}
impl<const N: usize> Reserve for Hands<N> {
    const ENABLED: bool = true;
    fn count(&self, color: u8, piece: PieceId) -> u16 {
        self.0[color as usize]
            .get(piece.0 as usize)
            .copied()
            .unwrap_or(0)
    }
    fn set(&mut self, color: u8, piece: PieceId, count: u16) {
        self.0[color as usize][piece.0 as usize] = count;
    }
}
pub trait Placement:
    Clone + core::fmt::Debug + PartialEq + Eq + Serialize + for<'de> Deserialize<'de>
{
    fn at(&self) -> [u16; 2];
    fn piece(&self) -> &str;
    fn color(&self) -> u8;
    fn origin(&self) -> Option<&str>;
    fn make(at: [u16; 2], piece: &str, color: u8, origin: Option<&str>) -> Self;
}
impl Placement for PlacedPiece {
    fn at(&self) -> [u16; 2] {
        self.at
    }
    fn piece(&self) -> &str {
        &self.piece
    }
    fn color(&self) -> u8 {
        self.color
    }
    fn origin(&self) -> Option<&str> {
        None
    }
    fn make(at: [u16; 2], piece: &str, color: u8, origin: Option<&str>) -> Self {
        assert!(origin.is_none());
        Self {
            at,
            piece: piece.into(),
            color,
        }
    }
}
pub trait CaptureRoyalRule: Clone + PartialEq + Eq + core::fmt::Debug {
    type Geometry: BoardGeometry + Clone + core::fmt::Debug + PartialEq + Eq;
    type State: PositionState;
    type Hands: Reserve;
    type Placement: Placement;
    fn origin(_piece: PieceId) -> Option<&'static str> {
        None
    }
    fn capture_return(_piece: PieceId) -> Option<PieceId> {
        None
    }
    fn file_limit(_piece: PieceId) -> usize {
        0
    }
    fn ray_limit(_piece: PieceId, _dx: isize, _dy: isize) -> Option<usize> {
        None
    }
    const ALLOW_NO_ROYALS: bool = false;
    const ID: u32;
    const PIECES: &'static [PieceDef];
    const INVENTORY: &'static [usize];
    const PROMOTION_RANKS: usize;
    fn initial() -> Vec<Self::Placement>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shoshogi;
impl CaptureRoyalRule for Shoshogi {
    type Geometry = Rect<9, 9>;
    type State = RuleState;
    type Hands = NoHands;
    type Placement = PlacedPiece;
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
pub struct WirePosition<S = RuleState, P = PlacedPiece> {
    pub side_to_move: u8,
    pub pieces: Vec<P>,
    pub hands: [BTreeMap<String, u16>; 2],
    pub state: S,
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
    state: R::State,
    hands: R::Hands,
    occupied: [<R::Geometry as BoardGeometry>::Bits; 2],
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
    pub fn from_wire(wire: &WirePosition<R::State, R::Placement>) -> Result<Self, String> {
        if wire.side_to_move > 1 {
            return Err("sideToMove must be 0 or 1".into());
        }
        if !R::Hands::ENABLED && wire.hands.iter().any(|h| !h.is_empty()) {
            return Err("this rule has no hands; hands must be [{},{}]".into());
        }
        let mut pos = Self {
            cells: vec![None; R::Geometry::SQUARES],
            side: wire.side_to_move,
            state: wire.state.clone(),
            hands: R::Hands::default(),
            occupied: [<R::Geometry as BoardGeometry>::Bits::EMPTY; 2],
            rule: PhantomData,
        };
        let mut count = vec![vec![0; R::INVENTORY.len()]; 2];
        for p in &wire.pieces {
            let (at, piece, color, origin) = (p.at(), p.piece(), p.color(), p.origin());
            if color > 1 {
                return Err("piece color must be 0 or 1".into());
            }
            let id = R::PIECES
                .iter()
                .enumerate()
                .position(|(i, d)| d.id == piece && R::origin(PieceId(i as u16)) == origin)
                .ok_or_else(|| format!("unknown piece ID or origin: {piece} {origin:?}"))?;
            let i = Self::index(at)?;
            if pos.cells[i].is_some() {
                return Err("duplicate square".into());
            }
            let def = &R::PIECES[id];
            if pos.relative_rank(i, color) < def.dead_ranks {
                return Err("unpromoted piece on a dead rank".into());
            }
            count[color as usize][def.base] += 1;
            pos.set_cell(
                i,
                Some(Cell {
                    piece: PieceId(id as u16),
                    color,
                }),
            );
        }
        for color in 0..2u8 {
            for (species, &amount) in &wire.hands[color as usize] {
                let id = R::PIECES
                    .iter()
                    .enumerate()
                    .find(|(i, d)| {
                        d.id == species
                            && R::origin(PieceId(*i as u16)).is_none()
                            && R::capture_return(PieceId(*i as u16)) == Some(PieceId(*i as u16))
                    })
                    .map(|(i, _)| PieceId(i as u16))
                    .ok_or_else(|| format!("unknown or non-droppable hand piece: {species}"))?;
                pos.hands.set(color, id, amount);
                count[color as usize][R::PIECES[id.0 as usize].base] += usize::from(amount);
            }
        }
        for (base, &initial) in R::INVENTORY.iter().enumerate() {
            let exceeded = if R::Hands::ENABLED {
                count[0][base] + count[1][base] > initial * 2
            } else {
                count.iter().any(|c| c[base] > initial)
            };
            if exceeded {
                return Err(format!("piece inventory exceeded for base {base}"));
            }
        }
        if !R::ALLOW_NO_ROYALS && pos.royals(0) == 0 && pos.royals(1) == 0 {
            return Err("both sides lack every royal".into());
        }
        Ok(pos)
    }
    pub fn to_wire(&self) -> WirePosition<R::State, R::Placement> {
        WirePosition {
            side_to_move: self.side,
            pieces: self
                .cells
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    c.map(|c| {
                        R::Placement::make(
                            Self::at(i),
                            Self::def(c).id,
                            c.color,
                            R::origin(c.piece),
                        )
                    })
                })
                .collect(),
            hands: std::array::from_fn(|color| {
                R::PIECES
                    .iter()
                    .enumerate()
                    .filter_map(|(id, def)| {
                        let count = self.hands.count(color as u8, PieceId(id as u16));
                        (count > 0).then(|| (def.id.to_string(), count))
                    })
                    .collect()
            }),
            state: self.state.clone(),
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
        self.state.hash_bytes(&mut add);
        for c in &self.cells {
            match c {
                None => add(0),
                Some(c) => {
                    add(c.color + 1);
                    for b in Self::def(*c).id.bytes() {
                        add(b);
                    }
                    if let Some(origin) = R::origin(c.piece) {
                        add(254);
                        for b in origin.bytes() {
                            add(b);
                        }
                    }
                    add(0);
                }
            }
        }
        if R::Hands::ENABLED {
            add(255);
            for color in 0..2u8 {
                for (id, def) in R::PIECES.iter().enumerate() {
                    let count = self.hands.count(color, PieceId(id as u16));
                    if count > 0 {
                        add(color + 1);
                        for b in def.id.bytes() {
                            add(b);
                        }
                        add(0);
                        for b in count.to_le_bytes() {
                            add(b);
                        }
                    }
                }
            }
        }
        hash
    }
    fn targets(&self, from: usize, cell: Cell) -> Vec<usize> {
        self.geometric_targets(from, cell, false)
    }
    fn geometric_targets(&self, from: usize, cell: Cell, include_friendly: bool) -> Vec<usize> {
        let sign = if cell.color == 0 { 1 } else { -1 };
        let def = Self::def(cell);
        let mut out = vec![];
        for &(x, y) in def.steps {
            if let Some(to) = R::Geometry::offset(from, x * sign, y * sign)
                && (include_friendly || self.cells[to].is_none_or(|c| c.color != cell.color))
            {
                out.push(to);
            }
        }
        let blockers = self.occupied[0].union(self.occupied[1]);
        for &(x, y) in def.rays {
            if let Some(limit) = R::ray_limit(cell.piece, x, y) {
                let mut current = from;
                for _ in 0..limit {
                    let Some(to) = R::Geometry::offset(current, x * sign, y * sign) else {
                        break;
                    };
                    if include_friendly || self.cells[to].is_none_or(|c| c.color != cell.color) {
                        out.push(to);
                    }
                    if blockers.contains(to) {
                        break;
                    }
                    current = to;
                }
                continue;
            }
            let mut ray = R::Geometry::ray(
                from,
                x * sign,
                y * sign,
                blockers,
                <R::Geometry as BoardGeometry>::Bits::FULL,
            );
            if !include_friendly {
                ray = ray.intersection(self.occupied[cell.color as usize].complement());
            }
            while let Some(to) = ray.first() {
                out.push(to);
                ray = ray.without(to);
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
    pub fn moves(&self) -> Vec<JsonMove>
    where
        R: CaptureRoyalRule<State = RuleState, Placement = PlacedPiece>,
    {
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
    fn set_cell(&mut self, square: usize, cell: Option<Cell>) {
        if let Some(old) = self.cells[square] {
            self.occupied[old.color as usize] = self.occupied[old.color as usize].without(square);
        }
        self.cells[square] = cell;
        if let Some(new) = cell {
            self.occupied[new.color as usize] = self.occupied[new.color as usize].with(square);
        }
    }
    fn play_unchecked(&self, mv: JsonMove) -> Self
    where
        R: CaptureRoyalRule<State = RuleState, Placement = PlacedPiece>,
    {
        let mut next = self.clone();
        let from = Self::index(mv.from).unwrap();
        let to = Self::index(mv.to).unwrap();
        let mut cell = next.cells[from].unwrap();
        next.set_cell(from, None);
        if mv.promotion {
            cell.piece = Self::def(cell).promotion.unwrap();
        }
        next.set_cell(to, Some(cell));
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
        let reserve: i32 = (0..2u8)
            .map(|color| {
                R::PIECES
                    .iter()
                    .enumerate()
                    .map(|(id, d)| {
                        i32::from(self.hands.count(color, PieceId(id as u16)))
                            * d.value
                            * if color == self.side { 1 } else { -1 }
                    })
                    .sum::<i32>()
            })
            .sum();
        (sum + reserve).clamp(-MATE + 1000, MATE - 1000)
    }
}

#[derive(Clone)]
pub struct Game<R: CaptureRoyalRule> {
    pub history: Vec<Position<R>>,
}
impl<R: CaptureRoyalRule<State = RuleState, Placement = PlacedPiece>> Game<R> {
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
    pub fn position_hash(&self) -> u64 {
        self.current().hash()
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

pub mod path;
mod search;
pub use search::SearchResult;
pub fn search_json(
    json: &str,
    max_depth: u8,
    timeout_ms: u32,
    node_budget: u32,
) -> Result<String, String> {
    let mut game = ShoGame::parse(json)?;
    search::search_json(&mut game, max_depth, timeout_ms, node_budget)
}
#[cfg(feature = "chushogi")]
pub mod chu;
#[cfg(feature = "shoshogi")]
pub mod compiled {
    pub use super::{
        PROTOCOL, ShoGame as CompiledGame, apply_move_json, inspect_json, legal_moves_json,
        perft_json, search_json, start_position_json, validate_position_json,
    };
    pub const NAME: &str = "shoshogi";
    pub const RULE_ID: u32 = 43;
}
#[cfg(feature = "chushogi")]
pub use chu as compiled;
#[cfg(test)]
mod tests;

#[cfg(feature = "classical-rules")]
pub mod classical;
#[cfg(any(
    feature = "torishogi",
    feature = "washogi",
    feature = "heian-dai",
    feature = "daishogi"
))]
pub use classical::compiled;

#[cfg(any(feature = "tenjiku-dai", feature = "dai-dai"))]
pub mod large;
#[cfg(any(feature = "tenjiku-dai", feature = "dai-dai"))]
pub use large::compiled;

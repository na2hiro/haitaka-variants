//! Stage-4 rules, sharing board geometry, completed-turn search and JSON v3.
use super::path::{LionState, Path, Prefix};
use super::search::SearchGame;
use super::*;
use serde::{Deserializer, Serializer};

pub const PROTOCOL: &str = "haitaka-json-v3";
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassicalState {}
impl PositionState for ClassicalState {
    fn hash_bytes(&self, _add: &mut impl FnMut(u8)) {}
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClassicalPiece {
    pub at: [u16; 2],
    pub piece: String,
    pub color: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}
impl Placement for ClassicalPiece {
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
        self.origin.as_deref()
    }
    fn make(at: [u16; 2], piece: &str, color: u8, origin: Option<&str>) -> Self {
        Self {
            at,
            piece: piece.into(),
            color,
            origin: origin.map(str::to_string),
        }
    }
}
struct Meta {
    origin: Option<&'static str>,
    capture_return: Option<PieceId>,
    file_limit: usize,
    limits: &'static [(isize, isize, usize)],
}
pub trait ClassicalRule: CaptureRoyalRule<Placement = ClassicalPiece> + Copy + Ord {
    const NAME: &'static str;
    const FORCED: bool;
    const CHECK: bool;
    const CHECKMATE: bool = Self::CHECK;
    const STALEMATE: bool;
    const ONLY_KINGS: bool;
    const LION: bool;
    const LION_CAPTURE_RULES: bool = Self::LION;
    const UCHIFU: Option<PieceId>;
    const INITIAL_SIDE: u8 = 0;
    const TRY: bool = false;
    const FORCE_CAPTURE: bool = false;
    /// Canonical physical reserve consumed when either face is dropped.
    fn drop_hand(id: PieceId) -> PieceId {
        id
    }
    fn droppable(id: PieceId) -> bool {
        Self::origin(id).is_none() && Self::capture_return(id) == Some(id)
    }
    /// A promotion flag describes the resulting face change, including a reverse flip.
    fn promotion_options(
        id: PieceId,
        from_rank: usize,
        to_rank: usize,
        _captured: bool,
    ) -> (bool, bool) {
        let def = &Self::PIECES[id.0 as usize];
        let can = def.promotion.is_some()
            && (from_rank < Self::PROMOTION_RANKS || to_rank < Self::PROMOTION_RANKS);
        (can, can && (Self::FORCED || to_rank < def.dead_ranks))
    }

    fn continuation(id: PieceId, delta: (isize, isize)) -> Option<&'static [(isize, isize)]>;
}
macro_rules! classical_rule {
    ($name:ident,$size:literal,$state:ty,$hands:ty,$id:literal,$label:literal,$dan:literal,$forced:literal,$check:literal,$stale:literal,$kings:literal,$lion:literal,$uchi:expr,$pieces:ident,$meta:ident,$inventory:ident,$initial:ident,$follow:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        pub struct $name;
        impl CaptureRoyalRule for $name {
            type Geometry = Rect<$size, $size>;
            type State = $state;
            type Hands = $hands;
            type Placement = ClassicalPiece;
            const ID: u32 = $id;
            const PIECES: &'static [PieceDef] = $pieces;
            const INVENTORY: &'static [usize] = $inventory;
            const PROMOTION_RANKS: usize = $dan;
            fn origin(id: PieceId) -> Option<&'static str> {
                $meta[id.0 as usize].origin
            }
            fn capture_return(id: PieceId) -> Option<PieceId> {
                $meta[id.0 as usize].capture_return
            }
            fn file_limit(id: PieceId) -> usize {
                $meta[id.0 as usize].file_limit
            }
            fn ray_limit(id: PieceId, dx: isize, dy: isize) -> Option<usize> {
                $meta[id.0 as usize]
                    .limits
                    .iter()
                    .find_map(|&(x, y, n)| (x == dx && y == dy).then_some(n))
            }
            fn initial() -> Vec<ClassicalPiece> {
                $initial
                    .iter()
                    .map(|&(at, id, color)| {
                        ClassicalPiece::make(
                            at,
                            $pieces[id as usize].id,
                            color,
                            Self::origin(PieceId(id)),
                        )
                    })
                    .collect()
            }
        }
        impl ClassicalRule for $name {
            const NAME: &'static str = $label;
            const FORCED: bool = $forced;
            const CHECK: bool = $check;
            const STALEMATE: bool = $stale;
            const ONLY_KINGS: bool = $kings;
            const LION: bool = $lion;
            const UCHIFU: Option<PieceId> = $uchi;
            fn continuation(
                id: PieceId,
                delta: (isize, isize),
            ) -> Option<&'static [(isize, isize)]> {
                $follow(id, delta)
            }
        }
    };
}
include!("classical/defs.rs");
#[cfg(any(
    feature = "narazu",
    feature = "nifu-ari",
    feature = "sho-kyo-left",
    feature = "sho-kyo-right",
    feature = "nakodo",
    feature = "queen-king",
    feature = "torazu-gin",
    feature = "ai-torazu-gin",
    feature = "zarikani",
    feature = "zarikani-tonbo",
    feature = "semai",
    feature = "semmai",
    feature = "hiroi",
    feature = "dobutsu",
    feature = "toruichi",
    feature = "hisha",
    feature = "kaku",
    feature = "seireigi",
    feature = "kyo-left",
    feature = "kyo-right",
    feature = "totsugeki",
    feature = "ai-happokei",
    feature = "happokei",
    feature = "patrol",
    feature = "lortap",
    feature = "shishio",
    feature = "hono-shishi",
    feature = "nigyoku"
))]
mod ordinary;
#[cfg(any(
    feature = "narazu",
    feature = "nifu-ari",
    feature = "sho-kyo-left",
    feature = "sho-kyo-right",
    feature = "nakodo",
    feature = "queen-king",
    feature = "torazu-gin",
    feature = "ai-torazu-gin",
    feature = "zarikani",
    feature = "zarikani-tonbo",
    feature = "semai",
    feature = "semmai",
    feature = "hiroi",
    feature = "dobutsu",
    feature = "toruichi",
    feature = "hisha",
    feature = "kaku",
    feature = "seireigi",
    feature = "kyo-left",
    feature = "kyo-right",
    feature = "totsugeki",
    feature = "ai-happokei",
    feature = "happokei",
    feature = "patrol",
    feature = "lortap",
    feature = "shishio",
    feature = "hono-shishi",
    feature = "nigyoku"
))]
use ordinary::*;
#[cfg(any(
    feature = "kyoto",
    feature = "kinkei-mai",
    feature = "microshogi",
    feature = "kyoto-minishogi"
))]
mod small_flip;
#[cfg(any(
    feature = "kyoto",
    feature = "kinkei-mai",
    feature = "microshogi",
    feature = "kyoto-minishogi"
))]
use small_flip::*;
/// Only unpromoted, rule-local reserve IDs can appear in a drop command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DropId<R: ClassicalRule> {
    id: PieceId,
    rule: PhantomData<R>,
}
impl<R: ClassicalRule> Serialize for DropId<R> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(R::PIECES[self.id.0 as usize].id)
    }
}
impl<'de, R: ClassicalRule> Deserialize<'de> for DropId<R> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let id = R::PIECES
            .iter()
            .enumerate()
            .find(|(i, p)| p.id == s && R::droppable(PieceId(*i as u16)))
            .map(|(i, _)| PieceId(i as u16))
            .ok_or_else(|| serde::de::Error::custom("unknown or non-droppable piece ID"))?;
        Ok(Self {
            id,
            rule: PhantomData,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    bound = ""
)]
pub enum ClassicalMove<R: ClassicalRule> {
    Move {
        schema_version: u8,
        from: [u16; 2],
        path: Path,
        promotion: bool,
    },
    Drop {
        schema_version: u8,
        piece: DropId<R>,
        to: [u16; 2],
    },
}
impl<R: ClassicalRule> ClassicalMove<R> {
    pub fn board(from: [u16; 2], path: Path, promotion: bool) -> Self {
        Self::Move {
            schema_version: 3,
            from,
            path,
            promotion,
        }
    }
    pub fn drop(id: PieceId, to: [u16; 2]) -> Self {
        Self::Drop {
            schema_version: 3,
            piece: DropId {
                id,
                rule: PhantomData,
            },
            to,
        }
    }
    fn version(self) -> u8 {
        match self {
            Self::Move { schema_version, .. } | Self::Drop { schema_version, .. } => schema_version,
        }
    }
    fn promotes(self) -> bool {
        matches!(
            self,
            Self::Move {
                promotion: true,
                ..
            }
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, bound = "")]
pub struct History<R: ClassicalRule> {
    pub initial: WirePosition<R::State, ClassicalPiece>,
    pub moves: Vec<ClassicalMove<R>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, bound = "")]
pub struct Request<R: ClassicalRule> {
    pub schema_version: u8,
    pub rule_id: u32,
    pub position: WirePosition<R::State, ClassicalPiece>,
    pub history: Option<History<R>>,
    pub pending: Option<Prefix>,
}
#[cfg(test)]
thread_local! {
    static IN_ESCAPE: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}
/// Reusable validation state of one move generation call.
struct Scratch<R: CaptureRoyalRule> {
    /// A copy of the generating position, restored after each candidate.
    board: Option<Position<R>>,
    /// The royal source square and `classical_royal_map` result.
    royal_map: Option<(usize, <R::Geometry as BoardGeometry>::Bits)>,
    targets: Vec<usize>,
}
/// Whether the side to move has a royal attacked, ignoring capture support,
/// and its pin candidates.
type Safety<R> = Option<(bool, <<R as CaptureRoyalRule>::Geometry as BoardGeometry>::Bits)>;
const fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
/// Static shape of every ray direction in a rule's piece set.
trait RayShape {
    /// Every ray is orthogonal or diagonal.
    const LINE_RAYS: bool;
    /// Every ray advances one lattice step along its line.
    const PRIMITIVE_RAYS: bool;
}
const fn ray_shape(pieces: &[PieceDef]) -> (bool, bool) {
    let (mut line, mut primitive) = (true, true);
    let mut i = 0;
    while i < pieces.len() {
        let rays = pieces[i].rays;
        let mut j = 0;
        while j < rays.len() {
            let (x, y) = (rays[j].0.unsigned_abs(), rays[j].1.unsigned_abs());
            line &= x == 0 || y == 0 || x == y;
            primitive &= gcd(x, y) == 1;
            j += 1;
        }
        i += 1;
    }
    (line, primitive)
}
impl<R: CaptureRoyalRule> RayShape for R {
    const LINE_RAYS: bool = ray_shape(R::PIECES).0;
    const PRIMITIVE_RAYS: bool = ray_shape(R::PIECES).1;
}
fn lion<R: ClassicalRule>(c: Cell) -> bool {
    matches!(R::PIECES[c.piece.0 as usize].id, "bl" | "dh")
}
impl<R: ClassicalRule> Position<R> {
    fn protected_classical_lion(&self, target: usize, capturing: u8, remove: usize) -> bool {
        let mut p = self.clone();
        p.set_cell(remove, None);
        p.cells.iter().enumerate().any(|(i, c)| {
            c.is_some_and(|c| {
                c.color != capturing && p.geometric_targets(i, c, true).contains(&target)
            })
        })
    }
    fn classical_capture_allowed(
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
        if victim.color == mover.color
            || !self.capture_allowed_at(from, to)
        {
            return false;
        }
        if !R::LION_CAPTURE_RULES || !lion::<R>(victim) {
            return true;
        }
        if !lion::<R>(mover) {
            return !self.state.senjishi();
        }
        if let Some((source, victim)) = first {
            if let Some(victim) = victim {
                if !matches!(Self::def(victim).id, "aa" | "cq") {
                    return true;
                }
                return !original.protected_classical_lion(to, mover.color, source);
            }
            return !self.protected_classical_lion(to, mover.color, from);
        }
        let (x, y) = R::Geometry::coordinates(from).unwrap();
        let (u, v) = R::Geometry::coordinates(to).unwrap();
        x.abs_diff(u).max(y.abs_diff(v)) == 1
            || !self.protected_classical_lion(to, mover.color, from)
    }
    fn classical_hop(&mut self, from: usize, to: usize) {
        let mover = self.cells[from].unwrap();
        let victim = self.cells[to];
        if R::LION_CAPTURE_RULES {
            self.state
                .set_senjishi(victim.is_some_and(lion::<R>) && !lion::<R>(mover));
        }
        if R::Hands::ENABLED {
            if let Some(victim) = victim {
                let id = R::captured_reserve(victim.piece, victim.color).expect("reserve capture mapping");
                self.hands.set(
                    mover.color,
                    id,
                    self.hands
                        .count(mover.color, id)
                        .checked_add(1)
                        .expect("validated inventory"),
                );
            }
        }
        self.set_cell(from, None);
        self.set_cell(to, Some(mover));
    }
    fn classical_apply(&self, mv: ClassicalMove<R>) -> Self {
        let mut next = self.clone();
        next.classical_play(mv);
        next
    }
    /// Apply `mv` to this position in place.
    fn classical_play(&mut self, mv: ClassicalMove<R>) {
        let next = self;
        match mv {
            ClassicalMove::Move {
                from,
                path,
                promotion,
                ..
            } => {
                let mut from = Self::index(from).unwrap();
                for &to in path.iter() {
                    let to = Self::index(to).unwrap();
                    next.classical_hop(from, to);
                    from = to;
                }
                if promotion {
                    let mut c = next.cells[from].unwrap();
                    c.piece = Self::def(c).promotion.unwrap();
                    next.set_cell(from, Some(c));
                }
            }
            ClassicalMove::Drop { piece, to, .. } => {
                let side = next.side;
                next.hands.set(
                    side,
                    R::drop_hand(piece.id),
                    next.hands.count(side, R::drop_hand(piece.id)) - 1,
                );
                next.set_cell(
                    Self::index(to).unwrap(),
                    Some(Cell {
                        piece: piece.id,
                        color: side,
                    }),
                );
            }
        }
        next.side ^= 1;
    }
    fn classical_file_allowed(&self, to: usize) -> bool {
        let c = self.cells[to].unwrap();
        let limit = R::file_limit(c.piece);
        if limit == 0 {
            return true;
        }
        let (file, _) = R::Geometry::coordinates(to).unwrap();
        let start = file * R::Geometry::HEIGHT;
        self.cells[start..start + R::Geometry::HEIGHT]
            .iter()
            .flatten()
            .filter(|v| v.color == c.color && Self::def(**v).id == Self::def(c).id)
            .count()
            <= limit
    }
    /// Match geometric_targets(from, c, false) for one destination, without
    /// allocating all destinations. This is geometry, not move/check legality.
    fn classical_attacks_square(&self, from: usize, c: Cell, to: usize) -> bool {
        if from == to || self.cells[to].is_some_and(|v| v.color == c.color) {
            return false;
        }
        let sign = if c.color == 0 { 1 } else { -1 };
        let def = Self::def(c);
        let (fx, fy) = R::Geometry::coordinates(from).unwrap();
        let (tx, ty) = R::Geometry::coordinates(to).unwrap();
        let (x, y) = (
            (tx as isize - fx as isize) * sign,
            (ty as isize - fy as isize) * sign,
        );
        if def.steps.contains(&(x, y)) {
            return true;
        }
        for &(dx, dy) in def.rays {
            let distance = if <R as RayShape>::LINE_RAYS && <R as RayShape>::PRIMITIVE_RAYS {
                // Unit rays: no division by a runtime direction.
                let distance = x.abs().max(y.abs());
                if x != dx * distance || y != dy * distance {
                    continue;
                }
                distance
            } else if dx != 0 {
                if x % dx != 0 {
                    continue;
                }
                x / dx
            } else {
                if x != 0 || dy == 0 || y % dy != 0 {
                    continue;
                }
                y / dy
            };
            if distance <= 0 || x != dx * distance || y != dy * distance {
                continue;
            }
            if R::ray_limit(c.piece, dx, dy).is_some_and(|limit| distance as usize > limit) {
                continue;
            }
            let mut at = from;
            let mut blocked = false;
            for _ in 1..distance {
                at = R::Geometry::offset(at, dx * sign, dy * sign).unwrap();
                if self.cells[at].is_some() {
                    blocked = true;
                    break;
                }
            }
            if !blocked {
                return true;
            }
        }
        false
    }
    /// Same answer as core `attacked`, including rule capture restrictions,
    /// without allocating every enemy destination list.
    fn classical_attacked_direct(&self, color: u8) -> bool {
        self.classical_attacked_by::<true>(color)
    }
    /// With `SUPPORT` false, capture-support conditions are ignored: then a
    /// royal can be attacked only where this reports an attack.
    fn classical_attacked_by<const SUPPORT: bool>(&self, color: u8) -> bool {
        let mut royals = self.occupied[color as usize];
        while let Some(to) = royals.first() {
            royals = royals.without(to);
            if !Self::def(self.cells[to].unwrap()).royal {
                continue;
            }
            let mut enemies = self.occupied[(color ^ 1) as usize];
            while let Some(from) = enemies.first() {
                enemies = enemies.without(from);
                let c = self.cells[from].unwrap();
                if self.classical_attacks_square(from, c, to)
                    && if SUPPORT {
                        self.capture_allowed_at(from, to)
                    } else {
                        R::capture_allowed(c.piece, self.cells[to].unwrap().piece, color)
                    }
                {
                    return true;
                }
            }
        }
        false
    }
    fn classical_attacked(&self, color: u8) -> bool {
        let attacked = self.classical_attacked_direct(color);
        #[cfg(test)]
        assert_eq!(attacked, self.attacked(color));
        attacked
    }
    /// Whether `color`'s royals are attacked in this position, reached from
    /// `before` by `mv`. If no royal was attacked before, ignoring capture
    /// support, a non-royal move can only add an attack by vacating a ray
    /// square of an enemy slider, and a drop can only block. `royal_map`, when
    /// given, holds every such enemy attack with the moving royal lifted off
    /// the board. Other royal moves or placements take the full query, as
    /// do mapped royal destinations under capture-support rules.
    fn classical_checked_after(
        &self,
        before: &Self,
        mv: ClassicalMove<R>,
        color: u8,
        safety: Safety<R>,
        royal_map: Option<<R::Geometry as BoardGeometry>::Bits>,
    ) -> bool {
        let fast = (|| {
            let (in_check, pins) = safety?;
            if in_check {
                return None;
            }
            match mv {
                ClassicalMove::Drop { to, .. } => {
                    let placed = self.cells[Self::index(to).unwrap()].unwrap();
                    (!Self::def(placed).royal).then_some(false)
                }
                ClassicalMove::Move { from, path, .. } => {
                    let from = Self::index(from).unwrap();
                    let to = Self::index(*path.last().unwrap()).unwrap();
                    if Self::def(before.cells[from].unwrap()).royal
                        || Self::def(self.cells[to].unwrap()).royal
                    {
                        // Other royals can only be uncovered through the source;
                        // a slider behind the moved royal attacked it unless the
                        // rule forbids that capture, so this is rarely reached.
                        return royal_map.and_then(|map| {
                            if !map.contains(to) {
                                Some(
                                    before.royals(color) > 1
                                        && self.classical_discovered(color, from),
                                )
                            } else {
                                (R::CAPTURE_SUPPORT == 0).then_some(true)
                            }
                        });
                    }
                    // Squares that lost a piece: the source and any captured
                    // intermediate square. Own ones matter only if pinned.
                    let mut attacked = false;
                    for at in core::iter::once(from)
                        .chain(path.iter().map(|&at| Self::index(at).unwrap()))
                    {
                        let Some(old) = before.cells[at] else {
                            continue;
                        };
                        attacked |= self.cells[at].is_none()
                            && (old.color != color || pins.contains(at))
                            && self.classical_discovered(color, at);
                    }
                    Some(attacked)
                }
            }
        })();
        let attacked = fast.unwrap_or_else(|| self.classical_attacked(color));
        #[cfg(test)]
        assert_eq!(attacked, self.attacked(color));
        attacked
    }
    /// `color`'s non-royal pieces that are the only piece on an enemy ray
    /// before one of `color`'s royals. Only these can uncover an attack.
    fn classical_pin_candidates(&self, color: u8) -> <R::Geometry as BoardGeometry>::Bits {
        let mut pins = <R::Geometry as BoardGeometry>::Bits::EMPTY;
        let mut enemies = self.occupied[(color ^ 1) as usize];
        while let Some(from) = enemies.first() {
            enemies = enemies.without(from);
            let c = self.cells[from].unwrap();
            let sign = if c.color == 0 { 1 } else { -1 };
            for &(dx, dy) in Self::def(c).rays {
                let limit = R::ray_limit(c.piece, dx, dy).unwrap_or(usize::MAX);
                let (mut at, mut first) = (from, None);
                for _ in 0..limit {
                    let Some(next) = R::Geometry::offset(at, dx * sign, dy * sign) else {
                        break;
                    };
                    at = next;
                    let Some(v) = self.cells[at] else {
                        continue;
                    };
                    if v.color != color {
                        break;
                    }
                    let royal = Self::def(v).royal;
                    match first {
                        None if !royal => first = Some(at),
                        Some(pinned) if royal => {
                            pins.insert(pinned);
                            break;
                        }
                        _ => break,
                    }
                }
            }
        }
        pins
    }
    /// Whether an enemy slider attacks one of `color`'s royals through the
    /// now-empty square `vacated`. Every ray square lies on the segment from
    /// the slider to the royal, so only pieces beyond `vacated` on the
    /// royal-to-`vacated` line can have gained an attack.
    fn classical_discovered(&self, color: u8, vacated: usize) -> bool {
        let (fx, fy) = R::Geometry::coordinates(vacated).unwrap();
        let mut royals = self.occupied[color as usize];
        while let Some(king) = royals.first() {
            royals = royals.without(king);
            if !Self::def(self.cells[king].unwrap()).royal {
                continue;
            }
            let (kx, ky) = R::Geometry::coordinates(king).unwrap();
            let (wx, wy) = (fx as isize - kx as isize, fy as isize - ky as isize);
            if <R as RayShape>::LINE_RAYS && wx != 0 && wy != 0 && wx.abs() != wy.abs() {
                continue;
            }
            let g = gcd(wx.unsigned_abs(), wy.unsigned_abs()) as isize;
            let (ux, uy) = (wx / g, wy / g);
            let mut at = vacated;
            while let Some(next) = R::Geometry::offset(at, ux, uy) {
                at = next;
                let Some(c) = self.cells[at] else {
                    continue;
                };
                if c.color != color
                    && !Self::def(c).rays.is_empty()
                    && self.classical_attacks_square(at, c, king)
                    && self.capture_allowed_at(at, king)
                {
                    return true;
                }
                // With unit-step rays, this piece blocks everything beyond it.
                if <R as RayShape>::PRIMITIVE_RAYS {
                    break;
                }
            }
        }
        false
    }
    #[cfg(test)]
    fn classical_acceptable(
        &self,
        mv: ClassicalMove<R>,
        stop: &mut dyn FnMut() -> bool,
    ) -> Option<bool> {
        self.classical_acceptable_known(
            mv,
            None,
            &mut Scratch {
                board: None,
                royal_map: None,
                targets: Vec::new(),
            },
            stop,
        )
    }
    /// `safety` is this position's own-royal attack state and pin candidates, when known.
    fn classical_acceptable_known(
        &self,
        mv: ClassicalMove<R>,
        safety: Safety<R>,
        scratch: &mut Scratch<R>,
        stop: &mut dyn FnMut() -> bool,
    ) -> Option<bool> {
        if stop() {
            return None;
        }
        // For check-free rules, legal generation has already validated every
        // hop/capture. Own file counts change only at the mover's start/end.
        // Keep the copied validation path for check and pawn-drop-mate rules.
        if !R::CHECK
            && !matches!(mv, ClassicalMove::Drop { piece, .. } if Some(piece.id) == R::UCHIFU)
        {
            let (piece, color, to, previous) = match mv {
                ClassicalMove::Move {
                    from,
                    path,
                    promotion,
                    ..
                } => {
                    let from = Self::index(from).unwrap();
                    let c = self.cells[from].unwrap();
                    let piece = if promotion {
                        Self::def(c).promotion.unwrap()
                    } else {
                        c.piece
                    };
                    (
                        piece,
                        c.color,
                        Self::index(*path.last().unwrap()).unwrap(),
                        Some((from, c.piece)),
                    )
                }
                ClassicalMove::Drop { piece, to, .. } => {
                    (piece.id, self.side, Self::index(to).unwrap(), None)
                }
            };
            let limit = R::file_limit(piece);
            let legal = if limit == 0 {
                true
            } else {
                let (file, _) = R::Geometry::coordinates(to).unwrap();
                let start = file * R::Geometry::HEIGHT;
                let mut count = self.cells[start..start + R::Geometry::HEIGHT]
                    .iter()
                    .flatten()
                    .filter(|c| c.color == color && c.piece == piece)
                    .count();
                if previous.is_some_and(|(from, old)| {
                    old == piece && R::Geometry::coordinates(from).unwrap().0 == file
                }) {
                    count -= 1;
                }
                count + 1 <= limit
            };
            #[cfg(test)]
            assert_eq!(
                self.classical_acceptable_copy(mv, &mut || false),
                Some(legal)
            );
            return Some(legal);
        }
        self.classical_acceptable_checked(mv, safety, scratch, stop)
    }
    /// Test oracle: the original copied-position validation with core attacks.
    #[cfg(test)]
    fn classical_acceptable_copy(
        &self,
        mv: ClassicalMove<R>,
        stop: &mut dyn FnMut() -> bool,
    ) -> Option<bool> {
        let next = self.classical_apply(mv);
        let to = match mv {
            ClassicalMove::Move { path, .. } => Self::index(*path.last().unwrap()).unwrap(),
            ClassicalMove::Drop { to, .. } => Self::index(to).unwrap(),
        };
        let placed = next.cells[to].unwrap();
        if (!R::ALLOW_DEAD_BOARD_PIECES
            && next.relative_rank(to, placed.color) < Self::def(placed).dead_ranks)
            || !next.classical_file_allowed(to)
            || (R::CHECK && next.attacked(self.side))
        {
            return Some(false);
        }
        if let ClassicalMove::Drop { piece, .. } = mv {
            if Some(piece.id) == R::UCHIFU
                && next.attacked(next.side)
                && !next.classical_has_check_escape(stop)?
            {
                return Some(false);
            }
        }
        Some(true)
    }
    /// Undo `classical_place`: copy back every square `mv` wrote.
    fn classical_unplace(&mut self, before: &Self, mv: ClassicalMove<R>) {
        match mv {
            ClassicalMove::Move { from, path, .. } => {
                for at in core::iter::once(from).chain(path.iter().copied()) {
                    let at = Self::index(at).unwrap();
                    self.set_cell(at, before.cells[at]);
                }
            }
            ClassicalMove::Drop { to, .. } => {
                let at = Self::index(to).unwrap();
                self.set_cell(at, before.cells[at]);
            }
        }
    }
    /// Write only `mv`'s board cells into `self`, a cell-for-cell copy of
    /// `before`; reserves, rule state and side are left as they were.
    fn classical_place(&mut self, before: &Self, mv: ClassicalMove<R>) {
        match mv {
            ClassicalMove::Move {
                from,
                path,
                promotion,
                ..
            } => {
                let mut from = Self::index(from).unwrap();
                let mut mover = before.cells[from].unwrap();
                for &to in path.iter() {
                    let to = Self::index(to).unwrap();
                    self.set_cell(from, None);
                    self.set_cell(to, Some(mover));
                    from = to;
                }
                if promotion {
                    mover.piece = Self::def(mover).promotion.unwrap();
                    self.set_cell(from, Some(mover));
                }
            }
            ClassicalMove::Drop { piece, to, .. } => self.set_cell(
                Self::index(to).unwrap(),
                Some(Cell {
                    piece: piece.id,
                    color: before.side,
                }),
            ),
        }
    }
    /// Every square `color ^ 1` attacks once `from`'s royal is lifted off
    /// the board, counting only attackers allowed to capture `royal`.
    /// `self` is a scratch copy and is restored before returning.
    fn classical_royal_map(
        &mut self,
        from: usize,
        royal: Cell,
        targets: &mut Vec<usize>,
    ) -> <R::Geometry as BoardGeometry>::Bits {
        let mut map = <R::Geometry as BoardGeometry>::Bits::EMPTY;
        self.set_cell(from, None);
        let mut enemies = self.occupied[(royal.color ^ 1) as usize];
        while let Some(at) = enemies.first() {
            enemies = enemies.without(at);
            let c = self.cells[at].unwrap();
            if !R::capture_allowed(c.piece, royal.piece, royal.color) {
                continue;
            }
            targets.clear();
            self.geometric_targets_into(at, c, true, targets);
            for &to in targets.iter() {
                map.insert(to);
            }
        }
        self.set_cell(from, Some(royal));
        map
    }
    /// Not in check, a royal move whose intermediate squares were empty
    /// leaves the board as `royal_map` sees it, apart from the destination
    /// itself, for attacks on the moved royal.
    fn classical_royal_map_applies(&self, mv: ClassicalMove<R>, safety: Safety<R>) -> Option<usize> {
        let ClassicalMove::Move {
            from,
            path,
            promotion: false,
            ..
        } = mv
        else {
            return None;
        };
        let from = Self::index(from).unwrap();
        (R::CACHE_ROYALS
            && safety.is_some_and(|(in_check, _)| !in_check)
            && Self::def(self.cells[from].unwrap()).royal
            && path[..path.len() - 1]
                .iter()
                .all(|&at| self.cells[Self::index(at).unwrap()].is_none()))
        .then_some(from)
    }
    fn classical_acceptable_checked(
        &self,
        mv: ClassicalMove<R>,
        safety: Safety<R>,
        scratch: &mut Scratch<R>,
        stop: &mut dyn FnMut() -> bool,
    ) -> Option<bool> {
        // Validate on a reusable board copy instead of a fresh position.
        let Scratch {
            board,
            royal_map,
            targets,
        } = scratch;
        let next = board.get_or_insert_with(|| self.clone());
        let map = if R::CHECK
            && let Some(from) = self.classical_royal_map_applies(mv, safety)
        {
            if royal_map.is_none_or(|(at, _)| at != from) {
                *royal_map = Some((
                    from,
                    next.classical_royal_map(from, self.cells[from].unwrap(), targets),
                ));
            }
            royal_map.map(|(_, map)| map)
        } else {
            None
        };
        next.classical_place(self, mv);
        let to = match mv {
            ClassicalMove::Move { path, .. } => Self::index(*path.last().unwrap()).unwrap(),
            ClassicalMove::Drop { to, .. } => Self::index(to).unwrap(),
        };
        let placed = next.cells[to].unwrap();
        let mut legal = !((!R::ALLOW_DEAD_BOARD_PIECES
            && next.relative_rank(to, placed.color) < Self::def(placed).dead_ranks)
            || !next.classical_file_allowed(to)
            || (R::CHECK && next.classical_checked_after(self, mv, self.side, safety, map)));
        let uchifu_check = legal
            && matches!(mv, ClassicalMove::Drop { piece, .. } if Some(piece.id) == R::UCHIFU)
            && next.classical_attacked(self.side ^ 1);
        next.classical_unplace(self, mv);
        if uchifu_check && !self.classical_apply(mv).classical_has_check_escape(stop)? {
            legal = false;
        }
        #[cfg(test)]
        {
            assert!(next.cells == self.cells && next.occupied == self.occupied);
            // Pawn-drop-mate escapes recurse into move generation; comparing
            // inside them too would grow exponentially with the recursion.
            if !IN_ESCAPE.with(|f| f.get()) {
                assert_eq!(Some(legal), self.classical_acceptable_copy(mv, &mut || false));
            }
        }
        Some(legal)
    }
    fn classical_has_check_escape(&self, stop: &mut dyn FnMut() -> bool) -> Option<bool> {
        #[cfg(test)]
        {
            let outer = IN_ESCAPE.with(|f| f.replace(true));
            let escape = self.classical_has_check_escape_inner(stop);
            IN_ESCAPE.with(|f| f.set(outer));
            escape
        }
        #[cfg(not(test))]
        self.classical_has_check_escape_inner(stop)
    }
    fn classical_has_check_escape_inner(&self, stop: &mut dyn FnMut() -> bool) -> Option<bool> {
        if R::CHECK { return Some(!self.classical_moves(1, stop)?.is_empty()); }
        // Test the escape before legality: validating a pawn drop can itself
        // search escapes, so validating every move first grows exponentially.
        let escapes = |mv| !self.classical_apply(mv).classical_attacked(self.side);
        Some(!self.classical_moves_where(1, Some(&escapes), stop)?.is_empty())
    }
    // Toruichi deliberately uses core's pseudo-capture test: a pinned non-royal
    // still creates an obligation. In check there is no capture obligation.
    fn classical_requires_capture(&self) -> bool {
        if self.classical_attacked(self.side) {
            return false;
        }
        let mut royal_targets = Vec::new();
        for (from, c) in self.cells.iter().enumerate() {
            let Some(c) = *c else {
                continue;
            };
            if c.color != self.side {
                continue;
            }
            for to in self.targets(from, c) {
                if self.cells[to].is_some_and(|victim| victim.color != self.side) {
                    if !Self::def(c).royal {
                        return true;
                    }
                    royal_targets.push(to);
                }
            }
        }
        royal_targets.into_iter().any(|target| {
            !self.cells.iter().enumerate().any(|(from, c)| {
                c.is_some_and(|c| {
                    c.color != self.side && self.geometric_targets(from, c, true).contains(&target)
                })
            })
        })
    }
    fn classical_is_capture(&self, mv: ClassicalMove<R>) -> bool {
        matches!(mv, ClassicalMove::Move { path, .. } if self.cells[Self::index(path[0]).unwrap()].is_some())
    }
    fn classical_append(
        &self,
        out: &mut Vec<ClassicalMove<R>>,
        mv: ClassicalMove<R>,
        limit: usize,
        must_capture: bool,
        keep: Option<&dyn Fn(ClassicalMove<R>) -> bool>,
        safety: Safety<R>,
        scratch: &mut Scratch<R>,
        stop: &mut dyn FnMut() -> bool,
    ) -> Option<bool> {
        if must_capture && !self.classical_is_capture(mv) {
            return Some(false);
        }
        if keep.is_some_and(|keep| !keep(mv)) {
            return Some(false);
        }
        if self.classical_acceptable_known(mv, safety, scratch, stop)? {
            out.push(mv);
        }
        Some(out.len() >= limit)
    }
    pub fn classical_moves(
        &self,
        limit: usize,
        stop: &mut dyn FnMut() -> bool,
    ) -> Option<Vec<ClassicalMove<R>>> {
        self.classical_moves_where(limit, None, stop)
    }
    /// Legal moves that also satisfy `keep`, which is tested first.
    fn classical_moves_where(
        &self,
        limit: usize,
        keep: Option<&dyn Fn(ClassicalMove<R>) -> bool>,
        stop: &mut dyn FnMut() -> bool,
    ) -> Option<Vec<ClassicalMove<R>>> {
        if self.capture_outcome() != Outcome::ONGOING || limit == 0 {
            return Some(vec![]);
        }
        let mut out = Vec::with_capacity(limit.min(64));
        // The capture obligation depends only on this position.
        let must_capture = R::FORCE_CAPTURE && self.classical_requires_capture();
        let safety = R::CHECK.then(|| {
            let in_check = self.classical_attacked_by::<false>(self.side);
            let pins = if in_check {
                <R::Geometry as BoardGeometry>::Bits::EMPTY
            } else {
                self.classical_pin_candidates(self.side)
            };
            (in_check, pins)
        });
        let mut scratch = Scratch {
            board: None,
            royal_map: None,
            targets: Vec::new(),
        };
        let mut pieces = self.occupied[self.side as usize];
        let mut targets = Vec::with_capacity(32);
        while let Some(from) = pieces.first() {
            if stop() {
                return None;
            }
            pieces = pieces.without(from);
            let c = self.cells[from].unwrap();
            let sign = if c.color == 0 { 1 } else { -1 };
            let (a, b) = R::Geometry::coordinates(from).unwrap();
            targets.clear();
            self.geometric_targets_into(from, c, false, &mut targets);
            for &to in &targets {
                if stop() {
                    return None;
                }
                if !self.classical_capture_allowed(self, from, to, None) {
                    continue;
                }
                let (x, y) = R::Geometry::coordinates(to).unwrap();
                let follow = R::continuation(
                    c.piece,
                    (
                        (x as isize - a as isize) * sign,
                        (y as isize - b as isize) * sign,
                    ),
                );
                let (can, must) = R::promotion_options(
                    c.piece,
                    self.relative_rank(from, c.color),
                    self.relative_rank(to, c.color),
                    self.cells[to].is_some(),
                );
                if !must
                    && self.classical_append(
                        &mut out,
                        ClassicalMove::board(Self::at(from), Path::one(Self::at(to)), false),
                        limit,
                        must_capture,
                        keep,
                        safety,
                        &mut scratch,
                        stop,
                    )?
                {
                    return Some(out);
                }
                if can
                    && follow.is_none()
                    && self.classical_append(
                        &mut out,
                        ClassicalMove::board(Self::at(from), Path::one(Self::at(to)), true),
                        limit,
                        must_capture,
                        keep,
                        safety,
                        &mut scratch,
                        stop,
                    )?
                {
                    return Some(out);
                }
                if let Some(follow) = follow {
                    let victim = self.cells[to];
                    let mut mid = self.clone();
                    mid.classical_hop(from, to);
                    if mid.capture_outcome() != Outcome::ONGOING {
                        continue;
                    }
                    for &(dx, dy) in follow {
                        if stop() {
                            return None;
                        }
                        let Some(end) = R::Geometry::offset(to, dx * sign, dy * sign) else {
                            continue;
                        };
                        if !mid.classical_capture_allowed(self, to, end, Some((from, victim))) {
                            continue;
                        }
                        if self.classical_append(
                            &mut out,
                            ClassicalMove::board(
                                Self::at(from),
                                Path::two(Self::at(to), Self::at(end)),
                                false,
                            ),
                            limit,
                            must_capture,
                            keep,
                            safety,
                            &mut scratch,
                            stop,
                        )? {
                            return Some(out);
                        }
                    }
                }
            }
        }
        if R::Hands::ENABLED {
            for id in 0..R::PIECES.len() {
                let id = PieceId(id as u16);
                if !R::droppable(id) || self.hands.count(self.side, R::drop_hand(id)) == 0 {
                    continue;
                }
                let def = &R::PIECES[id.0 as usize];
                for to in 0..R::Geometry::SQUARES {
                    if stop() {
                        return None;
                    }
                    if self.cells[to].is_some()
                        || self.relative_rank(to, self.side) < def.dead_ranks
                    {
                        continue;
                    }
                    if self.classical_append(
                        &mut out,
                        ClassicalMove::drop(id, Self::at(to)),
                        limit,
                        must_capture,
                        keep,
                        safety,
                        &mut scratch,
                        stop,
                    )? {
                        return Some(out);
                    }
                }
            }
        }
        // Equal moves are identical, so an unstable sort gives the same list.
        out.sort_unstable();
        out.dedup();
        Some(out)
    }
}
#[derive(Clone)]
pub struct ClassicalGame<R: ClassicalRule> {
    pub history: Vec<Position<R>>,
    pub played: Vec<ClassicalMove<R>>,
    pending: Option<Prefix>,
    pending_ply: usize,
    spare: Spare<Position<R>>,
}
/// Popped positions whose allocations later pushes reuse; never cloned.
struct Spare<T>(Vec<T>);
impl<T> Clone for Spare<T> {
    fn clone(&self) -> Self {
        Self(Vec::new())
    }
}
impl<R: ClassicalRule> ClassicalGame<R> {
    pub fn parse(json: &str) -> Result<Self, String> {
        let req: Request<R> = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if req.schema_version != 3 || req.rule_id != R::ID {
            return Err("unsupported schemaVersion or ruleId".into());
        }
        let current = Position::<R>::from_wire(&req.position)?;
        let mut game = if let Some(h) = req.history {
            let mut g = Self::root(Position::from_wire(&h.initial)?);
            for mv in h.moves {
                g.play(mv)?;
            }
            g
        } else {
            Self::root(current.clone())
        };
        if game.current() != &current {
            return Err("history must end at position including reserves and rule state".into());
        }
        game.pending_ply = game.played.len();
        game.pending = req.pending;
        if let Some(p) = game.pending {
            if !R::LION {
                return Err("this rule has no multi-step pending moves".into());
            }
            let from = Position::<R>::index(p.from)?;
            let via = Position::<R>::index(p.via)?;
            let c = game.current().cells[from].ok_or("pending source is empty")?;
            let (a, b) = R::Geometry::coordinates(from).unwrap();
            let (x, y) = R::Geometry::coordinates(via).unwrap();
            let sign = if c.color == 0 { 1 } else { -1 };
            if R::continuation(
                c.piece,
                (
                    (x as isize - a as isize) * sign,
                    (y as isize - b as isize) * sign,
                ),
            )
            .is_none()
                || game.moves().is_empty()
            {
                return Err("invalid pending path".into());
            }
            if game
                .current()
                .classical_apply(ClassicalMove::board(p.from, Path::one(p.via), false))
                .capture_outcome()
                != Outcome::ONGOING
            {
                return Err("pending path already ended the game".into());
            }
        }
        Ok(game)
    }
    pub fn root(position: Position<R>) -> Self {
        Self {
            history: vec![position],
            played: vec![],
            pending: None,
            pending_ply: 0,
            spare: Spare(Vec::new()),
        }
    }
    pub fn current(&self) -> &Position<R> {
        self.history.last().unwrap()
    }
    fn pending(&self) -> Option<Prefix> {
        if self.played.len() == self.pending_ply {
            self.pending
        } else {
            None
        }
    }
    fn prefix_allowed(&self, mv: ClassicalMove<R>) -> bool {
        self.pending().is_none_or(
            |p| matches!(mv,ClassicalMove::Move{from,path,..}if from==p.from&&path[0]==p.via),
        )
    }
    fn paths(&self, limit: usize, stop: &mut dyn FnMut() -> bool) -> Option<Vec<ClassicalMove<R>>> {
        let mut moves = self.current().classical_moves(
            if self.pending().is_some() {
                usize::MAX
            } else {
                limit
            },
            stop,
        )?;
        moves.retain(|&m| self.prefix_allowed(m));
        moves.truncate(limit);
        Some(moves)
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(&Request::<R> {
            schema_version: 3,
            rule_id: R::ID,
            position: self.current().to_wire(),
            history: Some(History {
                initial: self.history[0].to_wire(),
                moves: self.played.clone(),
            }),
            pending: self.pending(),
        })
        .unwrap()
    }
    fn basic_outcome(&self) -> Outcome {
        let p = self.current();
        let captured = p.capture_outcome();
        if captured != Outcome::ONGOING {
            return captured;
        }
        if R::TRY {
            if let Some(ClassicalMove::Move { path, .. }) = self.played.last() {
                let to = Position::<R>::index(*path.last().unwrap()).unwrap();
                let previous = p.side ^ 1;
                if p.cells[to].is_some_and(|c| {
                    c.color == previous
                        && Position::<R>::def(c).royal
                        && p.relative_rank(to, previous) == 0
                }) && !p.classical_attacked(previous)
                {
                    return Outcome::win(previous);
                }
            }
        }
        if R::ONLY_KINGS
            && p.cells
                .iter()
                .flatten()
                .all(|c| Position::<R>::def(*c).royal)
        {
            return Outcome::DRAW;
        }
        let mut matches = self
            .history
            .iter()
            .enumerate()
            .filter(|(_, q)| *q == p)
            .map(|(i, _)| i);
        let first = matches.next().unwrap();
        if matches.count() < 3 {
            return Outcome::ONGOING;
        }
        for victim in [p.side, p.side ^ 1] {
            if self.history[first + 1..]
                .iter()
                .filter(|q| q.side == victim)
                .all(|q| q.classical_attacked(victim))
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
        let outcome = self.basic_outcome();
        if outcome != Outcome::ONGOING {
            return Some(outcome);
        }
        if (R::STALEMATE || (R::CHECKMATE && self.current().classical_attacked(self.current().side)))
            && (if R::CHECK {
                self.paths(1, stop)?.is_empty()
            } else {
                !self.current().classical_has_check_escape(stop)?
            })
        {
            return Some(Outcome::win(self.current().side ^ 1));
        }
        Some(outcome)
    }
    pub fn outcome(&self) -> Outcome {
        self.outcome_cancel(&mut || false).unwrap()
    }
    pub fn moves(&self) -> Vec<ClassicalMove<R>> {
        if self.basic_outcome() != Outcome::ONGOING {
            vec![]
        } else {
            self.paths(usize::MAX, &mut || false).unwrap()
        }
    }
    pub fn play(&mut self, mv: ClassicalMove<R>) -> Result<(), String> {
        if mv.version() != 3 || !self.moves().contains(&mv) {
            return Err("illegal completed move or unsupported schemaVersion".into());
        }
        self.push(mv);
        Ok(())
    }
    fn push(&mut self, mv: ClassicalMove<R>) {
        let next = match self.spare.0.pop() {
            Some(mut next) => {
                next.clone_from(self.current());
                next.classical_play(mv);
                next
            }
            None => self.current().classical_apply(mv),
        };
        self.history.push(next);
        self.played.push(mv);
    }
    fn pop(&mut self) {
        if let Some(p) = self.history.pop() {
            self.spare.0.push(p);
        }
        self.played.pop();
    }
    pub fn position_hash(&self) -> u64 {
        let mut h = self.current().hash();
        if let Some(p) = self.pending() {
            for n in p.from.into_iter().chain(p.via) {
                h = h.rotate_left(7) ^ u64::from(n).wrapping_mul(0x9e3779b97f4a7c15);
            }
        }
        h
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
        let mut nodes = 0;
        for mv in self.moves() {
            self.push(mv);
            nodes += self.perft(depth - 1);
            self.pop();
        }
        nodes
    }
}
impl<R: ClassicalRule> SearchGame for ClassicalGame<R> {
    type Move = ClassicalMove<R>;
    const SCHEMA: u8 = 3;
    fn moves(&self, limit: usize, stop: &mut dyn FnMut() -> bool) -> Option<Vec<Self::Move>> {
        self.paths(limit, stop)
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
    fn push(&mut self, mv: Self::Move) {
        ClassicalGame::push(self, mv);
    }
    fn pop(&mut self) {
        ClassicalGame::pop(self);
    }
    fn key(&self) -> u64 {
        self.context_key()
    }
    fn capture_value(&self, mv: Self::Move) -> i32 {
        let ClassicalMove::Move { from, path, .. } = mv else {
            return 0;
        };
        let from = Position::<R>::index(from).unwrap();
        let mut taken = vec![];
        let mut value = 0;
        for &at in path.iter() {
            let i = Position::<R>::index(at).unwrap();
            if i != from && !taken.contains(&i) {
                if let Some(c) = self.current().cells[i] {
                    value += Position::<R>::def(c).value;
                }
                taken.push(i);
            }
        }
        value
    }
    fn promotion(mv: Self::Move) -> bool {
        mv.promotes()
    }
}
#[cfg(any(
    feature = "torishogi",
    feature = "washogi",
    feature = "heian-dai",
    feature = "daishogi",
    feature = "kyoto",
    feature = "kinkei-mai",
    feature = "microshogi",
    feature = "kyoto-minishogi",
    feature = "narazu",
    feature = "nifu-ari",
    feature = "sho-kyo-left",
    feature = "sho-kyo-right",
    feature = "nakodo",
    feature = "queen-king",
    feature = "torazu-gin",
    feature = "ai-torazu-gin",
    feature = "zarikani",
    feature = "zarikani-tonbo",
    feature = "semai",
    feature = "semmai",
    feature = "hiroi",
    feature = "dobutsu",
    feature = "toruichi",
    feature = "hisha",
    feature = "kaku",
    feature = "seireigi",
    feature = "kyo-left",
    feature = "kyo-right",
    feature = "totsugeki",
    feature = "ai-happokei",
    feature = "happokei",
    feature = "patrol",
    feature = "lortap",
    feature = "shishio",
    feature = "hono-shishi",
    feature = "nigyoku"
))]
pub mod compiled {
    use super::*;
    #[cfg(feature = "torishogi")]
    pub type Selected = Torishogi;
    #[cfg(feature = "washogi")]
    pub type Selected = Washogi;
    #[cfg(feature = "heian-dai")]
    pub type Selected = HeianDai;
    #[cfg(feature = "daishogi")]
    pub type Selected = Daishogi;
    #[cfg(feature = "kyoto")]
    pub type Selected = Kyoto;
    #[cfg(feature = "kinkei-mai")]
    pub type Selected = KinkeiMai;
    #[cfg(feature = "microshogi")]
    pub type Selected = Microshogi;
    #[cfg(feature = "kyoto-minishogi")]
    pub type Selected = KyotoMinishogi;

    #[cfg(feature = "narazu")]
    pub type Selected = Narazu;
    #[cfg(feature = "nifu-ari")]
    pub type Selected = NifuAri;

    #[cfg(feature = "sho-kyo-left")]
    pub type Selected = ShoKyoLeft;
    #[cfg(feature = "sho-kyo-right")]
    pub type Selected = ShoKyoRight;
    #[cfg(feature = "nakodo")]
    pub type Selected = Nakodo;
    #[cfg(feature = "queen-king")]
    pub type Selected = QueenKing;

    #[cfg(feature = "torazu-gin")]
    pub type Selected = TorazuGin;
    #[cfg(feature = "ai-torazu-gin")]
    pub type Selected = AiTorazuGin;
    #[cfg(feature = "zarikani")]
    pub type Selected = Zarikani;
    #[cfg(feature = "zarikani-tonbo")]
    pub type Selected = ZarikaniTonbo;

    #[cfg(feature = "semai")]
    pub type Selected = Semai;
    #[cfg(feature = "semmai")]
    pub type Selected = Semmai;
    #[cfg(feature = "hiroi")]
    pub type Selected = Hiroi;

    #[cfg(feature = "dobutsu")]
    pub type Selected = Dobutsu;

    #[cfg(feature = "toruichi")]
    pub type Selected = Toruichi;

    #[cfg(feature = "hisha")]
    pub type Selected = Hisha;
    #[cfg(feature = "kaku")]
    pub type Selected = Kaku;
    #[cfg(feature = "seireigi")]
    pub type Selected = Seireigi;
    #[cfg(feature = "kyo-left")]
    pub type Selected = KyoLeft;
    #[cfg(feature = "kyo-right")]
    pub type Selected = KyoRight;

    #[cfg(feature = "totsugeki")]
    pub type Selected = Totsugeki;
    #[cfg(feature = "ai-happokei")]
    pub type Selected = AiHappokei;
    #[cfg(feature = "happokei")]
    pub type Selected = Happokei;
    #[cfg(feature = "patrol")]
    pub type Selected = Patrol;
    #[cfg(feature = "lortap")]
    pub type Selected = Lortap;

    #[cfg(feature = "shishio")]
    pub type Selected = Shishio;
    #[cfg(feature = "hono-shishi")]
    pub type Selected = HonoShishi;
    #[cfg(feature = "nigyoku")]
    pub type Selected = Nigyoku;

    pub type CompiledGame = ClassicalGame<Selected>;
    pub const PROTOCOL: &str = super::PROTOCOL;
    pub const NAME: &str = Selected::NAME;
    pub const RULE_ID: u32 = Selected::ID;
    pub fn start_position_json() -> String {
        let p = Position::<Selected>::from_wire(&WirePosition {
            side_to_move: Selected::INITIAL_SIDE,
            pieces: Selected::initial(),
            hands: Default::default(),
            state: Default::default(),
        })
        .unwrap();
        CompiledGame::root(p).to_json()
    }
    pub fn validate_position_json(json: &str) -> Result<String, String> {
        Ok(CompiledGame::parse(json)?.to_json())
    }
    pub fn legal_moves_json(json: &str) -> Result<String, String> {
        serde_json::to_string(&CompiledGame::parse(json)?.moves()).map_err(|e| e.to_string())
    }
    pub fn apply_move_json(json: &str, mv: &str) -> Result<String, String> {
        let mut game = CompiledGame::parse(json)?;
        let mv = serde_json::from_str(mv).map_err(|e| e.to_string())?;
        game.play(mv)?;
        Ok(game.to_json())
    }
    pub fn inspect_json(json: &str) -> Result<String, String> {
        let g = CompiledGame::parse(json)?;
        Ok(serde_json::json!({"schemaVersion":3,"positionHash":format!("{:016x}",g.position_hash()),"contextHash":format!("{:016x}",g.context_key()),"outcome":g.outcome()}).to_string())
    }
    pub fn perft_json(json: &str, depth: u8) -> Result<String, String> {
        if depth > 4 {
            return Err("classical perft depth must be <= 4".into());
        }
        let mut g = CompiledGame::parse(json)?;
        let t = Instant::now();
        let nodes = g.perft(depth);
        let ms = t.elapsed().as_secs_f64() * 1000.;
        Ok(serde_json::json!({"nodes":nodes,"elapsedMs":ms,"nps":(nodes as f64*1000./ms.max(0.001))as u64}).to_string())
    }
    pub fn search_json(
        json: &str,
        max_depth: u8,
        timeout_ms: u32,
        node_budget: u32,
    ) -> Result<String, String> {
        let mut game = CompiledGame::parse(json)?;
        search::search_json(&mut game, max_depth, timeout_ms, node_budget)
    }
}
#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "classical/attack_tests.rs"]
mod attack_tests;

#[cfg(test)]
#[path = "classical/memory_tests.rs"]
mod memory_tests;

#[cfg(test)]
#[cfg(any(
    feature = "narazu",
    feature = "nifu-ari",
    feature = "sho-kyo-left",
    feature = "sho-kyo-right",
    feature = "nakodo",
    feature = "queen-king",
    feature = "torazu-gin",
    feature = "ai-torazu-gin",
    feature = "zarikani",
    feature = "zarikani-tonbo",
    feature = "semai",
    feature = "semmai",
    feature = "hiroi",
    feature = "dobutsu",
    feature = "toruichi",
    feature = "hisha",
    feature = "kaku",
    feature = "seireigi",
    feature = "kyo-left",
    feature = "kyo-right",
    feature = "totsugeki",
    feature = "ai-happokei",
    feature = "happokei",
    feature = "patrol",
    feature = "lortap",
    feature = "shishio",
    feature = "hono-shishi",
    feature = "nigyoku"
))]
#[path = "classical/selected_tests.rs"]
mod selected_tests;

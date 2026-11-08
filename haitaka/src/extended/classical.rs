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
    const STALEMATE: bool;
    const ONLY_KINGS: bool;
    const LION: bool;
    const UCHIFU: Option<PieceId>;
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
            .find(|(i, p)| {
                p.id == s
                    && R::origin(PieceId(*i as u16)).is_none()
                    && R::capture_return(PieceId(*i as u16)) == Some(PieceId(*i as u16))
            })
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
        if victim.color == mover.color {
            return false;
        }
        if !R::LION || !lion::<R>(victim) {
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
        if R::LION {
            self.state
                .set_senjishi(victim.is_some_and(lion::<R>) && !lion::<R>(mover));
        }
        if R::Hands::ENABLED {
            if let Some(victim) = victim {
                let id = R::capture_return(victim.piece).expect("reserve capture mapping");
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
                next.hands.set(
                    self.side,
                    piece.id,
                    next.hands.count(self.side, piece.id) - 1,
                );
                next.set_cell(
                    Self::index(to).unwrap(),
                    Some(Cell {
                        piece: piece.id,
                        color: self.side,
                    }),
                );
            }
        }
        next.side ^= 1;
        next
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
            let distance = if dx != 0 {
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
    fn classical_attacked_direct(&self, color: u8) -> bool {
        let mut royals = self.occupied[color as usize];
        while let Some(to) = royals.first() {
            royals = royals.without(to);
            if !Self::def(self.cells[to].unwrap()).royal {
                continue;
            }
            let mut enemies = self.occupied[(color ^ 1) as usize];
            while let Some(from) = enemies.first() {
                enemies = enemies.without(from);
                if self.classical_attacks_square(from, self.cells[from].unwrap(), to) {
                    return true;
                }
            }
        }
        false
    }
    fn classical_checked_for_legality(&self, color: u8) -> bool {
        // Keep other rules' attack/repetition/drop-mate paths unchanged.
        if matches!(R::ID, 15 | 50) {
            self.classical_attacked_direct(color)
        } else {
            self.attacked(color)
        }
    }
    fn classical_acceptable(
        &self,
        mv: ClassicalMove<R>,
        stop: &mut dyn FnMut() -> bool,
    ) -> Option<bool> {
        if stop() {
            return None;
        }
        let next = self.classical_apply(mv);
        let to = match mv {
            ClassicalMove::Move { path, .. } => Self::index(*path.last().unwrap()).unwrap(),
            ClassicalMove::Drop { to, .. } => Self::index(to).unwrap(),
        };
        if !next.classical_file_allowed(to)
            || (R::CHECK && next.classical_checked_for_legality(self.side))
        {
            return Some(false);
        }
        if let ClassicalMove::Drop { piece, .. } = mv {
            if Some(piece.id) == R::UCHIFU
                && next.attacked(next.side)
                && next.classical_moves(1, stop)?.is_empty()
            {
                return Some(false);
            }
        }
        Some(true)
    }
    fn classical_append(
        &self,
        out: &mut Vec<ClassicalMove<R>>,
        mv: ClassicalMove<R>,
        limit: usize,
        stop: &mut dyn FnMut() -> bool,
    ) -> Option<bool> {
        if self.classical_acceptable(mv, stop)? {
            out.push(mv);
        }
        Some(out.len() >= limit)
    }
    pub fn classical_moves(
        &self,
        limit: usize,
        stop: &mut dyn FnMut() -> bool,
    ) -> Option<Vec<ClassicalMove<R>>> {
        let mut out = vec![];
        if self.capture_outcome() != Outcome::ONGOING || limit == 0 {
            return Some(out);
        }
        let mut pieces = self.occupied[self.side as usize];
        while let Some(from) = pieces.first() {
            if stop() {
                return None;
            }
            pieces = pieces.without(from);
            let c = self.cells[from].unwrap();
            let def = Self::def(c);
            let sign = if c.color == 0 { 1 } else { -1 };
            let (a, b) = R::Geometry::coordinates(from).unwrap();
            for to in self.targets(from, c) {
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
                let zone = self.relative_rank(from, c.color) < R::PROMOTION_RANKS
                    || self.relative_rank(to, c.color) < R::PROMOTION_RANKS;
                let can = def.promotion.is_some() && zone;
                let must = can && (R::FORCED || self.relative_rank(to, c.color) < def.dead_ranks);
                if !must
                    && self.classical_append(
                        &mut out,
                        ClassicalMove::board(Self::at(from), Path::one(Self::at(to)), false),
                        limit,
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
                if self.hands.count(self.side, id) == 0 {
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
                        stop,
                    )? {
                        return Some(out);
                    }
                }
            }
        }
        out.sort();
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
        if R::ONLY_KINGS
            && p.cells
                .iter()
                .flatten()
                .all(|c| Position::<R>::def(*c).royal)
        {
            return Outcome::DRAW;
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
                .filter(|q| q.side == victim)
                .all(|q| q.attacked(victim))
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
        if (R::STALEMATE || (R::CHECK && self.current().attacked(self.current().side)))
            && self.paths(1, stop)?.is_empty()
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
        self.history.push(self.current().classical_apply(mv));
        self.played.push(mv);
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
            self.history.pop();
            self.played.pop();
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
        self.history.pop();
        self.played.pop();
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
    feature = "daishogi"
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
    pub type CompiledGame = ClassicalGame<Selected>;
    pub const PROTOCOL: &str = super::PROTOCOL;
    pub const NAME: &str = Selected::NAME;
    pub const RULE_ID: u32 = Selected::ID;
    pub fn start_position_json() -> String {
        let p = Position::<Selected>::from_wire(&WirePosition {
            side_to_move: 0,
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

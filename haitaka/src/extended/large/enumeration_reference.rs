//! Saved pre-metadata enumeration oracle; intentionally retains hop/finish/restore.
use super::*;
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
pub(super) fn enumerate(
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

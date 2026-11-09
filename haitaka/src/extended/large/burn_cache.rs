//! Only immutable turn-start actor graphs can reuse the burn coverage.
use super::*;
pub(super) struct BurnCache {
    actors: Vec<(usize, Cell)>,
    order: Vec<Resident>,
    coverage: [u8; Geometry::SQUARES],
    baseline: Vec<usize>,
}
impl BurnCache {
    pub fn empty() -> Self {
        Self {
            actors: vec![],
            order: vec![],
            coverage: [0; Geometry::SQUARES],
            baseline: vec![],
        }
    }
    pub fn new(p: &Pos, actives: &[Resident], fires: &[usize]) -> Self {
        let mut out = Self::empty();
        out.order = p.state.shitenno.clone();
        out.actors.extend(out.order.iter().map(|r| {
            let i = Pos::index(r.at).unwrap();
            (i, p.cells[i].unwrap())
        }));
        for &i in fires {
            let c = p.cells[i].unwrap();
            out.actors.push((i, c));
            if frozen(i, c.color, actives) {
                continue;
            }
            for &(dx, dy) in KING {
                if let Some(j) = Geometry::offset(i, dx, dy) {
                    out.coverage[j] |= 1 << (c.color ^ 1);
                }
            }
        }
        out.baseline = p
            .cells
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                c.filter(|c| out.coverage[i] & (1 << c.color) != 0)
                    .map(|_| i)
            })
            .collect();
        out
    }
    pub fn heap_bytes(&self) -> usize {
        self.actors.capacity() * std::mem::size_of::<(usize, Cell)>()
            + self.order.capacity() * std::mem::size_of::<Resident>()
            + self.baseline.capacity() * std::mem::size_of::<usize>()
    }
    pub fn apply(&self, p: &mut Pos, to: usize, d: &mut Delta) -> bool {
        if p.state.shitenno != self.order || self.actors.iter().any(|&(i, c)| p.cells[i] != Some(c))
        {
            return false;
        }
        if p.cells[to].is_some_and(|c| fire(c) || Pos::def(c).id == "fp")
            && !self
                .actors
                .iter()
                .any(|&(i, c)| i == to && p.cells[to] == Some(c))
        {
            return false;
        }
        for &i in &self.baseline {
            if i != to && p.cells[i].is_some_and(|c| self.coverage[i] & (1 << c.color) != 0) {
                d.set(p, i, None);
            }
        }
        if p.cells[to].is_some_and(|c| self.coverage[to] & (1 << c.color) != 0) {
            d.set(p, to, None);
        }
        settle_residents(p, to);
        true
    }
}

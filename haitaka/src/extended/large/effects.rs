//! Information shared only within one immutable turn-start position.
use super::*;

pub(super) struct EffectContext {
    pub actives: Vec<Resident>,
    pub resident_fires: Vec<Resident>,
    pub fire_squares: Vec<usize>,
    #[cfg(feature = "tenjiku-dai")]
    pub burn_cache: super::burn_cache::BurnCache,
}
impl EffectContext {
    pub fn heap_bytes(&self) -> usize {
        self.actives.capacity() * std::mem::size_of::<Resident>()
            + self.resident_fires.capacity() * std::mem::size_of::<Resident>()
            + self.fire_squares.capacity() * std::mem::size_of::<usize>()
            + {
                #[cfg(feature = "tenjiku-dai")]
                {
                    self.burn_cache.heap_bytes()
                }
                #[cfg(not(feature = "tenjiku-dai"))]
                {
                    0
                }
            }
    }
    pub fn new(p: &Pos) -> Self {
        if !ORDERED_EFFECTS {
            return Self {
                actives: vec![],
                resident_fires: vec![],
                fire_squares: vec![],
                #[cfg(feature = "tenjiku-dai")]
                burn_cache: super::burn_cache::BurnCache::empty(),
            };
        }
        let actives = active(p, None);
        let fire_squares: Vec<_> = p
            .cells
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.is_some_and(fire).then_some(i))
            .collect();
        let resident_fires = fire_squares
            .iter()
            .filter_map(|&i| {
                let c = p.cells[i].unwrap();
                (!frozen(i, c.color, &actives)).then_some(Resident {
                    at: Pos::at(i),
                    color: c.color,
                })
            })
            .collect();
        #[cfg(test)]
        assert_eq!(resident_fires, fires(p));
        #[cfg(feature = "tenjiku-dai")]
        let burn_cache = super::burn_cache::BurnCache::new(p, &actives, &fire_squares);
        Self {
            #[cfg(feature = "tenjiku-dai")]
            burn_cache,
            actives,
            resident_fires,
            fire_squares,
        }
    }
}
pub(super) fn settle_residents(p: &mut Pos, to: usize) {
    if !ORDERED_EFFECTS {
        p.state.shitenno = order(p, &p.state.shitenno, None);
        return;
    }
    #[cfg(test)]
    let expected = order(p, &p.state.shitenno, None);
    // Hops remove/relocate pieces; only the final moving/promoted piece can
    // introduce a resident absent from the prior, already normalized list.
    let cells = &p.cells;
    p.state.shitenno.retain(|r| {
        cells[Pos::index(r.at).unwrap()]
            .is_some_and(|c| c.color == r.color && Pos::def(c).id == "fp")
    });
    if let Some(c) = p.cells[to].filter(|c| Pos::def(*c).id == "fp") {
        let r = Resident {
            at: Pos::at(to),
            color: c.color,
        };
        if !p.state.shitenno.contains(&r) {
            p.state.shitenno.push(r);
        }
    }
    #[cfg(test)]
    assert_eq!(p.state.shitenno, expected);
}

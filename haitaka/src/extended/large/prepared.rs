//! Generation data belongs to one exact game position and active recursion frame.
use super::*;
use std::sync::{Arc, OnceLock};
const CACHE_LIMIT: usize = 4 * 1024 * 1024;
pub(super) const ENABLED: bool = cfg!(any(feature = "tenjiku-dai", feature = "taikyoku"));
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Hint {
    pub through: u8,
    pub flags: u8,
    pub promotion: u16,
}
impl Hint {
    const VALID: u8 = 0x80;
    const EARLY: u8 = 1;
    pub(super) fn new(through: u8, early: bool, promotion: Option<PieceId>) -> Self {
        Self {
            through,
            flags: Self::VALID | u8::from(early),
            promotion: promotion.map_or(u16::MAX, |p| p.0),
        }
    }
    pub(super) fn valid(self) -> bool {
        self.flags & Self::VALID != 0
    }
    pub(super) fn early(self) -> bool {
        self.flags & Self::EARLY != 0
    }
    pub(super) fn invalidate(&mut self) {
        self.flags &= !Self::VALID;
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Entry {
    pub mv: LargeMove,
    pub hint: Hint,
}
pub(super) struct Batch {
    owner: Arc<()>,
    epoch: u64,
    key: u64,
    prefix: Option<Pending>,
    entries: Vec<Entry>,
    effects: EffectContext,
    bytes: usize,
}
pub(super) struct Frame {
    pub cache: OnceLock<Box<Batch>>,
    pub epoch: u64,
    pub ancestor_bytes: usize,
}
impl LargeGame {
    fn cache_matches(&self, b: &Batch) -> bool {
        Arc::ptr_eq(&self.owner, &b.owner)
            && b.epoch == self.epoch
            && b.key == self.context_key()
            && b.prefix == self.prefix()
    }
    pub(super) fn generated_moves(
        &self,
        limit: usize,
        stop: &mut dyn FnMut() -> bool,
    ) -> Option<Vec<LargeMove>> {
        if !ENABLED {
            return enumerate(&self.position, self.prefix(), None, limit, stop);
        }
        if stop() {
            return None;
        }
        if limit == 0 {
            return Some(vec![]);
        }
        if let Some(b) = self
            .prepared
            .get()
            .filter(|b| limit == usize::MAX && self.cache_matches(b))
        {
            let mut out = Vec::with_capacity(limit.min(b.entries.len()));
            for e in b.entries.iter().take(limit) {
                if stop() {
                    return None;
                }
                out.push(e.mv);
            }
            return Some(out);
        }
        let (entries, effects) =
            enumerate_entries(&self.position, self.prefix(), None, limit, stop)?;
        let moves: Vec<_> = entries.iter().map(|e| e.mv).collect();
        #[cfg(test)]
        assert_eq!(
            moves,
            enumeration_reference::enumerate(
                &self.position,
                self.prefix(),
                None,
                limit,
                &mut || false
            )
            .unwrap()
        );
        // Limited/cancelled enumerations never claim to contain the complete list.
        let bytes = std::mem::size_of::<Batch>()
            + entries.capacity() * std::mem::size_of::<Entry>()
            + effects.actives.capacity() * std::mem::size_of::<Resident>()
            + effects.resident_fires.capacity() * std::mem::size_of::<Resident>()
            + effects.fire_squares.capacity() * std::mem::size_of::<usize>();
        if limit == usize::MAX && self.ancestor_bytes.saturating_add(bytes) <= CACHE_LIMIT {
            let _ = self.prepared.set(Box::new(Batch {
                owner: self.owner.clone(),
                epoch: self.epoch,
                key: self.context_key(),
                prefix: self.prefix(),
                entries,
                effects,
                bytes,
            }));
        }
        Some(moves)
    }
    pub(super) fn push_cached(&mut self, m: LargeMove) {
        if !ENABLED {
            self.push_original(m);
            return;
        }
        let cache = std::mem::take(&mut self.prepared);
        let source = cache.get().filter(|b| self.cache_matches(b));
        let hint = source.and_then(|b| {
            b.entries
                .binary_search_by_key(&m, |e| e.mv)
                .ok()
                .map(|i| b.entries[i].hint)
        });
        if let (Some(b), Some(h)) = (source, hint.filter(|h| h.valid())) {
            #[cfg(test)]
            let mut reference = self.clone();
            let mut delta = Delta::new(&self.position);
            let mut from = Pos::index(m.from).unwrap();
            for (i, &at) in m.path.iter().enumerate() {
                let to = Pos::index(at).unwrap();
                hop(
                    &mut self.position,
                    from,
                    to,
                    h.through & (1 << i) != 0,
                    &mut delta,
                );
                from = to;
            }
            if m.promotion {
                let mut c = self.position.cells[from].unwrap();
                c.piece = PieceId(h.promotion);
                delta.set(&mut self.position, from, Some(c));
            }
            if !h.early() {
                finish(&mut self.position, from, &b.effects, &mut delta);
            } else {
                settle_residents(&mut self.position, from);
            }
            self.record_move(m, delta);
            #[cfg(test)]
            {
                reference.push_original(m);
                assert_eq!(self.position, reference.position);
                assert_eq!(self.played, reference.played);
                assert_eq!(self.context_key(), reference.context_key());
            }
        } else {
            // Unknown moves never obtain trust from a hint belonging to another frame.
            self.push_original(m);
        }
        self.cache_frames.push(Frame {
            epoch: self.epoch,
            ancestor_bytes: self.ancestor_bytes,
            cache,
        });
        self.ancestor_bytes += self
            .cache_frames
            .last()
            .unwrap()
            .cache
            .get()
            .map_or(0, |b| b.bytes);
        self.epoch = self.next_epoch;
        self.next_epoch = self
            .next_epoch
            .checked_add(1)
            .expect("generation epoch exhausted");
    }
    pub(super) fn restore_cache(&mut self) {
        if !ENABLED {
            return;
        }
        let f = self.cache_frames.pop().unwrap();
        self.prepared = f.cache;
        self.epoch = f.epoch;
        self.ancestor_bytes = f.ancestor_bytes;
    }
    pub(super) fn discard_played_cache(&mut self) {
        if !ENABLED {
            return;
        }
        // Actual played history retains no generated candidate arrays.
        if let Some(f) = self.cache_frames.last_mut() {
            if let Some(b) = f.cache.take() {
                self.ancestor_bytes -= b.bytes;
            }
        }
        self.prepared.take();
    }
}
impl Clone for LargeGame {
    fn clone(&self) -> Self {
        Self {
            position: self.position.clone(),
            initial: self.initial.clone(),
            played: self.played.clone(),
            undo: self.undo.clone(),
            timeline: self.timeline.clone(),
            pending: self.pending,
            pending_ply: self.pending_ply,
            owner: Arc::new(()),
            epoch: self.epoch,
            next_epoch: self.next_epoch,
            prepared: OnceLock::new(),
            ancestor_bytes: 0,
            cache_frames: self
                .cache_frames
                .iter()
                .map(|f| Frame {
                    cache: OnceLock::new(),
                    epoch: f.epoch,
                    ancestor_bytes: 0,
                })
                .collect(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> LargeGame {
        LargeGame::parse(&compiled::start_position_json()).unwrap()
    }
    #[test]
    fn generated_frames_restore_and_played_history_discards_arrays() {
        if !ENABLED {
            return;
        }
        let mut g = root();
        let original = g.to_json();
        let root_key = g.context_key();
        let moves = g.moves();
        assert!(g.prepared.get().is_some());
        let first = moves[0];
        g.push(first);
        assert_eq!(g.cache_frames.len(), 1);
        let children = g.moves();
        assert!(g.prepared.get().is_some());
        if let Some(&m) = children.first() {
            g.push(m);
            g.moves();
            g.pop();
            assert_eq!(g.moves(), children);
        }
        g.pop();
        assert_eq!(g.context_key(), root_key);
        assert_eq!(g.to_json(), original);
        assert_eq!(g.moves(), moves);
        g.play(first).unwrap();
        assert_eq!(g.ancestor_bytes, 0);
        assert!(g.prepared.get().is_none());
        assert!(g.cache_frames.iter().all(|f| f.cache.get().is_none()));
        let cloned = g.clone();
        assert!(!Arc::ptr_eq(&g.owner, &cloned.owner));
        assert!(cloned.prepared.get().is_none());
        assert_eq!(cloned.to_json(), g.to_json());
    }
    #[test]
    fn foreign_and_stale_generation_hints_are_never_trusted() {
        if !ENABLED {
            return;
        }
        let mut a = root();
        let mut b = root();
        let moves = a.moves();
        let cache = a.prepared.take().unwrap();
        assert!(!b.cache_matches(&cache));
        b.prepared.set(cache).ok().unwrap();
        let mut expected = b.clone();
        expected.push_original(moves[0]);
        b.push(moves[0]);
        assert_eq!(b.position, expected.position);
        b.pop();
        a.moves();
        a.push(moves[0]);
        a.moves();
        let stale = a.prepared.take();
        a.pop();
        assert!(!a.cache_matches(&stale.unwrap()));
        assert_eq!(a.moves(), moves);
    }
    #[test]
    fn limited_cancelled_and_budget_exhausted_lists_do_not_cache() {
        if !ENABLED {
            return;
        }
        let mut g = root();
        let expected = g.moves();
        assert_eq!(
            g.generated_moves(1, &mut || false),
            enumeration_reference::enumerate(&g.position, g.prefix(), None, 1, &mut || false)
        );
        let mut calls = 0;
        assert!(
            g.generated_moves(usize::MAX, &mut || {
                calls += 1;
                calls > 3
            })
            .is_none()
        );
        assert_eq!(g.moves(), expected);
        g.prepared.take();
        assert_eq!(g.generated_moves(1, &mut || false).unwrap().len(), 1);
        assert!(g.prepared.get().is_none());
        assert!(g.generated_moves(usize::MAX, &mut || true).is_none());
        assert!(g.prepared.get().is_none());
        g.ancestor_bytes = CACHE_LIMIT;
        assert_eq!(g.moves(), expected);
        assert!(g.prepared.get().is_none());
        g.ancestor_bytes = 0;
        g.perft(1);
        assert!(g.prepared.get().is_none());
        assert!(g.cache_frames.is_empty());
        assert_eq!(g.ancestor_bytes, 0);
    }
}

#[cfg(test)]
mod corpus_tests {
    use super::*;
    /// Run explicitly with LARGE_CACHE_CORPUS pointing at the independently
    /// verified JSONL inputs; the ordinary suite needs no generated artifacts.
    #[test]
    #[ignore = "requires the frozen reference corpus"]
    fn every_cached_transition_matches_checked_application_and_restores() {
        if !ENABLED {
            return;
        }
        let path = std::env::var("LARGE_CACHE_CORPUS").expect("LARGE_CACHE_CORPUS JSONL path");
        let input = std::fs::read_to_string(path).unwrap();
        let mut positions = 0;
        let mut transitions = 0;
        for line in input.lines().filter(|l| !l.is_empty()) {
            let mut g = LargeGame::parse(line).unwrap();
            let root = g.to_json();
            let key = g.context_key();
            let context = g.context_hash();
            for m in g.moves() {
                let mut checked = g.clone();
                checked.play(m).unwrap();
                g.push(m);
                assert_eq!(g.position, checked.position);
                assert_eq!(g.to_json(), checked.to_json());
                assert_eq!(g.context_key(), checked.context_key());
                assert_eq!(g.context_hash(), checked.context_hash());
                g.pop();
                assert_eq!(g.to_json(), root);
                assert_eq!(g.context_key(), key);
                assert_eq!(g.context_hash(), context);
                transitions += 1;
            }
            positions += 1;
        }
        assert!(positions > 100);
        assert!(transitions > 1000);
        eprintln!("cached corpus: {positions} positions, {transitions} transitions");
    }
}

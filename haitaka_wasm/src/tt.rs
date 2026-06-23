use haitaka::{Color, Move, Piece, Square};

pub const DEFAULT_HASH_MB: usize = 16;
pub const MIN_HASH_MB: usize = 1;
pub const MAX_HASH_MB: usize = 1024;

const CLUSTER_SIZE: usize = 3;
const GENERATION_BITS: u8 = 3;
const GENERATION_DELTA: u8 = 1 << GENERATION_BITS;
const GENERATION_CYCLE: u16 = 255 + GENERATION_DELTA as u16;
const GENERATION_MASK: u16 = 0xF8;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchTtStats {
    pub tt_probes: u64,
    pub tt_hits: u64,
    pub tt_cutoffs: u64,
    pub tt_stores: u64,
    pub tt_collisions: u64,
    pub tt_hashfull: u32,
}

impl SearchTtStats {
    pub fn add_iteration(&mut self, iteration: Self) {
        self.tt_probes += iteration.tt_probes;
        self.tt_hits += iteration.tt_hits;
        self.tt_cutoffs += iteration.tt_cutoffs;
        self.tt_stores += iteration.tt_stores;
        self.tt_collisions += iteration.tt_collisions;
        self.tt_hashfull = iteration.tt_hashfull;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Bound {
    None = 0,
    Upper = 1,
    Lower = 2,
    Exact = 3,
}

impl Bound {
    const fn from_bits(bits: u8) -> Self {
        match bits & 0x3 {
            1 => Self::Upper,
            2 => Self::Lower,
            3 => Self::Exact,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct PackedMove(u16);

impl PackedMove {
    const NONE: Self = Self(0);
    const TO_MASK: u16 = 0x007F;
    const FROM_MASK: u16 = 0x3F80;
    const FROM_SHIFT: u16 = 7;
    const DROP_FLAG: u16 = 0x4000;
    const PROMOTION_FLAG: u16 = 0x8000;

    pub const fn none() -> Self {
        Self::NONE
    }

    pub fn from_move(mv: Move) -> Self {
        match mv {
            Move::Drop { piece, to } => {
                let piece_index = piece as u16;
                if piece_index >= Piece::HAND_NUM as u16 {
                    return Self::NONE;
                }
                Self((to as u16) | (piece_index << Self::FROM_SHIFT) | Self::DROP_FLAG)
            }
            Move::BoardMove {
                from,
                to,
                promotion,
            } => {
                let mut value = (to as u16) | ((from as u16) << Self::FROM_SHIFT);
                if promotion {
                    value |= Self::PROMOTION_FLAG;
                }
                Self(value)
            }
        }
    }

    pub fn to_move(self) -> Option<Move> {
        if self.0 == 0 {
            return None;
        }

        let to = Square::try_index((self.0 & Self::TO_MASK) as usize)?;
        let from_or_piece = ((self.0 & Self::FROM_MASK) >> Self::FROM_SHIFT) as usize;
        let is_drop = (self.0 & Self::DROP_FLAG) != 0;
        let is_promotion = (self.0 & Self::PROMOTION_FLAG) != 0;

        if is_drop {
            if is_promotion || from_or_piece >= Piece::HAND_NUM {
                return None;
            }
            let piece = Piece::try_index(from_or_piece)?;
            Some(Move::Drop { piece, to })
        } else {
            let from = Square::try_index(from_or_piece)?;
            Some(Move::BoardMove {
                from,
                to,
                promotion: is_promotion,
            })
        }
    }

    const fn raw(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TtData {
    pub score: i32,
    pub eval: i32,
    pub depth: u8,
    pub bound: Bound,
    pub best_move: Option<Move>,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct TtEntry {
    key16: u16,
    depth8: u8,
    gen_bound8: u8,
    move16: u16,
    score16: i16,
    eval16: i16,
}

const _: () = assert!(std::mem::size_of::<TtEntry>() == 10);

impl TtEntry {
    const fn empty() -> Self {
        Self {
            key16: 0,
            depth8: 0,
            gen_bound8: 0,
            move16: 0,
            score16: 0,
            eval16: 0,
        }
    }

    const fn is_occupied(self) -> bool {
        self.depth8 != 0
    }

    fn read(self) -> TtData {
        TtData {
            score: i32::from(self.score16),
            eval: i32::from(self.eval16),
            depth: self.depth8,
            bound: Bound::from_bits(self.gen_bound8),
            best_move: PackedMove(self.move16).to_move(),
        }
    }

    fn relative_age(self, generation: u8) -> u8 {
        let age = GENERATION_CYCLE
            .wrapping_add(u16::from(generation))
            .wrapping_sub(u16::from(self.gen_bound8));
        (age & GENERATION_MASK) as u8
    }

    fn save(
        &mut self,
        key: u64,
        score: i32,
        is_pv: bool,
        bound: Bound,
        depth: u8,
        best_move: Option<Move>,
        eval: i32,
        generation: u8,
    ) -> bool {
        let key16 = key as u16;
        let move16 = best_move
            .map(PackedMove::from_move)
            .unwrap_or_else(PackedMove::none)
            .raw();

        if move16 != 0 || key16 != self.key16 {
            self.move16 = move16;
        }

        let replace = bound == Bound::Exact
            || key16 != self.key16
            || i32::from(depth) + 2 * i32::from(is_pv) > i32::from(self.depth8) - 4
            || self.relative_age(generation) != 0;

        if !replace {
            return false;
        }

        debug_assert!(depth > 0);
        debug_assert!(i16::try_from(score).is_ok());
        debug_assert!(i16::try_from(eval).is_ok());

        self.key16 = key16;
        self.depth8 = depth;
        self.gen_bound8 = generation | ((is_pv as u8) << 2) | bound as u8;
        self.score16 = score as i16;
        self.eval16 = eval as i16;
        true
    }
}

impl Default for TtEntry {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Copy)]
#[repr(C, align(32))]
struct Cluster {
    entries: [TtEntry; CLUSTER_SIZE],
    padding: [u8; 2],
}

const _: () = assert!(std::mem::size_of::<Cluster>() == 32);

impl Cluster {
    const fn empty() -> Self {
        Self {
            entries: [TtEntry::empty(); CLUSTER_SIZE],
            padding: [0; 2],
        }
    }
}

impl Default for Cluster {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TtProbe {
    pub found: bool,
    pub data: Option<TtData>,
    index: usize,
    slot: usize,
}

pub struct TranspositionTable {
    clusters: Vec<Cluster>,
    generation: u8,
}

impl TranspositionTable {
    pub fn new(size_mb: usize) -> Self {
        let mut table = Self {
            clusters: Vec::new(),
            generation: 0,
        };
        table.resize(size_mb.max(MIN_HASH_MB).min(MAX_HASH_MB));
        table
    }

    pub fn resize(&mut self, size_mb: usize) {
        let size_mb = size_mb.max(MIN_HASH_MB).min(MAX_HASH_MB);
        let bytes = size_mb.saturating_mul(1024 * 1024);
        let cluster_count = (bytes / std::mem::size_of::<Cluster>()).max(2) & !1;
        self.clusters = vec![Cluster::empty(); cluster_count.max(2)];
        self.generation = 0;
    }

    pub fn clear(&mut self) {
        self.clusters.fill(Cluster::empty());
        self.generation = 0;
    }

    pub fn new_search(&mut self) {
        self.generation = self.generation.wrapping_add(GENERATION_DELTA);
    }

    pub fn probe(&mut self, key: u64, side_to_move: Color) -> TtProbe {
        let index = self.cluster_index(key, side_to_move);
        let key16 = key as u16;
        let cluster = &mut self.clusters[index];

        for slot in 0..CLUSTER_SIZE {
            let entry = &mut cluster.entries[slot];
            if entry.key16 == key16 {
                entry.gen_bound8 = self.generation | (entry.gen_bound8 & (GENERATION_DELTA - 1));
                return TtProbe {
                    found: entry.is_occupied(),
                    data: entry.is_occupied().then(|| entry.read()),
                    index,
                    slot,
                };
            }
        }

        let mut replace_slot = 0;
        let mut replace_value = i32::MAX;
        for slot in 0..CLUSTER_SIZE {
            let entry = cluster.entries[slot];
            let value = i32::from(entry.depth8) - i32::from(entry.relative_age(self.generation));
            if value < replace_value {
                replace_value = value;
                replace_slot = slot;
            }
        }

        TtProbe {
            found: false,
            data: None,
            index,
            slot: replace_slot,
        }
    }

    pub fn write(
        &mut self,
        probe: TtProbe,
        key: u64,
        score: i32,
        is_pv: bool,
        bound: Bound,
        depth: u8,
        best_move: Option<Move>,
        eval: i32,
    ) -> (bool, bool) {
        let entry = &mut self.clusters[probe.index].entries[probe.slot];
        let collision = entry.is_occupied() && entry.key16 != key as u16;
        let stored = entry.save(
            key,
            score,
            is_pv,
            bound,
            depth.max(1),
            best_move,
            eval,
            self.generation,
        );
        (stored, collision && stored)
    }

    pub fn hashfull(&self, max_age: u8) -> u32 {
        let max_age = max_age << GENERATION_BITS;
        let sample_count = self.clusters.len().min(1000);
        if sample_count == 0 {
            return 0;
        }

        let mut occupied = 0u32;
        for cluster in self.clusters.iter().take(sample_count) {
            for entry in &cluster.entries {
                if entry.is_occupied() && entry.relative_age(self.generation) <= max_age {
                    occupied += 1;
                }
            }
        }

        occupied / CLUSTER_SIZE as u32
    }

    fn cluster_index(&self, key: u64, side_to_move: Color) -> usize {
        debug_assert!(self.clusters.len() >= 2);
        let index = ((u128::from(key) * self.clusters.len() as u128) >> 64) as usize;
        (index & !1) | side_to_move as usize
    }
}

impl Default for TranspositionTable {
    fn default() -> Self {
        Self::new(DEFAULT_HASH_MB)
    }
}

pub fn validate_hash_size_mb(size_mb: u32) -> Result<usize, String> {
    let size_mb = size_mb as usize;
    if !(MIN_HASH_MB..=MAX_HASH_MB).contains(&size_mb) {
        return Err(format!(
            "Hash size must be between {MIN_HASH_MB} and {MAX_HASH_MB} MB"
        ));
    }
    Ok(size_mb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_board_moves_round_trip() {
        for mv in [
            Move::BoardMove {
                from: Square::A1,
                to: Square::I9,
                promotion: false,
            },
            Move::BoardMove {
                from: Square::I9,
                to: Square::A1,
                promotion: true,
            },
            Move::BoardMove {
                from: Square::E5,
                to: Square::E6,
                promotion: false,
            },
        ] {
            assert_eq!(PackedMove::from_move(mv).to_move(), Some(mv));
        }
    }

    #[test]
    fn packed_drops_round_trip() {
        for piece in Piece::ALL.into_iter().take(Piece::HAND_NUM) {
            for to in [Square::A1, Square::E5, Square::I9] {
                let mv = Move::Drop { piece, to };
                assert_eq!(PackedMove::from_move(mv).to_move(), Some(mv));
            }
        }
    }

    #[test]
    fn invalid_packed_moves_decode_to_none() {
        assert_eq!(PackedMove::none().to_move(), None);
        assert_eq!(
            PackedMove(PackedMove::DROP_FLAG | PackedMove::PROMOTION_FLAG).to_move(),
            None
        );
        assert_eq!(PackedMove(81).to_move(), None);
    }

    #[test]
    fn exact_bound_overwrites_shallower_entry() {
        let mut entry = TtEntry::empty();
        let key = 0x1234_5678_9abc_def0;
        assert!(entry.save(key, 10, false, Bound::Lower, 8, None, 0, 0));
        assert!(entry.save(key, 20, false, Bound::Exact, 1, None, 0, 0));
        let data = entry.read();
        assert_eq!(data.score, 20);
        assert_eq!(data.depth, 1);
        assert_eq!(data.bound, Bound::Exact);
    }

    #[test]
    fn same_key_empty_move_preserves_previous_best_move() {
        let mut entry = TtEntry::empty();
        let key = 0x1234;
        let mv = Move::Drop {
            piece: Piece::Pawn,
            to: Square::E5,
        };
        assert!(entry.save(key, 10, false, Bound::Exact, 4, Some(mv), 0, 0));
        assert!(entry.save(key, 20, false, Bound::Exact, 5, None, 0, 0));
        assert_eq!(entry.read().best_move, Some(mv));
    }

    #[test]
    fn different_key_replaces_and_reports_collision() {
        let mut tt = TranspositionTable::new(1);
        for key in [0, 1, 2] {
            let probe = tt.probe(key, Color::Black);
            let (stored, collision) = tt.write(probe, key, 10, false, Bound::Exact, 4, None, 0);
            assert!(stored);
            assert!(!collision);
        }

        let probe_b = tt.probe(3, Color::Black);
        let (stored, collision) = tt.write(probe_b, 3, 20, false, Bound::Exact, 4, None, 0);
        assert!(stored);
        assert!(collision);
    }

    #[test]
    fn generation_aging_affects_replacement_choice() {
        let mut entry = TtEntry::empty();
        let key = 0x1234;
        assert!(entry.save(key, 10, false, Bound::Lower, 8, None, 0, 0));
        assert!(!entry.save(key, 20, false, Bound::Lower, 1, None, 0, 0));
        assert!(entry.save(key, 30, false, Bound::Lower, 1, None, 0, GENERATION_DELTA));
        assert_eq!(entry.read().score, 30);
    }
}

//! Deterministic internal Zobrist keys, separate from the public FNV protocol.
//! Tokens are generated on demand, so no board-size x piece-count table is
//! allocated in Native or WASM. Internal PieceId includes promotion origin.
use super::*;

pub(super) const HISTORY_SEED: u64 = 0x6a09e667f3bcc909;

fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}
fn cell(i: usize, c: Option<Cell>) -> u64 {
    c.map_or(0, |c| {
        mix(0x8d12e9f5a763bc40
            ^ ((i as u64) << 17)
            ^ ((c.piece.0 as u64) << 1)
            ^ u64::from(c.color))
    })
}
fn side(color: u8) -> u64 {
    mix(0x49be63d792a015f0 ^ u64::from(color))
}
fn state(s: &TenjikuState) -> u64 {
    // Resident order affects freeze/burn priority. Active kings and resident
    // fire sets are derived from this ordered state and the board; no hidden
    // turn-start effect state survives a completed move.
    let mut h = mix(0xce7a392b41d605f8 ^ s.shitenno.len() as u64)
        ^ mix(0x17b9d3c042e568af ^ u64::from(s.senjishi));
    for (order, r) in s.shitenno.iter().enumerate() {
        h ^= mix(0xd506ab7e23189fc4
            ^ ((order as u64) << 32)
            ^ ((Pos::index(r.at).unwrap() as u64) << 1)
            ^ u64::from(r.color));
    }
    h
}
pub(super) fn full(p: &Pos) -> u64 {
    let domain = mix(0xa638e09d1274fb5c
        ^ ((RULE_ID as u64) << 32)
        ^ ((Geometry::WIDTH as u64) << 16)
        ^ Geometry::HEIGHT as u64);
    p.cells
        .iter()
        .enumerate()
        .fold(domain ^ side(p.side) ^ state(&p.state), |h, (i, &c)| {
            h ^ cell(i, c)
        })
}
pub(super) fn updated(mut key: u64, p: &Pos, delta: &Delta) -> u64 {
    // Delta retains each square's first value even if a multi-hop move changes
    // it repeatedly. XOR only its old/final identities after all turn effects.
    for &(i, old) in &delta.squares {
        key ^= cell(i, old) ^ cell(i, p.cells[i]);
    }
    key ^ side(delta.side) ^ side(p.side) ^ state(&delta.state) ^ state(&p.state)
}
pub(super) fn history(previous: u64, key: u64) -> u64 {
    previous.rotate_left(7) ^ key.wrapping_mul(0x9e3779b97f4a7c15)
}
#[derive(Clone, Copy, Debug)]
pub(super) struct TimelineEntry {
    pub key: u64,
    pub side: u8,
    pub context: u64,
}
impl TimelineEntry {
    pub fn root(p: &Pos) -> Self {
        let key = full(p);
        Self {
            key,
            side: p.side,
            context: history(HISTORY_SEED, key),
        }
    }
    pub fn next(previous: Self, key: u64, side: u8) -> Self {
        Self {
            key,
            side,
            context: history(previous.context, key),
        }
    }
}

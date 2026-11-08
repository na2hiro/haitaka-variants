use super::{Deserialize, PositionState, Serialize};
use serde::{Deserializer, Serializer};
use std::ops::Deref;
/// Fixed-size path codec retains both hops and returning-to-source moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BoundedPath<const N: usize> {
    steps: [[u16; 2]; N],
    len: u8,
}
pub type Path = BoundedPath<2>;
impl<const N: usize> BoundedPath<N> {
    pub fn from_steps(steps: &[[u16; 2]]) -> Option<Self> {
        if steps.is_empty() || steps.len() > N || steps.len() > 255 {
            return None;
        }
        let mut out = Self {
            steps: [[0, 0]; N],
            len: steps.len() as u8,
        };
        out.steps[..steps.len()].copy_from_slice(steps);
        Some(out)
    }
    pub fn one(to: [u16; 2]) -> Self {
        Self::from_steps(&[to]).unwrap()
    }
    pub fn two(first: [u16; 2], last: [u16; 2]) -> Self {
        Self::from_steps(&[first, last]).unwrap()
    }
}
impl<const N: usize> Deref for BoundedPath<N> {
    type Target = [[u16; 2]];
    fn deref(&self) -> &Self::Target {
        &self.steps[..self.len as usize]
    }
}
impl<const N: usize> Serialize for BoundedPath<N> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.deref().serialize(s)
    }
}
impl<'de, const N: usize> Deserialize<'de> for BoundedPath<N> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let steps = Vec::<[u16; 2]>::deserialize(d)?;
        Self::from_steps(&steps).ok_or_else(|| {
            serde::de::Error::custom(format!("path must contain 1..{N} destinations"))
        })
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Prefix {
    pub from: [u16; 2],
    pub via: [u16; 2],
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LionState {
    pub senjishi: bool,
}
impl PositionState for LionState {
    fn hash_bytes(&self, add: &mut impl FnMut(u8)) {
        add(u8::from(self.senjishi));
    }
    fn senjishi(&self) -> bool {
        self.senjishi
    }
    fn set_senjishi(&mut self, value: bool) {
        self.senjishi = value;
    }
}

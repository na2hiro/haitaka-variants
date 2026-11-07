use super::{Deserialize, PositionState, Serialize};
use serde::{Deserializer, Serializer};
use std::ops::Deref;
/// Fixed-size path codec retains both hops and returning-to-source moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Path {
    steps: [[u16; 2]; 2],
    len: u8,
}
impl Path {
    pub fn one(to: [u16; 2]) -> Self {
        Self {
            steps: [to, [0, 0]],
            len: 1,
        }
    }
    pub fn two(first: [u16; 2], last: [u16; 2]) -> Self {
        Self {
            steps: [first, last],
            len: 2,
        }
    }
}
impl Deref for Path {
    type Target = [[u16; 2]];
    fn deref(&self) -> &Self::Target {
        &self.steps[..self.len as usize]
    }
}
impl Serialize for Path {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.deref().serialize(s)
    }
}
impl<'de> Deserialize<'de> for Path {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let steps = Vec::<[u16; 2]>::deserialize(d)?;
        match steps.as_slice() {
            [a] => Ok(Self::one(*a)),
            [a, b] => Ok(Self::two(*a, *b)),
            _ => Err(serde::de::Error::custom(
                "path must contain one or two destinations",
            )),
        }
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

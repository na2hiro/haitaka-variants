// Each delta owns its storage; all changes remain in one contiguous slice.
use super::*;
type Change = (usize, Option<Cell>);

#[derive(Clone, Debug)]
pub(super) enum SquareChanges {
    Inline { cells: [Change; 2], len: usize },
    Spilled(Vec<Change>),
}
impl Default for SquareChanges {
    fn default() -> Self {
        Self::Inline {
            cells: [(0, None); 2],
            len: 0,
        }
    }
}
impl SquareChanges {
    pub fn iter(&self) -> std::slice::Iter<'_, Change> {
        match self {
            Self::Inline { cells, len } => cells[..*len].iter(),
            Self::Spilled(cells) => cells.iter(),
        }
    }
    pub fn push(&mut self, change: Change) {
        match self {
            Self::Inline { cells, len } if *len < cells.len() => {
                cells[*len] = change;
                *len += 1;
            }
            Self::Inline { cells, len } => {
                // Fire and transit captures can touch arbitrarily many squares.
                let mut spilled = Vec::with_capacity(4);
                spilled.extend_from_slice(&cells[..*len]);
                spilled.push(change);
                *self = Self::Spilled(spilled);
            }
            Self::Spilled(cells) => cells.push(change),
        }
    }
}

// Per-delta ownership; no shared scratch between recursive nodes.
use super::*;
type Change = (usize, Option<Cell>);

#[derive(Clone, Debug, Default)]
pub(super) struct SquareChanges {
    inline: [Change; 2],
    inline_len: usize,
    overflow: Vec<Change>,
}
impl SquareChanges {
    pub fn iter(&self) -> impl Iterator<Item = &Change> {
        self.inline[..self.inline_len]
            .iter()
            .chain(self.overflow.iter())
    }
    pub fn push(&mut self, change: Change) {
        if self.inline_len < self.inline.len() {
            self.inline[self.inline_len] = change;
            self.inline_len += 1;
        } else {
            // Through captures and fire effects are unbounded by hop count.
            self.overflow.push(change);
        }
    }
}

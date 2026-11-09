//! Dense board membership, and an adaptive index for large destination lists.
use super::*;
const WORDS: usize = Geometry::SQUARES.div_ceil(64);
pub(super) struct Marks {
    words: [u64; WORDS],
    cursor: usize,
}
impl Marks {
    pub fn new() -> Self {
        Self {
            words: [0; WORDS],
            cursor: 0,
        }
    }
    pub fn contains(&self, i: usize) -> bool {
        self.words[i / 64] & (1u64 << (i % 64)) != 0
    }
    pub fn insert(&mut self, i: usize) {
        self.words[i / 64] |= 1u64 << (i % 64);
        self.cursor = self.cursor.min(i / 64);
    }
    pub fn pop_first(&mut self) -> Option<usize> {
        while self.cursor < WORDS {
            let word = &mut self.words[self.cursor];
            if *word != 0 {
                let bit = word.trailing_zeros() as usize;
                *word &= *word - 1;
                return Some(self.cursor * 64 + bit);
            }
            self.cursor += 1;
        }
        None
    }
}
pub(super) struct RawTargets {
    rows: Vec<(usize, i8, bool)>,
    index: Option<Vec<u16>>,
}
impl RawTargets {
    pub fn new() -> Self {
        Self {
            rows: vec![],
            index: None,
        }
    }
    fn find(&self, to: usize) -> Option<usize> {
        match &self.index {
            Some(ix) => (ix[to] != u16::MAX).then_some(ix[to] as usize),
            None => self.rows.iter().position(|r| r.0 == to),
        }
    }
    pub fn contains(&self, to: usize) -> bool {
        self.find(to).is_some()
    }
    pub fn get_mut(&mut self, to: usize) -> Option<&mut (usize, i8, bool)> {
        let i = self.find(to)?;
        Some(&mut self.rows[i])
    }
    pub fn push(&mut self, row: (usize, i8, bool)) {
        let to = row.0;
        self.rows.push(row);
        if let Some(ix) = &mut self.index {
            ix[to] = (self.rows.len() - 1) as u16;
        } else if self.rows.len() == 32 {
            let mut ix = vec![u16::MAX; Geometry::SQUARES];
            for (i, r) in self.rows.iter().enumerate() {
                ix[r.0] = i as u16;
            }
            self.index = Some(ix);
        }
    }
}
impl IntoIterator for RawTargets {
    type Item = (usize, i8, bool);
    type IntoIter = std::vec::IntoIter<Self::Item>;
    fn into_iter(self) -> Self::IntoIter {
        self.rows.into_iter()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn marks_iterate_in_order_across_word_boundaries() {
        let mut m = Marks::new();
        let mut expected = vec![0, 63, 64, Geometry::SQUARES - 1];
        expected.sort_unstable();
        expected.dedup();
        for &i in expected.iter().rev() {
            m.insert(i);
            assert!(m.contains(i));
        }
        for i in expected {
            assert_eq!(m.pop_first(), Some(i));
            assert!(!m.contains(i));
        }
        assert_eq!(m.pop_first(), None);
    }
    #[test]
    fn destination_index_preserves_rows_and_merge_metadata() {
        let mut r = RawTargets::new();
        for i in 0..Geometry::SQUARES.min(40) {
            r.push((i, -2, false));
        }
        r.get_mut(10).unwrap().1 = 2;
        r.get_mut(10).unwrap().2 = true;
        assert!(r.contains(10));
        assert!(!r.contains(Geometry::SQUARES - 1));
        let rows: Vec<_> = r.into_iter().collect();
        assert_eq!(rows[10], (10, 2, true));
        assert_eq!(rows[20].0, 20);
    }
}

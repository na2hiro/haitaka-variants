//! Allocation-free square sets whose capacity and active length are static.
//!
//! `BITS` includes holes in a board's bounding rectangle. Geometry, rather than
//! storage, decides which of those squares are playable.

use core::ops::{BitAnd, BitOr, BitXor, Not, Shl, Shr};

/// Common operations needed by compile-time board geometries.
pub trait SquareSet: Copy + Eq + core::fmt::Debug {
    const BITS: usize;
    const EMPTY: Self;
    const FULL: Self;
    fn contains(self, index: usize) -> bool;
    fn with(self, index: usize) -> Self;
    fn without(self, index: usize) -> Self;
    /// Mutate one bit without copying a wide set through temporary values.
    fn insert(&mut self, index: usize) {
        *self = self.with(index);
    }
    fn remove(&mut self, index: usize) {
        *self = self.without(index);
    }
    fn pop_first(&mut self) -> Option<usize> {
        let i = self.first()?;
        self.remove(i);
        Some(i)
    }
    fn count(self) -> u32;
    fn first(self) -> Option<usize>;
    fn last(self) -> Option<usize>;
    fn intersection(self, rhs: Self) -> Self;
    fn union(self, rhs: Self) -> Self;
    fn symmetric_difference(self, rhs: Self) -> Self;
    fn complement(self) -> Self;
    /// Linear bit shifts. Geometric shifts must also mask row/file boundaries.
    fn shift_left(self, amount: usize) -> Self;
    fn shift_right(self, amount: usize) -> Self;
}

/// Fixed-size multiword bitboard, with zeroed unused bits in the last word.
///
/// Both parameters are explicit to work on stable Rust without generic const
/// expressions. For a 36x36 board use `MultiBitBoard<21, 1296>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct MultiBitBoard<const WORDS: usize, const BITS: usize>([u64; WORDS]);

impl<const WORDS: usize, const BITS: usize> MultiBitBoard<WORDS, BITS> {
    const VALID: () = assert!(BITS > 0 && WORDS == BITS.div_ceil(64));
    pub const EMPTY: Self = {
        let () = Self::VALID;
        Self([0; WORDS])
    };
    pub const FULL: Self = Self::new([u64::MAX; WORDS]);

    pub const fn new(mut words: [u64; WORDS]) -> Self {
        let () = Self::VALID;
        if !BITS.is_multiple_of(64) {
            words[WORDS - 1] &= (1u64 << (BITS % 64)) - 1;
        }
        Self(words)
    }

    pub const fn words(&self) -> &[u64; WORDS] {
        &self.0
    }

    pub const fn contains(self, index: usize) -> bool {
        index < BITS && self.0[index / 64] & (1u64 << (index % 64)) != 0
    }

    pub const fn with(mut self, index: usize) -> Self {
        assert!(index < BITS, "square index outside bitboard");
        self.0[index / 64] |= 1u64 << (index % 64);
        self
    }

    pub const fn without(mut self, index: usize) -> Self {
        assert!(index < BITS, "square index outside bitboard");
        self.0[index / 64] &= !(1u64 << (index % 64));
        self
    }

    pub const fn count(self) -> u32 {
        let mut n = 0;
        let mut i = 0;
        while i < WORDS {
            n += self.0[i].count_ones();
            i += 1;
        }
        n
    }

    pub const fn first(self) -> Option<usize> {
        let mut i = 0;
        while i < WORDS {
            if self.0[i] != 0 {
                return Some(i * 64 + self.0[i].trailing_zeros() as usize);
            }
            i += 1;
        }
        None
    }

    pub const fn last(self) -> Option<usize> {
        let mut i = WORDS;
        while i > 0 {
            i -= 1;
            if self.0[i] != 0 {
                return Some(i * 64 + 63 - self.0[i].leading_zeros() as usize);
            }
        }
        None
    }

    pub const fn intersection(self, rhs: Self) -> Self {
        let mut out = [0; WORDS];
        let mut i = 0;
        while i < WORDS {
            out[i] = self.0[i] & rhs.0[i];
            i += 1;
        }
        Self(out)
    }
    pub const fn union(self, rhs: Self) -> Self {
        let mut out = [0; WORDS];
        let mut i = 0;
        while i < WORDS {
            out[i] = self.0[i] | rhs.0[i];
            i += 1;
        }
        Self(out)
    }
    pub const fn symmetric_difference(self, rhs: Self) -> Self {
        let mut out = [0; WORDS];
        let mut i = 0;
        while i < WORDS {
            out[i] = self.0[i] ^ rhs.0[i];
            i += 1;
        }
        Self(out)
    }
    pub const fn complement(self) -> Self {
        let mut out = [0; WORDS];
        let mut i = 0;
        while i < WORDS {
            out[i] = !self.0[i];
            i += 1;
        }
        Self::new(out)
    }

    pub const fn shift_left(self, amount: usize) -> Self {
        if amount >= BITS {
            return Self::EMPTY;
        }
        let whole = amount / 64;
        let bits = amount % 64;
        let mut out = [0; WORDS];
        let mut i = whole;
        while i < WORDS {
            out[i] = self.0[i - whole] << bits;
            if bits != 0 && i > whole {
                out[i] |= self.0[i - whole - 1] >> (64 - bits);
            }
            i += 1;
        }
        Self::new(out)
    }

    pub const fn shift_right(self, amount: usize) -> Self {
        if amount >= BITS {
            return Self::EMPTY;
        }
        let whole = amount / 64;
        let bits = amount % 64;
        let mut out = [0; WORDS];
        let mut i = 0;
        while i + whole < WORDS {
            out[i] = self.0[i + whole] >> bits;
            if bits != 0 && i + whole + 1 < WORDS {
                out[i] |= self.0[i + whole + 1] << (64 - bits);
            }
            i += 1;
        }
        Self::new(out)
    }

    /// Mirror the active bit sequence, rather than the padded capacity.
    pub const fn reversed(self) -> Self {
        let mut out = Self::EMPTY;
        let mut i = 0;
        while i < BITS {
            if self.contains(i) {
                out = out.with(BITS - 1 - i);
            }
            i += 1;
        }
        out
    }
}

impl<const W: usize, const B: usize> Default for MultiBitBoard<W, B> {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl<const W: usize, const B: usize> SquareSet for MultiBitBoard<W, B> {
    const BITS: usize = B;
    const EMPTY: Self = Self::EMPTY;
    const FULL: Self = Self::FULL;
    fn contains(self, i: usize) -> bool {
        self.contains(i)
    }
    fn with(self, i: usize) -> Self {
        self.with(i)
    }
    fn without(self, i: usize) -> Self {
        self.without(i)
    }
    fn insert(&mut self, index: usize) {
        if W <= 4 {
            *self = self.with(index);
            return;
        }
        assert!(index < B, "square index outside bitboard");
        self.0[index / 64] |= 1u64 << (index % 64);
    }
    fn remove(&mut self, index: usize) {
        if W <= 4 {
            *self = self.without(index);
            return;
        }
        assert!(index < B, "square index outside bitboard");
        self.0[index / 64] &= !(1u64 << (index % 64));
    }
    fn pop_first(&mut self) -> Option<usize> {
        if W <= 4 {
            let i = self.first()?;
            *self = self.without(i);
            return Some(i);
        }
        for (word_index, word) in self.0.iter_mut().enumerate() {
            if *word != 0 {
                let bit = word.trailing_zeros() as usize;
                *word &= *word - 1;
                return Some(word_index * 64 + bit);
            }
        }
        None
    }
    fn count(self) -> u32 {
        self.count()
    }
    fn first(self) -> Option<usize> {
        self.first()
    }
    fn last(self) -> Option<usize> {
        self.last()
    }
    fn intersection(self, r: Self) -> Self {
        self.intersection(r)
    }
    fn union(self, r: Self) -> Self {
        self.union(r)
    }
    fn symmetric_difference(self, r: Self) -> Self {
        self.symmetric_difference(r)
    }
    fn complement(self) -> Self {
        self.complement()
    }
    fn shift_left(self, n: usize) -> Self {
        self.shift_left(n)
    }
    fn shift_right(self, n: usize) -> Self {
        self.shift_right(n)
    }
}

macro_rules! binary_op {
    ($trait:ident, $method:ident, $inherent:ident) => {
        impl<const W: usize, const B: usize> $trait for MultiBitBoard<W, B> {
            type Output = Self;
            fn $method(self, rhs: Self) -> Self {
                self.$inherent(rhs)
            }
        }
    };
}
binary_op!(BitAnd, bitand, intersection);
binary_op!(BitOr, bitor, union);
binary_op!(BitXor, bitxor, symmetric_difference);
impl<const W: usize, const B: usize> Not for MultiBitBoard<W, B> {
    type Output = Self;
    fn not(self) -> Self {
        self.complement()
    }
}
impl<const W: usize, const B: usize> Shl<usize> for MultiBitBoard<W, B> {
    type Output = Self;
    fn shl(self, n: usize) -> Self {
        self.shift_left(n)
    }
}
impl<const W: usize, const B: usize> Shr<usize> for MultiBitBoard<W, B> {
    type Output = Self;
    fn shr(self, n: usize) -> Self {
        self.shift_right(n)
    }
}

/// Native scalar backend with the same active-bit invariant as multiword sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct ScalarBitBoard<T, const BITS: usize>(T);

macro_rules! scalar {
    ($word:ty) => {
        impl<const B: usize> ScalarBitBoard<$word, B> {
            const MASK: $word = {
                assert!(B > 0 && B <= <$word>::BITS as usize);
                <$word>::MAX >> (<$word>::BITS as usize - B)
            };
            pub const fn new(value: $word) -> Self {
                Self(value & Self::MASK)
            }
            pub const fn value(self) -> $word {
                self.0
            }
        }
        impl<const B: usize> SquareSet for ScalarBitBoard<$word, B> {
            const BITS: usize = B;
            const EMPTY: Self = Self::new(0);
            const FULL: Self = Self::new(<$word>::MAX);
            fn contains(self, i: usize) -> bool {
                i < B && self.0 & (1 << i) != 0
            }
            fn with(self, i: usize) -> Self {
                assert!(i < B);
                Self(self.0 | (1 << i))
            }
            fn without(self, i: usize) -> Self {
                assert!(i < B);
                Self(self.0 & !(1 << i))
            }
            fn count(self) -> u32 {
                self.0.count_ones()
            }
            fn first(self) -> Option<usize> {
                (self.0 != 0).then(|| self.0.trailing_zeros() as usize)
            }
            fn last(self) -> Option<usize> {
                (self.0 != 0).then(|| <$word>::BITS as usize - 1 - self.0.leading_zeros() as usize)
            }
            fn intersection(self, r: Self) -> Self {
                Self(self.0 & r.0)
            }
            fn union(self, r: Self) -> Self {
                Self(self.0 | r.0)
            }
            fn symmetric_difference(self, r: Self) -> Self {
                Self(self.0 ^ r.0)
            }
            fn complement(self) -> Self {
                Self::new(!self.0)
            }
            fn shift_left(self, n: usize) -> Self {
                if n >= B {
                    Self::EMPTY
                } else {
                    Self::new(self.0 << n)
                }
            }
            fn shift_right(self, n: usize) -> Self {
                if n >= B {
                    Self::EMPTY
                } else {
                    Self(self.0 >> n)
                }
            }
        }
    };
}
scalar!(u64);
scalar!(u128);

#[cfg(test)]
mod mutation_tests {
    use super::*;
    fn check<S: SquareSet>() {
        let mut mutable = S::EMPTY;
        let mut value = S::EMPTY;
        for i in 0..S::BITS {
            mutable.insert(i);
            value = value.with(i);
            assert_eq!(mutable, value);
        }
        for i in (0..S::BITS).step_by(3) {
            mutable.remove(i);
            value = value.without(i);
            assert_eq!(mutable, value);
        }
        while let Some(i) = value.first() {
            assert_eq!(mutable.pop_first(), Some(i));
            value = value.without(i);
            assert_eq!(mutable, value);
        }
        assert_eq!(mutable.pop_first(), None);
        assert_eq!(mutable, S::EMPTY);
    }
    #[test]
    fn mutable_operations_match_values_and_preserve_padding() {
        check::<ScalarBitBoard<u64, 12>>();
        check::<ScalarBitBoard<u64, 64>>();
        check::<ScalarBitBoard<u128, 72>>();
        check::<MultiBitBoard<3, 169>>();
        check::<MultiBitBoard<4, 256>>();
        check::<MultiBitBoard<21, 1296>>();
    }
}

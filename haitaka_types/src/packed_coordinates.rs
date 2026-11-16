//! Compile-time coordinate encodings for transposition-table moves.
//!
//! These encode single-step moves and drops. A future multi-step rule needs its
//! own move representation rather than truncating a path to these two squares.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveCoordinates {
    pub from_or_piece: u16,
    pub to: u16,
    pub drop: bool,
    pub promotion: bool,
}

pub trait CoordinateCode: Copy {
    type Raw: Copy + Eq + Default;
    const NONE: Self;
    const MAX_SQUARES: usize;
    fn pack(fields: MoveCoordinates) -> Option<Self>;
    fn from_raw(raw: Self::Raw) -> Self;
    fn raw(self) -> Self::Raw;
    fn unpack(self) -> Option<MoveCoordinates>;
}

macro_rules! encoding {
    ($name:ident, $raw:ty, $bits:literal) => {
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
        #[repr(transparent)]
        pub struct $name($raw);
        impl CoordinateCode for $name {
            type Raw = $raw;
            const NONE: Self = Self(0);
            const MAX_SQUARES: usize = 1 << $bits;
            #[inline(always)]
            fn pack(f: MoveCoordinates) -> Option<Self> {
                if f.to as usize >= Self::MAX_SQUARES
                    || f.from_or_piece as usize >= Self::MAX_SQUARES
                    || (f.drop && f.promotion)
                    || (!f.drop && f.from_or_piece == f.to)
                {
                    return None;
                }
                Some(Self(
                    f.to as $raw
                        | ((f.from_or_piece as $raw) << $bits)
                        | ((f.drop as $raw) << (2 * $bits))
                        | ((f.promotion as $raw) << (2 * $bits + 1)),
                ))
            }
            #[inline(always)]
            fn from_raw(raw: Self::Raw) -> Self {
                Self(raw)
            }
            #[inline(always)]
            fn raw(self) -> Self::Raw {
                self.0
            }
            #[inline(always)]
            fn unpack(self) -> Option<MoveCoordinates> {
                if self.0 == 0 {
                    return None;
                }
                let mask: $raw = (1 << $bits) - 1;
                let fields = MoveCoordinates {
                    to: (self.0 & mask) as u16,
                    from_or_piece: ((self.0 >> $bits) & mask) as u16,
                    drop: self.0 & (1 << (2 * $bits)) != 0,
                    promotion: self.0 & (1 << (2 * $bits + 1)) != 0,
                };
                Self::pack(fields).map(|_| fields)
            }
        }
    };
}
encoding!(PackedCoordinates16, u16, 7);
encoding!(PackedCoordinates32, u32, 15);

/// Each storage backend selects an encoding that cannot truncate its squares.
pub trait GeometryMoveCode {
    type Code: CoordinateCode;
}
impl<const B: usize> GeometryMoveCode for crate::fixed_bitboard::ScalarBitBoard<u64, B> {
    type Code = PackedCoordinates16;
}
impl<const B: usize> GeometryMoveCode for crate::fixed_bitboard::ScalarBitBoard<u128, B> {
    type Code = PackedCoordinates16;
}
impl<const W: usize, const B: usize> GeometryMoveCode
    for crate::fixed_bitboard::MultiBitBoard<W, B>
{
    type Code = PackedCoordinates32;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wide_moves_preserve_large_coordinates_and_validate_bounds() {
        for f in [
            MoveCoordinates {
                from_or_piece: 1295,
                to: 1280,
                drop: false,
                promotion: true,
            },
            MoveCoordinates {
                from_or_piece: 255,
                to: 1295,
                drop: true,
                promotion: false,
            },
        ] {
            assert_eq!(PackedCoordinates32::pack(f).unwrap().unpack(), Some(f));
            assert!(PackedCoordinates16::pack(f).is_none());
        }
        assert!(PackedCoordinates32::NONE.unpack().is_none());
        assert!(
            PackedCoordinates32::pack(MoveCoordinates {
                from_or_piece: u16::MAX,
                to: 1,
                drop: false,
                promotion: false
            })
            .is_none()
        );
    }
}

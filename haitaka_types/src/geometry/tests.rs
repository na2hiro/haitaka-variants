extern crate std;
use super::*;
use core::mem::size_of;

fn verify_set<S: SquareSet>() {
    assert_eq!(S::FULL.count(), S::BITS as u32);
    assert_eq!(S::FULL.complement(), S::EMPTY);
    let mut seed = 0x123456789abcdefu64;
    for _ in 0..16 {
        let mut a = S::EMPTY;
        let mut b = S::EMPTY;
        let mut expected_a = [false; 1296];
        let mut expected_b = [false; 1296];
        for i in 0..S::BITS {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            expected_a[i] = seed & 1 != 0;
            expected_b[i] = seed & 2 != 0;
            if expected_a[i] {
                a = a.with(i);
            }
            if expected_b[i] {
                b = b.with(i);
            }
        }
        assert_eq!(
            a.count() as usize,
            expected_a[..S::BITS].iter().filter(|&&v| v).count()
        );
        assert_eq!(a.first(), expected_a[..S::BITS].iter().position(|&v| v));
        assert_eq!(a.last(), expected_a[..S::BITS].iter().rposition(|&v| v));
        for i in 0..S::BITS {
            assert_eq!(
                a.intersection(b).contains(i),
                expected_a[i] && expected_b[i]
            );
            assert_eq!(a.union(b).contains(i), expected_a[i] || expected_b[i]);
            assert_eq!(
                a.symmetric_difference(b).contains(i),
                expected_a[i] != expected_b[i]
            );
            assert_eq!(a.complement().contains(i), !expected_a[i]);
        }
        for shift in [
            0,
            1,
            63,
            64,
            65,
            127,
            128,
            S::BITS - 1,
            S::BITS,
            S::BITS + 1,
            usize::MAX,
        ] {
            let left = a.shift_left(shift);
            let right = a.shift_right(shift);
            for i in 0..S::BITS {
                assert_eq!(
                    left.contains(i),
                    i.checked_sub(shift).is_some_and(|j| expected_a[j])
                );
                assert_eq!(
                    right.contains(i),
                    i.checked_add(shift)
                        .filter(|&j| j < S::BITS)
                        .is_some_and(|j| expected_a[j])
                );
            }
        }
    }
}

fn verify_geometry<G: BoardGeometry>() {
    verify_set::<G::Bits>();
    for i in 0..G::SQUARES {
        let (f, r) = G::coordinates(i).unwrap();
        assert_eq!(G::index(f, r), Some(i));
        assert_eq!(G::flip(G::flip(i).unwrap()), Some(i));
        for dx in -1..=1 {
            for dy in -1..=1 {
                let expected = if f as isize + dx < 0
                    || r as isize + dy < 0
                    || f as isize + dx >= G::WIDTH as isize
                    || r as isize + dy >= G::HEIGHT as isize
                {
                    None
                } else {
                    Some((f as isize + dx) as usize * G::HEIGHT + (r as isize + dy) as usize)
                };
                assert_eq!(G::offset(i, dx, dy), expected);
            }
        }
    }
    assert_eq!(G::coordinates(G::SQUARES), None);
    assert_eq!(G::index(usize::MAX, usize::MAX), None);
    let corner = G::Bits::EMPTY.with(G::SQUARES - 1);
    assert_eq!(G::shifted(corner, 1, 0).count(), 0);
    assert_eq!(G::shifted(corner, 0, 1).count(), 0);
    let occupied = G::Bits::EMPTY.with(2);
    assert_eq!(G::ray(0, 0, 1, occupied, G::Bits::FULL).count(), 2);
    let playable = G::Bits::FULL.without(1);
    assert_eq!(G::ray(0, 0, 1, G::Bits::EMPTY, playable).count(), 0);
}

#[test]
fn every_shogitter_rectangle_roundtrips_and_matches_reference_sets() {
    macro_rules! check { ($($w:literal, $h:literal);* $(;)?) => { $(verify_geometry::<Rect<$w, $h>>();)* }; }
    check!(3,4; 4,5; 5,5; 5,6; 6,6; 7,6; 7,7; 9,7; 8,8; 9,8; 9,9; 9,10; 9,11;
        10,10; 11,11; 12,12; 13,13; 15,15; 16,16; 17,17; 19,19; 25,25; 36,36);
}

#[test]
fn native_word_boundaries_and_large_padding_are_masked() {
    verify_set::<ScalarBitBoard<u128, 128>>();
    verify_set::<MultiBitBoard<1, 64>>();
    verify_set::<MultiBitBoard<2, 65>>();
    verify_set::<MultiBitBoard<2, 128>>();
    let set = MultiBitBoard::<21, 1296>::new([u64::MAX; 21]);
    assert_eq!(set.count(), 1296);
    assert_eq!(set.words()[20], 0xffff);
    assert_eq!(set.reversed(), set);
    assert_eq!(
        MultiBitBoard::<21, 1296>::EMPTY.with(0).reversed().first(),
        Some(1295)
    );
}

#[test]
fn storage_and_index_sizes_follow_geometry() {
    assert_eq!(size_of::<<Rect<5, 5> as BoardGeometry>::Bits>(), 8);
    assert_eq!(size_of::<<Rect<9, 9> as BoardGeometry>::Bits>(), 16);
    assert_eq!(size_of::<<Rect<12, 12> as BoardGeometry>::Bits>(), 24);
    assert_eq!(size_of::<<Rect<36, 36> as BoardGeometry>::Bits>(), 168);
    assert_eq!(size_of::<<Rect<16, 16> as BoardGeometry>::Index>(), 1);
    assert_eq!(size_of::<<Rect<17, 17> as BoardGeometry>::Index>(), 2);
}

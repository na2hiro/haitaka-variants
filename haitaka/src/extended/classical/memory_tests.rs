use super::*;
fn position(extra: &[([u16; 2], &str, u8)]) -> Position<Daishogi> {
    let mut pieces = vec![
        ClassicalPiece::make([15, 15], "ah", 0, None),
        ClassicalPiece::make([15, 1], "ah", 1, None),
    ];
    pieces.extend(
        extra
            .iter()
            .map(|&(at, id, color)| ClassicalPiece::make(at, id, color, None)),
    );
    Position::from_wire(&WirePosition {
        side_to_move: 0,
        pieces,
        hands: Default::default(),
        state: Default::default(),
    })
    .unwrap()
}
#[test]
fn copy_free_validation_keeps_preexisting_file_excess_and_promotion_rules() {
    let p = position(&[([3, 5], "aa", 0), ([3, 7], "aa", 0)]);
    let moves = p.classical_moves(usize::MAX, &mut || false).unwrap();
    let plain = ClassicalMove::board([3, 5], Path::one([3, 4]), false);
    let promoted = ClassicalMove::board([3, 5], Path::one([3, 4]), true);
    assert!(!moves.contains(&plain));
    assert!(moves.contains(&promoted));
    assert_eq!(p.classical_acceptable(plain, &mut || false), Some(false));
    assert_eq!(p.classical_acceptable(promoted, &mut || false), Some(true));
}
#[test]
fn copy_free_validation_preserves_returning_lion_paths_and_cancellation() {
    let p = position(&[([8, 8], "bl", 0), ([9, 8], "aa", 1)]);
    let m = ClassicalMove::board([8, 8], Path::two([9, 8], [8, 8]), false);
    assert!(
        p.classical_moves(usize::MAX, &mut || false)
            .unwrap()
            .contains(&m)
    );
    assert_eq!(
        p.classical_acceptable(m, &mut || false),
        p.classical_acceptable_copy(m, &mut || false)
    );
    let mut calls = 0;
    assert!(
        p.classical_moves(usize::MAX, &mut || {
            calls += 1;
            calls > 10
        })
        .is_none()
    );
}
#[test]
fn check_free_candidates_match_copy_oracle_through_fixed_seed_game() {
    let p = Position::<Daishogi>::from_wire(&WirePosition {
        side_to_move: 0,
        pieces: Daishogi::initial(),
        hands: Default::default(),
        state: Default::default(),
    })
    .unwrap();
    let mut g = ClassicalGame::<Daishogi>::root(p);
    let initial = g.to_json();
    let mut seed = 0x49c04a19u32;
    for _ in 0..256 {
        let before = g.current().clone();
        let moves = g.moves();
        assert_eq!(g.current(), &before);
        if moves.is_empty() {
            break;
        }
        for &m in moves.iter().take(32) {
            assert_eq!(
                before.classical_acceptable(m, &mut || false),
                before.classical_acceptable_copy(m, &mut || false)
            );
        }
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        g.push(moves[seed as usize % moves.len()]);
    }
    while !g.played.is_empty() {
        g.pop();
    }
    assert_eq!(g.to_json(), initial);
}

use super::*;
fn root<R: ClassicalRule>(pieces: &[([u16; 2], &str, u8)]) -> ClassicalGame<R> {
    ClassicalGame::root(
        Position::from_wire(&WirePosition {
            side_to_move: 0,
            pieces: pieces
                .iter()
                .map(|&(at, id, color)| ClassicalPiece::make(at, id, color, None))
                .collect(),
            hands: Default::default(),
            state: Default::default(),
        })
        .unwrap(),
    )
}
fn board<R: ClassicalRule>(from: [u16; 2], to: [u16; 2], promotion: bool) -> ClassicalMove<R> {
    ClassicalMove::board(from, Path::one(to), promotion)
}
fn species<R: ClassicalRule>(g: &ClassicalGame<R>, at: [u16; 2]) -> &str {
    Position::<R>::def(g.current().cells[Position::<R>::index(at).unwrap()].unwrap()).id
}
#[test]
fn kyoto_and_kinkei_flip_each_board_move_but_not_kings() {
    let mut g = root::<Kyoto>(&[([5, 5], "ah", 0), ([1, 1], "ah", 1), ([3, 4], "ad", 0)]);
    assert!(!g.moves().contains(&board([3, 4], [2, 3], false)));
    g.play(board([3, 4], [2, 3], true)).unwrap();
    assert_eq!(species(&g, [2, 3]), "af");
    let mut g = root::<KinkeiMai>(&[([5, 5], "ah", 0), ([1, 1], "ah", 1), ([3, 4], "ae", 0)]);
    g.play(board([3, 4], [3, 3], true)).unwrap();
    assert_eq!(species(&g, [3, 3]), "ac");
    assert!(g.moves().iter().any(|m| matches!(
        m,
        ClassicalMove::Move {
            promotion: false,
            ..
        }
    )));
    assert!(!g.moves().iter().any(|m| matches!(
        m,
        ClassicalMove::Move {
            promotion: true,
            ..
        }
    )));
}
#[test]
fn capture_stores_one_physical_pair_and_either_face_can_be_dropped_without_flipping() {
    let mut g = root::<Kyoto>(&[
        ([5, 5], "ah", 0),
        ([1, 1], "ah", 1),
        ([3, 4], "ae", 0),
        ([3, 3], "ac", 1),
    ]);
    g.play(board([3, 4], [3, 3], true)).unwrap();
    assert_eq!(
        g.current().to_wire().hands[0],
        BTreeMap::from([("ae".into(), 1)])
    );
    g.play(board([1, 1], [1, 2], false)).unwrap();
    for (id, expected) in [(PieceId(4), "ae"), (PieceId(5), "ac")] {
        let mut next = g.clone();
        let mv = ClassicalMove::drop(id, [2, 5]);
        let json = serde_json::to_string(&mv).unwrap();
        assert_eq!(
            serde_json::from_str::<ClassicalMove<Kyoto>>(&json).unwrap(),
            mv
        );
        next.play(mv).unwrap();
        assert_eq!(species(&next, [2, 5]), expected);
        assert!(next.current().to_wire().hands[0].is_empty());
    }
    let mut wire = g.current().to_wire();
    wire.hands[0] = BTreeMap::from([("ac".into(), 1)]);
    assert!(Position::<Kyoto>::from_wire(&wire).is_err());
}
#[test]
fn flipped_position_is_used_for_checks_and_self_check_is_rejected() {
    let g = root::<Kyoto>(&[
        ([3, 5], "ah", 0),
        ([1, 1], "ah", 1),
        ([3, 4], "ad", 0),
        ([3, 1], "ag", 1),
    ]);
    assert!(!g.moves().contains(&board([3, 4], [2, 3], true)));
    let mut g = root::<Kyoto>(&[
        ([5, 5], "ah", 0),
        ([2, 1], "ah", 1),
        ([3, 4], "ae", 0),
        ([3, 3], "ad", 1),
    ]);
    g.play(board([3, 4], [3, 3], true)).unwrap();
    assert!(g.current().attacked(1));
}
#[test]
fn micro_only_flips_on_capture_and_dead_rank_restriction_only_applies_to_drops() {
    for capture in [false, true] {
        let mut pieces = vec![([1, 5], "ah", 0), ([4, 1], "ah", 1), ([3, 4], "ad", 0)];
        if capture {
            pieces.push(([2, 3], "ae", 1));
        }
        let mut g = root::<Microshogi>(&pieces);
        assert!(!g.moves().contains(&board([3, 4], [2, 3], !capture)));
        g.play(board([3, 4], [2, 3], capture)).unwrap();
        assert_eq!(species(&g, [2, 3]), if capture { "ab" } else { "ad" });
    }
    for capture in [false, true] {
        let mut pieces = vec![([4, 5], "ah", 0), ([4, 1], "ah", 1), ([1, 2], "aa", 0)];
        if capture {
            pieces.push(([1, 1], "ad", 1));
        }
        let mut g = root::<Microshogi>(&pieces);
        g.play(board([1, 2], [1, 1], capture)).unwrap();
        assert_eq!(species(&g, [1, 1]), if capture { "ac" } else { "aa" });
        assert_eq!(
            ClassicalGame::<Microshogi>::parse(&g.to_json())
                .unwrap()
                .to_json(),
            g.to_json()
        );
    }
}
#[test]
fn mixed_rule_follows_species_even_after_ownership_changes_and_uses_global_inventory() {
    let mut g = root::<KyotoMinishogi>(&[([5, 5], "ah", 0), ([1, 1], "ah", 1), ([3, 4], "ad", 0)]);
    g.play(board([3, 4], [2, 3], false)).unwrap();
    assert_eq!(species(&g, [2, 3]), "ad");
    let mut g = root::<KyotoMinishogi>(&[([5, 5], "ah", 0), ([1, 1], "ah", 1), ([3, 4], "0o", 0)]);
    assert!(!g.moves().contains(&board([3, 4], [2, 3], false)));
    g.play(board([3, 4], [2, 3], true)).unwrap();
    assert_eq!(species(&g, [2, 3]), "0q");
    let mut wire = WirePosition {
        side_to_move: 0,
        pieces: KyotoMinishogi::initial(),
        hands: Default::default(),
        state: ClassicalState::default(),
    };
    wire.pieces
        .push(ClassicalPiece::make([3, 3], "ag", 0, None));
    assert!(
        Position::<KyotoMinishogi>::from_wire(&wire)
            .unwrap_err()
            .contains("inventory")
    );
}
fn walks<R: ClassicalRule>() {
    let mut g = ClassicalGame::<R>::root(
        Position::from_wire(&WirePosition {
            side_to_move: 0,
            pieces: R::initial(),
            hands: Default::default(),
            state: Default::default(),
        })
        .unwrap(),
    );
    let start = g.to_json();
    let mut seed = 0x552319u32;
    for _ in 0..64 {
        let legal = g.moves();
        if legal.is_empty() {
            break;
        }
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let mv = legal[seed as usize % legal.len()];
        let before = g.to_json();
        let hash = g.position_hash();
        g.play(mv).unwrap();
        assert_eq!(
            ClassicalGame::<R>::parse(&g.to_json()).unwrap().to_json(),
            g.to_json()
        );
        SearchGame::pop(&mut g);
        assert_eq!(g.to_json(), before);
        assert_eq!(g.position_hash(), hash);
        g.play(mv).unwrap();
    }
    while !g.played.is_empty() {
        SearchGame::pop(&mut g);
    }
    assert_eq!(g.to_json(), start);
}
#[test]
fn complete_histories_physical_hands_and_undo_round_trip_for_all_four_rules() {
    walks::<Kyoto>();
    walks::<KinkeiMai>();
    walks::<Microshogi>();
    walks::<KyotoMinishogi>();
}

#[test]
fn nonchecking_no_move_position_remains_ongoing_as_in_shogitter() {
    let g = root::<KinkeiMai>(&[
        ([1, 1], "ac", 1),
        ([1, 4], "ac", 1),
        ([1, 5], "ae", 1),
        ([2, 3], "ae", 1),
        ([2, 4], "ac", 1),
        ([2, 5], "ac", 1),
        ([3, 4], "ac", 1),
        ([3, 5], "ah", 0),
        ([5, 1], "ac", 1),
        ([5, 4], "ah", 1),
    ]);
    assert!(!g.current().attacked(0));
    assert!(g.moves().is_empty());
    assert_eq!(g.outcome(), Outcome::ONGOING);
}

#[test]
fn micro_rejects_dead_face_drops_but_accepts_the_other_face() {
    let mut g = root::<Microshogi>(&[([4, 5], "ah", 0), ([4, 1], "ah", 1)]);
    let mut wire = g.current().to_wire();
    wire.hands[0].insert("aa".into(), 1);
    g = ClassicalGame::root(Position::from_wire(&wire).unwrap());
    assert!(!g.moves().contains(&ClassicalMove::drop(PieceId(0), [1, 1])));
    assert!(!g.moves().contains(&ClassicalMove::drop(PieceId(1), [1, 2])));
    assert!(g.moves().contains(&ClassicalMove::drop(PieceId(0), [1, 2])));
    assert!(g.moves().contains(&ClassicalMove::drop(PieceId(1), [1, 3])));
}

use super::*;
fn game<R: ClassicalRule>(
    pieces: &[([u16; 2], &str, u8, Option<&str>)],
    hands: [BTreeMap<String, u16>; 2],
) -> ClassicalGame<R> {
    let p = Position::<R>::from_wire(&WirePosition {
        side_to_move: 0,
        pieces: pieces
            .iter()
            .map(|&(at, id, color, origin)| ClassicalPiece::make(at, id, color, origin))
            .collect(),
        hands,
        state: Default::default(),
    })
    .unwrap();
    ClassicalGame::root(p)
}
fn tori(extra: &[([u16; 2], &str, u8, Option<&str>)], hand: u16) -> ClassicalGame<Torishogi> {
    let mut pieces = vec![([7, 7], "av", 0, None), ([7, 1], "av", 1, None)];
    pieces.extend_from_slice(extra);
    game(
        &pieces,
        [BTreeMap::from([("bb".into(), hand)]), BTreeMap::new()],
    )
}
fn board<R: ClassicalRule>(from: [u16; 2], to: [u16; 2], promote: bool) -> ClassicalMove<R> {
    ClassicalMove::board(from, Path::one(to), promote)
}
#[test]
fn initial_inventory_and_roundtrip_all_sizes() {
    fn check<R: ClassicalRule>(n: usize) {
        let p = Position::<R>::from_wire(&WirePosition {
            side_to_move: 0,
            pieces: R::initial(),
            hands: Default::default(),
            state: Default::default(),
        })
        .unwrap();
        assert_eq!(p.cells.iter().flatten().count(), n);
        let g = ClassicalGame::root(p);
        let s = g.to_json();
        assert_eq!(ClassicalGame::<R>::parse(&s).unwrap().to_json(), s);
        assert!(!g.moves().is_empty());
    }
    check::<Torishogi>(32);
    check::<Washogi>(54);
    check::<HeianDai>(68);
    check::<Daishogi>(130);
}
#[test]
fn reserve_storage_is_rule_specific() {
    assert_eq!(std::mem::size_of::<NoHands>(), 0);
    assert_eq!(std::mem::size_of::<Hands<7>>(), 28);
}
#[test]
fn forced_promotion_and_drop_do_not_share_conditions() {
    let g = tori(&[([3, 3], "bb", 0, None)], 1);
    assert!(g.moves().contains(&board([3, 3], [3, 2], true)));
    assert!(!g.moves().contains(&board([3, 3], [3, 2], false)));
    assert!(g.moves().contains(&ClassicalMove::drop(PieceId(6), [1, 2])));
    assert!(!g.moves().contains(&ClassicalMove::drop(PieceId(6), [1, 1])));
}
#[test]
fn third_swallow_is_forbidden_but_second_is_legal() {
    let g = tori(&[([3, 4], "bb", 0, None), ([3, 5], "bb", 0, None)], 1);
    assert!(!g.moves().contains(&ClassicalMove::drop(PieceId(6), [3, 3])));
    assert!(g.moves().contains(&ClassicalMove::drop(PieceId(6), [2, 3])));
}
#[test]
fn captured_promoted_swallow_returns_to_reserve() {
    let mut g = tori(&[([3, 4], "ba", 0, None), ([3, 3], "bd", 1, None)], 0);
    g.play(board([3, 4], [3, 3], false)).unwrap();
    assert_eq!(g.current().hands.count(0, PieceId(6)), 1);
    let q = ClassicalGame::<Torishogi>::parse(&g.to_json()).unwrap();
    assert_eq!(q.current(), g.current());
}
#[test]
fn captured_material_can_change_owner() {
    let g = tori(&[([1, 5], "ba", 0, None), ([2, 5], "ba", 0, None)], 0);
    assert_eq!(
        g.current()
            .cells
            .iter()
            .flatten()
            .filter(|c| c.piece == PieceId(5) && c.color == 0)
            .count(),
        2
    );
}
#[test]
fn finite_ray_is_blocked_and_never_a_second_hop() {
    let g = tori(&[([3, 3], "bc", 0, None), ([2, 4], "bb", 1, None)], 0);
    let ms = g.moves();
    assert!(ms.contains(&board([3, 3], [2, 4], false)));
    assert!(!ms.contains(&board([3, 3], [1, 5], false)));
    assert!(ms.contains(&board([3, 3], [5, 5], false)));
    assert!(!ms.contains(&board([3, 3], [6, 6], false)));
}
#[test]
fn swallow_drop_mate_is_rejected() {
    let g = game::<Torishogi>(
        &[
            ([7, 7], "av", 0, None),
            ([4, 1], "av", 1, None),
            ([3, 3], "ba", 0, None),
            ([5, 3], "ba", 0, None),
            ([3, 2], "ay", 0, None),
            ([5, 2], "az", 0, None),
        ],
        [BTreeMap::from([("bb".into(), 1)]), BTreeMap::new()],
    );
    assert!(!g.moves().contains(&ClassicalMove::drop(PieceId(6), [4, 2])));
}
#[test]
fn check_safety_filters_unrelated_moves() {
    let g = tori(&[([7, 3], "ay", 1, None), ([1, 5], "aw", 0, None)], 0);
    assert!(!g.moves().contains(&board([1, 5], [1, 4], false)));
}
#[test]
fn wa_cloud_eagle_is_not_royal() {
    let g = game::<Washogi>(
        &[([11, 11], "dw", 0, None), ([11, 1], "eh", 1, None)],
        Default::default(),
    );
    assert_eq!(g.outcome(), Outcome::win(0));
}
#[test]
fn ambiguous_origins_survive_json_and_hash() {
    let ps = [
        ([13, 13], "ah", 0, None),
        ([13, 1], "ah", 1, None),
        ([5, 5], "ae", 0, Some("ao")),
    ];
    let g = game::<HeianDai>(&ps, Default::default());
    assert_eq!(
        ClassicalGame::<HeianDai>::parse(&g.to_json())
            .unwrap()
            .current(),
        g.current()
    );
    let mut other = ps;
    other[2].3 = Some("ap");
    let h = game::<HeianDai>(&other, Default::default());
    assert_ne!(g.current().hash(), h.current().hash());
}
#[test]
fn dai_gold_requires_a_real_origin() {
    let mut v = serde_json::to_value(WirePosition {
        side_to_move: 0,
        pieces: vec![
            ClassicalPiece::make([15, 15], "ah", 0, None),
            ClassicalPiece::make([15, 1], "ah", 1, None),
            ClassicalPiece::make([5, 5], "do", 0, None),
        ],
        hands: [BTreeMap::<String, u16>::new(), BTreeMap::new()],
        state: LionState::default(),
    })
    .unwrap();
    let w = serde_json::from_value(v.clone()).unwrap();
    assert!(Position::<Daishogi>::from_wire(&w).is_err());
    v["pieces"][2]["origin"] = "ac".into();
    assert!(Position::<Daishogi>::from_wire(&serde_json::from_value(v).unwrap()).is_ok());
}
#[test]
fn heian_kings_only_draw() {
    let g = game::<HeianDai>(
        &[([13, 13], "ah", 0, None), ([13, 1], "ah", 1, None)],
        Default::default(),
    );
    assert_eq!(g.outcome(), Outcome::DRAW);
    assert!(g.moves().is_empty());
}
#[test]
fn dai_normal_promotion_within_five_ranks() {
    let g = game::<Daishogi>(
        &[
            ([15, 15], "ah", 0, None),
            ([15, 1], "ah", 1, None),
            ([5, 4], "ad", 0, None),
        ],
        Default::default(),
    );
    assert!(g.moves().contains(&board([5, 4], [5, 3], true)));
    assert!(g.moves().contains(&board([5, 4], [5, 3], false)));
}
#[test]
fn dai_lion_double_capture_and_pending() {
    let g = game::<Daishogi>(
        &[
            ([15, 15], "ah", 0, None),
            ([15, 1], "ah", 1, None),
            ([5, 5], "bl", 0, None),
            ([6, 5], "aa", 1, None),
            ([7, 5], "ad", 1, None),
        ],
        Default::default(),
    );
    let mv = ClassicalMove::board([5, 5], Path::two([6, 5], [7, 5]), false);
    assert!(g.moves().contains(&mv));
    let mut q: serde_json::Value = serde_json::from_str(&g.to_json()).unwrap();
    q["pending"] = serde_json::json!({"from":[5,5],"via":[6,5]});
    let mut pending = ClassicalGame::<Daishogi>::parse(&q.to_string()).unwrap();
    assert!(
        pending
            .moves()
            .iter()
            .all(|m| matches!(m,ClassicalMove::Move{from,path,..}if *from==[5,5]&&path[0]==[6,5]))
    );
    pending.play(mv).unwrap();
    assert_eq!(pending.current().side, 1);
    assert_eq!(pending.current().cells.iter().flatten().count(), 3);
}
#[test]
fn schema_and_reserved_fields_are_strict() {
    let g = tori(&[], 0);
    let mut q: serde_json::Value = serde_json::from_str(&g.to_json()).unwrap();
    q["schemaVersion"] = 2.into();
    assert!(ClassicalGame::<Torishogi>::parse(&q.to_string()).is_err());
    q["schemaVersion"] = 3.into();
    q["position"]["state"]["senjishi"] = false.into();
    assert!(ClassicalGame::<Torishogi>::parse(&q.to_string()).is_err());
    assert!(
        serde_json::from_str::<ClassicalMove<Torishogi>>(
            r#"{"schemaVersion":3,"kind":"drop","piece":"bd","to":[2,2]}"#
        )
        .is_err()
    );
}
#[test]
fn board_boundaries_and_padding_do_not_wrap() {
    fn check<G: BoardGeometry>() {
        let last = G::SQUARES - 1;
        assert_eq!(G::coordinates(last), Some((G::WIDTH - 1, G::HEIGHT - 1)));
        assert_eq!(G::offset(last, 1, 0), None);
        assert_eq!(G::offset(last, 0, 1), None);
        assert_eq!(G::coordinates(G::SQUARES), None);
        assert_eq!(G::shifted(G::Bits::EMPTY.with(last), 1, 0), G::Bits::EMPTY);
        for n in [63, 64, 127, 128, last] {
            if n >= G::SQUARES {
                continue;
            }
            assert_eq!(G::Bits::EMPTY.with(n).first(), Some(n));
            assert_eq!(
                G::index(G::coordinates(n).unwrap().0, G::coordinates(n).unwrap().1),
                Some(n)
            );
            assert_eq!(G::Bits::EMPTY.with(n).without(n), G::Bits::EMPTY);
        }
    }
    check::<Rect<7, 7>>();
    check::<Rect<11, 11>>();
    check::<Rect<13, 13>>();
    check::<Rect<15, 15>>();
}

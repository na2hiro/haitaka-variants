use super::*;
fn game(ps: &[([u16; 2], &str, u8)]) -> LargeGame {
    let mut pieces = vec![([36, 36], "cg", 0), ([36, 1], "cg", 1)];
    pieces.retain(|p| !ps.iter().any(|q| q.1 == "cg" && q.2 == p.2));
    pieces.extend_from_slice(ps);
    LargeGame::root(
        Pos::from_wire(&WirePosition {
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
    .unwrap()
}
fn mv(from: [u16; 2], path: &[[u16; 2]], promotion: bool) -> LargeMove {
    LargeMove {
        schema_version: 5,
        from,
        path: TurnPath::from_steps(path).unwrap(),
        promotion,
    }
}
fn id(g: &LargeGame, at: [u16; 2]) -> &'static str {
    Pos::def(g.position.cells[Pos::index(at).unwrap()].unwrap()).id
}

#[test]
fn initial_geometry_and_internal_ids_are_wide() {
    let g = LargeGame::parse(&compiled::start_position_json()).unwrap();
    assert_eq!(g.position.cells.iter().flatten().count(), 804);
    assert!(PIECES.len() > 356);
    assert_eq!(Pos::index([36, 36]).unwrap(), 1295);
    assert_eq!(<Geometry as BoardGeometry>::Bits::FULL.count(), 1296);
    for i in [255, 256, 624, 625, 1295] {
        assert_eq!(Pos::index(Pos::at(i)).unwrap(), i);
    }
    assert!(PIECES.iter().enumerate().any(|(i, _)| i > 255));
    assert!(!g.moves().is_empty());
}
#[test]
fn eleven_rank_zone_is_optional_and_not_capture_conversion() {
    for (from, to, eligible) in [
        ([18, 13], [18, 12], false),
        ([18, 12], [18, 11], true),
        ([18, 11], [18, 12], true),
    ] {
        let g = game(&[(from, "ag", 0), (to, "aa", 1)]);
        assert!(g.moves().contains(&mv(from, &[to], false)));
        assert_eq!(g.moves().contains(&mv(from, &[to], true)), eligible);
    }
    let mut g = game(&[([18, 18], "ag", 0), ([18, 17], "iz", 1)]);
    g.play(mv([18, 18], &[[18, 17]], false)).unwrap();
    assert_eq!(id(&g, [18, 17]), "ag");
}
#[test]
fn last_rank_forces_pawn_promotion() {
    let g = game(&[([18, 2], "aa", 0)]);
    assert!(g.moves().contains(&mv([18, 2], &[[18, 1]], true)));
    assert!(!g.moves().contains(&mv([18, 2], &[[18, 1]], false)));
}
#[test]
fn ambiguous_promotion_preserves_origin_and_hash() {
    let mut g = game(&[([18, 12], "ag", 0)]);
    g.play(mv([18, 12], &[[18, 11]], true)).unwrap();
    assert_eq!(id(&g, [18, 11]), "dm");
    assert_eq!(
        LargeRule::origin(
            g.position.cells[Pos::index([18, 11]).unwrap()]
                .unwrap()
                .piece
        ),
        Some("ag")
    );
    let q = g.to_json();
    assert_eq!(LargeGame::parse(&q).unwrap().to_json(), q);
    let mut a = game(&[([18, 18], "dm", 0)]).position.to_wire();
    a.pieces
        .iter_mut()
        .find(|p| p.at == [18, 18])
        .unwrap()
        .origin = Some("ag".into());
    let h = LargeGame::root(Pos::from_wire(&a).unwrap())
        .unwrap()
        .position_hash();
    a.pieces
        .iter_mut()
        .find(|p| p.at == [18, 18])
        .unwrap()
        .origin = Some("ve".into());
    assert_ne!(
        h,
        LargeGame::root(Pos::from_wire(&a).unwrap())
            .unwrap()
            .position_hash()
    );
}
#[test]
fn empty_square_requirements_prohibit_duplicate_ray_target() {
    let g = game(&[([18, 18], "ni", 0)]);
    let from = Pos::index([18, 18]).unwrap();
    let to = Pos::index([23, 13]).unwrap();
    assert!(
        targets(&g.position, from, None, None)
            .iter()
            .any(|t| t.to == to)
    );
    let g = game(&[([18, 18], "ni", 0), ([21, 15], "aa", 1)]);
    assert!(
        !targets(&g.position, from, None, None)
            .iter()
            .any(|t| t.to == to)
    );
}
#[test]
fn jump_limit_counts_occupied_squares_not_distance() {
    let g = game(&[
        ([18, 18], "vr", 0),
        ([19, 17], "aa", 1),
        ([22, 14], "aa", 1),
        ([25, 11], "aa", 1),
        ([28, 8], "aa", 1),
    ]);
    let from = Pos::index([18, 18]).unwrap();
    let ts = targets(&g.position, from, None, None);
    assert!(ts.iter().any(|t| t.to == Pos::index([28, 8]).unwrap()));
    assert!(!ts.iter().any(|t| t.to == Pos::index([29, 7]).unwrap()));
}
#[test]
fn three_hop_bunju_paths_round_trip_and_undo() {
    let mut g = game(&[([18, 12], "tn", 0)]);
    let before = g.to_json();
    let h = g.position_hash();
    let m = mv([18, 12], &[[18, 11], [18, 10], [18, 11]], false);
    assert!(g.moves().contains(&m));

    for m in g.moves() {
        g.push(m);
        assert_eq!(
            LargeGame::parse(&g.to_json()).unwrap().position_hash(),
            g.position_hash()
        );
        g.pop();
        assert_eq!(g.position_hash(), h);
    }
    assert_eq!(g.to_json(), before);
}
#[test]
fn multiple_royals_and_early_lion_capture() {
    let mut g = game(&[
        ([18, 18], "ag", 0),
        ([18, 17], "cg", 1),
        ([35, 1], "uc", 1),
        ([34, 1], "wz", 1),
    ]);
    g.play(mv([18, 18], &[[18, 17]], false)).unwrap();
    assert_eq!(g.outcome(), Outcome::ONGOING);
    let mut g = game(&[([18, 18], "bl", 0), ([18, 17], "cg", 1)]);
    g.play(mv([18, 18], &[[18, 17]], false)).unwrap();
    assert_eq!(g.outcome(), Outcome::win(0));
}
#[test]
fn final_square_moves_and_rolls_back_without_index_truncation() {
    let mut g = game(&[([1, 36], "cg", 0), ([35, 36], "ag", 0), ([36, 36], "ad", 1)]);
    let before = g.to_json();
    let h = g.position_hash();
    g.play(mv([35, 36], &[[36, 36]], false)).unwrap();
    assert_eq!(g.position.cells[1295].unwrap().color, 0);
    g.pop();
    assert_eq!(g.to_json(), before);
    assert_eq!(g.position_hash(), h);
}

#[test]
fn transit_capture_removes_both_sides_and_restores_every_square() {
    let mut g = game(&[
        ([18, 18], "wa", 0),
        ([17, 17], "aa", 0),
        ([16, 16], "aa", 1),
        ([15, 15], "ad", 1),
    ]);
    let before = g.to_json();
    let h = g.position_hash();
    g.play(mv([18, 18], &[[14, 14]], false)).unwrap();
    for at in [[17, 17], [16, 16], [15, 15]] {
        assert!(g.position.cells[Pos::index(at).unwrap()].is_none());
    }
    assert_eq!(id(&g, [14, 14]), "wa");
    g.pop();
    assert_eq!(g.to_json(), before);
    assert_eq!(g.position_hash(), h);
}

#[test]
fn intermediary_pawn_and_go_between_do_not_authorize_protected_lion_capture() {
    for victim in ["aa", "cq", "ad"] {
        let g = game(&[
            ([18, 18], "bl", 0),
            ([18, 17], victim, 1),
            ([18, 16], "bl", 1),
            ([18, 13], "ag", 1),
        ]);
        assert_eq!(
            g.moves()
                .contains(&mv([18, 18], &[[18, 17], [18, 16]], false)),
            victim == "ad"
        );
    }
}
#[test]
fn transit_capture_can_remove_both_last_royals_at_once() {
    let q = WirePosition {
        side_to_move: 0,
        pieces: vec![
            ClassicalPiece::make([18, 18], "wa", 0, None),
            ClassicalPiece::make([17, 17], "uc", 0, None),
            ClassicalPiece::make([16, 16], "uc", 1, None),
        ],
        hands: Default::default(),
        state: Default::default(),
    };
    let mut g = LargeGame::root(Pos::from_wire(&q).unwrap()).unwrap();
    g.play(mv([18, 18], &[[14, 14]], false)).unwrap();
    assert_eq!(g.outcome(), Outcome::DRAW);
}

#[test]
fn four_hop_bunju_path_and_three_hop_pending_resume() {
    let mut g = game(&[([18, 18], "tn", 0)]);
    let before = g.to_json();
    let hash = g.position_hash();
    let m = mv([18, 18], &[[17, 17], [16, 16], [15, 15], [14, 14]], false);
    assert!(g.moves().contains(&m));
    g.play(m).unwrap();
    assert_eq!(id(&g, [14, 14]), "tn");
    g.pop();
    assert_eq!(g.to_json(), before);
    assert_eq!(g.position_hash(), hash);
    let mut q: Request = serde_json::from_str(&before).unwrap();
    q.pending = Some(Pending {
        from: [18, 18],
        path: BoundedPath::from_steps(&[[17, 17], [16, 16], [15, 15]]).unwrap(),
    });
    let mut resumed = LargeGame::parse(&serde_json::to_string(&q).unwrap()).unwrap();
    assert!(resumed.moves().contains(&m));
    resumed.play(m).unwrap();
    assert_eq!(id(&resumed, [14, 14]), "tn");
    q.schema_version = 4;
    assert!(LargeGame::parse(&serde_json::to_string(&q).unwrap()).is_err());
}

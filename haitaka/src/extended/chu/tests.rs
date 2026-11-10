use super::*;
fn game(pieces: &[([u16; 2], &str, u8)], side: u8, senjishi: bool) -> ChuGame {
    let wire = WirePosition {
        side_to_move: side,
        pieces: pieces
            .iter()
            .map(|&(at, piece, color)| PlacedPiece {
                at,
                piece: piece.into(),
                color,
            })
            .collect(),
        hands: Default::default(),
        state: ChuState { senjishi },
    };
    ChuGame {
        history: vec![ChuPosition::from_wire(&wire).unwrap()],
        played: vec![],
        pending: None,
        pending_ply: 0,
    }
}
fn kings<'a>(extra: &[([u16; 2], &'a str, u8)]) -> Vec<([u16; 2], &'a str, u8)> {
    let mut p = vec![([12, 12], "ah", 0), ([12, 1], "ah", 1)];
    p.extend_from_slice(extra);
    p
}
fn one(from: [u16; 2], to: [u16; 2], promotion: bool) -> PathMove {
    PathMove {
        schema_version: 2,
        from,
        path: Path::one(to),
        promotion,
    }
}
fn two(from: [u16; 2], via: [u16; 2], to: [u16; 2]) -> PathMove {
    PathMove {
        schema_version: 2,
        from,
        path: Path::two(via, to),
        promotion: false,
    }
}
#[test]
fn initial_inventory_and_multiword_occupancy_are_real() {
    let g = ChuGame::parse(&start_position_json()).unwrap();
    assert_eq!(g.current().cells.len(), 144);
    assert_eq!(g.current().occupied[0].count(), 46);
    assert_eq!(g.current().occupied[1].count(), 46);
    for color in 0..2 {
        assert_eq!(g.current().occupied[color].words()[2] >> 16, 0);
    }
    for i in [63, 64, 127, 128, 143] {
        let at = ChuPosition::at(i);
        assert_eq!(ChuPosition::index(at).unwrap(), i);
    }
    assert_eq!(
        core::mem::size_of::<<Rect<12, 12> as BoardGeometry>::Bits>(),
        24
    );
}
#[test]
fn all_39_piece_ids_and_both_directions_generate_in_bounds() {
    for color in 0..2 {
        for d in CHU_PIECES {
            let mut pieces = kings(&[]);
            if d.id == "ah" {
                pieces.retain(|p| p.2 != color);
            }
            pieces.push(([6, 6], d.id, color));
            let g = game(&pieces, color, false);
            let moves: Vec<_> = g.moves().into_iter().filter(|m| m.from == [6, 6]).collect();
            assert!(!moves.is_empty(), "{}", d.id);
            for m in moves {
                for &at in m.path.iter() {
                    assert!(ChuPosition::index(at).is_ok());
                }
            }
        }
    }
}
#[test]
fn lion_paths_capture_twice_return_and_stop_without_advancing_twice() {
    let root = game(
        &kings(&[([5, 5], "bl", 0), ([6, 5], "aa", 1), ([7, 5], "ad", 1)]),
        0,
        false,
    );
    let mut g = root.clone();
    let m = two([5, 5], [6, 5], [7, 5]);
    assert!(g.moves().contains(&m));
    g.play(m).unwrap();
    assert_eq!(g.history.len(), 2);
    assert_eq!(g.played.len(), 1);
    assert_eq!(g.current().side, 1);
    assert!(g.current().cells[ChuPosition::index([6, 5]).unwrap()].is_none());
    assert_eq!(root.capture_value(m), 550);
    let mut igui = root.clone();
    igui.play(two([5, 5], [6, 5], [5, 5])).unwrap();
    assert_eq!(
        igui.current().cells[ChuPosition::index([5, 5]).unwrap()]
            .unwrap()
            .piece,
        root.current().cells[ChuPosition::index([5, 5]).unwrap()]
            .unwrap()
            .piece
    );
    assert!(igui.current().cells[ChuPosition::index([6, 5]).unwrap()].is_none());
    let mut stop = root.clone();
    stop.play(one([5, 5], [6, 5], false)).unwrap();
    assert_eq!(stop.current().side, 1);
    assert_ne!(igui.current().hash(), stop.current().hash());
    for q in [g, igui, stop] {
        let parsed = ChuGame::parse(&q.to_json()).unwrap();
        assert_eq!(parsed.current(), q.current());
        assert_eq!(parsed.played, q.played);
    }
}
#[test]
fn protected_lions_adjacency_shadow_and_tsukegui() {
    for victim in ["bl", "dh"] {
        let g = game(
            &kings(&[([5, 5], "bl", 0), ([7, 5], victim, 1), ([7, 1], "ag", 1)]),
            0,
            false,
        );
        assert!(!g.moves().contains(&one([5, 5], [7, 5], false)));
        let adjacent = game(
            &kings(&[([5, 5], "bl", 0), ([6, 5], victim, 1), ([6, 1], "ag", 1)]),
            0,
            false,
        );
        assert!(adjacent.moves().contains(&one([5, 5], [6, 5], false)));
    }
    for (first, allowed) in [("ad", true), ("aa", false), ("cq", false)] {
        let g = game(
            &kings(&[
                ([5, 5], "bl", 0),
                ([6, 5], first, 1),
                ([7, 5], "bl", 1),
                ([7, 1], "ag", 1),
            ]),
            0,
            false,
        );
        assert_eq!(
            g.moves().contains(&two([5, 5], [6, 5], [7, 5])),
            allowed,
            "{first}"
        );
    }
    let shadow = game(
        &kings(&[([5, 5], "bl", 0), ([7, 5], "bl", 1), ([3, 5], "ag", 1)]),
        0,
        false,
    );
    assert!(!shadow.moves().contains(&one([5, 5], [7, 5], false)));
    let removed_foot = game(
        &kings(&[
            ([5, 5], "bl", 0),
            ([6, 4], "cq", 1),
            ([7, 5], "bl", 1),
            ([3, 5], "ag", 1),
        ]),
        0,
        false,
    );
    assert!(!removed_foot.moves().contains(&two([5, 5], [6, 4], [7, 5])));
}
#[test]
fn senjishi_is_explicit_and_changes_legal_moves_and_hash() {
    let p = kings(&[([5, 5], "ag", 0), ([5, 3], "bl", 1)]);
    let off = game(&p, 0, false);
    let on = game(&p, 0, true);
    let m = one([5, 5], [5, 3], false);
    assert!(off.moves().contains(&m));
    assert!(!on.moves().contains(&m));
    assert_ne!(off.current().hash(), on.current().hash());
    let parsed = ChuGame::parse(&on.to_json()).unwrap();
    assert_eq!(parsed.current().state, on.current().state);
    let mut taken = off.clone();
    taken.play(m).unwrap();
    assert!(taken.current().state.senjishi);
    let mut value: serde_json::Value = serde_json::from_str(&taken.to_json()).unwrap();
    value["position"]["state"]["senjishi"] = false.into();
    assert!(ChuGame::parse(&value.to_string()).is_err());
}
#[test]
fn hawk_and_eagle_follow_only_their_specific_second_steps() {
    let hawk = game(&kings(&[([5, 5], "cz", 0)]), 0, false);
    assert!(hawk.moves().contains(&two([5, 5], [5, 4], [5, 5])));
    assert!(hawk.moves().contains(&two([5, 5], [5, 4], [5, 3])));
    assert!(!hawk.moves().contains(&two([5, 5], [5, 4], [6, 4])));
    let eagle = game(&kings(&[([5, 5], "db", 0)]), 0, false);
    assert!(eagle.moves().contains(&two([5, 5], [6, 4], [7, 3])));
    assert!(eagle.moves().contains(&two([5, 5], [6, 4], [5, 5])));
    assert!(!eagle.moves().contains(&two([5, 5], [6, 4], [5, 3])));
}
#[test]
fn chu_promotion_differs_from_sho_and_dead_pawns_remain_legal() {
    let entry = game(&kings(&[([5, 5], "ad", 0)]), 0, false);
    assert!(entry.moves().contains(&one([5, 5], [5, 4], true)));
    let inside = game(&kings(&[([5, 3], "ad", 0)]), 0, false);
    assert!(!inside.moves().contains(&one([5, 3], [5, 2], true)));
    let capture = game(&kings(&[([5, 3], "ad", 0), ([5, 2], "aa", 1)]), 0, false);
    assert!(capture.moves().contains(&one([5, 3], [5, 2], true)));
    let exit = game(&kings(&[([3, 3], "af", 0), ([5, 5], "aa", 1)]), 0, false);
    assert!(exit.moves().contains(&one([3, 3], [5, 5], true)));
    for id in ["aa", "ab"] {
        let mut g = game(&kings(&[([5, 2], id, 0)]), 0, false);
        assert!(g.moves().contains(&one([5, 2], [5, 1], true)));
        assert!(g.moves().contains(&one([5, 2], [5, 1], false)));
        g.play(one([5, 2], [5, 1], false)).unwrap();
        assert!(ChuGame::parse(&g.to_json()).is_ok());
    }
}
#[test]
fn full_turn_terminal_and_budget_fallback_never_return_partial_paths() {
    let mut g = game(
        &[([12, 12], "ah", 0), ([5, 5], "bl", 0), ([6, 5], "ah", 1)],
        0,
        false,
    );
    assert!(g.moves().contains(&one([5, 5], [6, 5], false)));
    assert!(!g.moves().contains(&two([5, 5], [6, 5], [5, 5]))); // game ends on the first capture
    g.play(one([5, 5], [6, 5], false)).unwrap();
    assert_eq!(g.outcome(), Outcome::win(0));
    assert!(g.moves().is_empty());
    let start = start_position_json();
    let result: serde_json::Value =
        serde_json::from_str(&search_json(&start, 100, 1000, 1).unwrap()).unwrap();
    assert_eq!(result["nodeLimitReached"], true);
    let mv: PathMove = serde_json::from_value(result["bestMove"].clone()).unwrap();
    assert!(ChuGame::parse(&start).unwrap().moves().contains(&mv));
    let mut p = ChuGame::parse(&start).unwrap();
    let before = p.to_json();
    let mut calls = 0;
    assert!(
        p.current()
            .legal_paths(usize::MAX, &mut || {
                calls += 1;
                calls > 5
            })
            .is_none()
    );
    assert_eq!(before, p.to_json());
    p.play(mv).unwrap();
    assert_eq!(p.history.len(), 2);
}
#[test]
fn wire_path_codec_rejects_unknown_fields_and_invalid_steps() {
    for raw in [
        r#"{"schemaVersion":2,"from":[1,1],"path":[],"promotion":false}"#,
        r#"{"schemaVersion":2,"from":[1,1],"path":[[1,2],[1,3],[1,4]],"promotion":false}"#,
        r#"{"schemaVersion":2,"from":[1,1],"path":[[1,2]],"promotion":false,"to":[1,2]}"#,
    ] {
        assert!(serde_json::from_str::<PathMove>(raw).is_err());
    }
    let mut g = ChuGame::parse(&start_position_json()).unwrap();
    assert!(g.play(one([1, 9], [0, 8], false)).is_err());
    let mut q: serde_json::Value = serde_json::from_str(&g.to_json()).unwrap();
    q["position"]["state"]["unknown"] = true.into();
    assert!(ChuGame::parse(&q.to_string()).is_err());
}
#[test]
fn pending_paths_are_hashed_filtered_and_restored_without_advancing_history() {
    let mut g = game(
        &kings(&[([5, 5], "bl", 0), ([6, 5], "aa", 1), ([7, 5], "ad", 1)]),
        0,
        false,
    );
    let original = g.position_hash();
    g.pending = Some(Prefix {
        from: [5, 5],
        via: [6, 5],
    });
    g.pending_ply = 0;
    assert_ne!(g.position_hash(), original);
    assert_eq!(g.history.len(), 1);
    let moves = g.moves();
    assert!(!moves.is_empty());
    assert!(
        moves
            .iter()
            .all(|m| m.from == [5, 5] && m.path[0] == [6, 5])
    );
    let q = ChuGame::parse(&g.to_json()).unwrap();
    assert_eq!(q.position_hash(), g.position_hash());
    assert_eq!(q.moves(), moves);
    let r: serde_json::Value =
        serde_json::from_str(&search_json(&g.to_json(), 100, 1000, 1).unwrap()).unwrap();
    let mv: PathMove = serde_json::from_value(r["bestMove"].clone()).unwrap();
    assert!(moves.contains(&mv));
    g.play(mv).unwrap();
    assert_eq!(g.history.len(), 2);
    assert!(g.active_pending().is_none());
    let mut bad: serde_json::Value = serde_json::from_str(&q.to_json()).unwrap();
    bad["pending"]["via"] = serde_json::json!([7, 5]);
    assert!(ChuGame::parse(&bad.to_string()).is_err());
}
#[test]
fn stalemate_loses_and_complete_turn_repetition_distinguishes_senjishi() {
    let mut stale = game(
        &[
            ([1, 1], "ah", 0),
            ([12, 12], "ah", 1),
            ([11, 12], "aa", 1),
            ([12, 11], "aa", 1),
            ([11, 11], "aa", 1),
        ],
        0,
        false,
    );
    stale.play(one([1, 1], [1, 2], false)).unwrap();
    assert_eq!(stale.outcome(), Outcome::win(0));
    let mut g = game(&kings(&[([5, 6], "ag", 0), ([5, 5], "bl", 1)]), 0, false);
    g.play(one([5, 6], [5, 5], false)).unwrap();
    assert!(g.current().state.senjishi);
    for _ in 0..3 {
        for m in [
            one([12, 1], [11, 1], false),
            one([5, 5], [5, 6], false),
            one([11, 1], [12, 1], false),
            one([5, 6], [5, 5], false),
        ] {
            g.play(m).unwrap();
        }
    }
    assert_eq!(g.outcome(), Outcome::ONGOING);
    g.play(one([12, 1], [11, 1], false)).unwrap();
    assert_eq!(g.outcome(), Outcome::DRAW);
}
#[test]
fn rays_and_hashes_cross_word_boundaries_without_padding_bits() {
    for from in [63usize, 64, 127, 128, 143] {
        let mut g = game(
            &[
                ([2, 11], "ah", 0),
                ([1, 1], "ah", 1),
                (ChuPosition::at(from), "ag", 0),
            ],
            0,
            false,
        );
        let p = g.current();
        let mover = p.cells[from].unwrap();
        let targets = p.targets(from, mover);
        assert_eq!(targets.len(), 22);
        let to = if from == 143 { 142 } else { from + 1 };
        assert!(targets.contains(&to));
        let original = p.hash();
        g.play(one(ChuPosition::at(from), ChuPosition::at(to), false))
            .unwrap();
        let rebuilt = ChuPosition::from_wire(&g.current().to_wire()).unwrap();
        assert_ne!(original, rebuilt.hash());
        assert_eq!(rebuilt.hash(), g.current().hash());
        assert!(!rebuilt.occupied[0].contains(from));
        assert!(rebuilt.occupied[0].contains(to));
        assert_eq!(rebuilt.occupied[0].words()[2] >> 16, 0);
    }
}

use super::*;
fn game(ps: &[([u16; 2], &str, u8)]) -> LargeGame {
    let mut pieces = vec![([17, 17], "ah", 0), ([17, 1], "ah", 1)];
    pieces.retain(|p| !ps.iter().any(|q| q.1 == "ah" && q.2 == p.2));
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
        schema_version: 4,
        from,
        path: TurnPath::from_steps(path).unwrap(),
        promotion,
    }
}
#[test]
fn initial_inventory_and_wide_indices() {
    assert_eq!(PIECES.len(), 85);
    assert_eq!(DaiDai::initial().len(), 192);
    let p = LargeGame::parse(&compiled::start_position_json()).unwrap();
    assert!(!p.moves().is_empty());
    assert_eq!(Pos::index([17, 17]).unwrap(), 288);
    assert_eq!(Pos::at(256), [16, 2]);
    assert_eq!(
        p.position.occupied[0].count() + p.position.occupied[1].count(),
        192
    );
    assert_eq!(
        LargeGame::parse(&p.to_json()).unwrap().to_json(),
        p.to_json()
    );
}
#[test]
fn capture_promotes_but_quiet_move_does_not() {
    let mut g = game(&[([8, 8], "co", 0), ([9, 7], "aa", 1)]);
    assert!(!g.moves().contains(&mv([8, 8], &[[9, 7]], false)));
    g.play(mv([8, 8], &[[9, 7]], true)).unwrap();
    assert_eq!(
        Pos::def(g.position.cells[Pos::index([9, 7]).unwrap()].unwrap()).id,
        "hb"
    );
    let g = game(&[([8, 8], "co", 0)]);
    assert!(g.moves().iter().all(|m| !m.promotion));
}
#[test]
fn capture_without_promotion_target_keeps_species() {
    let mut g = game(&[([16, 17], "ag", 0), ([17, 17], "aa", 1), ([1, 17], "ah", 0)]);
    g.play(mv([16, 17], &[[17, 17]], false)).unwrap();
    assert_eq!(Pos::def(g.position.cells[288].unwrap()).id, "ag");
}
#[test]
fn lion_capture_defers_promotion_until_stop_or_second_hop() {
    let mut g = game(&[([8, 8], "bl", 0), ([8, 7], "ad", 1), ([8, 6], "ad", 1)]);
    let before = g.to_json();
    assert!(g.moves().contains(&mv([8, 8], &[[8, 7]], true)));
    g.play(mv([8, 8], &[[8, 7], [8, 6]], true)).unwrap();
    assert_eq!(
        Pos::def(g.position.cells[Pos::index([8, 6]).unwrap()].unwrap()).id,
        "ie"
    );
    g.pop();
    assert_eq!(g.to_json(), before);
}
#[test]
fn komainu_completes_three_captures_and_undoes_them() {
    let mut g = game(&[
        ([8, 8], "go", 0),
        ([8, 7], "ad", 1),
        ([8, 6], "ad", 1),
        ([8, 5], "ae", 1),
    ]);
    let before = g.to_json();
    g.play(mv([8, 8], &[[8, 7], [8, 6], [8, 5]], true)).unwrap();
    assert_eq!(
        Pos::def(g.position.cells[Pos::index([8, 5]).unwrap()].unwrap()).id,
        "ig"
    );
    g.pop();
    assert_eq!(g.to_json(), before);
}
#[test]
fn hooks_turn_only_on_empty_squares() {
    let g = game(&[([8, 8], "gh", 0), ([8, 6], "aa", 1), ([9, 8], "aa", 0)]);
    assert!(g.moves().contains(&mv([8, 8], &[[10, 7]], false)));
    assert!(!g.moves().contains(&mv([8, 8], &[[10, 6]], false)));
}
#[test]
fn royal_capture_while_moving_ends_before_deferred_promotion() {
    let mut g = game(&[([8, 8], "bl", 0), ([8, 7], "ah", 1)]);
    g.play(mv([8, 8], &[[8, 7]], false)).unwrap();
    assert_eq!(g.outcome(), Outcome::win(0));
    assert_eq!(
        Pos::def(g.position.cells[Pos::index([8, 7]).unwrap()].unwrap()).id,
        "bl"
    );
}

#[test]
fn senjishi_is_in_state_and_prevents_nonlion_recapture() {
    let mut g = game(&[
        ([8, 8], "ag", 0),
        ([8, 6], "bl", 1),
        ([10, 8], "ag", 1),
        ([10, 6], "bl", 0),
    ]);
    g.play(mv([8, 8], &[[8, 6]], false)).unwrap();
    assert!(g.position.state.senjishi);
    assert!(!g.moves().contains(&mv([10, 8], &[[10, 6]], false)));
    let h = g.position.hash();
    g.position.state.senjishi = false;
    assert_ne!(g.position.hash(), h);
    assert!(g.moves().contains(&mv([10, 8], &[[10, 6]], false)));
}
#[test]
fn every_generated_move_restores_wide_board_and_hash_after_undo() {
    let mut g = game(&[
        ([16, 16], "go", 0),
        ([16, 15], "aa", 1),
        ([15, 16], "ad", 1),
    ]);
    let before = g.to_json();
    let hash = g.position_hash();
    for m in g.moves() {
        g.push(m);
        assert_eq!(
            LargeGame::parse(&g.to_json()).unwrap().position_hash(),
            g.position_hash()
        );
        g.pop();
        assert_eq!(g.position_hash(), hash);
        assert_eq!(g.to_json(), before);
    }
}

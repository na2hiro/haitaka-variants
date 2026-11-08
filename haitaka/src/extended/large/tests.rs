use super::*;
fn game(extra: &[([u16; 2], &str, u8)], reverse: bool) -> LargeGame {
    let mut ps = vec![([16, 16], "ah", 0), ([16, 1], "ah", 1)];
    ps.retain(|p| !extra.iter().any(|e| e.1 == "ah" && p.2 == e.2));
    ps.extend_from_slice(extra);
    let mut p = Pos::from_wire(&WirePosition {
        side_to_move: 0,
        pieces: ps
            .iter()
            .map(|&(at, id, color)| ClassicalPiece::make(at, id, color, None))
            .collect(),
        hands: Default::default(),
        state: Default::default(),
    })
    .unwrap();
    p.state.shitenno = order(&p, &[], None);
    if reverse {
        p.state.shitenno.reverse();
    }
    LargeGame::root(p).unwrap()
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
fn every_piece_has_an_explicit_movement_and_initial_inventory() {
    assert_eq!(PIECES.len(), 66);
    assert_eq!(MOVEMENT.len(), 66);
    assert_eq!(TenjikuDai::initial().len(), 156);
    let g = LargeGame::parse(&compiled::start_position_json()).unwrap();
    assert!(!g.moves().is_empty());
    assert_eq!(
        g.to_json(),
        LargeGame::parse(&g.to_json()).unwrap().to_json()
    );
}
#[test]
fn fire_burns_only_on_completed_turn_and_uses_resident_priority() {
    let mut g = game(
        &[
            ([8, 10], "fe", 0),
            ([9, 8], "fe", 1),
            ([7, 8], "aa", 1),
            ([8, 8], "aa", 1),
        ],
        false,
    );
    let before = g.to_json();
    g.play(mv([8, 10], &[[8, 9], [8, 8]], false)).unwrap();
    assert_eq!(g.position.cells[Pos::index([8, 8]).unwrap()], None);
    assert!(g.position.cells[Pos::index([9, 8]).unwrap()].is_some_and(fire));
    assert!(g.position.cells[Pos::index([7, 8]).unwrap()].is_some());
    g.pop();
    assert_eq!(g.to_json(), before);
}
#[test]
fn three_king_steps_stop_on_capture() {
    let g = game(&[([8, 8], "fe", 0), ([10, 7], "aa", 1)], false);
    assert!(
        g.moves()
            .contains(&mv([8, 8], &[[8, 7], [9, 6], [10, 6]], false))
    );
    assert!(
        !g.moves()
            .contains(&mv([8, 8], &[[9, 7], [10, 7], [11, 7]], false))
    );
}
#[test]
fn water_buffalo_promotion_burns_immediately() {
    let mut g = game(&[([8, 6], "fh", 0), ([9, 4], "aa", 1)], false);
    g.play(mv([8, 6], &[[8, 5]], true)).unwrap();
    assert!(g.position.cells[Pos::index([8, 5]).unwrap()].is_some_and(|c| Pos::def(c).id == "zg"));
    assert_eq!(g.position.cells[Pos::index([9, 4]).unwrap()], None);
}
#[test]
fn frozen_fire_does_not_burn_and_intermediate_freeze_does_not_stop_a_path() {
    let mut g = game(
        &[([7, 7], "fp", 1), ([8, 10], "fe", 0), ([9, 8], "ag", 1)],
        false,
    );
    g.play(mv([8, 10], &[[8, 9], [8, 8]], false)).unwrap();
    assert!(g.position.cells[Pos::index([8, 8]).unwrap()].is_some_and(fire));
    assert!(g.position.cells[Pos::index([9, 8]).unwrap()].is_some());
}
#[test]
fn ordered_heavenly_kings_change_moves_and_hash() {
    let a = game(&[([8, 8], "fp", 0), ([9, 9], "fp", 1)], false);
    let b = game(&[([8, 8], "fp", 0), ([9, 9], "fp", 1)], true);
    assert_ne!(a.position_hash(), b.position_hash());
    assert!(a.moves().iter().any(|m| m.from == [8, 8]));
    assert!(!b.moves().iter().any(|m| m.from == [8, 8]));
}
#[test]
fn mandatory_igui_preserves_resident_priority() {
    let mut g = game(
        &[([8, 8], "fp", 0), ([9, 9], "aa", 1), ([7, 7], "fp", 1)],
        true,
    );
    let order = g.position.state.shitenno.clone();
    assert!(!g.moves().contains(&mv([8, 8], &[[9, 9]], false)));
    g.play(mv([8, 8], &[[9, 9], [8, 8]], false)).unwrap();
    assert_eq!(g.position.state.shitenno, order);
}
#[test]
fn arriving_king_burns_before_it_can_freeze_resident_fire() {
    let mut g = game(&[([8, 11], "fp", 0), ([9, 8], "fe", 1)], false);
    g.play(mv([8, 11], &[[8, 8]], false)).unwrap();
    assert_eq!(g.position.cells[Pos::index([8, 8]).unwrap()], None);
    assert!(g.position.state.shitenno.is_empty());
}
#[test]
fn both_royals_burning_is_a_draw_and_undo_restores_every_square() {
    let mut g = game(
        &[
            ([2, 2], "fe", 0),
            ([3, 3], "ah", 1),
            ([15, 15], "fe", 1),
            ([8, 8], "aa", 0),
        ],
        false,
    );
    let old = g.to_json();
    g.play(mv([8, 8], &[[8, 7]], false)).unwrap();
    assert_eq!(g.outcome(), Outcome::DRAW);
    assert_eq!(
        LargeGame::parse(&g.to_json()).unwrap().outcome(),
        Outcome::DRAW
    );
    g.pop();
    assert_eq!(old, g.to_json());
}
#[test]
fn generals_skip_lower_pieces_but_cannot_land_on_or_cross_exceptions() {
    let g = game(
        &[
            ([8, 8], "fk", 0),
            ([8, 6], "aa", 0),
            ([8, 4], "aa", 1),
            ([8, 2], "ah", 1),
        ],
        false,
    );
    let ms = g.moves();
    assert!(ms.contains(&mv([8, 8], &[[8, 4]], false)));
    assert!(ms.contains(&mv([8, 8], &[[8, 3]], false)));
    assert!(!ms.contains(&mv([8, 8], &[[8, 2]], false)));
    assert!(!ms.contains(&mv([8, 8], &[[8, 1]], false)));
}
#[test]
fn passing_a_promotable_lion_can_promote_after_one_or_two_hops() {
    let g = game(&[([8, 6], "bl", 0)], false);
    assert!(g.moves().contains(&mv([8, 6], &[[8, 5]], true)));
    assert!(g.moves().contains(&mv([8, 6], &[[8, 5], [8, 6]], true)));
}
#[test]
fn royal_capture_while_continuing_does_not_apply_end_turn_fire() {
    let mut g = game(
        &[([8, 8], "bl", 0), ([9, 8], "ah", 1), ([10, 8], "fe", 1)],
        false,
    );
    g.play(mv([8, 8], &[[9, 8]], false)).unwrap();
    assert!(g.position.cells[Pos::index([9, 8]).unwrap()].is_some_and(lion));
    assert_eq!(g.outcome(), Outcome::win(0));
}
#[test]
fn pending_second_hop_restricts_the_entire_prefix() {
    let g = game(&[([8, 10], "fe", 0), ([9, 8], "fe", 1)], false);
    let mut q: Request = serde_json::from_str(&g.to_json()).unwrap();
    q.pending = Some(Pending {
        from: [8, 10],
        path: BoundedPath::two([8, 9], [7, 10]),
    });
    let p = LargeGame::parse(&serde_json::to_string(&q).unwrap()).unwrap();
    assert!(
        p.moves()
            .iter()
            .all(|m| m.from == [8, 10] && m.path.starts_with(&[[8, 9], [7, 10]]))
    );
}
#[test]
fn every_generated_path_replays_and_delta_restores_hash_state_and_occupancy() {
    for ps in [
        vec![([8, 8], "fe", 0), ([9, 6], "aa", 1)],
        vec![([8, 8], "fp", 0), ([9, 9], "aa", 1), ([7, 7], "fe", 1)],
        vec![([8, 6], "bl", 0)],
    ] {
        let mut g = game(&ps, false);
        let old = g.to_json();
        let hash = g.position_hash();
        let occupied = g.position.occupied;
        for m in g.moves() {
            g.play(m).unwrap();
            assert_eq!(
                g.position_hash(),
                LargeGame::parse(&g.to_json()).unwrap().position_hash()
            );
            g.pop();
            assert_eq!(g.position_hash(), hash);
            assert_eq!(g.position.occupied, occupied);
            assert_eq!(g.to_json(), old);
        }
    }
}
#[test]
fn wire_state_and_version_are_strict_and_255_is_a_real_square() {
    let g = game(&[([8, 8], "fp", 0)], false);
    let mut q: Request = serde_json::from_str(&g.to_json()).unwrap();
    q.position
        .state
        .shitenno
        .push(q.position.state.shitenno[0].clone());
    assert!(LargeGame::parse(&serde_json::to_string(&q).unwrap()).is_err());
    assert_eq!(Pos::index([16, 16]).unwrap(), 255);
    assert!(Pos::index([17, 1]).is_err());
    assert!(TurnPath::from_steps(&[[1, 1]; 4]).is_none());
    assert!(serde_json::from_str::<super::super::path::Path>("[[1,1],[2,2],[3,3]]").is_err());
}
#[test]
fn lazy_repetition_attack_checks_use_restored_ordered_states() {
    let mut g = game(&[], false);
    for _ in 0..3 {
        for (from, to) in [
            ([16, 16], [15, 16]),
            ([16, 1], [15, 1]),
            ([15, 16], [16, 16]),
            ([15, 1], [16, 1]),
        ] {
            g.play(mv(from, &[to], false)).unwrap();
        }
    }
    assert_eq!(g.outcome(), Outcome::DRAW);
    let base = game(&[([8, 1], "ah", 1), ([8, 8], "ag", 0)], false);
    let mut p = base.position;
    p.side = 1;
    let mut g = LargeGame::root(p).unwrap();
    for _ in 0..3 {
        for (from, to) in [
            ([8, 1], [9, 1]),
            ([8, 8], [9, 8]),
            ([9, 1], [8, 1]),
            ([9, 8], [8, 8]),
        ] {
            g.play(mv(from, &[to], false)).unwrap();
        }
    }
    assert_eq!(g.outcome(), Outcome::win(1));
}

use super::*;
fn game(ps: &[([u16; 2], &str, u8)]) -> LargeGame {
    let mut pieces = vec![([19, 19], "ah", 0), ([19, 1], "ah", 1)];
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
fn id(g: &LargeGame, at: [u16; 2]) -> &'static str {
    Pos::def(g.position.cells[Pos::index(at).unwrap()].unwrap()).id
}
#[test]
fn initial_and_large_indices() {
    let g = LargeGame::parse(&compiled::start_position_json()).unwrap();
    assert_eq!(PIECES.len(), 98);
    assert_eq!(g.position.cells.iter().flatten().count(), 192);
    assert_eq!(Pos::index([19, 19]).unwrap(), 360);
    assert!(!g.moves().is_empty());
}
#[test]
fn ordinary_capture_preserves_ambiguous_origin() {
    let mut g = game(&[([9, 9], "ag", 0), ([9, 8], "aa", 1)]);
    g.play(mv([9, 9], &[[9, 8]], true)).unwrap();
    assert_eq!(id(&g, [9, 8]), "do");
    assert_eq!(
        LargeRule::origin(g.position.cells[Pos::index([9, 8]).unwrap()].unwrap().piece),
        Some("ag")
    );
    assert_eq!(
        g.to_json(),
        LargeGame::parse(&g.to_json()).unwrap().to_json()
    );
}
#[test]
fn all_special_victims_choose_their_own_target() {
    for (v, target) in [("ih", "iz"), ("iz", "iz"), ("ii", "jb"), ("jb", "jb")] {
        let mut g = game(&[([9, 9], "ag", 0), ([9, 8], v, 1)]);
        g.play(mv([9, 9], &[[9, 8]], true)).unwrap();
        assert_eq!(id(&g, [9, 8]), target);
    }
}
#[test]
fn royal_and_elephant_exceptions() {
    for (m, target) in [("ah", "ah"), ("iy", "iy"), ("cf", "iy")] {
        let mut g = game(&[([9, 9], m, 0), ([9, 8], "ih", 1)]);
        g.play(mv([9, 9], &[[9, 8]], m == "cf")).unwrap();
        assert_eq!(id(&g, [9, 8]), target);
    }
}
#[test]
fn latest_special_capture_wins_and_pass_retains_it() {
    for (a, b, target) in [("ih", "ii", "jb"), ("ii", "ih", "iz"), ("ih", "aa", "iz")] {
        let mut g = game(&[([9, 9], "bl", 0), ([9, 8], a, 1), ([9, 7], b, 1)]);
        let before = g.to_json();
        g.play(mv([9, 9], &[[9, 8], [9, 7]], true)).unwrap();
        assert_eq!(id(&g, [9, 7]), target);
        g.pop();
        assert_eq!(g.to_json(), before);
        g.play(mv([9, 9], &[[9, 8]], true)).unwrap();
        assert_eq!(id(&g, [9, 8]), if a == "ih" { "iz" } else { "jb" });
    }
}
#[test]
fn emperor_ignores_other_emperors_but_not_rook_protection() {
    let g = game(&[
        ([9, 9], "ix", 0),
        ([7, 7], "aa", 1),
        ([7, 5], "ag", 1),
        ([5, 5], "ix", 1),
    ]);
    assert!(!g.moves().contains(&mv([9, 9], &[[7, 7]], true)));
    assert!(g.moves().contains(&mv([9, 9], &[[1, 1]], false)));
    let g = game(&[([9, 9], "ix", 0), ([7, 7], "aa", 1), ([5, 5], "ix", 1)]);
    assert!(g.moves().contains(&mv([9, 9], &[[7, 7]], false)));
}
#[test]
fn king_becomes_nonroyal_on_ordinary_capture() {
    let mut g = game(&[([9, 9], "ah", 0), ([9, 8], "aa", 1)]);
    g.play(mv([9, 9], &[[9, 8]], true)).unwrap();
    assert_eq!(id(&g, [9, 8]), "ix");
    assert_eq!(g.outcome(), Outcome::win(1));
    let mut g = game(&[([9, 9], "ah", 0), ([9, 8], "ah", 1)]);
    g.play(mv([9, 9], &[[9, 8]], true)).unwrap();
    assert_eq!(g.outcome(), Outcome::DRAW);
    assert!(LargeGame::parse(&g.to_json()).is_ok());
}
#[test]
fn origins_affect_hash_and_special_conversion_clears_them() {
    let mut a = game(&[([9, 9], "do", 0), ([9, 8], "ii", 1)]);
    let mut q = a.position.to_wire();
    q.pieces.iter_mut().find(|p| p.at == [9, 9]).unwrap().origin = Some("ag".into());
    a = LargeGame::root(Pos::from_wire(&q).unwrap()).unwrap();
    let mut other = q.clone();
    other
        .pieces
        .iter_mut()
        .find(|p| p.at == [9, 9])
        .unwrap()
        .origin = Some("aa".into());
    assert_ne!(
        a.position_hash(),
        LargeGame::root(Pos::from_wire(&other).unwrap())
            .unwrap()
            .position_hash()
    );
    a.play(mv([9, 9], &[[9, 8]], true)).unwrap();
    let c = a.position.cells[Pos::index([9, 8]).unwrap()].unwrap();
    assert_eq!(id(&a, [9, 8]), "jb");
    assert_eq!(LargeRule::origin(c.piece), None);
}

#[test]
fn conversions_use_a_total_inventory_bound() {
    let g = game(&[([9, 9], "iz", 0), ([8, 8], "iz", 0), ([7, 7], "iz", 0)]);
    assert!(LargeGame::parse(&g.to_json()).is_ok());
    let mut q = g.position.to_wire();
    q.pieces.retain(|p| p.color == 1);
    for i in 0..97 {
        q.pieces
            .push(ClassicalPiece::make(Pos::at(i), "aa", 0, None));
    }
    assert!(Pos::from_wire(&q).unwrap_err().contains("piece count"));
}
#[test]
fn every_special_path_undoes_its_board_state_and_hash() {
    let mut g = game(&[([9, 9], "bl", 0), ([9, 8], "ih", 1), ([9, 7], "ii", 1)]);
    let before = g.to_json();
    let hash = g.position_hash();
    for m in g.moves() {
        g.push(m);
        assert_eq!(
            LargeGame::parse(&g.to_json()).unwrap().position_hash(),
            g.position_hash()
        );
        g.pop();
        assert_eq!(g.to_json(), before);
        assert_eq!(g.position_hash(), hash);
    }
}

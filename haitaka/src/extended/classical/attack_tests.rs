use super::*;
fn empty<R: ClassicalRule>() -> Position<R> {
    let mut p = Position::<R>::from_wire(&WirePosition {
        side_to_move: 0,
        pieces: R::initial(),
        hands: Default::default(),
        state: Default::default(),
    })
    .unwrap();
    for i in 0..R::Geometry::SQUARES {
        p.set_cell(i, None);
    }
    p
}
fn put<R: ClassicalRule>(p: &mut Position<R>, at: [u16; 2], id: &str, color: u8) {
    let piece = PieceId(R::PIECES.iter().position(|d| d.id == id).unwrap() as u16);
    p.set_cell(
        Position::<R>::index(at).unwrap(),
        Some(Cell { piece, color }),
    );
}
fn same<R: ClassicalRule>(p: &Position<R>) {
    for color in 0..2 {
        assert_eq!(
            p.classical_attacked_direct(color),
            p.attacked(color),
            "rule {} color {}",
            R::ID,
            color
        );
    }
}
fn all_empty_targets<R: ClassicalRule>() {
    let mut p = empty::<R>();
    for color in 0..2 {
        for id in 0..R::PIECES.len() {
            let c = Cell {
                piece: PieceId(id as u16),
                color,
            };
            for from in 0..R::Geometry::SQUARES {
                p.set_cell(from, Some(c));
                let expected = p.geometric_targets(from, c, false);
                for to in 0..R::Geometry::SQUARES {
                    assert_eq!(
                        p.classical_attacks_square(from, c, to),
                        expected.contains(&to),
                        "rule {} piece {} color {} from {} to {}",
                        R::ID,
                        id,
                        color,
                        from,
                        to
                    );
                }
                p.set_cell(from, None);
            }
        }
    }
}
#[test]
fn every_piece_side_origin_and_edge_matches_geometric_targets() {
    all_empty_targets::<Washogi>();
    all_empty_targets::<HeianDai>();
    all_empty_targets::<Torishogi>();
}
fn blocked_targets<R: ClassicalRule>() {
    let from = R::Geometry::index(R::Geometry::WIDTH / 2, R::Geometry::HEIGHT / 2).unwrap();
    for color in 0..2 {
        for id in 0..R::PIECES.len() {
            for blockers in [color, color ^ 1] {
                let mut p = empty::<R>();
                let c = Cell {
                    piece: PieceId(id as u16),
                    color,
                };
                p.set_cell(from, Some(c));
                for &(dx, dy) in KING {
                    if let Some(to) = R::Geometry::offset(from, dx * 2, dy * 2) {
                        p.set_cell(
                            to,
                            Some(Cell {
                                piece: PieceId(0),
                                color: blockers,
                            }),
                        );
                    }
                }
                let expected = p.geometric_targets(from, c, false);
                for to in 0..R::Geometry::SQUARES {
                    assert_eq!(
                        p.classical_attacks_square(from, c, to),
                        expected.contains(&to),
                        "rule {} piece {} blocker color {} to {}",
                        R::ID,
                        id,
                        blockers,
                        to
                    );
                }
            }
        }
    }
}
#[test]
fn friendly_enemy_blockers_and_finite_rays_match() {
    blocked_targets::<Washogi>();
    blocked_targets::<HeianDai>();
    blocked_targets::<Torishogi>();
}
fn walk<R: ClassicalRule>() {
    let mut g = ClassicalGame::<R>::root(
        Position::<R>::from_wire(&WirePosition {
            side_to_move: 0,
            pieces: R::initial(),
            hands: Default::default(),
            state: Default::default(),
        })
        .unwrap(),
    );
    let mut seed = 0xa77ac001_u32 + R::ID;
    for _ in 0..128 {
        same(g.current());
        let moves = g.moves();
        for &m in moves.iter().take(16) {
            same(&g.current().classical_apply(m));
        }
        if moves.is_empty() {
            break;
        }
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        g.play(moves[seed as usize % moves.len()]).unwrap();
    }
}
#[test]
fn fixed_seed_legal_walks_and_candidate_positions_match() {
    walk::<Washogi>();
    walk::<HeianDai>();
}
#[test]
fn pinned_attacker_still_attacks_and_multiple_royals_are_any_royal() {
    let mut p = empty::<HeianDai>();
    put(&mut p, [5, 5], "ah", 0);
    put(&mut p, [7, 1], "ah", 1);
    put(&mut p, [7, 5], "aq", 1);
    put(&mut p, [7, 9], "at", 0);
    assert!(p.classical_attacked_direct(0));
    same(&p);
    put(&mut p, [1, 13], "ah", 0);
    same(&p);
    p.set_cell(Position::<HeianDai>::index([5, 5]).unwrap(), None);
    assert!(!p.classical_attacked_direct(0));
    same(&p);
    p.set_cell(Position::<HeianDai>::index([1, 13]).unwrap(), None);
    assert!(!p.classical_attacked_direct(0));
    same(&p);
}
#[test]
fn capture_opens_ray_and_royal_destination_is_checked_after_move() {
    let mut p = empty::<HeianDai>();
    put(&mut p, [7, 9], "ah", 0);
    put(&mut p, [13, 1], "ah", 1);
    put(&mut p, [7, 1], "at", 1);
    put(&mut p, [7, 5], "ad", 0);
    assert!(!p.classical_attacked_direct(0));
    same(&p);
    p.set_cell(Position::<HeianDai>::index([7, 5]).unwrap(), None);
    assert!(p.classical_attacked_direct(0));
    same(&p);
    let m = ClassicalMove::<HeianDai>::board([7, 9], Path::one([6, 9]), false);
    let next = p.classical_apply(m);
    assert!(!next.classical_attacked_direct(0));
    same(&next);
}

/// Diagnostic only: time the old and direct queries on the same fixed positions.
#[test]
#[ignore = "run explicitly on an idle, pinned CPU"]
fn benchmark_old_and_direct_attack_queries() {
    fn run<R: ClassicalRule>() {
        use std::{
            hint::black_box,
            time::{Duration, Instant},
        };
        let mut g = ClassicalGame::<R>::root(
            Position::<R>::from_wire(&WirePosition {
                side_to_move: 0,
                pieces: R::initial(),
                hands: Default::default(),
                state: Default::default(),
            })
            .unwrap(),
        );
        let mut positions = vec![];
        let mut seed = 0xa77ac001_u32 + R::ID;
        for _ in 0..128 {
            positions.push(g.current().clone());
            let moves = g.moves();
            if moves.is_empty() {
                break;
            }
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            g.play(moves[seed as usize % moves.len()]).unwrap();
        }
        for p in &positions {
            same(p);
        }
        let mut old = vec![];
        let mut direct = vec![];
        for _ in 0..3 {
            for optimized in [false, true] {
                let start = Instant::now();
                let mut queries = 0u64;
                while start.elapsed() < Duration::from_secs(1) {
                    for p in &positions {
                        for color in 0..2 {
                            let p = black_box(p);
                            black_box(if optimized {
                                p.classical_attacked_direct(color)
                            } else {
                                p.attacked(color)
                            });
                            queries += 1;
                        }
                    }
                }
                let ns = start.elapsed().as_secs_f64() * 1e9 / queries as f64;
                if optimized {
                    direct.push(ns);
                } else {
                    old.push(ns);
                }
            }
        }
        old.sort_by(f64::total_cmp);
        direct.sort_by(f64::total_cmp);
        println!(
            "{}",
            serde_json::json!({"ruleId":R::ID,"positions":positions.len(),"queriesPerPosition":2,"oldMedianNs":old[1],"directMedianNs":direct[1],"speedup":old[1]/direct[1],"oldSamplesNs":old,"directSamplesNs":direct})
        );
    }
    run::<Washogi>();
    run::<HeianDai>();
}

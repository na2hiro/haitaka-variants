use super::*;

pub(super) fn assert_incremental(g: &LargeGame) {
    assert_eq!(g.timeline.len(), g.undo.len() + 1);
    assert_eq!(g.played.len(), g.undo.len());
    let current = g.timeline.last().unwrap();
    assert_eq!(current.key, hash::full(&g.position));
    assert_eq!(current.side, g.position.side);
    let context = g.timeline.iter().fold(hash::HISTORY_SEED, |h, p| {
        h.rotate_left(7) ^ p.key.wrapping_mul(0x9e3779b97f4a7c15)
    });
    assert_eq!(current.context, context);
}
fn legacy_context(g: &LargeGame, hashes: &[u64]) -> u64 {
    let mut terminal = *hashes.last().unwrap();
    if let Some(p) = g.prefix() {
        for n in p.from.into_iter().chain(p.path.iter().flatten().copied()) {
            terminal = terminal.rotate_left(7) ^ u64::from(n).wrapping_mul(0x9e3779b97f4a7c15);
        }
    }
    hashes.iter().fold(0x6a09e667f3bcc909u64, |h, &p| {
        h.rotate_left(7) ^ p.wrapping_mul(0x9e3779b97f4a7c15)
    }) ^ terminal.rotate_left(13)
}
fn random(seed: &mut u32) -> usize {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed as usize
}
#[test]
fn incremental_keys_and_legacy_context_match_128_seeded_games_and_long_history() {
    let initial = LargeGame::parse(&compiled::start_position_json()).unwrap();
    for game in 0..129u32 {
        let mut g = initial.clone();
        let root_json = g.to_json();
        let root_key = g.context_key();
        let mut hashes = vec![g.position.hash()];
        let mut seed = 0x2a98c10b ^ RULE_ID ^ (game + 1).wrapping_mul(0x9e3779b9);
        let plies = if game == 128 { 128 } else { 16 };
        for ply in 0..plies {
            let moves = g.moves();
            if moves.is_empty() {
                break;
            }
            let m = moves[random(&mut seed) % moves.len()];
            let old_key = g.context_key();
            let old_position = g.position.clone();
            g.push(m);
            hashes.push(g.position.hash());
            assert_incremental(&g);
            assert_eq!(g.context_hash(), legacy_context(&g, &hashes));
            let key = g.context_key();
            g.pop();
            hashes.pop();
            assert_eq!(g.position, old_position);
            assert_eq!(g.context_key(), old_key);
            assert_eq!(g.context_hash(), legacy_context(&g, &hashes));
            g.push(m);
            hashes.push(g.position.hash());
            assert_eq!(g.context_key(), key);
            if ply % 8 == 7 {
                let reloaded = LargeGame::parse(&g.to_json()).unwrap();
                assert_eq!(reloaded.context_key(), key);
                assert_eq!(reloaded.context_hash(), legacy_context(&g, &hashes));
            }
        }
        while !g.undo.is_empty() {
            g.pop();
            hashes.pop();
            assert_eq!(g.context_hash(), legacy_context(&g, &hashes));
        }
        assert_eq!(g.to_json(), root_json);
        assert_eq!(g.context_key(), root_key);
    }
}
#[test]
fn delta_repeated_squares_side_state_order_and_all_internal_piece_ids() {
    let mut p = LargeGame::parse(&compiled::start_position_json())
        .unwrap()
        .position;
    let original = p.clone();
    let key = hash::full(&p);
    let i = Geometry::SQUARES - 1;
    let j = Geometry::SQUARES - 2;
    let mut identities = BTreeSet::new();
    for id in 0..PIECES.len() {
        for color in 0..2 {
            let mut d = Delta::new(&p);
            d.set(&mut p, i, None);
            d.set(
                &mut p,
                i,
                Some(Cell {
                    piece: PieceId(id as u16),
                    color,
                }),
            );
            assert!(identities.insert(hash::full(&p)));
            d.set(&mut p, j, None);
            // A return-to-origin / igui path may touch a square repeatedly.
            d.set(&mut p, i, original.cells[i]);
            d.set(
                &mut p,
                i,
                Some(Cell {
                    piece: PieceId(id as u16),
                    color,
                }),
            );
            p.side ^= 1;
            p.state.senjishi = !p.state.senjishi;
            p.state.shitenno = vec![
                Resident {
                    at: Pos::at(i),
                    color: 0,
                },
                Resident {
                    at: Pos::at(j),
                    color: 1,
                },
            ];
            assert_eq!(hash::updated(key, &p, &d), hash::full(&p));
            let ordered = hash::full(&p);
            p.state.shitenno.reverse();
            assert_ne!(ordered, hash::full(&p));
            assert_eq!(hash::updated(key, &p, &d), hash::full(&p));
            d.restore(&mut p);
            assert_eq!(p, original);
            assert_eq!(hash::full(&p), key);
        }
    }
    // Wide square indices must not alias; identities include promotion origin.
    let mut positions = BTreeSet::new();
    let mut indices = vec![0, 1, 255, 256, Geometry::SQUARES - 1];
    indices.retain(|&i| i < Geometry::SQUARES);
    indices.sort_unstable();
    indices.dedup();
    for i in indices {
        let mut d = Delta::new(&p);
        d.set(
            &mut p,
            i,
            Some(Cell {
                piece: PieceId(0),
                color: 0,
            }),
        );
        assert!(positions.insert(hash::full(&p)));
        assert_eq!(hash::updated(key, &p, &d), hash::full(&p));
        d.restore(&mut p);
    }
}
fn sparse() -> LargeGame {
    let royal = PIECES.iter().position(|p| p.royal).unwrap();
    let mut p = LargeGame::parse(&compiled::start_position_json())
        .unwrap()
        .position;
    for i in 0..Geometry::SQUARES {
        p.set_cell(i, None);
    }
    p.state = TenjikuState::default();
    for (i, color) in [
        (Geometry::SQUARES - 1, 0),
        (Pos::index([Geometry::WIDTH as u16, 1]).unwrap(), 1),
    ] {
        p.set_cell(
            i,
            Some(Cell {
                piece: PieceId(royal as u16),
                color,
            }),
        );
    }
    LargeGame::root(p).unwrap()
}
#[test]
fn same_board_different_history_has_distinct_search_context_and_repeats_correctly() {
    let mut g = sparse();
    let initial = g.position.clone();
    let root_key = g.context_key();
    let root_position_key = g.timeline[0].key;
    let n = Geometry::WIDTH as u16;
    let cycle = [
        ([n, n], [n - 1, n]),
        ([n, 1], [n - 1, 1]),
        ([n - 1, n], [n, n]),
        ([n - 1, 1], [n, 1]),
    ];
    for repetition in 0..3 {
        for (from, to) in cycle {
            g.play(LargeMove {
                schema_version: SCHEMA_VERSION,
                from,
                path: TurnPath::one(to),
                promotion: false,
            })
            .unwrap();
        }
        assert_eq!(g.position, initial);
        assert_eq!(g.timeline.last().unwrap().key, root_position_key);
        assert_ne!(g.context_key(), root_key);
        assert_eq!(
            g.basic_outcome(),
            if repetition == 2 {
                Outcome::DRAW
            } else {
                Outcome::ONGOING
            }
        );
    }
    while !g.undo.is_empty() {
        g.pop();
    }
    assert_eq!(g.context_key(), root_key);
}
#[test]
fn pending_paths_remain_in_search_and_legacy_context_after_resume_and_undo() {
    let mut g = sparse();
    let piece = MOVEMENT.iter().position(|m| !m.states.is_empty()).unwrap();
    let middle = Geometry::index(Geometry::WIDTH / 2, Geometry::HEIGHT / 2).unwrap();
    g.position.set_cell(
        middle,
        Some(Cell {
            piece: PieceId(piece as u16),
            color: 0,
        }),
    );
    g = LargeGame::root(g.position).unwrap();
    let longest = g.moves().into_iter().max_by_key(|m| m.path.len()).unwrap();
    assert!(longest.path.len() >= 2);
    let base_key = g.context_key();
    for hops in 1..longest.path.len() {
        let mut q: Request = serde_json::from_str(&g.to_json()).unwrap();
        q.pending = Some(Pending {
            from: longest.from,
            path: BoundedPath::from_steps(&longest.path[..hops]).unwrap(),
        });
        let mut pending = LargeGame::parse(&serde_json::to_string(&q).unwrap()).unwrap();
        let key = pending.context_key();
        assert_ne!(key, base_key);
        assert_eq!(pending.timeline[0].key, g.timeline[0].key);
        assert_eq!(
            pending.context_hash(),
            legacy_context(&pending, &[pending.position.hash()])
        );
        pending.play(longest).unwrap();
        assert!(pending.prefix().is_none());
        assert_incremental(&pending);
        let reloaded = LargeGame::parse(&pending.to_json()).unwrap();
        assert_eq!(reloaded.context_key(), pending.context_key());
        pending.pop();
        assert_eq!(pending.context_key(), key);
        assert!(pending.prefix().is_some());
    }
}
#[test]
#[ignore = "run explicitly on an idle, pinned CPU"]
fn benchmark_full_fnv_and_delta_key() {
    use std::{
        hint::black_box,
        time::{Duration, Instant},
    };
    let mut g = LargeGame::parse(&compiled::start_position_json()).unwrap();
    let mut cases = vec![];
    let mut seed = 0x91a02c36 ^ RULE_ID;
    for _ in 0..64 {
        let moves = g.moves();
        if moves.is_empty() {
            break;
        }
        let before = g.timeline.last().unwrap().key;
        g.push(moves[random(&mut seed) % moves.len()]);
        cases.push((g.position.clone(), g.undo.last().unwrap().clone(), before));
    }
    let mut old = vec![];
    let mut delta = vec![];
    for _ in 0..3 {
        for optimized in [false, true] {
            let start = Instant::now();
            let mut calls = 0u64;
            while start.elapsed() < Duration::from_secs(1) {
                for (p, d, before) in &cases {
                    black_box(if optimized {
                        hash::updated(black_box(*before), black_box(p), black_box(d))
                    } else {
                        black_box(p).hash()
                    });
                    calls += 1;
                }
            }
            let ns = start.elapsed().as_secs_f64() * 1e9 / calls as f64;
            if optimized {
                delta.push(ns);
            } else {
                old.push(ns);
            }
        }
    }
    old.sort_by(f64::total_cmp);
    delta.sort_by(f64::total_cmp);
    println!(
        "{}",
        serde_json::json!({
            "rule": NAME, "cases": cases.len(), "oldMedianNs": old[1],
            "deltaMedianNs": delta[1], "speedup": old[1]/delta[1],
            "oldSamplesNs": old, "deltaSamplesNs": delta,
            "timelineBytesPerPly": std::mem::size_of::<TimelineEntry>(),
            "priorTimelineBytesPerPly": std::mem::size_of::<(u64,u8)>()
        })
    );
}

#[test]
fn inline_delta_spills_without_losing_any_square_and_restores_its_clone() {
    let mut p = LargeGame::parse(&compiled::start_position_json())
        .unwrap()
        .position;
    let original = p.clone();
    let key = hash::full(&p);
    let occupied: Vec<_> = p
        .cells
        .iter()
        .enumerate()
        .filter_map(|(i, c)| c.map(|_| i))
        .collect();
    assert!(occupied.len() > 4);
    let mut delta = Delta::new(&p);
    for &i in &occupied {
        delta.set(&mut p, i, None);
        // Repeated touches retain the first old value, including after spill.
        delta.set(&mut p, i, original.cells[i]);
        delta.set(&mut p, i, None);
    }
    assert_eq!(delta.squares.iter().count(), occupied.len());
    p.side ^= 1;
    p.state.senjishi = !p.state.senjishi;
    assert_eq!(hash::updated(key, &p, &delta), hash::full(&p));
    let mut restored_clone = p.clone();
    delta.clone().restore(&mut restored_clone);
    assert_eq!(restored_clone, original);
    delta.restore(&mut p);
    assert_eq!(p, original);
    assert_eq!(hash::full(&p), key);
}

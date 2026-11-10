use super::*;

pub(super) fn reference(
    p: &Pos,
    from: usize,
    status: Option<u8>,
    ignore: Option<usize>,
    wanted: Option<usize>,
    cached_active: Option<&[Resident]>,
) -> Vec<Target> {
    let Some(c) = at_cell(p, from, ignore) else {
        return vec![];
    };
    if MAKA_PROMOTION && Pos::def(c).id == "ix" {
        if let Some(to) = wanted {
            if at_cell(p, to, ignore).is_none() {
                return vec![Target {
                    to,
                    next: None,
                    through: false,
                }];
            }
            if at_cell(p, to, ignore).is_some_and(|v| v.color == c.color) {
                return vec![];
            }
            let defended = p.cells.iter().enumerate().any(|(i, v)| {
                Some(i) != ignore
                    && v.is_some_and(|v| v.color != c.color && Pos::def(v).id != "ix")
                    && targets(p, i, None, ignore).iter().any(|t| t.to == to)
            });
            return if defended {
                vec![]
            } else {
                vec![Target {
                    to,
                    next: None,
                    through: false,
                }]
            };
        }
        let mut defended = BTreeSet::new();
        for (i, v) in p.cells.iter().enumerate() {
            if Some(i) != ignore && v.is_some_and(|v| v.color != c.color && Pos::def(v).id != "ix")
            {
                defended.extend(targets(p, i, None, ignore).into_iter().map(|t| t.to));
            }
        }
        return (0..Geometry::SQUARES)
            .filter(|&i| at_cell(p, i, ignore).is_none() || !defended.contains(&i))
            .map(|to| Target {
                to,
                next: None,
                through: false,
            })
            .collect();
    }
    let m = &MOVEMENT[c.piece.0 as usize];
    if status.is_none() {
        let computed;
        let actives = match cached_active {
            Some(a) => a,
            None => {
                computed = active(p, ignore);
                &computed
            }
        };
        if frozen(from, c.color, actives) {
            return vec![];
        }
    }
    let prims = if let Some(s) = status {
        m.states.get(s as usize).copied().unwrap_or(&[])
    } else {
        m.root
    };
    let mut raw: Vec<(usize, i8, bool)> = vec![];
    let mut prohibited = BTreeSet::new();
    let sign = if c.color == 0 { 1 } else { -1 };
    for prim in prims {
        let (dx, dy) = (prim.delta.0 * sign, prim.delta.1 * sign);
        let mut now = from;
        for _ in 0..=prim.skip {
            if let Some(next) = Geometry::offset(now, dx, dy) {
                now = next;
            } else {
                now = usize::MAX;
                break;
            }
        }
        if now == usize::MAX {
            continue;
        }
        let mut blocked = false;
        #[cfg(feature = "taikyoku")]
        for &(ex, ey) in prim.empty {
            if Geometry::offset(
                from,
                dx + (if dx > 0 { 1 } else { -1 }) * ex,
                dy + (if dy > 0 { 1 } else { -1 }) * ey,
            )
            .is_some_and(|i| at_cell(p, i, ignore).is_some())
            {
                blocked = true;
                break;
            }
        }
        for r in prim.occupied {
            if let Some(i) = Geometry::offset(
                from,
                dx + (if dx > 0 { 1 } else { -1 }) * r.delta.0,
                dy + (if dy > 0 { 1 } else { -1 }) * r.delta.1,
            ) {
                if !at_cell(p, i, ignore)
                    .is_some_and(|v| r.kind == 3 || (r.kind == 1) == (v.color == c.color))
                {
                    blocked = true;
                    break;
                }
            }
        }
        if blocked {
            prohibited.insert(now);
            continue;
        }
        let ray = matches!(prim.kind, 2 | 6 | 100 | 102) || prim.limit > 0;
        let max = if prim.limit > 0 {
            usize::from(prim.limit)
        } else if ray {
            Geometry::SQUARES
        } else {
            1
        };
        #[cfg(feature = "taikyoku")]
        let mut jumps = 0;
        for _ in 0..max {
            #[cfg(feature = "taikyoku")]
            if jumps > prim.jump_limit {
                break;
            }
            let v = at_cell(p, now, ignore);
            if v.is_some_and(|v| m.exceptions.contains(&Pos::def(v).id)) {
                break;
            }
            if let Some(t) = raw.iter_mut().find(|t| t.0 == now) {
                t.2 |= prim.kind == 100;
                if t.1 == -2 && prim.next != -2 {
                    t.1 = prim.next;
                }
            } else {
                raw.push((now, prim.next, prim.kind == 100));
            }
            if prim.kind == 102 && v.is_none() {
                for (tx, ty) in [(-dy, dx), (dy, -dx)] {
                    let mut turn = now;
                    while let Some(j) = Geometry::offset(turn, tx, ty) {
                        if !raw.iter().any(|r| r.0 == j) {
                            raw.push((j, prim.next, false));
                        }
                        if at_cell(p, j, ignore).is_some() {
                            break;
                        }
                        turn = j;
                    }
                }
            }
            #[cfg(feature = "taikyoku")]
            if v.is_some() {
                jumps += 1;
            }
            if !ray || (!matches!(prim.kind, 6 | 100) && v.is_some()) {
                break;
            }
            let Some(next) = Geometry::offset(now, dx, dy) else {
                break;
            };
            now = next;
        }
    }
    if status.is_none() && Pos::def(c).id == "fp" {
        let mut frontier = BTreeSet::from([from]);
        for step in 1..=3 {
            let mut next = BTreeSet::new();
            for via in frontier {
                for &(dx, dy) in KING {
                    let Some(to) = Geometry::offset(via, dx, dy) else {
                        continue;
                    };
                    if to == from {
                        continue;
                    }
                    let v = at_cell(p, to, ignore);
                    if step == 1 {
                        next.insert(to);
                    } else if !v.is_some_and(|v| v.color == c.color) {
                        if !adjacent(from, to) && !raw.iter().any(|r| r.0 == to) {
                            raw.push((to, -2, false));
                        }
                        if v.is_none() {
                            next.insert(to);
                        }
                    }
                }
            }
            frontier = next;
        }
    }
    raw.into_iter()
        .filter(|(to, _, _)| !prohibited.contains(to))
        .map(|(to, n, through)| Target {
            to,
            through,
            next: if n >= 0 && !(m.stop_capture && at_cell(p, to, ignore).is_some()) {
                Some(n as u8)
            } else {
                None
            },
        })
        .collect()
}

#[test]
fn selected_targets_match_full_generator_for_every_piece_state_and_ignored_blockers() {
    let mut p = LargeGame::parse(&compiled::start_position_json())
        .unwrap()
        .position;
    for i in 0..Geometry::SQUARES {
        p.set_cell(i, None);
    }
    p.state = TenjikuState::default();
    let from = Geometry::index(Geometry::WIDTH / 2, Geometry::HEIGHT / 2).unwrap();
    for color in 0..2 {
        for id in 0..PIECES.len() {
            p.set_cell(
                from,
                Some(Cell {
                    piece: PieceId(id as u16),
                    color,
                }),
            );
            let mut blockers = vec![];
            for (n, &(dx, dy)) in KING.iter().enumerate() {
                let to = Geometry::offset(from, dx * 2, dy * 2).unwrap();
                p.set_cell(
                    to,
                    Some(Cell {
                        piece: PieceId(0),
                        color: (n as u8 % 2) ^ color,
                    }),
                );
                blockers.push(to);
            }
            for status in
                std::iter::once(None).chain((0..MOVEMENT[id].states.len()).map(|s| Some(s as u8)))
            {
                for ignore in [None, Some(blockers[0])] {
                    let full = reference(&p, from, status, ignore, None, None);
                    assert_eq!(
                        targets_with_active(&p, from, status, ignore, None, None),
                        full,
                        "full destinations: piece {id}, status {status:?}, color {color}"
                    );
                    let mut wanted: Vec<_> = full
                        .iter()
                        .take(16)
                        .chain(full.iter().rev().take(16))
                        .map(|t| t.to)
                        .collect();
                    wanted.extend([0, from, Geometry::SQUARES - 1]);
                    wanted.extend(blockers.iter().copied());
                    wanted.sort_unstable();
                    wanted.dedup();
                    for to in wanted {
                        let result = targets_with_active(&p, from, status, ignore, Some(to), None);
                        let expected: Vec<_> = reference(&p, from, status, ignore, Some(to), None)
                            .into_iter()
                            .filter(|t| t.to == to)
                            .collect();
                        assert_eq!(
                            result, expected,
                            "piece {id}, status {status:?}, color {color}, to {to}"
                        );
                    }
                }
            }
            p.set_cell(from, None);
            for to in blockers {
                p.set_cell(to, None);
            }
        }
    }
}

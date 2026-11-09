//! Single-destination queries. Special path families retain the general evaluator.
use super::*;

pub(super) fn target_one(
    p: &Pos,
    from: usize,
    status: Option<u8>,
    ignore: Option<usize>,
    to: usize,
    cached: Option<&[Resident]>,
) -> Option<Target> {
    let result = selected(p, from, status, ignore, to, cached);
    #[cfg(test)]
    {
        let expected = known_target_tests::reference(p, from, status, ignore, Some(to), cached)
            .into_iter()
            .find(|t| t.to == to);
        assert_eq!(
            result, expected,
            "single target from={from} to={to} state={status:?}"
        );
    }
    result
}
fn selected(
    p: &Pos,
    from: usize,
    status: Option<u8>,
    ignore: Option<usize>,
    to: usize,
    cached: Option<&[Resident]>,
) -> Option<Target> {
    let c = at_cell(p, from, ignore)?;
    if to >= Geometry::SQUARES {
        return None;
    }
    let m = &MOVEMENT[c.piece.0 as usize];
    let prims = status.map_or(m.root, |s| m.states.get(s as usize).copied().unwrap_or(&[]));
    if Pos::def(c).id == "fp"
        || (MAKA_PROMOTION && Pos::def(c).id == "ix")
        || prims.iter().any(|v| v.kind == 102 || v.delta == (0, 0))
    {
        return targets_selected(p, from, status, ignore, Some(to), cached)
            .into_iter()
            .next();
    }
    if status.is_none() {
        let computed;
        let actives = match cached {
            Some(a) => a,
            None => {
                computed = active(p, ignore);
                &computed
            }
        };
        if frozen(from, c.color, actives) {
            return None;
        }
    }
    let (fx, fy) = Geometry::coordinates(from)?;
    let (tx, ty) = Geometry::coordinates(to)?;
    let (ax, ay) = (tx as isize - fx as isize, ty as isize - fy as isize);
    let sign = if c.color == 0 { 1 } else { -1 };
    let mut raw: Option<(i8, bool)> = None;
    let mut prohibited = false;
    for prim in prims {
        let (dx, dy) = (prim.delta.0 * sign, prim.delta.1 * sign);
        let k = if dx != 0 {
            if ax % dx != 0 {
                continue;
            }
            ax / dx
        } else {
            if ax != 0 || ay % dy != 0 {
                continue;
            }
            ay / dy
        };
        if k < 1 || ax != k * dx || ay != k * dy {
            continue;
        }
        let start = isize::from(prim.skip) + 1;
        let ray = matches!(prim.kind, 2 | 6 | 100 | 102) || prim.limit > 0;
        let max = if prim.limit > 0 {
            usize::from(prim.limit)
        } else if ray {
            Geometry::SQUARES
        } else {
            1
        };
        if k < start || (k - start) as usize >= max {
            continue;
        }
        let mut now = from;
        for _ in 0..start {
            now = Geometry::offset(now, dx, dy)?;
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
            if now == to {
                prohibited = true;
            }
            continue;
        }
        #[cfg(feature = "taikyoku")]
        let mut jumps = 0;
        for step in start..=k {
            #[cfg(feature = "taikyoku")]
            if jumps > prim.jump_limit {
                break;
            }
            let v = at_cell(p, now, ignore);
            if v.is_some_and(|v| m.exceptions.contains(&Pos::def(v).id)) {
                break;
            }
            if step == k {
                match &mut raw {
                    Some((n, through)) => {
                        *through |= prim.kind == 100;
                        if *n == -2 && prim.next != -2 {
                            *n = prim.next;
                        }
                    }
                    None => raw = Some((prim.next, prim.kind == 100)),
                }
                break;
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
    if prohibited {
        return None;
    }
    raw.map(|(n, through)| Target {
        to,
        through,
        next: if n >= 0 && !(m.stop_capture && at_cell(p, to, ignore).is_some()) {
            Some(n as u8)
        } else {
            None
        },
    })
}

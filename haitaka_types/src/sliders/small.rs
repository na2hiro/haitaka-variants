//! Precomputed rays and first-blocker clipping for compact boards.
use crate::*;

const DIRECTIONS: [(i8, i8); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];
const RAYS: [[BitBoard; Square::NUM]; 8] = {
    let mut rays = [[BitBoard::EMPTY; Square::NUM]; 8];
    let mut d = 0;
    while d < 8 {
        let mut i = 0;
        while i < Square::NUM {
            let mut square = Square::index_const(i);
            while let Some(next) = square.try_offset(DIRECTIONS[d].0, DIRECTIONS[d].1) {
                rays[d][i].0 |= next.bitboard().0;
                square = next;
            }
            i += 1;
        }
        d += 1;
    }
    rays
};

#[inline(always)]
const fn clipped_ray(direction: usize, square: Square, occupied: BitBoard) -> BitBoard {
    let ray = RAYS[direction][square as usize];
    let blockers = ray.0 & occupied.0;
    if blockers == 0 {
        return ray;
    }
    let (dx, dy) = DIRECTIONS[direction];
    let first = if dx as i32 * Rank::NUM as i32 + dy as i32 > 0 {
        blockers.trailing_zeros() as usize
    } else {
        BitBoardWord::BITS as usize - 1 - blockers.leading_zeros() as usize
    };
    BitBoard(ray.0 & !RAYS[direction][first].0)
}

#[inline(always)]
pub const fn small_rook_moves(square: Square, occupied: BitBoard) -> BitBoard {
    get_rook_file_moves(square, occupied).bitor(get_rook_rank_moves(square, occupied))
}
#[inline(always)]
pub const fn small_bishop_moves(square: Square, occupied: BitBoard) -> BitBoard {
    clipped_ray(4, square, occupied)
        .bitor(clipped_ray(5, square, occupied))
        .bitor(clipped_ray(6, square, occupied))
        .bitor(clipped_ray(7, square, occupied))
}
#[inline(always)]
pub const fn get_rook_file_moves(square: Square, occupied: BitBoard) -> BitBoard {
    clipped_ray(2, square, occupied).bitor(clipped_ray(3, square, occupied))
}
#[inline(always)]
pub const fn get_rook_rank_moves(square: Square, occupied: BitBoard) -> BitBoard {
    clipped_ray(0, square, occupied).bitor(clipped_ray(1, square, occupied))
}
#[inline(always)]
pub const fn get_lance_moves(color: Color, square: Square, occupied: BitBoard) -> BitBoard {
    clipped_ray(
        match color {
            Color::Black => 3,
            Color::White => 2,
        },
        square,
        occupied,
    )
}
pub const fn rook_pseudo_attacks(square: Square) -> BitBoard {
    small_rook_moves(square, BitBoard::EMPTY)
}
pub const fn bishop_pseudo_attacks(square: Square) -> BitBoard {
    small_bishop_moves(square, BitBoard::EMPTY)
}
pub const fn lance_pseudo_attacks(color: Color, square: Square) -> BitBoard {
    get_lance_moves(color, square, BitBoard::EMPTY)
}

// Independent step-by-step versions are useful to verify the first-blocker code.
const fn slow(square: Square, occupied: BitBoard, start: usize, end: usize) -> BitBoard {
    let mut out = BitBoard::EMPTY;
    let mut d = start;
    while d < end {
        let mut current = square;
        while let Some(next) = current.try_offset(DIRECTIONS[d].0, DIRECTIONS[d].1) {
            out.0 |= next.bitboard().0;
            if occupied.has(next) {
                break;
            }
            current = next;
        }
        d += 1;
    }
    out
}
pub const fn get_rook_moves_slow(s: Square, o: BitBoard) -> BitBoard {
    slow(s, o, 0, 4)
}
pub const fn get_bishop_moves_slow(s: Square, o: BitBoard) -> BitBoard {
    slow(s, o, 4, 8)
}
pub const fn get_lance_moves_slow(s: Square, o: BitBoard, c: Color) -> BitBoard {
    let d = match c {
        Color::Black => 3,
        Color::White => 2,
    };
    slow(s, o, d, d + 1)
}
pub const fn get_rook_relevant_blockers(s: Square) -> BitBoard {
    rook_pseudo_attacks(s)
}
pub const fn get_bishop_relevant_blockers(s: Square) -> BitBoard {
    bishop_pseudo_attacks(s)
}
pub const fn get_lance_relevant_blockers(s: Square, c: Color) -> BitBoard {
    lance_pseudo_attacks(c, s)
}

const fn between(from: Square, to: Square) -> BitBoard {
    let dx = to.file() as i8 - from.file() as i8;
    let dy = to.rank() as i8 - from.rank() as i8;
    if (dx == 0 && dy == 0) || (dx != 0 && dy != 0 && dx.abs() != dy.abs()) {
        return BitBoard::EMPTY;
    }
    let mut out = BitBoard::EMPTY;
    let mut current = from;
    while let Some(next) = current.try_offset(dx.signum(), dy.signum()) {
        if next as usize == to as usize {
            break;
        }
        out.0 |= next.bitboard().0;
        current = next;
    }
    out
}
#[inline(always)]
pub const fn get_between_rays(from: Square, to: Square) -> BitBoard {
    const TABLE: [[BitBoard; Square::NUM]; Square::NUM] = {
        let mut out = [[BitBoard::EMPTY; Square::NUM]; Square::NUM];
        let mut i = 0;
        while i < Square::NUM {
            let mut j = 0;
            while j < Square::NUM {
                out[i][j] = between(Square::index_const(i), Square::index_const(j));
                j += 1;
            }
            i += 1;
        }
        out
    };
    TABLE[from as usize][to as usize]
}
#[inline(always)]
pub const fn line_ray(from: Square, to: Square) -> BitBoard {
    let mut d = 0;
    while d < 8 {
        if RAYS[d][from as usize].has(to) {
            let opposite = d ^ 1;
            // Diagonal ordering pairs (4,7) and (5,6), not (4,5).
            let opposite = if d >= 4 { 11 - d } else { opposite };
            return RAYS[d][from as usize]
                .bitor(RAYS[opposite][from as usize])
                .bitor(from.bitboard());
        }
        d += 1;
    }
    BitBoard::EMPTY
}

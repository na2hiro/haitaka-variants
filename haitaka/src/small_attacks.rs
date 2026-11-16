//! Compile-time step attack tables for compact boards.
use crate::*;

const fn steps(square: Square, color: Color, offsets: &[(i8, i8)]) -> BitBoard {
    let mut out = BitBoard::EMPTY;
    let mut i = 0;
    let orientation = match color {
        Color::Black => 1,
        Color::White => -1,
    };
    while i < offsets.len() {
        if let Some(to) = square.try_offset(offsets[i].0 * orientation, offsets[i].1 * orientation)
        {
            out.0 |= to.bitboard().0;
        }
        i += 1;
    }
    out
}
macro_rules! attacks {
    ($name:ident, $offsets:expr) => {
        #[inline(always)]
        pub const fn $name(color: Color, square: Square) -> BitBoard {
            const TABLE: [[BitBoard; Square::NUM]; 2] = {
                let mut table = [[BitBoard::EMPTY; Square::NUM]; 2];
                let mut i = 0;
                while i < Square::NUM {
                    table[Color::Black as usize][i] =
                        steps(Square::index_const(i), Color::Black, $offsets);
                    table[Color::White as usize][i] =
                        steps(Square::index_const(i), Color::White, $offsets);
                    i += 1;
                }
                table
            };
            TABLE[color as usize][square as usize]
        }
    };
}
attacks!(pawn_attacks, &[(0, -1)]);
attacks!(knight_attacks, &[(1, -2), (-1, -2)]);
attacks!(
    silver_attacks,
    &[(0, -1), (1, -1), (-1, -1), (1, 1), (-1, 1)]
);
attacks!(
    gold_attacks,
    &[(0, -1), (1, -1), (-1, -1), (1, 0), (-1, 0), (0, 1)]
);
attacks!(
    king_attacks,
    &[
        (0, -1),
        (1, -1),
        (-1, -1),
        (1, 0),
        (-1, 0),
        (0, 1),
        (1, 1),
        (-1, 1)
    ]
);

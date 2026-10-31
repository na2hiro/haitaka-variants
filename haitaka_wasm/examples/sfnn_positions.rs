//! Deterministic legal positions for the external-reference P1 fixtures.
use haitaka::Board;
fn main() {
    let mut state = 1u64;
    for game in 0..4 {
        let mut board = Board::startpos();
        for ply in 0..256 {
            if ply % 8 == 0 || ply % 8 == 1 {
                println!("{game}:{ply}\t{board}");
            }
            let mut moves = Vec::new();
            board.generate_moves(|group| {
                moves.extend(group);
                false
            });
            if moves.is_empty() {
                break;
            }
            // Explicit LCG algorithm: independent of rand/StdRng implementation changes.
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            board.play(moves[(state % moves.len() as u64) as usize]);
        }
    }
}

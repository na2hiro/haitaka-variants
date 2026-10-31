//! P4 fixed opening suite: LCG64 seed 1, fifty legal eight-ply positions.
use haitaka::Board;

fn main() {
    let mut state = 1u64;
    for _ in 0..50 {
        let mut board = Board::startpos();
        for _ in 0..8 {
            let mut moves = Vec::new();
            board.generate_moves(|group| {
                moves.extend(group);
                false
            });
            assert!(!moves.is_empty());
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            board.play(moves[(state % moves.len() as u64) as usize]);
        }
        println!("{board}");
    }
}

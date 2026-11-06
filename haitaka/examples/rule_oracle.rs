//! Complete legal-move transitions for comparison with a separate rule implementation.
use haitaka::*;
fn emit(board: &Board) -> Vec<Move> {
    println!("{}\t=\t{}\t{:?}", board, board, board.status());
    let mut legal = Vec::new();
    board.generate_moves(|group| {
        legal.extend(group);
        false
    });
    legal.sort_by_key(ToString::to_string);
    for mv in &legal {
        let mut child = board.clone();
        child.play_unchecked(*mv);
        println!("{}\t{}\t{}\t{:?}", board, mv, child, child.status());
    }
    legal
}
fn main() {
    let count = std::env::args()
        .nth(1)
        .map(|s| s.parse::<usize>().unwrap())
        .unwrap_or(128);
    let mut board = Board::startpos();
    let mut seed = 0x571ab13u64;
    for _ in 0..count {
        let moves = emit(&board);
        if moves.is_empty() {
            board = Board::startpos();
        } else {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            board.play_unchecked(moves[seed as usize % moves.len()]);
        }
    }
    if let Some(path) = std::env::args().nth(2) {
        for sfen in std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .filter(|s| !s.is_empty())
        {
            emit(&sfen.parse().unwrap());
        }
    }
}

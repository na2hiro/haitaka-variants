//! Emit deterministic positions and complete legal-move sets for another engine.
use haitaka::{Board, Move};

fn main() {
    let count = std::env::args()
        .nth(1)
        .map(|v| v.parse::<usize>().expect("position count"))
        .unwrap_or(128);
    let mut board = Board::startpos();
    let mut seed = 0x5a17101u64;
    for index in 0..count {
        let mut legal: Vec<Move> = Vec::new();
        board.generate_moves(|group| {
            legal.extend(group);
            false
        });
        let mut names = legal.iter().map(ToString::to_string).collect::<Vec<_>>();
        names.sort();
        println!("{}\t{}", board, names.join(" "));
        if legal.is_empty() || index % 64 == 63 {
            board = Board::startpos();
        } else {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            board.play_unchecked(legal[seed as usize % legal.len()]);
        }
    }
}

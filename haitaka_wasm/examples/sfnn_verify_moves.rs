//! P4 independent legality check for recorded browser search replies.
use haitaka::Board;
use std::io::{self, BufRead};

fn main() {
    let mut checked = 0;
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let (sfen, usi) = line.split_once('\t').expect("SFEN TAB USI required");
        let board = Board::from_sfen(sfen).expect("valid corpus SFEN");
        let mut legal = false;
        board.generate_moves(|group| {
            for mv in group {
                if mv.to_string() == usi {
                    legal = true;
                }
            }
            false
        });
        assert!(legal, "illegal reply {usi} for {sfen}");
        checked += 1;
    }
    assert!(checked > 0, "no recorded replies supplied");
    println!("PASS: {checked} browser replies are legal");
}

// Verification-only WASM wrapper. Production exports remain unchanged.
use haitaka::{Board, Move};
use std::fmt::Write;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn oracle(roots: &str) -> Result<String, JsValue> {
    let mut out = String::new();
    for root in roots.lines() {
        let board = Board::from_sfen(root).map_err(|e| JsValue::from_str(&e.to_string()))?;
        writeln!(out, "{}\t=\t{}\t{:?}", board, board, board.status()).unwrap();
        let mut moves: Vec<Move> = Vec::new();
        board.generate_moves(|group| { moves.extend(group); false });
        moves.sort_by_key(ToString::to_string);
        for mv in moves {
            let mut child = board.clone();
            child.play_unchecked(mv);
            writeln!(out, "{}\t{}\t{}\t{:?}", board, mv, child, child.status()).unwrap();
        }
    }
    Ok(out)
}

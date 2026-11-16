//! Static diagnostic only; counterfactual hands are never used to generate moves.
use haitaka::{Board, Color, Piece, RuleSpec, SelectedRule, Square};
use haitaka_wasm::{
    handcrafted_static_eval,
    sfnn::{MICRO_ARCH, MINI_ARCH, SfnnModel},
};
use serde_json::json;
use std::{
    io::{self, BufRead},
    path::PathBuf,
};

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let dir = PathBuf::from(&args[1]);
    let arch = if args.get(2).map(String::as_str) == Some("mini") {
        MINI_ARCH
    } else {
        MICRO_ARCH
    };
    let nn = std::fs::read(dir.join("nn.bin")).unwrap();
    let progress = std::fs::read(dir.join("progress.bin")).unwrap();
    let model = SfnnModel::from_bytes(&nn, &progress, arch).unwrap();
    let mut zero = SfnnModel::from_bytes(&nn, &progress, arch).unwrap();
    zero.diagnostic_zero_missing_slot();
    println!(
        "{}",
        json!({"kind":"metadata","rule":SelectedRule::NAME,"arch":arch,"missingSlotRow":model.diagnostic_missing_slot()})
    );
    let mut cases = vec![("start".to_string(), Board::startpos())];
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let fields = line.split('\t').collect::<Vec<_>>();
        let id = fields[0];
        let mut board = Board::from_sfen(fields[1]).unwrap();
        cases.push((format!("{id}:0"), board.clone()));
        if let Some(moves) = fields.get(2) {
            for (ply, text) in moves.split_whitespace().enumerate() {
                board.try_play(text.parse().unwrap()).unwrap();
                if (ply + 1) % 12 == 0 {
                    cases.push((format!("{id}:{}", ply + 1), board.clone()));
                }
            }
        }
    }
    for (id, board) in cases {
        let us = board.side_to_move();
        let base = model.evaluate_full_refresh(&board);
        let base_zero = zero.evaluate_full_refresh(&board);
        println!(
            "{}",
            json!({"kind":"position","id":id,"sfen":board.to_string(),"sfnn":base.eval_cp,"bucket":base.bucket,"progress":base.selected_sum_q16,"zeroSlotSfnn":base_zero.eval_cp,"handcrafted":handcrafted_static_eval(&board)})
        );
        for sq in Square::ALL {
            let Some(pc) = board.colored_piece_on(sq) else {
                continue;
            };
            if pc.piece == Piece::King {
                continue;
            }
            let mut removed = board.clone();
            // Board's unchecked_put toggles the occupied square via XOR.
            removed.unchecked_put(pc.color, pc.piece, sq);
            let actual = model.evaluate_full_refresh(&removed);
            let actual_zero = zero.evaluate_full_refresh(&removed);
            let mut ghost = removed.clone();
            let receiver = !pc.color;
            ghost.unchecked_set_hand(
                receiver,
                pc.piece.unpromote(),
                board.num_in_hand(receiver, pc.piece.unpromote()) + 1,
            );
            let ghost_eval = model.evaluate_full_refresh(&ghost);
            println!(
                "{}",
                json!({"kind":"remove","id":id,"square":sq.to_string(),"piece":format!("{:?}",pc.piece),"owner":if pc.color==us {"us"} else {"them"},"actual":actual.eval_cp,"delta":actual.eval_cp-base.eval_cp,"bucket":actual.bucket,"zeroSlotDelta":actual_zero.eval_cp-base_zero.eval_cp,"ghostHandDelta":ghost_eval.eval_cp-base.eval_cp})
            );
        }
        let mut legal = Vec::new();
        board.generate_moves(|g| {
            legal.extend(g);
            false
        });
        for mv in legal {
            let Some(captured) = board.colored_piece_on(mv.to()) else {
                continue;
            };
            if captured.piece == Piece::King {
                continue;
            }
            let mut child = board.clone();
            child.try_play(mv).unwrap();
            let after = model.evaluate_full_refresh(&child);
            let zero_after = zero.evaluate_full_refresh(&child);
            let mut ghost = child.clone();
            ghost.unchecked_set_hand(
                us,
                captured.piece.unpromote(),
                board.num_in_hand(us, captured.piece.unpromote()) + 1,
            );
            let phantom = model.evaluate_full_refresh(&ghost);
            println!(
                "{}",
                json!({"kind":"capture","id":id,"move":mv.to_string(),"piece":format!("{:?}",captured.piece),"actualMoverScore":-after.eval_cp,"actualGain":-after.eval_cp-base.eval_cp,"zeroSlotGain":-zero_after.eval_cp-base_zero.eval_cp,"ghostHandGain":-phantom.eval_cp-base.eval_cp,"bucketBefore":base.bucket,"bucketAfter":after.bucket,"childSfen":child.to_string()})
            );
        }
        assert!(Color::ALL.iter().all(|&c| board.is_hand_empty(c)));
    }
}

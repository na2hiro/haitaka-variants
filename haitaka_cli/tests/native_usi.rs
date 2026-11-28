#![cfg(not(feature = "extended-json"))]

mod support;

use std::process::Command;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use haitaka::Board;
use support::{UsiProcess, assert_legal_bestmove};

fn engine() -> UsiProcess {
    UsiProcess::spawn(Command::new(env!("CARGO_BIN_EXE_haitaka-usi")))
}

#[test]
fn executable_uses_the_compiled_rules_and_applies_a_depth_cap_to_clocks() {
    let mut usi = engine();
    usi.send("usi");
    let hello = usi.read_through("usiok");
    assert!(hello.iter().any(|line| line
        == &format!(
            "id name Haitaka Variants {} ({})",
            env!("CARGO_PKG_VERSION"),
            haitaka_cli::usi::rule_profile()
        )));
    assert!(
        hello
            .iter()
            .any(|line| line.starts_with("option name MaxDepth "))
    );
    assert!(
        hello
            .iter()
            .any(|line| line.starts_with("option name USI_Hash "))
    );
    usi.send("setoption name USI_Hash value 1");
    usi.send("setoption name USI_Ponder value false");
    usi.send("setoption name MaxDepth value 1");
    usi.send("setoption name MaxDepth value 101");
    assert_eq!(usi.read_through("info string").len(), 1);
    usi.send("isready");
    assert_eq!(usi.read_through("readyok"), ["readyok"]);

    let mut board = Board::startpos();
    let mut first = None;
    board.generate_moves(|moves| {
        first = moves.into_iter().next();
        first.is_some()
    });
    let first = first.unwrap();
    usi.send(&format!("position startpos moves {first}"));
    board.play(first);
    usi.send("go btime 90000 wtime 90000 byoyomi 100");
    let timed = usi.read_through("bestmove ");
    assert!(timed[0].starts_with("info depth 1 "), "{timed:?}");
    assert_legal_bestmove(&board, &timed);

    usi.send("go movetime 10000 depth 2");
    let explicit = usi.read_through("bestmove ");
    assert!(explicit[0].starts_with("info depth 2 "), "{explicit:?}");
    assert_legal_bestmove(&board, &explicit);

    usi.send("go btime 0 wtime 0 byoyomi 0");
    let zero = usi.read_through("bestmove ");
    assert!(zero[0].contains("interruptionReason deadline"));
    assert_legal_bestmove(&board, &zero);
    usi.send("quit");
    assert!(usi.wait_for_exit().is_empty());
}

#[test]
fn infinite_search_waits_for_stop_and_stop_returns_one_legal_move() {
    let mut usi = engine();
    let board = Board::startpos();
    usi.send("position startpos");
    usi.send("go infinite depth 1");
    assert_eq!(usi.read_through("info depth 1 ").len(), 1);
    usi.send("isready");
    assert_eq!(usi.read_through("readyok"), ["readyok"]);
    assert_eq!(
        usi.lines.recv_timeout(Duration::from_millis(75)),
        Err(RecvTimeoutError::Timeout)
    );
    usi.send("stop");
    assert_legal_bestmove(&board, &usi.read_through("bestmove "));
    usi.send("stop");
    usi.send("isready");
    assert_eq!(usi.read_through("readyok"), ["readyok"]);

    usi.send("go depth 100");
    usi.send("stop");
    let stopped = usi.read_through("bestmove ");
    assert!(stopped[0].contains("interruptionReason cancelled"));
    assert_legal_bestmove(&board, &stopped);

    usi.send("go nodes 18446744073709551615");
    usi.send("stop");
    assert_legal_bestmove(&board, &usi.read_through("bestmove "));

    usi.send("go mate 1000");
    assert_eq!(usi.read_through("checkmate"), ["checkmate notimplemented"]);
    usi.send("quit");
    assert!(usi.wait_for_exit().is_empty());
}

#[test]
fn replacing_a_position_or_ending_input_discards_the_previous_search() {
    let board = Board::startpos();
    let mut usi = engine();
    usi.send("go infinite depth 1");
    usi.read_through("info depth 1 ");
    usi.send("position sfen invalid");
    usi.read_through("info string invalid position");
    usi.send("go depth 1");
    assert_eq!(
        usi.read_through("bestmove "),
        [
            "info string invalid go command: no valid position",
            "bestmove resign"
        ]
    );

    usi.send("gameover draw");
    usi.send("usinewgame");
    usi.send(&format!("position sfen {board}"));
    usi.send("go depth 1");
    assert_legal_bestmove(&board, &usi.read_through("bestmove "));
    usi.send("go depth 100");
    usi.stdin.take();
    assert!(usi.wait_for_exit().is_empty());
}

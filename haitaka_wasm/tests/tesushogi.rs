#![cfg(all(feature = "tesushogi", not(target_arch = "wasm32")))]
use haitaka_wasm::UsiSession;

#[test]
fn tesushogi_usi_replays_beyond_49_plies_with_selected_limit() {
    for limit in [49, 99, 149] {
        let mut session = UsiSession::default();
        assert!(
            session
                .handle_line("usi")
                .iter()
                .any(|s| s.contains("option name MoveLimit"))
        );
        assert!(
            session
                .handle_line(&format!("setoption name MoveLimit value {limit}"))
                .is_empty()
        );
        let moves = ["5i5h", "5a5b", "5h5i", "5b5a"];
        let before = (0..limit - 1)
            .map(|i| moves[i as usize % 4])
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            session
                .handle_line(&format!("position startpos moves {before}"))
                .is_empty()
        );
        assert!(session.terminal_notifications().is_empty());
        let search = session.handle_line("go movetime 1 depth 1");
        assert!(
            search
                .iter()
                .any(|s| s.starts_with("bestmove ") && s != "bestmove resign"),
            "{search:?}"
        );
        let last = moves[(limit - 1) as usize % 4];
        assert!(
            session
                .handle_line(&format!("position startpos moves {before} {last}"))
                .is_empty()
        );
        let terminal = session.terminal_notifications().join("\n");
        assert!(terminal.contains("winner white"), "{terminal}");
        assert!(
            !session
                .handle_line(&format!("position startpos moves {before} {last} 7g7f"))
                .is_empty()
        );
        // A failed replay is atomic; the valid terminal board is retained.
        assert_eq!(session.terminal_notifications().join("\n"), terminal);
    }
}

#[test]
fn tesushogi_usi_search_finds_mate_on_last_allowed_move() {
    for limit in [49, 99, 149] {
        let mut session = UsiSession::default();
        assert!(
            session
                .handle_line(&format!("setoption name MoveLimit value {limit}"))
                .is_empty()
        );
        assert!(
            session
                .handle_line(&format!(
                    "position sfen 8k/6G2/7B1/9/9/9/9/9/K8 b R {limit}"
                ))
                .is_empty()
        );
        let lines = session.handle_line("go movetime 5000 depth 1");
        assert!(
            lines.iter().any(|line| line == "bestmove R*1b"),
            "{lines:?}"
        );
        assert!(lines.iter().all(|line| !line.contains("dfpnSelected 1")));
    }
}

#[test]
fn tesushogi_usi_rejects_bad_limits_without_changing_selected_limit() {
    let mut session = UsiSession::default();
    session.handle_line("setoption name MoveLimit value 99");
    for bad in ["0", "50", "-1", "abc", "149 extra"] {
        assert!(
            session
                .handle_line(&format!("setoption name MoveLimit value {bad}"))
                .join("\n")
                .contains("invalid setoption")
        );
    }
    session.handle_line(&format!(
        "position sfen {}",
        haitaka::SFEN_STARTPOS.replace(" 1", " 50")
    ));
    assert!(session.terminal_notifications().is_empty());
}

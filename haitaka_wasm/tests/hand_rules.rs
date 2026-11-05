use haitaka::*;
use haitaka_wasm::*;
#[test]
#[cfg(any(feature = "nakamaware", feature = "suika"))]
fn hand_rules_search_selects_king_capture() {
    let board = Board::from_sfen("4k4/9/9/9/9/9/9/9/K3R4 b - 1").unwrap();
    let result = search_board_impl_handcrafted(&board, 1).unwrap();
    assert!(matches!(
        result.best_move.as_deref(),
        Some("5i5a" | "5i5a+")
    ));
    assert!(result.best_score.unwrap() > 29_000);
}
#[test]
#[cfg(feature = "suika")]
fn hand_rules_search_selects_king_fusion() {
    let board = Board::from_sfen("8k/9/9/4r4/4R4/9/9/9/K8 b R 1").unwrap();
    let result = search_board_impl_handcrafted(&board, 1).unwrap();
    assert_eq!(result.best_move.as_deref(), Some("5e5d"));
    assert!(result.best_score.unwrap() > 29_000);
}
#[test]
fn hand_rules_sfnn_rejects_incompatible_rules() {
    if cfg!(any(
        feature = "share",
        feature = "suika",
        feature = "zoshoku"
    )) {
        let err = load_sfnn_impl(&[], &[], "SFNN_ka2_2048_8_64_progress8").unwrap_err();
        assert!(err.contains("SFNN_UNSUPPORTED_RULESET"));
    }
}

#[test]
fn hand_rules_native_wasm_fixtures() {
    let Some(directory) = std::env::var_os("HAITAKA_HAND_WASM_FIXTURES") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    let rule = [
        (cfg!(feature = "nakamaware"), "nakamaware"),
        (cfg!(feature = "fushiyo"), "fushiyo"),
        (cfg!(feature = "share"), "share"),
        (cfg!(feature = "zoshoku"), "zoshoku"),
        (cfg!(feature = "babanuki"), "babanuki"),
        (cfg!(feature = "tensuu"), "tensuu"),
        (cfg!(feature = "suika"), "suika"),
    ]
    .into_iter()
    .find_map(|(yes, name)| yes.then_some(name))
    .unwrap();
    let model = if cfg!(feature = "zoshoku") {
        None
    } else {
        let path = std::env::var_os("HAITAKA_HAND_SMOKE_MODEL").expect("smoke model required");
        Some(NnueModel::from_bytes(&std::fs::read(path).unwrap()).unwrap())
    };
    let hand = if cfg!(feature = "share") {
        "Pp"
    } else if cfg!(any(feature = "babanuki", feature = "suika")) {
        "P"
    } else {
        "-"
    };
    let mut b = Board::from_sfen(&format!("8k/9/9/4p4/4R4/9/9/9/K8 b {hand} 1")).unwrap();
    let before = b.clone();
    let state = model.as_ref().map(|m| m.build_position_state_full(&b));
    let mv = "5e5d".parse().unwrap();
    b.play(mv);
    if let (Some(model), Some(state)) = (&model, state) {
        let delta = model.apply_move(&before, &b, &state, mv);
        assert_eq!(
            model.evaluate_from_state(&b, &delta),
            model.evaluate_full_refresh(&b)
        );
    }
    let mut boards = vec![
        Board::startpos(),
        before.clone(),
        b.clone(),
        before.color_swapped(),
        b.color_swapped(),
    ];
    if cfg!(feature = "share") {
        boards.push(
            Board::from_sfen("8k/9/9/9/9/9/9/9/K8 b 2R2B4G4S4N4L18P2r2b4g4s4n4l18p 1").unwrap(),
        );
    }
    if cfg!(feature = "zoshoku") {
        boards.push(Board::from_sfen("8k/9/9/9/9/9/9/9/K8 b 1000P 1").unwrap());
    }
    if cfg!(any(feature = "nakamaware", feature = "suika")) {
        let mut terminal = Board::from_sfen("4k4/9/9/9/9/9/9/9/K3R4 b - 1").unwrap();
        terminal.play("5i5a".parse().unwrap());
        boards.push(terminal);
    }
    let rows:Vec<_>=boards.iter().map(|board| {
        let mut moves=0; board.generate_moves(|m|{moves+=m.into_iter().count();false});
        let score=if board.winner().is_some() {None} else {model.as_ref().map(|m|m.evaluate_full_refresh(board))};
        let mut session=UsiSession::default(); session.handle_line(&format!("position sfen {board}"));
        serde_json::json!({"sfen":board.to_string(),"nodes":moves,"eval":score,"usi":session.terminal_notifications()})
    }).collect();
    std::fs::write(
        directory.join(format!("{rule}.json")),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

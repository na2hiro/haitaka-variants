use super::*;
fn game(pieces: &[([u16; 2], &str, u8)], side: u8) -> ShoGame {
    let position = WirePosition {
        side_to_move: side,
        pieces: pieces
            .iter()
            .map(|&(at, piece, color)| PlacedPiece {
                at,
                piece: piece.into(),
                color,
            })
            .collect(),
        hands: Default::default(),
        state: RuleState {},
    };
    ShoGame {
        history: vec![Position::from_wire(&position).unwrap()],
    }
}
fn mv(from: [u16; 2], to: [u16; 2], promotion: bool) -> JsonMove {
    JsonMove {
        schema_version: 1,
        from,
        to,
        promotion,
    }
}
#[test]
fn elephant_and_prince_have_distinct_rule_local_movement_for_both_sides() {
    for color in 0..2 {
        for (id, expected) in [("cf", 7), ("cg", 8)] {
            let g = game(
                &[([1, 9], "ah", 0), ([9, 1], "ah", 1), ([5, 5], id, color)],
                color,
            );
            let moves: Vec<_> = g.moves().into_iter().filter(|m| m.from == [5, 5]).collect();
            assert_eq!(moves.len(), expected);
            let back = if color == 0 { [5, 6] } else { [5, 4] };
            assert_eq!(moves.iter().any(|m| m.to == back), id == "cg");
            assert_eq!(g.current().royals(color), if id == "cg" { 2 } else { 1 });
        }
    }
}
#[test]
fn promotion_creates_a_second_royal_without_changing_king_identity() {
    let mut g = game(
        &[([9, 9], "ah", 0), ([9, 1], "ah", 1), ([4, 4], "cf", 0)],
        0,
    );
    assert!(g.moves().contains(&mv([4, 4], [4, 3], false)));
    let old_hash = g.current().hash();
    g.play(mv([4, 4], [4, 3], true)).unwrap();
    assert_eq!(g.current().royals(0), 2);
    assert_ne!(old_hash, g.current().hash());
    assert_eq!(
        g.current()
            .to_wire()
            .pieces
            .iter()
            .find(|p| p.at == [4, 3])
            .unwrap()
            .piece,
        "cg"
    );
    let restored = ShoGame::parse(&g.to_json()).unwrap();
    assert_eq!(restored.current(), g.current());
    assert_eq!(restored.current().hash(), g.current().hash());
}
#[test]
fn king_or_prince_capture_only_ends_game_when_it_was_the_last_royal() {
    for (victim, survivor) in [("ah", "cg"), ("cg", "ah")] {
        let mut g = game(
            &[
                ([9, 9], "ah", 0),
                ([5, 2], "ag", 0),
                ([5, 1], victim, 1),
                ([9, 1], survivor, 1),
            ],
            0,
        );
        g.play(mv([5, 2], [5, 1], false)).unwrap();
        assert_eq!(g.outcome(), Outcome::ONGOING);
        assert!(!g.moves().is_empty());
        assert!(g.current().to_wire().hands.iter().all(BTreeMap::is_empty));
        let parsed = ShoGame::parse(&g.to_json()).unwrap();
        assert_eq!(parsed.current().royals(1), 1);
        let mut final_capture = game(
            &[([9, 9], "ah", 0), ([5, 2], "ag", 0), ([5, 1], victim, 1)],
            0,
        );
        final_capture.play(mv([5, 2], [5, 1], false)).unwrap();
        assert_eq!(final_capture.outcome(), Outcome::win(0));
        assert!(final_capture.moves().is_empty());
        assert!(ShoGame::parse(&final_capture.to_json()).is_ok());
    }
}
#[test]
fn exposure_adjacent_kings_and_pins_are_legal_but_friendly_capture_and_edges_are_not() {
    let mut g = game(
        &[
            ([5, 5], "ah", 0),
            ([5, 3], "ah", 1),
            ([1, 1], "ag", 1),
            ([1, 9], "ae", 0),
        ],
        0,
    );
    assert!(g.moves().contains(&mv([5, 5], [5, 4], false)));
    assert!(g.play(mv([1, 9], [0, 9], false)).is_err());
    assert!(g.play(mv([5, 5], [1, 9], false)).is_err());
    g.play(mv([5, 5], [5, 4], false)).unwrap();
    assert!(g.moves().contains(&mv([5, 3], [5, 4], false)));
}
#[test]
fn promotion_zones_forced_promotion_and_slider_blockers() {
    let g = game(
        &[
            ([9, 9], "ah", 0),
            ([9, 1], "ah", 1),
            ([3, 2], "aa", 0),
            ([4, 3], "ac", 0),
            ([5, 5], "ag", 0),
            ([5, 4], "ae", 0),
            ([7, 5], "aa", 1),
        ],
        0,
    );
    assert!(!g.moves().contains(&mv([3, 2], [3, 1], false)));
    assert!(g.moves().contains(&mv([3, 2], [3, 1], true)));
    assert!(
        g.moves()
            .iter()
            .filter(|m| m.from == [4, 3])
            .all(|m| m.promotion)
    );
    assert!(!g.moves().iter().any(|m| m.from == [5, 5] && m.to == [5, 3]));
    assert!(g.moves().contains(&mv([5, 5], [7, 5], false)));
    assert!(!g.moves().contains(&mv([5, 5], [8, 5], false)));
}
#[test]
fn strict_wire_validation_and_canonicalization() {
    let json = start_position_json();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let mut reversed = value.clone();
    reversed["position"]["pieces"]
        .as_array_mut()
        .unwrap()
        .reverse();
    reversed["history"] = serde_json::json!([]);
    assert_eq!(validate_position_json(&reversed.to_string()).unwrap(), json);
    for (key, v) in [
        ("ruleId", serde_json::json!(48)),
        ("schemaVersion", serde_json::json!(2)),
        ("royals", serde_json::json!([])),
    ] {
        let mut bad = value.clone();
        bad[key] = v;
        assert!(ShoGame::parse(&bad.to_string()).is_err());
    }
    for change in 0..7 {
        let mut bad = value.clone();
        match change {
            0 => bad["position"]["state"]["lionCapture"] = true.into(),
            1 => bad["position"]["hands"][0]["aa"] = 1.into(),
            2 => bad["position"]["pieces"][0]["piece"] = "unknown".into(),
            3 => bad["position"]["pieces"][0]["color"] = 2.into(),
            4 => bad["position"]["pieces"][0]["at"] = serde_json::json!([0, 1]),
            5 => bad["position"]["sideToMove"] = 2.into(),
            _ => bad["position"]["pieces"][0]["at"] = bad["position"]["pieces"][1]["at"].clone(),
        }
        assert!(ShoGame::parse(&bad.to_string()).is_err(), "case {change}");
    }
    assert!(
        serde_json::from_str::<JsonMove>(
            r#"{"schemaVersion":1,"from":[1,1],"to":[1,2],"promotion":false,"path":[]}"#
        )
        .is_err()
    );
}
#[test]
fn histories_are_validated_and_context_is_part_of_tt_identity() {
    let mut g = ShoGame::parse(&start_position_json()).unwrap();
    let mv = g.moves()[0];
    g.play(mv).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&g.to_json()).unwrap();
    let parsed = ShoGame::parse(&g.to_json()).unwrap();
    assert_eq!(parsed.context_key(), g.context_key());
    json["history"] = serde_json::json!([]);
    let fresh = ShoGame::parse(&json.to_string()).unwrap();
    assert_eq!(fresh.current().hash(), g.current().hash());
    assert_ne!(fresh.context_key(), g.context_key());
    json["history"] = serde_json::json!([fresh.current().to_wire(), fresh.current().to_wire()]);
    assert!(ShoGame::parse(&json.to_string()).is_err());
    json["history"] = serde_json::json!([ShoGame::parse(&start_position_json())
        .unwrap()
        .current()
        .to_wire()]);
    assert!(ShoGame::parse(&json.to_string()).is_err());
}
#[test]
fn fourfold_repetition_and_continuous_check_use_the_same_royal_set() {
    let mut g = game(&[([1, 9], "ah", 0), ([9, 1], "cg", 1)], 0);
    let cycle = [
        mv([1, 9], [2, 9], false),
        mv([9, 1], [8, 1], false),
        mv([2, 9], [1, 9], false),
        mv([8, 1], [9, 1], false),
    ];
    for _ in 0..3 {
        for m in cycle {
            g.play(m).unwrap();
        }
    }
    assert_eq!(g.outcome(), Outcome::DRAW);
    assert!(g.moves().is_empty());
    assert_eq!(
        ShoGame::parse(&g.to_json()).unwrap().outcome(),
        Outcome::DRAW
    );
    let mut check = game(
        &[([9, 9], "ah", 0), ([4, 1], "cg", 1), ([5, 2], "ag", 0)],
        0,
    );
    let cycle = [
        mv([5, 2], [4, 2], false),
        mv([4, 1], [5, 1], false),
        mv([4, 2], [5, 2], false),
        mv([5, 1], [4, 1], false),
    ];
    for _ in 0..3 {
        for m in cycle {
            check.play(m).unwrap();
        }
    }
    assert_eq!(check.outcome(), Outcome::win(1));
    assert!(check.play(cycle[0]).is_err());
    assert_eq!(
        ShoGame::parse(&check.to_json()).unwrap().outcome(),
        check.outcome()
    );
}
#[test]
fn search_captures_last_royal_and_returns_legal_fallback_on_interruption() {
    let g = game(
        &[([9, 9], "ah", 0), ([5, 2], "ag", 0), ([5, 1], "cg", 1)],
        0,
    );
    let result: serde_json::Value =
        serde_json::from_str(&search_json(&g.to_json(), 4, 1000, 10000).unwrap()).unwrap();
    let best: JsonMove = serde_json::from_value(result["bestMove"].clone()).unwrap();
    assert_eq!(best.to, [5, 1]);
    assert!(result["score"].as_i64().unwrap() > 29000);
    let start = ShoGame::parse(&start_position_json()).unwrap();
    let result: serde_json::Value =
        serde_json::from_str(&search_json(&start.to_json(), 100, 1000, 1).unwrap()).unwrap();
    assert_eq!(result["nodeLimitReached"], true);
    assert_eq!(result["completedDepth"], 0);
    let best: JsonMove = serde_json::from_value(result["bestMove"].clone()).unwrap();
    assert!(start.moves().contains(&best));
    let ended = game(&[([9, 9], "ah", 0)], 1);
    let result: serde_json::Value =
        serde_json::from_str(&search_json(&ended.to_json(), 3, 1000, 0).unwrap()).unwrap();
    assert!(result["bestMove"].is_null());
    assert_eq!(result["outcome"]["winner"], 0);
}

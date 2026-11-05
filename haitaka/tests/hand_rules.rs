use haitaka::*;
fn board(hand: &str) -> Board {
    Board::from_sfen(&format!("8k/9/9/4p4/4R4/9/9/9/K8 b {hand} 1")).unwrap()
}
fn play(b: &mut Board, text: &str) {
    let mv = text.parse().unwrap();
    assert!(b.is_legal(mv), "{text} in {b}");
    b.try_play(mv).unwrap();
}

#[test]
fn hand_rules_capture_and_sfen_roundtrip() {
    let hand = if cfg!(feature = "share") {
        "Pp"
    } else if cfg!(any(feature = "babanuki", feature = "suika")) {
        "P"
    } else {
        "-"
    };
    let mut b = board(hand);
    play(&mut b, "5e5d");
    let expected = if cfg!(any(
        feature = "fushiyo",
        feature = "babanuki",
        feature = "suika"
    )) {
        0
    } else if cfg!(any(feature = "share", feature = "zoshoku")) {
        2
    } else {
        1
    };
    assert_eq!(b.num_in_hand(Color::Black, Piece::Pawn), expected);
    if cfg!(feature = "suika") {
        assert_eq!(b.num_in_hand(Color::Black, Piece::Lance), 1);
    }
    if cfg!(feature = "share") {
        assert_eq!(b.hand(Color::Black), b.hand(Color::White));
    }
    let restored = Board::from_sfen(&b.to_string()).unwrap();
    #[cfg(not(feature = "ningenshogi"))]
    assert_eq!(b.hash(), restored.hash());
    assert_eq!(b.to_string(), restored.to_string());
}

#[test]
fn hand_rules_generation_agrees_with_legality() {
    let mut b = Board::startpos();
    let mut seed = 17u64;
    for _ in 0..200 {
        let mut moves = vec![];
        b.generate_moves(|mvs| {
            moves.extend(mvs);
            false
        });
        for mv in &moves {
            assert!(b.is_legal(*mv), "{mv} in {b}");
        }
        let mut by_parts = vec![];
        b.generate_board_moves(|mvs| {
            by_parts.extend(mvs);
            false
        });
        b.generate_drops(|mvs| {
            by_parts.extend(mvs);
            false
        });
        moves.sort_by_key(|m| m.to_string());
        by_parts.sort_by_key(|m| m.to_string());
        assert_eq!(moves, by_parts);
        if moves.is_empty() {
            break;
        }
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        b.try_play(moves[seed as usize % moves.len()]).unwrap();
    }
}

#[cfg(feature = "nakamaware")]
#[test]
fn hand_rules_friendly_capture_and_king_loss() {
    let mut b = Board::from_sfen("8k/9/9/4P4/4R4/9/9/9/K8 b - 1").unwrap();
    play(&mut b, "5e5d");
    assert_eq!(b.color_on("5d".parse().unwrap()), Some(Color::Black));
    assert_eq!(
        b.num_in_hand(Color::Black, Piece::Pawn),
        if cfg!(feature = "fushiyo") {
            0
        } else if cfg!(feature = "zoshoku") {
            2
        } else {
            1
        }
    );
    let mut b = Board::from_sfen("8k/9/9/4K4/4R4/9/9/9/9 b - 1").unwrap();
    play(&mut b, "5e5d");
    assert_eq!(b.winner(), Some(Color::White));
    assert!(!b.generate_moves(|_| true));
    assert!(Board::from_sfen(&b.to_string()).is_ok());
}

#[cfg(any(feature = "nakamaware", feature = "suika"))]
#[test]
fn hand_rules_ignored_check_and_king_capture() {
    let mut b = Board::from_sfen("4k4/9/9/9/9/9/9/9/K3R4 b - 1").unwrap();
    play(&mut b, "5i5a");
    assert_eq!(b.winner(), Some(Color::Black));
    assert!(!b.generate_moves(|_| true));
    assert!(Board::from_sfen(&b.to_string()).is_ok());
    let b = Board::from_sfen("8k/9/9/9/9/9/9/9/K3rR3 b - 1").unwrap();
    assert!(b.is_legal("4i4h".parse().unwrap()));
    let result = b.dfpn(&DfpnOptions::default());
    assert_eq!(result.status, DfpnStatus::Unknown);
    assert_eq!(result.stats.nodes, 0);
}

#[cfg(feature = "fushiyo")]
#[test]
fn hand_rules_no_drops_even_in_check() {
    let mut b = board("-");
    b.unchecked_set_hand(Color::Black, Piece::Pawn, 1);
    assert!(!b.generate_drops(|_| true));
    assert!(!b.generate_drops_for(Piece::Pawn, |_| true));
    assert!(!b.is_legal("P*4f".parse().unwrap()));
}

#[cfg(feature = "share")]
#[test]
fn hand_rules_shared_drop_consumes_both_hands() {
    let mut b = board("Pp");
    play(&mut b, "P*4f");
    assert!(b.is_hand_empty(Color::Black));
    assert!(b.is_hand_empty(Color::White));
    assert!(Board::from_sfen("8k/9/9/9/9/9/9/9/K8 b P 1").is_err());
}

#[cfg(feature = "zoshoku")]
#[test]
fn hand_rules_large_inventory_roundtrip_and_hash() {
    for n in [19, 31, 32, 255, 256, 1000] {
        let mut b = board(&format!("{n}P"));
        let before = b.hash();
        play(&mut b, "5e5d");
        assert_eq!(b.num_in_hand(Color::Black, Piece::Pawn), n + 2);
        assert_ne!(before, b.hash());
        assert_eq!(b.hash(), Board::from_sfen(&b.to_string()).unwrap().hash());
    }
    assert!(Board::from_sfen("8k/9/9/9/9/9/9/9/K8 b 999999999999999999999P 1").is_err());
}

#[cfg(feature = "tensuu")]
#[test]
fn hand_rules_points_threshold_and_major_pieces() {
    let mut b = board("2G2L");
    assert!(b.winner().is_none());
    play(&mut b, "5e5d");
    assert_eq!(b.winner(), Some(Color::Black));
    let b = board("RB");
    assert_eq!(b.immediate_winner(), None);
}

#[cfg(feature = "suika")]
#[test]
fn hand_rules_suika_chain_and_king_fusion() {
    let mut b = board("RBG SNLP".replace(' ', "").as_str());
    play(&mut b, "5e5d");
    assert_eq!(b.num_in_hand(Color::Black, Piece::King), 1);
    assert_eq!(b.winner(), Some(Color::Black));
    assert!(Board::from_sfen(&b.to_string()).is_ok());
}

#[test]
#[cfg(all(
    feature = "nakamaware",
    feature = "shihonkangen",
    not(feature = "suika")
))]
fn hand_rules_capture_king_respects_immediate_loss() {
    let mut b = Board::from_sfen("9/9/9/4k4/4R4/9/9/9/K8 b RBGS 1").unwrap();
    play(&mut b, "5e5d");
    assert_eq!(b.winner(), Some(Color::White));
}

#[test]
#[cfg(all(
    feature = "nakamaware",
    not(feature = "donor-category"),
    not(feature = "king-constraint-category"),
    not(feature = "win-condition-category"),
    not(any(
        feature = "fushiyo",
        feature = "share",
        feature = "zoshoku",
        feature = "babanuki"
    ))
))]
fn hand_rules_nakamaware_reference_start_perft() {
    let b = Board::startpos();
    let mut nodes = 0;
    b.generate_moves(|mvs| {
        nodes += mvs.into_iter().count();
        false
    });
    assert_eq!(nodes, 51); // Shogitter rule 24 reference.
}

#[test]
fn hand_rules_capture_commutes_with_color_swap() {
    let mut original = board("-");
    let mut swapped = original.color_swapped();
    let mv: Move = "5e5d".parse().unwrap();
    let flipped = match mv {
        Move::BoardMove {
            from,
            to,
            promotion,
        } => Move::BoardMove {
            from: from.flip(),
            to: to.flip(),
            promotion,
        },
        _ => unreachable!(),
    };
    original.try_play(mv).unwrap();
    swapped.try_play(flipped).unwrap();
    assert!(original.color_swapped().same_position(&swapped));
}

#[test]
#[cfg(all(feature = "share", feature = "sanmaime", not(feature = "tensuu")))]
fn hand_rules_shared_threshold_belongs_to_the_mover() {
    let mut b = board("2P2p");
    play(&mut b, "5e5d");
    assert_eq!(b.winner(), Some(Color::Black));
}

#[test]
#[cfg(all(feature = "tensuu", feature = "shihonkangen", not(feature = "share")))]
fn hand_rules_points_respect_immediate_loss() {
    let mut b = board("2G2L");
    play(&mut b, "5e5d");
    assert_eq!(b.winner(), Some(Color::White));
}

#[test]
#[cfg(any(feature = "nakamaware", feature = "suika"))]
fn hand_rules_geometric_checks_survive_ignored_evasions() {
    let mut b = Board::from_sfen("9/9/4k4/9/4K4/9/9/9/9 b - 1").unwrap();
    play(&mut b, "5e5d");
    assert!(b.checkers().has("5d".parse().unwrap()));
    let restored = Board::from_sfen(&b.to_string()).unwrap();
    assert_eq!(b.checkers(), restored.checkers());
    if cfg!(feature = "oute") {
        assert_eq!(b.winner(), Some(Color::Black));
    } else {
        assert!(b.null_move().is_some());
    }
    let mut b = Board::from_sfen("9/9/4k4/4P4/9/9/9/L8/K8 b - 1").unwrap();
    play(&mut b, "9h9g");
    assert!(b.checkers().has("5d".parse().unwrap()));
    assert_eq!(
        b.checkers(),
        Board::from_sfen(&b.to_string()).unwrap().checkers()
    );
}

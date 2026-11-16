use super::*;

#[test]
fn discarded_captures_replace_board_features_with_missing_slots() {
    // Nonzero row zero catches an omitted missing-piece update even if a
    // particular production model happens to assign zero weights to that row.
    let model = SfnnModel {
        biases: vec![3; 32],
        weights: (0..FEATURES * 32).map(|i| (i % 31) as i16 - 15).collect(),
        progress: (0..PROGRESS_COUNT).map(|i| (i % 19) as i32 - 9).collect(),
        stacks: Vec::new(),
        #[cfg(all(feature = "sfnn-diagnostics", not(target_arch = "wasm32")))]
        diagnostic_ghost_hands: false,
    };
    for (sfen, text) in [
        ("8k/9/6g2/6S2/9/9/9/9/K8 b - 1", "3d3c+"),
        ("8k/9/9/9/9/6s2/6G2/9/K8 w - 2", "3f3g+"),
        ("8k/9/6p2/5K3/9/9/9/9/9 b - 1", "4d3c"),
        ("9/9/9/9/9/5k3/6P2/9/K8 w - 2", "4f3g"),
    ] {
        let parent = Board::from_sfen(sfen).unwrap();
        let mv: Move = text.parse().unwrap();
        let state = model.build_position_state_full(&parent);
        let mut child = parent.clone();
        child.try_play(mv).unwrap();
        assert!(Color::ALL.iter().all(|&c| child.is_hand_empty(c)));
        assert_eq!(
            model.apply_move(&parent, &child, &state, mv),
            model.build_position_state_full(&child),
            "{sfen}: {text}"
        );
    }
}

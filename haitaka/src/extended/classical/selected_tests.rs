//! Selected ordinary-rule regressions: recorded perft counts and fixed-seed
//! walks whose test-only assertions compare fast attack paths with core.
use super::compiled::{CompiledGame, RULE_ID, start_position_json};
use super::*;
const PERFT: &[(u32, [u64; 3])] = &[
    (111, [30, 900, 25440]),
    (138, [30, 900, 25470]),
    (46, [32, 1024, 30912]),
    (47, [32, 1024, 30976]),
    (65, [37, 1369, 47290]),
    (147, [30, 900, 25855]),
    (30, [20, 600, 11580]),
    (31, [30, 900, 25470]),
    (153, [9, 81, 909]),
    (165, [9, 81, 882]),
    (79, [28, 784, 21209]),
    (90, [23, 529, 11764]),
    (91, [40, 1600, 59520]),
    (17, [4, 17, 118]),
    (28, [30, 900, 24419]),
    (92, [73, 5066, 425056]),
    (103, [77, 5651, 414597]),
    (159, [30, 900, 25470]),
    (44, [28, 784, 21726]),
    (45, [29, 841, 23954]),
    (88, [30, 900, 25170]),
    (21, [32, 1024, 30656]),
    (20, [26, 780, 19560]),
    (112, [30, 900, 25470]),
    (113, [30, 900, 25411]),
    (22, [48, 1440, 98659]),
    (117, [28, 1036, 28673]),
    (163, [30, 900, 25530]),
];
#[test]
fn selected_rule_perft_matches_recorded_counts() {
    let expected = PERFT.iter().find(|(id, _)| *id == RULE_ID).unwrap().1;
    let mut g = CompiledGame::parse(&start_position_json()).unwrap();
    for (depth, &nodes) in expected.iter().enumerate() {
        assert_eq!(
            g.perft(depth as u8 + 1),
            nodes,
            "rule {RULE_ID} depth {}",
            depth + 1
        );
    }
}
#[test]
fn selected_rule_walks_keep_fast_and_core_attack_answers_equal() {
    for walk in 0..12u32 {
        let mut g = CompiledGame::parse(&start_position_json()).unwrap();
        let mut seed = 0x5eed_0001_u32
            .wrapping_add(RULE_ID.wrapping_mul(977))
            .wrapping_add(walk * 7919);
        for _ in 0..240 {
            let p = g.current().clone();
            for color in 0..2 {
                assert_eq!(p.classical_attacked_direct(color), p.attacked(color));
            }
            let moves = g.moves();
            let _ = g.outcome();
            if moves.is_empty() {
                break;
            }
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            // Favour captures so walks reach open, tactical positions.
            let captures: Vec<_> = moves
                .iter()
                .copied()
                .filter(|&m| p.classical_is_capture(m))
                .collect();
            let pool = if !captures.is_empty() && seed % 3 == 0 {
                &captures
            } else {
                &moves
            };
            g.play(pool[(seed >> 2) as usize % pool.len()]).unwrap();
        }
    }
}
/// The shielding royal is itself attacked, so every move takes the full
/// query; stepping aside would still uncover the other royal.
#[cfg(feature = "nigyoku")]
#[test]
fn a_royal_shielding_another_cannot_step_aside() {
    let position = |rook: bool| {
        let mut pieces = vec![
            ClassicalPiece::make([5, 9], "ah", 0, None),
            ClassicalPiece::make([5, 8], "ah", 0, None),
            ClassicalPiece::make([1, 1], "ah", 1, None),
        ];
        if rook {
            pieces.push(ClassicalPiece::make([5, 1], "ag", 1, None));
        }
        Position::<Nigyoku>::from_wire(&WirePosition {
            side_to_move: 0,
            pieces,
            hands: Default::default(),
            state: Default::default(),
        })
        .unwrap()
    };
    let aside = ClassicalMove::board([5, 8], Path::one([4, 8]), false);
    let moves = |rook| {
        position(rook)
            .classical_moves(usize::MAX, &mut || false)
            .unwrap()
    };
    assert!(moves(false).contains(&aside));
    assert!(!moves(true).contains(&aside));
}

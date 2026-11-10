#![cfg(feature = "minishogi")]
use haitaka::*;

fn board(sfen: &str) -> Board {
    Board::from_sfen(sfen).unwrap()
}
fn mv(text: &str) -> Move {
    text.parse().unwrap()
}
fn moves(board: &Board) -> Vec<Move> {
    let mut out = Vec::new();
    board.generate_moves(|group| {
        out.extend(group);
        false
    });
    out
}
fn perft(board: &Board, depth: u8, bulk: bool) -> u64 {
    if depth == 0 {
        return 1;
    }
    let mut nodes = 0;
    board.generate_moves(|group| {
        if bulk && depth == 1 {
            nodes += group.into_iter().len() as u64;
        } else {
            for mv in group {
                let mut child = board.clone();
                child.play_unchecked(mv);
                nodes += perft(&child, depth - 1, bulk);
            }
        }
        false
    });
    nodes
}

#[test]
fn compact_coordinates_and_start_position() {
    assert_eq!(core::mem::size_of::<BitBoard>(), 8);
    assert_eq!(Square::NUM, 25);
    assert_eq!(Rank::NUM, 5);
    assert_eq!(File::NUM, 5);
    assert_eq!(Square::E5 as usize, 24);
    assert_eq!(Square::A2 as usize, 5);
    assert!("6a".parse::<Square>().is_err());
    assert!("1f".parse::<Square>().is_err());
    let start = Board::startpos();
    assert_eq!(start.to_string(), "rbsgk/4p/5/P4/KGSBR b - 1");
    assert_eq!(start.occupied().len(), 12);
    assert_eq!(moves(&start).len(), 14);
    for square in Square::ALL {
        assert_eq!(square.to_string().parse::<Square>().unwrap(), square);
        assert_eq!(square.flip().flip(), square);
        let bits = square.bitboard();
        assert_eq!(bits.rotate(), square.flip().bitboard());
        assert_eq!(bits.flip_files(), square.flip_file().bitboard());
        assert_eq!(bits.flip_ranks(), square.flip_rank().bitboard());
    }
    assert_eq!(BitBoard::FULL.count_zeros(), 0);
    assert_eq!(BitBoard::EMPTY.count_zeros(), 25);
    assert_eq!(BitBoard::FULL.shift_west(5), BitBoard::EMPTY);
    assert_eq!(Square::A1.bitboard().shift_north(1), BitBoard::EMPTY);
    assert_eq!(Square::E5.bitboard().shift_south(1), BitBoard::EMPTY);
    assert_eq!(
        bitboard! { . . . . X . . . . . . . . . . . . . . . X . . . . },
        Square::A1.bitboard() | Square::E5.bitboard()
    );
}

#[test]
fn startpos_perft_matches_fairy_stockfish_and_bulk_counting() {
    let start = Board::startpos();
    for depth in 0..=4 {
        assert_eq!(perft(&start, depth, true), perft(&start, depth, false));
    }
    assert_eq!(perft(&start, 5, true), 533_203);
}

#[test]
fn promotion_is_limited_to_one_rank_for_both_colors() {
    let b = board("4k/2S2/5/5/K4 b - 1");
    for text in ["3b3a", "3b3a+"] {
        assert!(b.is_legal(mv(text)), "{text}");
    }
    assert!(!b.is_legal(mv("3b3c+")));
    let leaving = board("2S1k/5/5/5/K4 b - 1");
    assert!(leaving.is_legal(mv("3a2b+")));
    let pawn = board("4k/2P2/5/5/K4 b - 1");
    assert!(pawn.is_legal(mv("3b3a+")));
    assert!(!pawn.is_legal(mv("3b3a")));
    let white = board("4k/5/5/2p2/K4 w - 1");
    assert!(white.is_legal(mv("3d3e+")));
    assert!(!white.is_legal(mv("3d3e")));
    assert!(!Piece::Silver.can_promote(Color::White, Square::D3));
    assert!(Piece::Silver.can_promote(Color::White, Square::E3));
}

#[test]
fn captures_demote_and_drop_restrictions_use_the_small_board() {
    let mut b = board("4k/2+s2/2R2/5/K4 b - 1");
    b.play(mv("3c3b"));
    assert_eq!(b.num_in_hand(Color::Black, Piece::Silver), 1);
    assert_eq!(Board::from_sfen(&b.to_string()).unwrap(), b);
    let b = board("4k/5/2P2/5/K4 b P 1");
    assert!(!b.is_legal(mv("P*3d")));
    assert!(!b.is_legal(mv("P*2a")));
    assert!(b.is_legal(mv("P*2d")));
    let white = board("4k/5/2p2/5/K4 w p 1");
    assert!(!white.is_legal(mv("P*3b")));
    assert!(!white.is_legal(mv("P*2e")));
    assert!(white.is_legal(mv("P*2b")));
}

#[test]
fn pawn_drop_mate_is_filtered_but_ordinary_check_is_legal() {
    let mate = board("3pk/5/3G1/5/K4 b P 1");
    assert!(!mate.is_legal(mv("P*1b")));
    assert!(!moves(&mate).contains(&mv("P*1b")));
    let check = board("3pk/5/5/5/K4 b P 1");
    assert!(check.is_legal(mv("P*1b")));
}

#[test]
fn pins_and_mate_use_compact_rays() {
    let pinned = board("2r1k/5/5/2S2/2K2 b - 1");
    assert!(pinned.is_legal(mv("3d3c")));
    assert!(!pinned.is_legal(mv("3d2c")));
    let mate = board("4k/3G1/3R1/5/K4 w - 1");
    assert_eq!(mate.status(), GameStatus::Won);
    let in_one = board("4k/5/3G1/3R1/K4 b - 1");
    let result = in_one.dfpn(&DfpnOptions {
        max_nodes: Some(10000),
        ..Default::default()
    });
    assert_eq!(result.status, DfpnStatus::Mate);
}

#[test]
fn malformed_shapes_and_excess_inventory_return_errors() {
    for sfen in [
        "6/5/5/5/5 b - 1",
        "4k/5/5/5/K5 b - 1",
        "4k/5/5/K4 b - 1",
        "4k/5/5/5/5/K4 b - 1",
        "4k/5/5/5/K4+ b - 1",
        "4k/0p4/5/5/K4 b - 1",
        "4k/5/5/5/K4 b N 1",
        "4k/5/5/5/K4 b L 1",
        "4k/5/5/5/K4 b 3S 1",
        "4k/5/5/5/K4 b 255P255p 1",
    ] {
        assert!(Board::from_sfen(sfen).is_err(), "{sfen}");
    }
    assert!(Board::tsume("4k/5/5/5/5 b 3P 1").is_err());
    let tsume = Board::tsume("4k/5/5/5/5 b - 1").unwrap();
    assert_eq!(tsume.num_in_hand(Color::White, Piece::Pawn), 2);
    assert_eq!(tsume.num_in_hand(Color::White, Piece::Lance), 0);
}

fn cycle_history(sfen: &str, cycle: &[&str], repetitions: usize) -> PositionHistory {
    let mut b = board(sfen);
    let mut history = PositionHistory::new(b.clone());
    for _ in 0..repetitions {
        for &text in cycle {
            b.play(mv(text));
            history.push(b.clone());
        }
    }
    history
}

#[test]
fn repetition_matches_shogitter_instead_of_fairy_stockfish() {
    let cycle = ["5e5d", "1a1b", "5d5e", "1b1a"];
    let sfen = "4k/5/5/5/K4 b - 1";
    assert_eq!(
        cycle_history(sfen, &cycle, 2).adjudication(),
        HistoryAdjudication::Ongoing
    );
    assert_eq!(
        cycle_history(sfen, &cycle, 3).adjudication(),
        HistoryAdjudication::RepetitionDraw
    );
    let checking = ["2b1b", "1a2a", "1b2b", "2a1a"];
    assert_eq!(
        cycle_history("4k/3R1/5/5/K4 b - 1", &checking, 3).adjudication(),
        HistoryAdjudication::PerpetualCheckLoss(Color::Black)
    );
    assert_eq!(
        HISTORY_RULES_VERSION,
        "minishogi-shogitter-history-adjudication-v1"
    );
}

#[test]
fn random_play_roundtrips_and_generator_agrees_with_individual_legality() {
    let mut seed = 0x5a17101u64;
    for _ in 0..8 {
        let mut b = Board::startpos();
        for _ in 0..48 {
            let legal = moves(&b);
            for &from in &Square::ALL {
                if b.color_on(from) == Some(b.side_to_move()) {
                    for &to in &Square::ALL {
                        for promotion in [false, true] {
                            let candidate = Move::BoardMove {
                                from,
                                to,
                                promotion,
                            };
                            assert_eq!(
                                legal.contains(&candidate),
                                b.is_legal(candidate),
                                "{} {candidate}",
                                b
                            );
                        }
                    }
                }
            }
            for i in 0..Piece::HAND_NUM {
                for to in Square::ALL {
                    let candidate = Move::Drop {
                        piece: Piece::index_const(i),
                        to,
                    };
                    assert_eq!(legal.contains(&candidate), b.is_legal(candidate));
                }
            }
            if legal.is_empty() {
                break;
            }
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            b.play(legal[seed as usize % legal.len()]);
            assert_eq!(board(&b.to_string()), b);
        }
    }
}

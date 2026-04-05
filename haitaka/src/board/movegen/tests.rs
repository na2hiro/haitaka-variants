use rand::rng;
use rand::rngs::ThreadRng;
use rand::seq::IndexedRandom;

// Movegenerator tests
use super::*;

// Tests the generation of board moves based on giving a subset of squares
#[test]
fn subset_movegen_habu_position() {
    fn visit(board: &Board, depth: u8) {
        let random = board.hash();
        let subset_a = BitBoard::new(random.into());
        let subset_b = !subset_a;
        let mut subset_moves = 0;

        board.generate_board_moves_for(subset_a, |moves| {
            subset_moves += moves.len();
            false
        });
        board.generate_board_moves_for(subset_b, |moves| {
            subset_moves += moves.len();
            false
        });

        let mut total_moves = 0;
        board.generate_board_moves(|moves| {
            total_moves += moves.len();
            false
        });
        assert_eq!(subset_moves, total_moves);

        if depth > 0 {
            board.generate_moves(|moves| {
                for mv in moves {
                    let mut board = board.clone();
                    board.play_unchecked(mv);
                    visit(&board, depth - 1);
                }
                false
            });
        }
    }
    // Famous Habu-Kato game (https://en.wikipedia.org/wiki/Shogi_notation)
    // with sublime Silver drop sacrifice on square 5b
    let board = "ln1g5/1r2S1k2/p2pppn2/2ps2p2/1p7/2P6/PPSPPPPLP/2G2K1pr/LN4G1b w BGSLPnp 62"
        .parse()
        .unwrap();
    visit(&board, 2);
}

fn test_is_legal(board: Board) {
    use std::collections::HashSet;

    // both board_moves and drops are included
    let mut legals = HashSet::new();
    board.generate_moves(|mvs| {
        legals.extend(mvs);
        false
    });

    for from in Square::ALL {
        for to in Square::ALL {
            for promotion in [true, false] {
                let mv = Move::BoardMove {
                    from,
                    to,
                    promotion,
                };
                assert_eq!(legals.contains(&mv), board.is_legal(mv), "{}", mv);
            }
        }
    }
}

fn test_forbidden_drops(board: &Board) {
    use std::collections::HashSet;

    let mut legals = HashSet::new();
    board.generate_drops(|mvs| {
        legals.extend(mvs);
        false
    });

    let forbidden = match board.side_to_move() {
        Color::White => Rank::I.bitboard(),
        Color::Black => Rank::A.bitboard(),
    };

    let forbidden_for_knight = match board.side_to_move() {
        Color::White => Rank::H.bitboard(),
        Color::Black => Rank::B.bitboard(),
    };

    for to in forbidden {
        for piece in [Piece::Pawn, Piece::Lance, Piece::Knight] {
            let mv = Move::Drop { piece, to };
            assert!(!legals.contains(&mv));
        }
    }

    for to in forbidden_for_knight {
        let mv = Move::Drop {
            piece: Piece::Knight,
            to,
        };
        assert!(!legals.contains(&mv));
    }
}

fn test_nifu(board: &Board) {
    let color = board.side_to_move();

    board.generate_drops_for(Piece::Pawn, |mvs| {
        for mv in mvs {
            assert_eq!(mv.piece().unwrap(), Piece::Pawn);
            assert!(board.pawn_drop_ok(color, mv.to()));
        }
        false
    });

    let pawns = board.colored_pieces(color, Piece::Pawn);
    for square in pawns {
        let forbidden = square.file().bitboard() & !board.occupied();
        for to in forbidden {
            let mv = Move::Drop {
                piece: Piece::Pawn,
                to,
            };
            assert!(!board.is_legal_drop(mv));
        }
    }
}

#[test]
fn legality_simple() {
    test_is_legal(Board::startpos());
    test_is_legal(
        "ln1g5/1r2S1k2/p2pppn2/2ps2p2/1p7/2P6/PPSPPPPLP/2G2K1pr/LN4G1b w BGSLPnp 62"
            .parse()
            .unwrap(),
    );
}

#[test]
#[cfg(not(feature = "annan"))]
fn legality_drops() {
    let board: Board = "ln1g5/1r2S1k2/p2pppn2/2ps2p2/1p7/2P6/PPSPPPPLP/2G2K1pr/LN4G1b w BGSLPnp 62"
        .parse()
        .unwrap();
    test_forbidden_drops(&board);
    test_nifu(&board);
}

#[test]
fn non_check() {
    let sfen: &str = "lnsgk1snl/1r4gb1/p1pppp2p/6pR1/1p7/2P6/PP1PPPP1P/1BG6/LNS1KGSNL w Pp 12";
    let board: Board = sfen.parse().unwrap();
    let checkers = board.checkers();
    assert!(checkers.is_empty());
}

#[test]
fn pawn_push_mate_is_valid() {
    // White King on 1e is almost mate
    let sfen = "lns4+Rl/1r1g5/p1p1pSp1p/1p1p1p3/8k/7N1/PPPPPPP1P/1B7/LNSGKGSNL b BG2p 25";
    let board: Board = sfen.parse().unwrap();
    assert!(board.checkers().is_empty());

    assert_eq!(board.side_to_move(), Color::Black);
    let mv = Move::Drop {
        piece: Piece::Gold,
        to: Square::F1,
    };
    assert!(board.is_legal_drop(mv));
    assert!(board.is_legal(mv));

    let mv = Move::Drop {
        piece: Piece::Gold,
        to: Square::E2,
    };
    assert!(board.is_legal_drop(mv));
    assert!(board.is_legal(mv));

    let mv = Move::BoardMove {
        from: Square::G1,
        to: Square::F1,
        promotion: false,
    };
    assert!(board.is_legal_board_move(mv));
    assert!(board.is_legal(mv));
}

#[test]
#[cfg(not(feature = "annan"))]
fn discount_pawn_drop_mate_in_perft() {
    // See old discussion at: https://www.talkchess.com/forum3/viewtopic.php?f=7&t=71550
    //
    // Testing this SFEN did expose a bug in the haitaka 0.2.1 code:
    // When generating Pawn drops, all drops would be skipped if the first drop we looked
    // at happened to be an illegal checkmate.
    let sfen: &str = "7lk/9/8S/9/9/9/9/7L1/8K b P 1";
    let board: Board = Board::tsume(sfen).unwrap();
    assert_eq!(board.side_to_move(), Color::Black);
    assert!(board.has_in_hand(Color::Black, Piece::Pawn));

    let mut num_moves = 0;
    board.generate_moves(|mvs| {
        // remember that the listener may be called back multiple times
        num_moves += mvs.into_iter().len();
        false
    });
    assert_eq!(num_moves, 85);
}

#[test]
fn donot_move_into_check() {
    let sfen: &str = "7lk/9/8S/9/9/9/9/7L1/8K b P 1";
    let mut board: Board = Board::tsume(sfen).unwrap();
    assert_eq!(board.side_to_move(), Color::Black);

    // Ki1-h1
    let mv = Move::BoardMove {
        from: Square::I1,
        to: Square::H1,
        promotion: false,
    };
    assert!(board.is_legal(mv));

    board.play_unchecked(mv);
    assert_eq!(board.side_to_move(), Color::White);
    assert_eq!(board.checkers, BitBoard::EMPTY);

    // L2ax2g+
    let mv = Move::BoardMove {
        from: Square::A2,
        to: Square::H2,
        promotion: true,
    };
    assert!(board.is_legal(mv));
    board.play_unchecked(mv);

    assert_eq!(board.side_to_move(), Color::Black);
    assert_eq!(board.checkers.len(), 1);
    assert!(board.checkers.has(Square::H2));
    assert_eq!(board.piece_on(Square::H2).unwrap(), Piece::PLance);
    assert_eq!(board.color_on(Square::H2).unwrap(), Color::White);

    let mv = Move::Drop {
        piece: Piece::Pawn,
        to: Square::E5,
    };
    assert!(!board.is_legal(mv));

    board.generate_moves(|mvs| {
        for mv in mvs {
            assert!(mv.is_board_move());
            let from: Square = mv.from().unwrap();
            let piece = board.piece_on(from).unwrap();
            assert_eq!(piece, Piece::King);
        }
        false
    });
}

#[test]
fn no_drop_on_top() {
    let board: Board = "ln1g5/1r4k2/p2pppn2/2ps2p2/1p7/2P6/PPSPPPPLP/2G2K1pr/LN4G1b b BG2SLPnp 61"
        .parse()
        .unwrap();
    assert_eq!(board.side_to_move(), Color::Black);
    let open_squares = !board.occupied();
    board.generate_drops(|mvs| {
        for mv in mvs {
            assert!(open_squares.has(mv.to()));
        }
        false
    });
}

#[test]
fn checkers_are_updated() {
    let sfen: &str = "7lk/9/8S/9/9/9/9/7L1/8K b P 1";
    let mut board: Board = Board::tsume(sfen).unwrap();

    // After K1i2i L2ax2h the Black King should be in check
    // and only King moves should be legal

    let mv1 = Move::BoardMove {
        from: Square::I1,
        to: Square::I2,
        promotion: false,
    };
    let mv2 = Move::BoardMove {
        from: Square::A2,
        to: Square::H2,
        promotion: false,
    };
    let mv3 = Move::BoardMove {
        from: Square::C1,
        to: Square::D2,
        promotion: true,
    };

    board.play_unchecked(mv1);
    assert_eq!(board.side_to_move(), Color::White);
    assert_eq!(board.checkers().len(), 0);

    board.play_unchecked(mv2);
    assert_eq!(board.side_to_move(), Color::Black);
    assert_eq!(board.checkers().len(), 1);
    assert!(board.checkers.has(Square::H2));
    assert!(!board.is_legal(mv3));
}

#[test]
fn tsume() {
    let sfen = "lpg6/3s2R2/1kpppp3/p8/9/P8/2N6/9/9 b BGN 1";
    // from_sfen will fail - since there is only one King on board
    assert!(matches!(
        Board::from_sfen(sfen),
        Err(SFENParseError::InvalidBoard)
    ));
    // tsume will succeed
    let board = Board::tsume(sfen).unwrap();
    assert!(board.has(Color::White, Piece::King));
    assert!(!board.has(Color::Black, Piece::King));
    assert_eq!(board.num_in_hand(Color::White, Piece::Gold), 2);
    assert_eq!(board.num_in_hand(Color::White, Piece::Silver), 3);
}

#[test]
#[cfg(not(feature = "annan"))]
fn generate_checks() {
    let sfen = "lpg6/3s2R2/1kpppp3/p8/9/P8/2N6/9/9 b BGN 1";
    let board = Board::tsume(sfen).unwrap();

    let mut nmoves: usize = 0;
    let mut nmoves_iter: usize = 0;
    let mut nmoves_into_iter: usize = 0;

    board.generate_board_moves(|mvs| {
        for mv in mvs {
            assert!(mv.is_board_move());
            nmoves += 1;
        }
        nmoves_iter += mvs.len();
        nmoves_into_iter += mvs.into_iter().len();
        false
    });
    assert_eq!(nmoves, 29);
    assert_eq!(nmoves_iter, 16); // this doesn't count promotions
    assert_eq!(nmoves_into_iter, 29); // should match nmoves

    let mut nchecks: usize = 0;
    let mut nchecks_iter: usize = 0;
    let mut nchecks_into_iter: usize = 0;

    board.generate_checks(|mvs| {
        for mv in mvs {
            assert!(mv.is_drop());
            nchecks += 1;
        }
        nchecks_iter += mvs.len();
        nchecks_into_iter += mvs.len();
        false
    });
    assert_eq!(nchecks, 15);
    assert_eq!(nchecks_iter, 15);
    assert_eq!(nchecks_into_iter, 15);
}

#[test]
#[cfg(not(feature = "annan"))]
fn play_tsume() {
    // first tsume in Zoku Tsumu-ya-Tsumuzaru-ya
    // by the First Meijin, Ohashi Sokei
    let sfen = "lpg6/3s2R2/1kpppp3/p8/9/P8/2N6/9/9 b BGN 1";
    let mut board = Board::tsume(sfen).unwrap();

    assert_eq!(board.side_to_move(), Color::Black);
    assert_eq!(board.status(), GameStatus::Ongoing);

    let moves = "\
        N*7e K8c-7b 
        B*8c K7b-8b 
        B8c-9b+ L9ax9b 
        R3bx6b= G7ax6b 
        S*8c K8b-9c 
        S8c-9b+ K9cx9b 
        G*8c K9b-9a L*9b";
    let moves: Vec<Move> = moves
        .split_ascii_whitespace()
        .map(|s| Move::parse(s).unwrap())
        .collect();

    for mv in moves {
        let mut v: Vec<Move> = Vec::new();

        if board.side_to_move() == Color::Black {
            board.generate_checks(|mvs| {
                v.extend(mvs);
                false
            });
        } else {
            assert_eq!(board.checkers.len(), 1);
            board.generate_moves(|mvs| {
                v.extend(mvs);
                false
            });
        }

        assert!(v.contains(&mv), "Move {mv} not found");
        board.play(mv);
    }

    assert_eq!(board.side_to_move(), Color::White);
    assert_eq!(board.status(), GameStatus::Won); // meaning: won for Black
}

#[test]
fn invalid_tsume() {
    // invalid position: White King is in check
    let sfen = "8l/5gB2/7Gk/7p1/7sp/9/9/9/9 b R";
    assert!(Board::tsume(sfen).is_err());
}

#[test]
#[cfg(not(feature = "annan"))]
fn discovered_checks1() {
    let sfen = "8l/5gB2/7G1/7pk/7sp/9/9/9/9 b R";
    let board = Board::tsume(sfen).unwrap();

    let checkers = board.checkers();
    let pinned = board.pinned();
    assert!(checkers.is_empty());
    assert!(pinned.is_empty());

    let mut moves: Vec<Move> = Vec::new();
    let mut checks: Vec<Move> = Vec::new();

    let gold = board.pieces(Piece::Gold);
    board.generate_board_moves_for(gold, |mvs| {
        moves.extend(mvs);
        false
    });
    assert_eq!(moves.len(), 5);

    board.generate_checks(|mvs| {
        for mv in mvs {
            if mv.is_board_move() {
                checks.push(mv);
            }
        }
        false
    });
    assert_eq!(checks.len(), 5);
    let mv: Move = "2c1b".parse::<Move>().unwrap();
    assert!(checks.contains(&mv));

    checks.clear();
    board.generate_checks(|mvs| {
        checks.extend(mvs);
        false
    });
    assert_eq!(checks.len(), 7);
}

#[test]
fn pinners() {
    let sfen = "8l/5gB2/8k/7p1/7sp/9/9/9/8K b RG";
    let mut board = Board::tsume(sfen).unwrap();

    assert!(board.checkers.is_empty());
    assert!(board.pinned.is_empty());

    let mv: Move = "G*2c".parse::<Move>().unwrap();
    assert!(board.is_legal(mv));
    board.play_unchecked(mv);

    assert!(board.checkers.len() == 1);
    assert!(board.pinned.is_empty());

    let mv: Move = "1c1d".parse::<Move>().unwrap();
    assert!(board.is_legal(mv));
    board.play_unchecked(mv);

    assert!(board.checkers.len() == 0);
    assert!(board.pinned.is_empty());
}

#[test]
#[cfg(not(feature = "annan"))]
fn undiscovered_checks() {
    /*
        R . . . . . G . k
        . . . . . . P . .
        . . . . . . . . .
        . . . . . . . . .
        . . . . . . . . .
        . . . . . . . . .
        . . . . . . . . .
        . . . . . . . . .
        . . . . . . . . .
    */

    let sfen = "R5G1k/6P2/9/9/9/9/9/9/9 b - 1";
    let board = Board::tsume(sfen).unwrap();

    let mv: Move = "3a2a".parse::<Move>().unwrap();
    assert_eq!(
        mv,
        Move::BoardMove {
            from: Square::A3,
            to: Square::A2,
            promotion: false
        }
    );
    assert!(board.is_legal(mv));

    let mut moves: Vec<Move> = Vec::new();
    let mut checks: Vec<Move> = Vec::new();

    board.generate_moves(|mvs| {
        moves.extend(mvs);
        false
    });

    board.generate_checks(|mvs| {
        checks.extend(mvs);
        false
    });

    assert!(moves.contains(&mv));
    assert!(checks.contains(&mv));
    assert_eq!(checks.len(), 1);
}

#[test]
fn discovered_checks2() {
    /*
    . . . . . . . . .
    . . . . . . . k .
    . . . . . . . . .
    . . . . . S . . .
    . . . . . . . . .
    . . . B . . . . .
    . . . . . . . . .
    . . . . . . . . .
    . . . . . . . . .
    */

    let sfen = "9/7k1/9/5S3/9/3B5/9/9/9 b - 1";
    let board = Board::tsume(sfen).unwrap();

    let mv = "4d3c".parse::<Move>().unwrap();
    assert!(board.is_legal(mv));

    let mv: Move = "4d3c+".parse::<Move>().unwrap();
    assert!(board.is_legal(mv));

    let silver = Square::D4.bitboard();

    let mut moves: Vec<Move> = Vec::new();
    let mut checks: Vec<Move> = Vec::new();

    board.generate_board_moves_for(silver, |mvs| {
        moves.extend(mvs);
        false
    });

    board.generate_checks(|mvs| {
        checks.extend(mvs);
        false
    });

    assert_eq!(moves.len(), 8);
    assert_eq!(checks.len(), 7);

    let mv = "4d3c".parse::<Move>().unwrap();
    assert!(moves.contains(&mv));
    assert!(checks.contains(&mv));

    let mv = "4d3c+".parse::<Move>().unwrap();
    assert!(moves.contains(&mv));
    assert!(checks.contains(&mv));

    let mv = "4d5e".parse::<Move>().unwrap();
    assert!(moves.contains(&mv));
    assert!(!checks.contains(&mv));
}

#[test]
#[cfg(not(feature = "annan"))]
fn discovered_checks3() {
    /*
    . . . . . . . . .
    . . . . . . . k .
    . . . . . . . . .
    . . . . . . . S .
    . . . . . . . . .
    . . . . . . . . .
    . . . . . . . . .
    . . . . . . . L .
    . . . . . . . . .
    */

    let sfen = "9/7k1/9/7S1/9/9/9/7L1/9 b -";
    let board = Board::tsume(sfen).unwrap();

    let mut moves: Vec<Move> = Vec::new();
    let mut checks: Vec<Move> = Vec::new();

    board.generate_moves(|mvs| {
        moves.extend(mvs);
        false
    });

    board.generate_checks(|mvs| {
        checks.extend(mvs);
        false
    });

    assert_eq!(moves.len(), 11);
    assert_eq!(checks.len(), 8);
}

#[test]
fn fuzzing_generate_moves() {
    let mut rng = rng();

    fn rollout(board: &mut Board, depth: usize, rng: &mut ThreadRng) -> bool {
        if depth == 0 {
            return true;
        }
        let mut v: Vec<Move> = Vec::new();
        board.generate_moves(|mvs| {
            v.extend(mvs);
            false
        });

        if v.is_empty() {
            return true;
        }
        let mv = v.choose(rng).unwrap();
        board.play_unchecked(*mv);
        rollout(board, depth - 1, rng)
    }
    for _ in 0..100 {
        let mut board = Board::startpos();
        assert!(rollout(&mut board, 100, &mut rng));
    }
}

#[test]
fn fuzzing_checks() {
    let mut rng = rng();

    // Zoku Tsumuya Tsumazaruya #198
    let sfen = "+P+n1g1+Pp+P1/2gg+p+s+pLn/1gppP1S+Pp/1+s+PPSPPPk/N1L2N+PL1/6L1+P/9/9/9 b - 1";

    fn rollout(board: &mut Board, depth: usize, rng: &mut ThreadRng) -> bool {
        if depth == 0 {
            return true;
        }
        let color = board.side_to_move();
        let mut v: Vec<Move> = Vec::new();
        if color == Color::Black {
            board.generate_checks(|mvs| {
                v.extend(mvs);
                false
            });
        } else {
            board.generate_moves(|mvs| {
                v.extend(mvs);
                false
            });
        }

        if v.is_empty() {
            return true;
        }
        let mv = v.choose(rng).unwrap();
        board.play(*mv);
        rollout(board, depth - 1, rng)
    }
    for _ in 0..200 {
        let mut board = Board::tsume(sfen).unwrap();
        assert!(rollout(&mut board, 100, &mut rng));
    }
}

#[test]
fn board_hash_trait_works() {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let board1 = Board::startpos();
    let board2 = Board::startpos();
    let mut board3 = Board::startpos();
    board3.play_unchecked("7g7f".parse().unwrap());

    let mut hasher1 = DefaultHasher::new();
    Hash::hash(&board1, &mut hasher1);
    let hash1 = hasher1.finish();

    let mut hasher2 = DefaultHasher::new();
    Hash::hash(&board2, &mut hasher2);
    let hash2 = hasher2.finish();

    let mut hasher3 = DefaultHasher::new();
    Hash::hash(&board3, &mut hasher3);
    let hash3 = hasher3.finish();

    assert_eq!(hash1, hash2, "Hashes of identical boards should match");
    assert_ne!(hash1, hash3, "Hashes of different boards should differ");
}

// ========================
// Annan-specific tests
// ========================

/// Capturing the backer resolves check when the checker only attacks via backing.
///
/// Position: Black King on 5i, White King on 1a, White Pawn on 5g backed by
/// White Rook on 5f. The Pawn moves like a Rook and checks the King along the file.
/// Black Rook on 5a can capture the backer on 5f, removing the check.
#[test]
#[cfg(feature = "annan")]
fn annan_capture_backer_resolves_check() {
    // rank a: Black Rook on 5a
    // rank c: White King on 9c (safe from Black Rook — different file)
    // rank f: White Rook on 5f (backer)
    // rank g: White Pawn on 5g (checker, moves like Rook due to backing)
    // rank i: Black King on 5i
    let board: Board = "4R4/9/k8/9/9/4r4/4p4/9/4K4 b - 1".parse().unwrap();

    assert!(
        !board.checkers().is_empty(),
        "Black King should be in check from backed Pawn"
    );

    // Capturing the backer (White Rook on 5f) with Black Rook from 5a
    let capture_backer = Move::BoardMove {
        from: Square::A5, // 5a
        to: Square::F5,   // 5f
        promotion: false,
    };
    assert!(
        board.is_legal(capture_backer),
        "Capturing the backer should be legal to resolve check"
    );

    // Verify: after capturing backer, Black is no longer in check
    let mut after = board.clone();
    after.play_unchecked(capture_backer);
    assert!(
        after.checkers().is_empty(),
        "After capturing backer, check should be resolved"
    );
}

/// Capturing the backer does NOT resolve check when the checker natively attacks the king.
///
/// Position: White Gold on 5h backed by White Rook on 5g.
/// The Gold moves like a Rook due to backing, but natively also attacks 5i.
/// Capturing the Rook (backer on 5g) doesn't help — Gold still checks natively.
#[test]
#[cfg(feature = "annan")]
fn annan_capture_backer_does_not_help_native_check() {
    // rank b: White King on 9b (safe)
    // rank g: White Rook on 5g (backer)
    // rank h: White Gold on 5h (checker — natively attacks 5i as Gold)
    // rank i: Black King on 5i
    // Black has a Rook on 5a to have a piece that could capture 5g
    let board: Board = "4R4/k8/9/9/9/9/4r4/4g4/4K4 b - 1".parse().unwrap();

    assert!(
        !board.checkers().is_empty(),
        "Black King should be in check from Gold"
    );

    // Capturing the backer (Rook on 5g) should NOT resolve check
    // because the Gold on 5h natively attacks 5i.
    let capture_backer = Move::BoardMove {
        from: Square::A5, // 5a
        to: Square::G5,   // 5g
        promotion: false,
    };
    assert!(
        !board.is_legal(capture_backer),
        "Capturing backer should NOT be legal when checker natively attacks king"
    );
}

/// Drops can interpose against a non-slider that is effectively a slider due to backing.
///
/// Position: White Pawn on 5g backed by White Rook on 5f → Pawn checks like Rook.
/// Black should be able to drop a piece on 5h to interpose.
#[test]
#[cfg(feature = "annan")]
fn annan_drop_interpose_backed_slider() {
    // rank b: White King on 9b (safe)
    // rank f: White Rook on 5f (backer)
    // rank g: White Pawn on 5g (checker, moves like Rook)
    // rank i: Black King on 5i
    let board: Board = "9/k8/9/9/9/4r4/4p4/9/4K4 b G 1".parse().unwrap();

    assert!(
        !board.checkers().is_empty(),
        "Black King should be in check"
    );

    // Drop Gold on 5h to interpose
    let drop = Move::Drop {
        piece: Piece::Gold,
        to: Square::H5, // 5h
    };
    assert!(
        board.is_legal(drop),
        "Dropping on interposition square should be legal against backed-slider check"
    );
}

/// Drops cannot interpose against an unbacked non-slider check.
#[test]
#[cfg(feature = "annan")]
fn annan_drop_no_interpose_non_slider() {
    // White Pawn on 5h (no backer) gives check to Black King on 5i
    // Black has a Gold in hand — but cannot interpose (no ray to block)
    // rank b: White King on 9b (safe)
    // rank h: White Pawn on 5h (no backer, checks King on 5i)
    // rank i: Black King on 5i
    let board: Board = "9/k8/9/9/9/9/9/4p4/4K4 b G 1".parse().unwrap();

    assert!(
        !board.checkers().is_empty(),
        "Black King should be in check from Pawn"
    );

    // No valid drop should resolve check (non-slider, nothing to interpose)
    let mut any_legal_drop = false;
    board.generate_drops(|moves| {
        for _mv in moves {
            any_legal_drop = true;
            return true;
        }
        false
    });
    assert!(!any_legal_drop, "No drop should resolve a non-slider check");
}

#[test]
#[cfg(feature = "annan")]
fn annan_sideways_pawn_move_creating_nifu_is_illegal() {
    let board: Board = "8k/9/4P4/9/9/9/3P5/3G5/K8 b - 1".parse().unwrap();

    let mv = Move::BoardMove {
        from: Square::G6, // 6g
        to: Square::G5,   // 5g
        promotion: false,
    };

    assert!(!board.is_legal_board_move(mv));
    assert!(!board.is_legal(mv));

    let mut generated = false;
    board.generate_board_moves(|mvs| {
        generated |= mvs.has(mv);
        generated
    });
    assert!(
        !generated,
        "A pawn move that creates nifu should not be generated"
    );
}

#[test]
#[cfg(feature = "annan")]
fn annan_nifu_pawn_move_is_legal_only_with_promotion() {
    let board: Board = "8k/9/3P5/3GG4/9/9/4P4/9/K8 b - 1".parse().unwrap();

    let non_promo = Move::BoardMove {
        from: Square::C6, // 6c
        to: Square::C5,   // 5c
        promotion: false,
    };
    let promo = Move::BoardMove {
        from: Square::C6, // 6c
        to: Square::C5,   // 5c
        promotion: true,
    };

    assert!(!board.is_legal_board_move(non_promo));
    assert!(board.is_legal_board_move(promo));

    let mut generated_non_promo = false;
    let mut generated_promo = false;
    board.generate_board_moves(|mvs| {
        generated_non_promo |= mvs.has(non_promo);
        generated_promo |= mvs.has(promo);
        false
    });

    assert!(
        !generated_non_promo,
        "The non-promotion nifu move should not be generated"
    );
    assert!(
        generated_promo,
        "The promotion move should still be generated"
    );
}

#[test]
#[cfg(feature = "annan")]
fn annan_sideways_pawn_move_updates_pawnless_files() {
    let board: Board = "8k/9/9/9/9/9/3P5/3G5/K8 b - 1".parse().unwrap();

    let mv = Move::BoardMove {
        from: Square::G6, // 6g
        to: Square::G5,   // 5g
        promotion: false,
    };

    assert!(board.is_legal_board_move(mv));

    let mut after = board.clone();
    after.play_unchecked(mv);

    assert!(
        after.pawn_drop_ok(Color::Black, Square::E6),
        "The source file should become pawnless after the pawn moves away"
    );
    assert!(
        !after.pawn_drop_ok(Color::Black, Square::E5),
        "The destination file should stop being pawnless after the pawn moves in"
    );
}

#[test]
#[cfg(feature = "annan")]
fn annan_nifu_pawn_move_does_not_count_as_check() {
    let board: Board = "9/9/4P4/9/9/5k3/3P5/3GG4/K8 b - 1".parse().unwrap();

    let mv = Move::BoardMove {
        from: Square::G6, // 6g
        to: Square::G5,   // 5g
        promotion: false,
    };

    assert!(!board.is_legal_board_move(mv));

    let mut generated = false;
    board.generate_checks(|mvs| {
        generated |= mvs.has(mv);
        generated
    });
    assert!(
        !generated,
        "A pawn move that becomes nifu should not be treated as a checking move"
    );

    let mut after = board.clone();
    after.play_unchecked(mv);
    assert!(
        after.checkers().is_empty(),
        "The moved nifu pawn itself should not be counted as a checker"
    );
}

#[test]
#[cfg(feature = "annan")]
fn annan_illegal_pawn_drop_mate_is_not_legal_or_generated() {
    let board: Board =
        "1nsg1gb1+B/1r1k5/2pp2p1p/1p4gp1/2n3n2/1P1PP2P1/2P1LKP1P/2+r6/2+l2GS+l1 w SL3Psn2p 1"
            .parse()
            .unwrap();
    let pawn_drop_mate: Move = "P*4f".parse().unwrap();

    assert!(
        !board.is_legal_drop(pawn_drop_mate),
        "illegal pawn-drop mate should not be legal"
    );
    assert!(
        !board.is_legal(pawn_drop_mate),
        "illegal pawn-drop mate should be excluded from legal moves"
    );

    let mut generated_drop = false;
    board.generate_drops(|moves| {
        generated_drop |= moves.has(pawn_drop_mate);
        generated_drop
    });
    assert!(
        !generated_drop,
        "illegal pawn-drop mate should not be generated as a drop"
    );

    let mut generated_move = false;
    board.generate_moves(|moves| {
        generated_move |= moves.has(pawn_drop_mate);
        generated_move
    });
    assert!(
        !generated_move,
        "illegal pawn-drop mate should not be generated as a legal move"
    );

    let mut generated_check = false;
    board.generate_checks(|moves| {
        generated_check |= moves.has(pawn_drop_mate);
        generated_check
    });
    assert!(
        !generated_check,
        "illegal pawn-drop mate should not be generated as a checking move"
    );
}

#[test]
#[cfg(feature = "annan")]
fn annan_double_check_can_be_resolved_by_capturing_shared_backer() {
    let board: Board =
        "1nsgkgs+Bl/1r5b1/2pp2p1p/1p5P1/2n6/1P1Pl4/2P2PP1P/5K3/1+lS+rpGSN+l w N4Pgp 1"
            .parse()
            .unwrap();
    let mating_try: Move = "6i5h".parse().unwrap();
    assert!(board.is_legal(mating_try));

    let mut after = board.clone();
    after.play_unchecked(mating_try);

    assert_eq!(
        after.checkers().len(),
        2,
        "the mating try currently creates a double check"
    );

    let defense: Move = "4i5h".parse().unwrap();
    assert!(
        after.is_legal(defense),
        "capturing the shared checker/backer should resolve both checks"
    );

    let mut generated = false;
    after.generate_moves(|mvs| {
        generated |= mvs.has(defense);
        generated
    });
    assert!(generated, "the legal defense should be generated");

    let mut resolved = after.clone();
    resolved.play_unchecked(defense);
    assert!(
        resolved.checkers().is_empty(),
        "after the capture, Black should no longer be in check"
    );
}

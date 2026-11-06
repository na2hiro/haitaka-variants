#![cfg(feature = "small-board")]
use haitaka::*;

fn position(pieces: &[(&str, char)], hand: &str) -> Board {
    let mut cells = vec![vec![None; File::NUM]; Rank::NUM];
    for &(name, piece) in pieces {
        let square: Square = name.parse().unwrap();
        cells[square.rank() as usize][square.file() as usize] = Some(piece);
    }
    let rows = cells
        .iter()
        .map(|row| {
            let mut text = String::new();
            let mut empty = 0;
            for cell in row.iter().rev() {
                if let Some(piece) = cell {
                    if empty > 0 {
                        text.push(char::from_digit(empty, 10).unwrap());
                        empty = 0;
                    }
                    text.push(*piece);
                } else {
                    empty += 1;
                }
            }
            if empty > 0 {
                text.push(char::from_digit(empty, 10).unwrap());
            }
            text
        })
        .collect::<Vec<_>>()
        .join("/");
    format!("{rows} b {hand} 1").parse().unwrap()
}
fn kings() -> Vec<(String, char)> {
    vec![
        ("1a".to_owned(), 'k'),
        (
            Square::new(File::ALL[File::NUM - 1], Rank::ALL[Rank::NUM - 1]).to_string(),
            'K',
        ),
    ]
}
fn with_piece(square: &str, piece: char, hand: &str) -> Board {
    let kings = kings();
    position(
        &[
            (kings[0].0.as_str(), 'k'),
            (kings[1].0.as_str(), 'K'),
            (square, piece),
        ],
        hand,
    )
}
fn legal(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|group| {
        moves.extend(group);
        false
    });
    moves
}

#[test]
fn starting_position_matches_selected_inventory_and_geometry() {
    let b = Board::startpos();
    assert_eq!(b.to_string(), SelectedRule::STARTPOS);
    assert_eq!(Square::NUM, File::NUM * Rank::NUM);
    assert_eq!(core::mem::size_of::<BitBoard>(), 8);
    for (i, &count) in SelectedRule::INVENTORY.iter().enumerate() {
        let piece = Piece::index_const(i);
        let on_board = Square::ALL
            .iter()
            .filter(|&&s| b.piece_on(s).is_some_and(|p| p.unpromote() == piece))
            .count();
        let in_hand = b.num_in_hand(Color::Black, piece) + b.num_in_hand(Color::White, piece);
        assert_eq!(on_board + in_hand as usize, count as usize, "{piece:?}");
    }
    assert!(!legal(&b).is_empty());
}

#[test]
fn promotion_regions_and_forced_promotion_follow_board_height() {
    let silver = with_piece("3c", 'S', "-");
    assert_eq!(
        silver.is_legal("3c3b+".parse().unwrap()),
        SelectedRule::PROMOTION_RANKS == 2
    );
    assert!(silver.is_legal("3c3b".parse().unwrap()));
    if SelectedRule::INVENTORY[0] > 0 {
        let pawn = with_piece("3b", 'P', "-");
        assert!(pawn.is_legal("3b3a+".parse().unwrap()));
        assert!(!pawn.is_legal("3b3a".parse().unwrap()));
    }
    if SelectedRule::INVENTORY[2] > 0 {
        let knight = with_piece("3d", 'N', "-");
        assert!(knight.is_legal("3d2b+".parse().unwrap()));
        assert!(!knight.is_legal("3d2b".parse().unwrap()));
    }
    let gold = with_piece("3c", 'G', "-");
    assert!(!gold.is_legal("3c3b+".parse().unwrap()));
}

#[test]
fn knight_lance_drop_and_promotion_edges() {
    if SelectedRule::INVENTORY[1] == 0 {
        return;
    }
    let b = with_piece("3d", 'L', "NL");
    assert!(b.is_legal("3d3a+".parse().unwrap()));
    assert!(!b.is_legal("3d3a".parse().unwrap()));
    for text in ["L*2a", "N*2a", "N*2b"] {
        assert!(!b.is_legal(text.parse().unwrap()), "{text}");
    }
    for text in ["L*2b", "N*2c"] {
        assert!(b.is_legal(text.parse().unwrap()), "{text}");
    }
}

#[test]
fn random_play_roundtrips_and_agrees_with_individual_legality() {
    let mut seed = 0x571ab10u64;
    for _ in 0..3 {
        let mut b = Board::startpos();
        for _ in 0..40 {
            let moves = legal(&b);
            for from in Square::ALL {
                if b.color_on(from) != Some(b.side_to_move()) {
                    continue;
                }
                for to in Square::ALL {
                    for promotion in [false, true] {
                        let mv = Move::BoardMove {
                            from,
                            to,
                            promotion,
                        };
                        assert_eq!(moves.contains(&mv), b.is_legal(mv), "{b} {mv}");
                    }
                }
            }
            for piece in &Piece::ALL[..Piece::HAND_NUM] {
                for to in Square::ALL {
                    let mv = Move::Drop { piece: *piece, to };
                    assert_eq!(moves.contains(&mv), b.is_legal(mv), "{b} {mv}");
                }
            }
            if moves.is_empty() {
                break;
            }
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            b.play(moves[seed as usize % moves.len()]);
            assert_eq!(b, b.to_string().parse::<Board>().unwrap());
        }
    }
}

#[cfg(feature = "gorogoro-plus")]
#[test]
fn pawn_drop_mate_protected_by_knight_is_excluded() {
    let b: Board = "s3s/2+P1k/gP1l1/p3S/LG2N/2KG1 b GN2Psp 25".parse().unwrap();
    let mv: Move = "P*1c".parse().unwrap();
    assert!(!b.is_legal(mv));
    assert!(!legal(&b).contains(&mv));
}

#[test]
fn bitboard_literals_follow_rectangular_file_major_coordinates() {
    #[cfg(feature = "judkins")]
    let corners = bitboard! {
        X . . . . .
        . . . . . .
        . . . . . .
        . . . . . .
        . . . . . .
        . . . . . X
    };
    #[cfg(any(feature = "gorogoro", feature = "gorogoro-plus"))]
    let corners = bitboard! {
        X . . . .
        . . . . .
        . . . . .
        . . . . .
        . . . . .
        . . . . X
    };
    #[cfg(any(feature = "minishogi", feature = "minishogi-kingin"))]
    let corners = bitboard! {
        X . . . .
        . . . . .
        . . . . .
        . . . . .
        . . . . X
    };
    let expected = Square::new(File::ALL[File::NUM - 1], Rank::A).bitboard()
        | Square::new(File::One, Rank::ALL[Rank::NUM - 1]).bitboard();
    assert_eq!(corners, expected);
}

#[test]
fn mate_search_works_with_king_gold_silver_on_each_geometry() {
    let kings = kings();
    let b = position(
        &[
            ("1a", 'k'),
            (kings[1].0.as_str(), 'K'),
            ("2c", 'G'),
            ("3c", 'S'),
        ],
        "-",
    );
    let result = b.dfpn(&DfpnOptions {
        max_nodes: Some(10000),
        ..Default::default()
    });
    assert_eq!(result.status, DfpnStatus::Mate);
    let mv = result.pv[0];
    assert!(b.is_legal(mv));
    let mut next = b;
    next.play(mv);
    assert_eq!(next.status(), GameStatus::Won);
}

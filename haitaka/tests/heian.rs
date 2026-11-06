#![cfg(feature = "heian")]
use haitaka::*;
fn position(pieces: &[(&str, char)], side: Color) -> Board {
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
    format!(
        "{rows} {side} - {}",
        if side == Color::Black { 1 } else { 2 }
    )
    .parse()
    .unwrap()
}
fn king() -> String {
    Square::new(File::ALL[File::NUM - 1], Rank::ALL[Rank::NUM - 1]).to_string()
}
fn setup(pieces: &[(&str, char)]) -> Board {
    let k = king();
    let mut all = vec![("1a", 'k'), (k.as_str(), 'K')];
    all.extend_from_slice(pieces);
    position(&all, Color::Black)
}
fn moves(b: &Board) -> Vec<Move> {
    let mut result = Vec::new();
    b.generate_moves(|group| {
        result.extend(group);
        false
    });
    result
}
#[test]
fn geometry_storage_and_initial_material() {
    let b = Board::startpos();
    assert_eq!(b.to_string(), SelectedRule::STARTPOS);
    assert_eq!(b.occupied().len() as usize, SelectedRule::MAX_BOARD_PIECES);
    assert_eq!(
        size_of::<BitBoard>(),
        if Square::NUM <= 64 { 8 } else { 16 }
    );
    assert!(moves(&b).iter().all(|mv| mv.is_board_move()));
    assert_eq!(BitBoard::FULL.len() as usize, Square::NUM);
    assert_eq!(BitBoard::FULL.flip_files().flip_ranks(), BitBoard::FULL);
    for square in Square::ALL {
        assert_eq!(square.flip().flip(), square);
        assert_eq!(
            square.bitboard().flip_files(),
            square.flip_file().bitboard()
        );
        assert_eq!(square.to_string().parse::<Square>().unwrap(), square);
    }
    assert!(File::One.east().is_empty());
    assert!(File::ALL[File::NUM - 1].west().is_empty());
}
#[test]
fn four_pieces_promote_to_canonical_gold() {
    for (piece, from, text) in [
        ('P', "3d", "3d3c+"),
        ('L', "3d", "3d3c+"),
        ('S', "3d", "3d3c+"),
        ('N', "3e", "3e2c+"),
    ] {
        let mut b = setup(&[(from, piece)]);
        let mv: Move = text.parse().unwrap();
        assert!(b.is_legal(mv));
        b.play(mv);
        assert_eq!(b.piece_on(mv.to()), Some(Piece::Gold));
        assert!(!b.to_string().contains('+'));
        assert_eq!(b, b.to_string().parse::<Board>().unwrap());
    }
    let silver = setup(&[("3c", 'S')]);
    assert!(silver.is_legal("3c4d+".parse().unwrap()));
    let gold = setup(&[("3b", 'G')]);
    assert!(!gold.is_legal("3b3c+".parse().unwrap()));
}
#[test]
fn forced_promotion_and_capture_discard() {
    for (piece, from, plain, prom) in [
        ('P', "3b", "3b3a", "3b3a+"),
        ('L', "3b", "3b3a", "3b3a+"),
        ('N', "3d", "3d2b", "3d2b+"),
    ] {
        let b = setup(&[(from, piece)]);
        assert!(!b.is_legal(plain.parse().unwrap()));
        assert!(b.is_legal(prom.parse().unwrap()));
    }
    let mut b = setup(&[("3d", 'S'), ("3b", 'g')]);
    b.play("3d3c+".parse().unwrap());
    b.play("3b3c".parse().unwrap());
    assert_eq!(b.piece_on("3c".parse().unwrap()), Some(Piece::Gold));
    for c in Color::ALL {
        assert!(b.is_hand_empty(c));
    }
    assert_eq!(b, b.to_string().parse::<Board>().unwrap());
    assert!(!b.is_legal("P*4d".parse().unwrap()));
}
#[test]
fn gold_counts_and_illegal_sfen_are_checked() {
    let b = setup(&[
        ("2d", 'G'),
        ("3d", 'G'),
        ("4d", 'G'),
        ("5d", 'G'),
        ("6d", 'G'),
    ]);
    assert_eq!(b, b.to_string().parse::<Board>().unwrap());
    let valid = b.to_string();
    for bad in [
        valid.replace(" b - ", " b P "),
        valid.replacen('G', "R", 1),
        valid.replacen('G', "B", 1),
        valid.replacen('G', "+P", 1),
    ] {
        assert!(Board::from_sfen(&bad).is_err(), "{bad}");
    }
    assert!(Board::tsume(&valid).is_err());
    assert!(Board::from_training_sfen(&valid).is_err());
}
#[test]
fn only_kings_draw_stops_generation_including_final_capture() {
    let mut b = position(&[("1a", 'k'), ("4d", 'K'), ("3c", 'p')], Color::Black);
    b.play("4d3c".parse().unwrap());
    assert_eq!(b.status(), GameStatus::Drawn);
    assert!(b.is_rule_draw());
    assert!(moves(&b).is_empty());
    assert!(!b.is_legal("1a1b".parse().unwrap()));
    assert_eq!(b, b.to_string().parse::<Board>().unwrap());
    assert!(!b.generate_board_moves(|_| panic!("terminal move")));
    assert!(!b.generate_board_moves_for(BitBoard::FULL, |_| panic!("terminal subset")));
    assert!(!b.generate_checks(|_| panic!("terminal check")));
    let result = b.dfpn(&DfpnOptions::default());
    assert_eq!(
        result.interruption_reason,
        Some(DfpnInterruptionReason::UnsupportedRuleset)
    );
}
#[test]
fn generated_and_individual_legality_and_hash_agree() {
    let mut seed = 0x571ab13u64;
    for _ in 0..3 {
        let mut b = Board::startpos();
        for _ in 0..128 {
            let legal = moves(&b);
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
                        assert_eq!(legal.contains(&mv), b.is_legal(mv), "{b} {mv}");
                    }
                }
            }
            if legal.is_empty() {
                break;
            }
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            b.play(legal[seed as usize % legal.len()]);
            let reparsed: Board = b.to_string().parse().unwrap();
            assert_eq!(b.hash(), reparsed.hash());
            assert_eq!(b, reparsed);
        }
    }
}

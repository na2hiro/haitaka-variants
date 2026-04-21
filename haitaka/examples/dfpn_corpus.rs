use std::env;
use std::process;

use haitaka::{Board, DfpnOptions, DfpnStatus, Move};

#[derive(Clone, Copy)]
struct ProblemCase {
    id: &'static str,
    label: &'static str,
    source: &'static str,
    sfen: &'static str,
    tracked: bool,
    advertised_steps: Option<usize>,
}

#[cfg(not(feature = "annan"))]
const CASES: &[ProblemCase] = &[
    ProblemCase {
        id: "zoku_first_meijin",
        label: "First Meijin classic from Zoku Tsumu-ya-Tsumuzaru-ya",
        source: "repo: haitaka/src/board/movegen/tests.rs",
        sfen: "lpg6/3s2R2/1kpppp3/p8/9/P8/2N6/9/9 b BGN 1",
        tracked: true,
        advertised_steps: Some(15),
    },
    ProblemCase {
        id: "zoku_198",
        label: "Zoku Tsumuya Tsumazaruya #198",
        source: "repo: haitaka/src/board/movegen/tests.rs",
        sfen: "+P+n1g1+Pp+P1/2gg+p+s+pLn/1gppP1S+Pp/1+s+PPSPPPk/N1L2N+PL1/6L1+P/9/9/9 b - 1",
        tracked: false,
        advertised_steps: None,
    },
    ProblemCase {
        id: "bestsel_beginner_13",
        label: "Best Selection 13-move beginner",
        source: "https://tsumeshogi.com/problems/c02eow9nc7",
        sfen: "5S1ks/4r1Ln1/5+R3/6Ppp/9/9/9/9/9 b LP2b4g2s3n2l14p 1",
        tracked: true,
        advertised_steps: Some(13),
    },
    ProblemCase {
        id: "bestsel_live_shape_13",
        label: "Best Selection live-shape 13",
        source: "https://tsumeshogi.com/problems/_1ageefuqp",
        sfen: "4+B2nl/7k1/6p1p/6bN1/9/9/9/9/9 b R2GLr2g4s2n2l16p 1",
        tracked: true,
        advertised_steps: Some(13),
    },
    ProblemCase {
        id: "bestsel_lightning_7",
        label: "Best Selection lightning 7",
        source: "https://tsumeshogi.com/problems/lq2fbabgnn",
        sfen: "9/9/9/4+B4/7+B1/5k3/4p1ps1/4s4/9 b 4G2r2s4n4l16p 1",
        tracked: true,
        advertised_steps: Some(7),
    },
    ProblemCase {
        id: "bestsel_44_19",
        label: "Best Selection 44-style 19",
        source: "https://tsumeshogi.com/problems/zwegpd9lmw",
        sfen: "8l/6S1k/6spp/6p2/9/9/9/9/9 b R3GNr2bg2s3n3l15p 1",
        tracked: true,
        advertised_steps: Some(19),
    },
    ProblemCase {
        id: "bestsel_44_7",
        label: "Best Selection 44-style 7",
        source: "https://tsumeshogi.com/problems/nyaryuiiki",
        sfen: "6s1k/7+b1/7+RN/5B3/9/9/9/9/9 b NPr4g3s2n4l17p 1",
        tracked: true,
        advertised_steps: Some(7),
    },
    ProblemCase {
        id: "bestsel_congrats_37",
        label: "Best Selection congratulatory 37",
        source: "https://tsumeshogi.com/problems/v-zwjicr0i",
        sfen: "9/9/3+p1+p3/2+p1+p1L2/4P3k/3+B5/1G7/B5S2/5RR2 b 2S3gs4n3l13p 1",
        tracked: true,
        advertised_steps: Some(37),
    },
    ProblemCase {
        id: "bestsel_twitter_13",
        label: "Best Selection Twitter 13",
        source: "https://tsumeshogi.com/problems/mg-9tiyjzz",
        sfen: "6+Bnk/6g2/9/9/9/9/9/9/9 b 2R2Nb3g4sn4l18p 1",
        tracked: true,
        advertised_steps: Some(13),
    },
    ProblemCase {
        id: "bestsel_swapped_7",
        label: "Best Selection swapped 7",
        source: "https://tsumeshogi.com/problems/fitejd88ks",
        sfen: "9/9/9/7R1/7S1/5S1k1/6+psR/8L/9 b G2b3gs4n3l17p 1",
        tracked: true,
        advertised_steps: Some(7),
    },
];

#[cfg(feature = "annan")]
const CASES: &[ProblemCase] = &[
    ProblemCase {
        id: "annan_aaaa_1",
        label: "Annan 1-ply by aaaa",
        source: "https://tsumeshogi.com/problems/qyde-vo2mq",
        sfen: "8k/7p1/8+P/9/9/9/9/9/9 b B2rb4g4s4n4l16p 1",
        tracked: true,
        advertised_steps: Some(1),
    },
    ProblemCase {
        id: "annan_maeda_1",
        label: "Annan 1-ply by t9maeda",
        source: "https://tsumeshogi.com/problems/wuxoka-ppv",
        sfen: "7k1/9/7S1/9/9/9/9/9/9 b N2r2b4g3s3n4l18p 1",
        tracked: true,
        advertised_steps: Some(1),
    },
    ProblemCase {
        id: "annan_walrus_5",
        label: "Annan walrus 5",
        source: "https://tsumeshogi.com/problems/lu1yp6fcqw",
        sfen: "7nk/6+r2/7Bp/9/9/9/9/9/9 b GNrb3g4s2n4l17p 1",
        tracked: true,
        advertised_steps: Some(5),
    },
    ProblemCase {
        id: "annan_ogata_11a",
        label: "Annan 11 by Ogata A",
        source: "https://tsumeshogi.com/problems/ig0vg-rhom",
        sfen: "5s3/7S1/7k1/4l2p1/2+R6/6b2/9/9/9 b BGSr3gs4n3l17p 1",
        tracked: true,
        advertised_steps: Some(11),
    },
    ProblemCase {
        id: "annan_boxfish_9",
        label: "Annan boxfish 9",
        source: "https://tsumeshogi.com/problems/ljnmhral7h",
        sfen: "6kn1/5g3/5+Bp1p/7p1/9/9/9/9/9 b SNL2rb3g3s2n3l15p 1",
        tracked: true,
        advertised_steps: Some(9),
    },
    ProblemCase {
        id: "annan_ogata_9",
        label: "Annan 9 by Ogata",
        source: "https://tsumeshogi.com/problems/sskygaom5v",
        sfen: "5sg2/5pkl1/9/6B2/7N1/9/6R2/9/9 b rb3g3s3n3l17p 1",
        tracked: true,
        advertised_steps: Some(9),
    },
    ProblemCase {
        id: "annan_easy_3",
        label: "Annan makes it easy 3",
        source: "https://tsumeshogi.com/problems/quuhsbghwt",
        sfen: "8k/6R2/9/6r2/9/9/9/9/9 b N2b4g4s3n4l18p 1",
        tracked: true,
        advertised_steps: Some(3),
    },
    ProblemCase {
        id: "annan_ogata_11b",
        label: "Annan 11 by Ogata B",
        source: "https://tsumeshogi.com/problems/gylwwugh0y",
        sfen: "9/9/9/6pg1/6nkb/9/7p1/9/7+p1 b B2GNP2rg4s2n4l14p 1",
        tracked: true,
        advertised_steps: Some(11),
    },
];

fn usage() -> ! {
    eprintln!("USAGE: dfpn_corpus [--nodes <n>] [--time-ms <ms>] [--tt-mb <mb>] [--pv <n>]");
    process::exit(1);
}

fn parse_board(sfen: &str) -> Result<Board, String> {
    Board::from_sfen(sfen)
        .or_else(|_| Board::tsume(sfen))
        .map_err(|err| format!("failed to parse SFEN: {err}"))
}

fn verify_mating_line(board: &Board, pv: &[Move]) -> bool {
    if pv.is_empty() {
        return false;
    }

    let attacker = board.side_to_move();
    let mut board = board.clone();
    for &mv in pv {
        if !board.is_legal(mv) {
            return false;
        }
        let is_attacker_turn = board.side_to_move() == attacker;
        board.play_unchecked(mv);
        if is_attacker_turn && board.checkers().is_empty() {
            return false;
        }
    }

    board.side_to_move() != attacker && !board.generate_moves(|_| true)
}

fn variant_name() -> &'static str {
    #[cfg(feature = "annan")]
    {
        "annan"
    }
    #[cfg(not(feature = "annan"))]
    {
        "standard"
    }
}

fn main() {
    let mut options = DfpnOptions {
        max_time_ms: Some(5_000),
        tt_megabytes: 64,
        ..DfpnOptions::default()
    };
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--nodes" => {
                let Some(value) = args.next() else {
                    usage();
                };
                options.max_nodes = Some(value.parse().unwrap_or_else(|_| usage()));
            }
            "--time-ms" => {
                let Some(value) = args.next() else {
                    usage();
                };
                options.max_time_ms = Some(value.parse().unwrap_or_else(|_| usage()));
            }
            "--tt-mb" => {
                let Some(value) = args.next() else {
                    usage();
                };
                options.tt_megabytes = value.parse().unwrap_or_else(|_| usage());
            }
            "--pv" => {
                let Some(value) = args.next() else {
                    usage();
                };
                options.max_pv_moves = value.parse().unwrap_or_else(|_| usage());
            }
            "--help" | "-h" => usage(),
            _ => usage(),
        }
    }

    println!("variant: {}", variant_name());
    println!(
        "limits: time_ms={:?} nodes={:?} tt_mb={} max_pv={}",
        options.max_time_ms, options.max_nodes, options.tt_megabytes, options.max_pv_moves
    );
    println!("cases: {}", CASES.len());
    println!();

    let tracked_cases = CASES.iter().filter(|case| case.tracked).count();
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut exact_length_matches = 0usize;
    let mut exact_length_total = 0usize;

    for case in CASES {
        let board = parse_board(case.sfen).unwrap_or_else(|err| {
            eprintln!("{} parse error: {}", case.id, err);
            process::exit(1);
        });
        let result = board.dfpn(&options);
        let legal_mate =
            result.status == DfpnStatus::Mate && verify_mating_line(&board, &result.pv);
        let advertised = case.advertised_steps;
        let exact_length = advertised.is_some_and(|steps| {
            exact_length_total += 1;
            result.pv.len() == steps
        });

        if case.tracked {
            if legal_mate {
                passed += 1;
                if exact_length {
                    exact_length_matches += 1;
                }
            } else {
                failed += 1;
            }
        }

        let outcome = if case.tracked {
            if legal_mate { "PASS" } else { "FAIL" }
        } else {
            "INFO"
        };
        let advertised_text = advertised
            .map(|steps| steps.to_string())
            .unwrap_or_else(|| "-".to_string());
        let length_note = if advertised.is_some() {
            if exact_length {
                "len=exact"
            } else {
                "len=diff"
            }
        } else {
            "len=n/a"
        };
        println!(
            "{outcome} id={} status={} pv_len={}/{} {} nodes={} ms={:.3}",
            case.id,
            result.status.as_str(),
            result.pv.len(),
            advertised_text,
            length_note,
            result.stats.nodes,
            result.stats.elapsed_ms
        );
        println!("  label: {}", case.label);
        println!("  source: {}", case.source);
        if !result.pv.is_empty() {
            let line = result
                .pv
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ");
            println!("  pv: {}", line);
        }
    }

    println!();
    println!(
        "summary: tracked={} passed={}/{} failed={} exact_lengths={}/{}",
        tracked_cases, passed, tracked_cases, failed, exact_length_matches, exact_length_total
    );
}

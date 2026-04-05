use std::env;
use std::process;

use haitaka::{Board, DfpnOptions, DfpnStatus, SFEN_STARTPOS};

fn usage() -> ! {
    eprintln!("USAGE: dfpn [--nodes <n>] [--time-ms <ms>] [--tt-mb <mb>] [--pv <n>] [<SFEN>]");
    process::exit(1);
}

fn parse_board(sfen: &str) -> Result<Board, String> {
    Board::from_sfen(sfen)
        .or_else(|_| Board::tsume(sfen))
        .map_err(|err| format!("failed to parse SFEN: {err}"))
}

fn main() {
    let mut options = DfpnOptions::default();
    let mut sfen: Option<String> = None;
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
            _ if sfen.is_none() => sfen = Some(arg),
            _ => usage(),
        }
    }

    let sfen = sfen.unwrap_or_else(|| SFEN_STARTPOS.to_string());
    let board = parse_board(&sfen).unwrap_or_else(|err| {
        eprintln!("{err}");
        process::exit(1);
    });
    let result = board.dfpn(&options);

    println!("status: {}", result.status.as_str());
    if result.status == DfpnStatus::Mate && !result.pv.is_empty() {
        let line = result
            .pv
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        println!("pv: {line}");
    }
    println!(
        "nodes: {} tt_hits: {} tt_stores: {} tt_collisions: {} elapsed_ms: {:.3}",
        result.stats.nodes,
        result.stats.tt_hits,
        result.stats.tt_stores,
        result.stats.tt_collisions,
        result.stats.elapsed_ms
    );
}

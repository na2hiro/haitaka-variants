//! Lishogi's Chu notation at the native CLI boundary.
use crate::ENGINE_NAME;
use crate::usi::{EngineEvalKind, UsiArgs};
use anyhow::{Context, Result, anyhow, bail, ensure};
use haitaka::extended::chu::{ChuGame, ChuPosition, ChuState, Path, PathMove};
use haitaka::extended::{PlacedPiece, WirePosition, chu};
use std::io::{self, BufRead, Write};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};

fn parse_move(text: &str) -> Result<PathMove> {
    let promotion = text.ends_with('+');
    let mut rest = text.strip_suffix(['+', '=', '?']).unwrap_or(text);
    let mut squares = [[0; 2]; 3];
    let mut count = 0;
    while !rest.is_empty() {
        ensure!(count < squares.len(), "too many squares in {text}");
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        ensure!(
            (1..=2).contains(&digits) && !rest.starts_with('0'),
            "invalid file in {text}"
        );
        let file = rest[..digits].parse::<u16>()?;
        let rank = rest.as_bytes().get(digits).copied().unwrap_or_default();
        ensure!(
            (1..=12).contains(&file) && (b'a'..=b'l').contains(&rank),
            "invalid square in {text}"
        );
        squares[count] = [file, u16::from(rank - b'a' + 1)];
        count += 1;
        rest = &rest[digits + 1..];
    }
    ensure!(count >= 2, "expected a source and destination in {text}");
    Ok(PathMove {
        schema_version: 2,
        from: squares[0],
        path: Path::from_steps(&squares[1..count]).expect("one or two destinations"),
        promotion,
    })
}

fn format_move(mv: PathMove) -> String {
    let mut text = String::new();
    for [file, rank] in std::iter::once(mv.from).chain(mv.path.iter().copied()) {
        text.push_str(&file.to_string());
        text.push(char::from(b'a' + (rank - 1) as u8));
    }
    if mv.promotion {
        text.push('+');
    }
    text
}

// Each SFEN symbol identifies the original piece and its specific promoted face.
fn piece_id(symbol: u8, promoted: bool) -> Option<&'static str> {
    let (base, promotion) = match symbol.to_ascii_lowercase() {
        b'p' => ("aa", Some("ai")),
        b'l' => ("ab", Some("cu")),
        b's' => ("ad", Some("de")),
        b'g' => ("ae", Some("dd")),
        b'b' => ("af", Some("dl")),
        b'r' => ("ag", Some("dm")),
        b'k' => ("ah", None),
        b'h' => ("am", Some("cz")),
        b'd' => ("an", Some("db")),
        b'n' => ("bl", None),
        b'e' => ("cf", Some("cg")),
        b'c' => ("ci", Some("df")),
        b'f' => ("cl", Some("dg")),
        b'a' => ("cm", Some("cv")),
        b't' => ("cn", Some("cw")),
        b'o' => ("co", Some("dh")),
        b'x' => ("cp", Some("dj")),
        b'i' => ("cq", Some("dk")),
        b'm' => ("cr", Some("cx")),
        b'v' => ("cs", Some("cy")),
        b'q' => ("ct", None),
        _ => return None,
    };
    if promoted { promotion } else { Some(base) }
}

fn parse_sfen(board: &str, side: &str, hands: &str, move_number: &str) -> Result<ChuGame> {
    ensure!(hands == "-", "Chu SFEN has no pieces in hand");
    ensure!(
        move_number.parse::<u32>()? > 0,
        "move number must be positive"
    );
    let side_to_move = match side {
        "b" => 0,
        "w" => 1,
        _ => bail!("invalid side to move"),
    };
    let mut pieces = Vec::new();
    let mut ranks = 0;
    for row in board.split('/') {
        ranks += 1;
        ensure!(ranks <= 12, "too many ranks in Chu SFEN");
        let mut remaining = 12u16;
        let mut bytes = row.bytes().peekable();
        while let Some(mut token) = bytes.next() {
            if token.is_ascii_digit() {
                ensure!(token != b'0', "invalid empty run in Chu SFEN");
                let mut run = u16::from(token - b'0');
                while bytes.peek().is_some_and(u8::is_ascii_digit) {
                    run = run * 10 + u16::from(bytes.next().unwrap() - b'0');
                    ensure!(run <= 12, "rank too wide in Chu SFEN");
                }
                remaining = remaining
                    .checked_sub(run)
                    .context("rank too wide in Chu SFEN")?;
                continue;
            }
            ensure!(remaining != 0, "rank too wide in Chu SFEN");
            let promoted = token == b'+';
            if promoted {
                token = bytes.next().context("missing promoted piece in Chu SFEN")?;
            }
            let piece = piece_id(token, promoted).context("unknown piece in Chu SFEN")?;
            pieces.push(PlacedPiece {
                at: [remaining, ranks],
                piece: piece.to_owned(),
                color: u8::from(token.is_ascii_lowercase()),
            });
            remaining -= 1;
        }
        ensure!(remaining == 0, "rank too short in Chu SFEN");
    }
    ensure!(ranks == 12, "expected twelve ranks in Chu SFEN");
    let position = ChuPosition::from_wire(&WirePosition {
        side_to_move,
        pieces,
        hands: Default::default(),
        state: ChuState::default(),
    })
    .map_err(|error| anyhow!(error))?;
    Ok(ChuGame::from_position(position))
}

fn startpos() -> ChuGame {
    ChuGame::parse(&chu::start_position_json()).expect("built-in Chu start position is valid")
}

fn position(tokens: &[&str]) -> Result<ChuGame> {
    let (mut game, tail) = match tokens {
        ["startpos", tail @ ..] => (startpos(), tail),
        ["sfen", board, side, hands, move_number, tail @ ..] => {
            (parse_sfen(board, side, hands, move_number)?, tail)
        }
        _ => bail!("expected position startpos or position sfen"),
    };
    match tail {
        [] => {}
        ["moves", moves @ ..] => {
            for text in moves {
                let mv = parse_move(text)?;
                game.play(mv).map_err(|err| anyhow!("{text}: {err}"))?;
            }
        }
        _ => bail!("expected moves after position"),
    }
    Ok(game)
}

struct Budget {
    depth: u8,
    millis: u32,
    nodes: u32,
    infinite: bool,
}

fn go(tokens: &[&str], side: usize, max_depth: u8) -> Result<Budget> {
    let mut depth = None;
    let mut millis = None;
    let mut nodes = None;
    let mut time = [None, None];
    let mut inc = [0, 0];
    let mut byoyomi = 0u32;
    let mut moves_to_go = 30;
    let mut tokens = tokens.iter().copied();
    let mut infinite = false;
    while let Some(key) = tokens.next() {
        if key == "infinite" {
            infinite = true;
            continue;
        }
        let value = tokens
            .next()
            .with_context(|| format!("{key} requires a number"))?
            .parse::<u32>()?;
        match key {
            "depth" => depth = Some(value),
            "movetime" => millis = Some(value),
            "nodes" => nodes = Some(value),
            "btime" => time[0] = Some(value),
            "wtime" => time[1] = Some(value),
            "binc" => inc[0] = value,
            "winc" => inc[1] = value,
            "byoyomi" => byoyomi = value,
            "movestogo" => moves_to_go = value.max(1),
            _ => bail!("unsupported go parameter {key}"),
        }
    }
    ensure!(
        !infinite || (millis.is_none() && time == [None, None] && byoyomi == 0 && inc == [0, 0]),
        "infinite conflicts with a clock"
    );
    let depth_cap = depth.unwrap_or(if infinite { 100 } else { u32::from(max_depth) });
    ensure!(
        (1..=100).contains(&depth_cap) && nodes != Some(0),
        "invalid depth or node limit"
    );
    let millis = if let Some(ms) = millis {
        ms.max(1)
    } else if time[side].is_some() || inc[side] > 0 || byoyomi > 0 {
        (time[side].unwrap_or(0) / moves_to_go)
            .saturating_add(byoyomi)
            .saturating_add(inc[side])
            .max(1)
    } else if infinite || nodes.is_some() || depth.is_some() {
        u32::MAX
    } else {
        1000
    };
    Ok(Budget {
        depth: depth_cap as u8,
        millis,
        nodes: nodes.unwrap_or(0),
        infinite,
    })
}

struct Running {
    stop: Arc<AtomicBool>,
    worker: JoinHandle<Result<()>>,
}

impl Running {
    fn stop(self) -> Result<()> {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.thread().unpark();
        self.worker
            .join()
            .map_err(|_| anyhow!("Chu search worker panicked"))?
    }
}

fn report(lines: &[&str]) -> Result<()> {
    let mut stdout = io::stdout().lock();
    for line in lines {
        writeln!(stdout, "{line}")?;
    }
    stdout.flush()?;
    Ok(())
}

fn launch(mut game: ChuGame, budget: Budget) -> Running {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let worker = thread::spawn(move || {
        let result = chu::search_position_cancellable(
            &mut game,
            budget.depth,
            budget.millis,
            budget.nodes,
            &flag,
        )
        .map_err(|error| anyhow!(error))?;
        if budget.infinite {
            while !flag.load(Ordering::Relaxed) {
                thread::park();
            }
        }
        let best = result
            .best_move
            .map_or_else(|| "resign".to_owned(), format_move);
        let stats = result.stats;
        let info = format!(
            "info depth {} time {:.0} nodes {} nps {} score cp {} pvsProbes {} pvsResearches {} lmrReductions {} lmrResearches {}",
            result.completed_depth,
            result.elapsed_ms,
            result.states,
            result.nps,
            result.score,
            stats.pvs_probes,
            stats.pvs_researches,
            stats.lmr_reductions,
            stats.lmr_researches,
        );
        let pv = if result.best_move.is_some() {
            format!("{info} pv {best}")
        } else {
            info
        };
        report(&[&pv, &format!("bestmove {best}")])
    });
    Running { stop, worker }
}

pub(super) fn run(args: UsiArgs) -> Result<()> {
    ensure!(
        args.eval == EngineEvalKind::Handcrafted
            && args.nnue.is_none()
            && args.nnue_multiplier == 1,
        "Chu USI uses handcrafted evaluation"
    );
    ensure!(
        (1..=100).contains(&args.movetime_max_depth),
        "depth cap must be 1..100"
    );

    let mut game = Some(startpos());
    let mut running: Option<Running> = None;
    for line in io::stdin().lock().lines() {
        let line = line?;
        let words: Vec<_> = line.split_whitespace().collect();
        match words.as_slice() {
            ["usi"] => report(&[
                &format!("id name {ENGINE_NAME} Chu"),
                "option name USI_Variant type combo default chushogi var chushogi",
                "usiok",
            ])?,
            ["isready"] => report(&["readyok"])?,
            ["setoption", "name", "USI_Variant", "value", "chushogi"] => {}
            ["stop"] => {
                if let Some(search) = running.take() {
                    search.stop()?;
                }
            }
            ["quit"] => break,
            ["usinewgame"] | ["gameover", ..] => {
                if let Some(search) = running.take() {
                    search.stop()?;
                }
                game = Some(startpos());
            }
            ["position", tail @ ..] => {
                if let Some(search) = running.take() {
                    search.stop()?;
                }
                match position(tail) {
                    Ok(next) => game = Some(next),
                    Err(err) => {
                        game = None;
                        report(&[&format!("info string invalid position command: {err}")])?;
                    }
                }
            }
            ["go", tail @ ..] => {
                if let Some(search) = running.take() {
                    search.stop()?;
                }
                let request = game.as_ref().context("no valid position").and_then(|game| {
                    go(
                        tail,
                        usize::from(game.side_to_move()),
                        args.movetime_max_depth,
                    )
                    .map(|budget| (game.clone(), budget))
                });
                match request {
                    Ok((game, budget)) => running = Some(launch(game, budget)),
                    Err(err) => report(&[
                        &format!("info string invalid go command: {err}"),
                        "bestmove resign",
                    ])?,
                }
            }
            [] => {}
            _ => report(&["info string unsupported Chu USI command"])?,
        }
    }
    if let Some(search) = running {
        search.stop()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lishogi_start_sfen_matches_the_json_board() {
        let board = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL";
        let parsed = parse_sfen(board, "b", "-", "1").unwrap();
        assert_eq!(parsed.to_json(), startpos().to_json());
    }

    #[test]
    fn complete_paths_and_promoted_pieces_survive_the_adapter() {
        let p = position(&[
            "sfen",
            "k11/12/12/12/5p6/6N5/12/12/12/12/12/K11",
            "b",
            "-",
            "1",
            "moves",
            "6f7e6f",
        ])
        .unwrap();
        assert_eq!(p.played.len(), 1);
        assert_eq!(format_move(p.played[0]), "6f7e6f");
        assert_eq!(p.current().to_wire().pieces.len(), 3);
        assert_eq!(p.current().to_wire().side_to_move, 1);
        for text in ["12l11k", "8d8c+", "12f11e12f"] {
            assert_eq!(format_move(parse_move(text).unwrap()), text);
        }
        for text in ["P*5e", "13l1a", "0a1a", "5a5m", "5a5b5c5d", "5a", "5aあ"] {
            assert!(parse_move(text).is_err(), "{text}");
        }
        assert_eq!(piece_id(b'H', true), Some("cz"));
        assert_eq!(piece_id(b'B', true), Some("dl"));
    }

    #[test]
    fn time_uses_the_side_to_move_and_bounds() {
        let b = go(
            &["btime", "60000", "wtime", "30000", "byoyomi", "2000"],
            1,
            64,
        )
        .unwrap();
        assert_eq!((b.depth, b.millis, b.nodes), (64, 3000, 0));
        assert!(go(&["nodes", "0"], 0, 64).is_err());
        assert!(go(&["depth", "101"], 0, 64).is_err());
    }
}

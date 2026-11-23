//! Plan phase 3 self-play teacher generator (simple, throughput-first).
//!
//! Every played position is labeled by the same fixed-node search that chooses
//! the move, so a label costs one search. Diversity comes from random opening
//! plies and an occasional random move. Games end on a real result, on
//! adjudication once |score| stays above a limit, or at a ply cap. Output is the
//! existing packed 72-byte training record (score from the side to move, result
//! from the side to move: 1 win, -1 loss, 0 draw/unknown).

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use haitaka::{Board, Color, GameStatus, Move};
use haitaka_wasm::{
    NnueModel, SearchEvalMode, SearchWorkspace,
    search_board_impl_handcrafted_with_node_budget_in_workspace,
    search_board_impl_with_eval_mode_and_node_budget_in_workspace,
};
use rand::rngs::StdRng;
use rand::seq::IndexedRandom;
use rand::{Rng, SeedableRng};
use serde_json::json;

use crate::dataset::pack_board_for_training;

const MATE_THRESHOLD: i32 = 29_000;
const SCORE_CLIP: i32 = 12_000;

pub struct GenerateArgs {
    pub games: u64,
    /// Global index of the first game, so shards of one run stay reproducible.
    pub first_game: u64,
    pub nodes: u64,
    pub random_plies: u16,
    pub random_move_rate: f64,
    pub adjudicate_score: i32,
    pub adjudicate_plies: u16,
    pub max_plies: u16,
    pub record_from_ply: u16,
    pub seed: u64,
    pub jobs: usize,
    pub nnue: Option<PathBuf>,
    pub output: PathBuf,
}

#[derive(Default)]
struct GameOut {
    records: Vec<u8>,
    positions: u64,
    plies: u64,
    nodes: u64,
    adjudicated: bool,
    capped: bool,
}

fn legal_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|batch| {
        moves.extend(batch.into_iter().filter(|m| board.is_legal(*m)));
        false
    });
    moves.sort_by_key(ToString::to_string);
    moves
}

fn search(
    board: &Board,
    nodes: u64,
    model: &Option<Arc<NnueModel>>,
    ws: &mut SearchWorkspace,
) -> Result<(Option<Move>, Option<i32>, u64)> {
    let summary = match model {
        Some(m) => search_board_impl_with_eval_mode_and_node_budget_in_workspace(
            board,
            nodes,
            64,
            m.clone(),
            SearchEvalMode::Incremental,
            ws,
        ),
        None => search_board_impl_handcrafted_with_node_budget_in_workspace(board, nodes, 64, ws),
    }
    .map_err(|e| anyhow!(e))?;
    let mv = summary
        .best_move
        .as_deref()
        .and_then(|t| Move::from_str(t).ok())
        .filter(|m| board.is_legal(*m));
    let score = if summary.completed_depth > 0 {
        summary.best_score
    } else {
        None
    };
    Ok((mv, score, summary.total_nodes))
}

fn play_game(index: u64, args: &GenerateArgs, model: &Option<Arc<NnueModel>>) -> Result<GameOut> {
    let mut rng = StdRng::seed_from_u64(args.seed ^ index.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    let mut board = Board::startpos();
    let mut out = GameOut::default();
    for _ in 0..args.random_plies {
        let moves = legal_moves(&board);
        let Some(&mv) = moves.choose(&mut rng) else {
            return Ok(out);
        };
        board.play_unchecked(mv);
    }
    let mut ws = SearchWorkspace::new();
    // (packed, score, ply, side to move)
    let mut pending: Vec<([u8; 64], i16, u16, Color)> = Vec::new();
    let mut lopsided = 0u16;
    let mut winner: Option<Color> = None;
    let mut ply = args.random_plies;
    loop {
        match board.status() {
            GameStatus::Ongoing => {}
            GameStatus::Won => {
                winner = Some(!board.side_to_move());
                break;
            }
            GameStatus::WonBySideToMove => {
                winner = Some(board.side_to_move());
                break;
            }
            GameStatus::Drawn => break,
        }
        if ply >= args.max_plies {
            out.capped = true;
            break;
        }
        let (mv, score, nodes) = search(&board, args.nodes, model, &mut ws)?;
        out.nodes += nodes;
        let Some(mut mv) = mv else { break };
        if let Some(score) = score {
            if score.abs() < MATE_THRESHOLD && ply >= args.record_from_ply {
                let packed = pack_board_for_training(&board)?;
                let clipped = score.clamp(-SCORE_CLIP, SCORE_CLIP) as i16;
                pending.push((packed, clipped, ply, board.side_to_move()));
            }
            if score.abs() >= args.adjudicate_score {
                lopsided += 1;
                if lopsided >= args.adjudicate_plies {
                    // side to move's view: positive means side to move is winning
                    winner = Some(if score > 0 {
                        board.side_to_move()
                    } else {
                        !board.side_to_move()
                    });
                    out.adjudicated = true;
                    break;
                }
            } else {
                lopsided = 0;
            }
        }
        if args.random_move_rate > 0.0 && rng.random_bool(args.random_move_rate) {
            let moves = legal_moves(&board);
            if let Some(&random) = moves.choose(&mut rng) {
                mv = random;
            }
        }
        board.play_unchecked(mv);
        ply += 1;
        out.plies += 1;
    }
    for (packed, score, ply, stm) in pending {
        let result: i8 = match winner {
            Some(c) if c == stm => 1,
            Some(_) => -1,
            None => 0,
        };
        out.records.extend_from_slice(&packed);
        out.records.extend_from_slice(&score.to_le_bytes());
        out.records.extend_from_slice(&0u16.to_le_bytes());
        out.records.extend_from_slice(&ply.to_le_bytes());
        out.records.push(result as u8);
        out.records.push(0);
        out.positions += 1;
    }
    Ok(out)
}

pub fn run(args: GenerateArgs) -> Result<()> {
    let model = match &args.nnue {
        Some(path) => Some(Arc::new(
            NnueModel::from_bytes(
                &fs::read(path).with_context(|| format!("read {}", path.display()))?,
            )
            .map_err(|e| anyhow!("load {}: {e}", path.display()))?,
        )),
        None => None,
    };
    let started = Instant::now();
    let jobs = args.jobs.max(1);
    let finished = AtomicU64::new(0);
    let step = (args.games / 20).max(1);
    let results: Vec<Result<Vec<(u64, GameOut)>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..jobs)
            .map(|worker| {
                let args = &args;
                let model = model.clone();
                let finished = &finished;
                scope.spawn(move || -> Result<Vec<(u64, GameOut)>> {
                    let mut done = Vec::new();
                    let mut index = worker as u64;
                    while index < args.games {
                        done.push((index, play_game(args.first_game + index, args, &model)?));
                        let n = finished.fetch_add(1, Ordering::Relaxed) + 1;
                        if n % step == 0 {
                            eprintln!(
                                "progress {n}/{} games, {:.0}s",
                                args.games,
                                started.elapsed().as_secs_f64()
                            );
                        }
                        index += jobs as u64;
                    }
                    Ok(done)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut games: Vec<(u64, GameOut)> = Vec::new();
    for r in results {
        games.extend(r?);
    }
    games.sort_by_key(|(i, _)| *i);
    let mut bytes = Vec::new();
    let (mut positions, mut plies, mut nodes, mut adjudicated, mut capped) = (0, 0, 0, 0, 0);
    for (_, g) in &games {
        bytes.extend_from_slice(&g.records);
        positions += g.positions;
        plies += g.plies;
        nodes += g.nodes;
        adjudicated += g.adjudicated as u64;
        capped += g.capped as u64;
    }
    write(&args.output, &bytes)?;
    let elapsed = started.elapsed().as_secs_f64();
    let report = json!({
        "schema": "haitaka-small-net-selfplay-generation-v1",
        "games": args.games,
        "firstGame": args.first_game,
        "nodesPerMove": args.nodes,
        "evaluator": args.nnue.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "handcrafted".into()),
        "randomPlies": args.random_plies,
        "randomMoveRate": args.random_move_rate,
        "adjudicateScore": args.adjudicate_score,
        "adjudicatePlies": args.adjudicate_plies,
        "maxPlies": args.max_plies,
        "recordFromPly": args.record_from_ply,
        "seed": args.seed,
        "jobs": jobs,
        "positions": positions,
        "searchedPlies": plies,
        "adjudicatedGames": adjudicated,
        "cappedGames": capped,
        "totalNodes": nodes,
        "elapsedSec": elapsed,
        "positionsPerSec": positions as f64 / elapsed,
        "positionsPerWorkerSec": positions as f64 / elapsed / jobs as f64,
    });
    write(
        &args.output.with_extension("json"),
        serde_json::to_string_pretty(&report)?.as_bytes(),
    )?;
    eprintln!("{}", serde_json::to_string(&report)?);
    Ok(())
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    Ok(())
}

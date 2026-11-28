//! Plan phase 3 self-play teacher generator (simple, throughput-first).
//!
//! Every played position is labeled by the same fixed-node search that chooses
//! the move, so a label costs one search. Diversity comes from random opening
//! plies and an occasional random move. Games end on a real result, on
//! adjudication once |score| stays above a limit, or at a ply cap. Output is the
//! existing packed 72-byte training record (score from the side to move, result
//! from the side to move: 1 win, -1 loss, 0 draw/unknown).

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail, ensure};
use haitaka::{Board, Color, GameStatus, Move};
use haitaka_wasm::{
    NnueModel, SearchEvalMode, SearchWorkspace,
    search_board_impl_handcrafted_with_node_budget_in_workspace,
    search_board_impl_with_eval_mode_and_node_budget_in_workspace,
};
use rand::rngs::StdRng;
use rand::seq::IndexedRandom;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

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
    /// Games per resumable shard file under `<output>.shards/`.
    pub shard_games: u64,
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

/// Rule features this binary was built with (part of the shard identity).
fn build_rule() -> String {
    let rules: &[(&str, bool)] = &[
        ("annan", cfg!(feature = "annan")),
        ("anhoku", cfg!(feature = "anhoku")),
        ("antouzai", cfg!(feature = "antouzai")),
        ("taimen", cfg!(feature = "taimen")),
        ("haimen", cfg!(feature = "haimen")),
        ("neko", cfg!(feature = "neko")),
        ("nekoneko", cfg!(feature = "nekoneko")),
        ("yokoneko", cfg!(feature = "yokoneko")),
        ("yokonekoneko", cfg!(feature = "yokonekoneko")),
        ("tenkyo", cfg!(feature = "tenkyo")),
        ("tenjiku", cfg!(feature = "tenjiku")),
        ("anki", cfg!(feature = "anki")),
        ("oute", cfg!(feature = "oute")),
        ("sanmaime", cfg!(feature = "sanmaime")),
        ("inochinotokin", cfg!(feature = "inochinotokin")),
        ("shihonkangen", cfg!(feature = "shihonkangen")),
        ("ningenshogi", cfg!(feature = "ningenshogi")),
        ("tennouzan", cfg!(feature = "tennouzan")),
        ("igyoku", cfg!(feature = "igyoku")),
        ("hitomasuou", cfg!(feature = "hitomasuou")),
        ("nakamaware", cfg!(feature = "nakamaware")),
        ("fushiyo", cfg!(feature = "fushiyo")),
        ("share", cfg!(feature = "share")),
        ("zoshoku", cfg!(feature = "zoshoku")),
        ("babanuki", cfg!(feature = "babanuki")),
        ("tensuu", cfg!(feature = "tensuu")),
        ("suika", cfg!(feature = "suika")),
    ];
    let on: Vec<&str> = rules.iter().filter(|(_, b)| *b).map(|(n, _)| *n).collect();
    if cfg!(feature = "small-board") {
        <haitaka::SelectedRule as haitaka::RuleSpec>::NAME.into()
    } else if on.is_empty() {
        "standard".into()
    } else {
        on.join("+")
    }
}

/// Everything that decides a game's bytes. A shard is reused only when this matches.
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct Identity {
    rule: String,
    first_game: u64,
    nodes: u64,
    random_plies: u16,
    random_move_rate: f64,
    adjudicate_score: i32,
    adjudicate_plies: u16,
    max_plies: u16,
    record_from_ply: u16,
    seed: u64,
    teacher_sha256: Option<String>,
    shard_games: u64,
}

#[derive(Serialize, Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "camelCase")]
struct ShardStats {
    positions: u64,
    plies: u64,
    nodes: u64,
    adjudicated: u64,
    capped: u64,
    bytes: u64,
}

impl ShardStats {
    fn add(&mut self, o: &ShardStats) {
        self.positions += o.positions;
        self.plies += o.plies;
        self.nodes += o.nodes;
        self.adjudicated += o.adjudicated;
        self.capped += o.capped;
        self.bytes += o.bytes;
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShardManifest {
    schema: String,
    identity: Identity,
    shard_index: u64,
    game_start: u64,
    game_count: u64,
    stats: ShardStats,
    sha256: String,
}

const SHARD_SCHEMA: &str = "haitaka-small-net-selfplay-shard-v1";

#[derive(Clone, Copy)]
struct ShardPlan {
    index: u64,
    start: u64,
    count: u64,
}

fn shard_paths(dir: &Path, index: u64) -> (PathBuf, PathBuf) {
    (
        dir.join(format!("shard-{index:06}.bin")),
        dir.join(format!("shard-{index:06}.json")),
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// A finished shard from an earlier run, or None when it has to be (re)generated.
fn reusable_shard(dir: &Path, plan: ShardPlan, identity: &Identity) -> Result<Option<ShardStats>> {
    let (bin, json) = shard_paths(dir, plan.index);
    let Ok(text) = fs::read(&json) else {
        return Ok(None);
    };
    let manifest: ShardManifest =
        serde_json::from_slice(&text).with_context(|| format!("parse {}", json.display()))?;
    if manifest.schema != SHARD_SCHEMA || manifest.identity != *identity {
        bail!(
            "{} was generated with different settings ({:?}, now {:?}); use another --output or delete {}",
            json.display(),
            manifest.identity,
            identity,
            dir.display()
        );
    }
    if manifest.game_start != plan.start || manifest.game_count != plan.count {
        return Ok(None);
    }
    let Ok(bytes) = fs::read(&bin) else {
        return Ok(None);
    };
    if bytes.len() as u64 != manifest.stats.bytes || sha256_hex(&bytes) != manifest.sha256 {
        eprintln!(
            "{} does not match its manifest; regenerating",
            bin.display()
        );
        return Ok(None);
    }
    Ok(Some(manifest.stats))
}

/// Writes one finished shard atomically: bin then manifest, each via a temp file and rename.
fn write_shard(
    dir: &Path,
    plan: ShardPlan,
    identity: &Identity,
    games: Vec<GameOut>,
) -> Result<ShardStats> {
    let mut bytes = Vec::new();
    let mut stats = ShardStats::default();
    for g in &games {
        bytes.extend_from_slice(&g.records);
        stats.positions += g.positions;
        stats.plies += g.plies;
        stats.nodes += g.nodes;
        stats.adjudicated += g.adjudicated as u64;
        stats.capped += g.capped as u64;
    }
    stats.bytes = bytes.len() as u64;
    let manifest = ShardManifest {
        schema: SHARD_SCHEMA.into(),
        identity: identity.clone(),
        shard_index: plan.index,
        game_start: plan.start,
        game_count: plan.count,
        stats,
        sha256: sha256_hex(&bytes),
    };
    let (bin, json) = shard_paths(dir, plan.index);
    write_atomic(&bin, &bytes)?;
    write_atomic(&json, serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    Ok(stats)
}

/// Games are played in parallel and grouped into shards of `shard_games` consecutive
/// game indices. Each shard is written to `<output>.shards/` as soon as all of its games
/// finish, so an interrupted run loses at most the shards still in flight; rerunning the
/// same command reuses the finished shards. Every game is seeded by its own index, so the
/// assembled output is the same however the run was split or resumed.
pub fn run(args: GenerateArgs) -> Result<()> {
    ensure!(args.shard_games > 0, "--shard-games must be positive");
    let (model, teacher_sha256) = match &args.nnue {
        Some(path) => {
            let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
            let sha = sha256_hex(&bytes);
            let model = NnueModel::from_bytes(&bytes)
                .map_err(|e| anyhow!("load {}: {e}", path.display()))?;
            (Some(Arc::new(model)), Some(sha))
        }
        None => (None, None),
    };
    let identity = Identity {
        rule: build_rule(),
        first_game: args.first_game,
        nodes: args.nodes,
        random_plies: args.random_plies,
        random_move_rate: args.random_move_rate,
        adjudicate_score: args.adjudicate_score,
        adjudicate_plies: args.adjudicate_plies,
        max_plies: args.max_plies,
        record_from_ply: args.record_from_ply,
        seed: args.seed,
        teacher_sha256,
        shard_games: args.shard_games,
    };
    let shard_dir = PathBuf::from(format!("{}.shards", args.output.display()));
    fs::create_dir_all(&shard_dir).with_context(|| format!("create {}", shard_dir.display()))?;
    let plans: Vec<ShardPlan> = (0..args.games.div_ceil(args.shard_games))
        .map(|index| {
            let start = index * args.shard_games;
            ShardPlan {
                index,
                start,
                count: args.shard_games.min(args.games - start),
            }
        })
        .collect();

    let mut stats: Vec<Option<ShardStats>> = Vec::with_capacity(plans.len());
    for plan in &plans {
        stats.push(reusable_shard(&shard_dir, *plan, &identity)?);
    }
    let resumed_shards = stats.iter().filter(|s| s.is_some()).count();
    let resumed_games: u64 = plans
        .iter()
        .zip(&stats)
        .filter(|(_, s)| s.is_some())
        .map(|(p, _)| p.count)
        .sum();
    if resumed_shards > 0 {
        eprintln!(
            "resuming: {resumed_shards}/{} shards ({resumed_games} games) already in {}",
            plans.len(),
            shard_dir.display()
        );
    }
    let pending: Vec<u64> = plans
        .iter()
        .zip(&stats)
        .filter(|(_, s)| s.is_none())
        .flat_map(|(p, _)| p.start..p.start + p.count)
        .collect();

    let started = Instant::now();
    let jobs = args.jobs.max(1);
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let finished = AtomicU64::new(resumed_games);
    let step = (args.games / 20).max(1);
    // shard index -> (slots for its games, games still missing)
    let open: Mutex<HashMap<u64, (Vec<Option<GameOut>>, u64)>> = Mutex::new(HashMap::new());
    let written: Mutex<Vec<(u64, ShardStats)>> = Mutex::new(Vec::new());
    let results: Vec<Result<()>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..jobs)
            .map(|_| {
                let (args, model, plans, identity, shard_dir) =
                    (&args, model.clone(), &plans, &identity, &shard_dir);
                let (next, failed, finished, open, written, pending) =
                    (&next, &failed, &finished, &open, &written, &pending);
                scope.spawn(move || -> Result<()> {
                    let body = || -> Result<()> {
                        while !failed.load(Ordering::Relaxed) {
                            let k = next.fetch_add(1, Ordering::Relaxed);
                            let Some(&index) = pending.get(k) else {
                                break;
                            };
                            let game = play_game(args.first_game + index, args, &model)?;
                            let plan = plans[(index / args.shard_games) as usize];
                            let ready = {
                                let mut open = open.lock().unwrap();
                                let entry = open.entry(plan.index).or_insert_with(|| {
                                    ((0..plan.count).map(|_| None).collect(), plan.count)
                                });
                                entry.0[(index - plan.start) as usize] = Some(game);
                                entry.1 -= 1;
                                if entry.1 == 0 {
                                    open.remove(&plan.index).map(|(slots, _)| slots)
                                } else {
                                    None
                                }
                            };
                            if let Some(slots) = ready {
                                let games = slots.into_iter().map(Option::unwrap).collect();
                                let s = write_shard(shard_dir, plan, identity, games)?;
                                written.lock().unwrap().push((plan.index, s));
                            }
                            let n = finished.fetch_add(1, Ordering::Relaxed) + 1;
                            if n % step == 0 {
                                eprintln!(
                                    "progress {n}/{} games, {:.0}s",
                                    args.games,
                                    started.elapsed().as_secs_f64()
                                );
                            }
                        }
                        Ok(())
                    };
                    let r = body();
                    if r.is_err() {
                        failed.store(true, Ordering::Relaxed);
                    }
                    r
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for r in results {
        r?;
    }
    let mut generated = ShardStats::default();
    for (index, s) in written.into_inner().unwrap() {
        generated.add(&s);
        stats[index as usize] = Some(s);
    }

    // Assemble the shards in game order, checking each against its manifest.
    let mut total = ShardStats::default();
    let tmp = args.output.with_extension("bin.tmp");
    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)?;
    }
    {
        let mut out = BufWriter::new(
            File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?,
        );
        for (plan, s) in plans.iter().zip(&stats) {
            let s = s.ok_or_else(|| anyhow!("shard {} was not generated", plan.index))?;
            let (bin, _) = shard_paths(&shard_dir, plan.index);
            let bytes = fs::read(&bin).with_context(|| format!("read {}", bin.display()))?;
            ensure!(
                bytes.len() as u64 == s.bytes,
                "{} has an unexpected size",
                bin.display()
            );
            out.write_all(&bytes)?;
            total.add(&s);
        }
        out.flush()?;
    }
    fs::rename(&tmp, &args.output)
        .with_context(|| format!("rename to {}", args.output.display()))?;

    let elapsed = started.elapsed().as_secs_f64();
    let report = json!({
        "schema": "haitaka-small-net-selfplay-generation-v1",
        "games": args.games,
        "firstGame": args.first_game,
        "nodesPerMove": args.nodes,
        "evaluator": args.nnue.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "handcrafted".into()),
        "teacherSha256": identity.teacher_sha256,
        "rule": identity.rule,
        "randomPlies": args.random_plies,
        "randomMoveRate": args.random_move_rate,
        "adjudicateScore": args.adjudicate_score,
        "adjudicatePlies": args.adjudicate_plies,
        "maxPlies": args.max_plies,
        "recordFromPly": args.record_from_ply,
        "seed": args.seed,
        "jobs": jobs,
        "shardGames": args.shard_games,
        "shards": plans.len(),
        "resumedShards": resumed_shards,
        "resumedGames": resumed_games,
        "positions": total.positions,
        "searchedPlies": total.plies,
        "adjudicatedGames": total.adjudicated,
        "cappedGames": total.capped,
        "totalNodes": total.nodes,
        "elapsedSec": elapsed,
        // Rates count only the games generated in this run.
        "generatedPositions": generated.positions,
        "positionsPerSec": generated.positions as f64 / elapsed,
        "positionsPerWorkerSec": generated.positions as f64 / elapsed / jobs as f64,
    });
    write_atomic(
        &args.output.with_extension("json"),
        serde_json::to_string_pretty(&report)?.as_bytes(),
    )?;
    eprintln!("{}", serde_json::to_string(&report)?);
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("rename to {}", path.display()))?;
    Ok(())
}

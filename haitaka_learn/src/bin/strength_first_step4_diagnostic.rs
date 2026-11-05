//! Bounded CPU-only move diagnostic for the strength-first Step 4 lane.
//!
//! The binary has four deliberately narrow modes. `extract` reconstructs a
//! deterministic reusable parent pool from retained development games,
//! `probe` runs one frozen root evaluator over a parent range, and `label`
//! searches only the predeclared union moves supplied by the orchestration
//! layer. `features` exports the same fixed positions for CPU checkpoint
//! evaluation. It cannot train, play games, or enumerate every legal child.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail, ensure};
use clap::{Parser, ValueEnum};
use haitaka::{Board, Color, GameStatus, Move};
use haitaka_wasm::{
    NnueModel, SearchEvalMode, SearchWorkspace, r1_donor_single_active_feature_indices,
    search_board_impl_handcrafted_with_node_budget_in_workspace,
    search_board_impl_with_eval_mode_and_node_budget_in_workspace,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const POOL_DOMAIN: &[u8] = b"haitaka-strength-first-step4-parent-pool-v1\0";
const MAX_DEPTH: u8 = 64;
const MATE_THRESHOLD: i32 = 29_000;
const MATE_BASE: i32 = 30_000;
const SCORE_CLIP: i32 = 12_000;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Mode {
    Extract,
    Probe,
    Label,
    Features,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum Evaluator {
    Handcrafted,
    Nnue,
}

#[derive(Debug, Parser)]
#[command(name = "strength-first-step4-diagnostic")]
struct Args {
    #[arg(long, value_enum)]
    mode: Mode,
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output_dir: PathBuf,
    #[arg(long, value_enum)]
    evaluator: Option<Evaluator>,
    #[arg(long)]
    model: Option<PathBuf>,
    #[arg(long, default_value_t = 1)]
    nnue_multiplier: i32,
    #[arg(long)]
    model_label: Option<String>,
    #[arg(long)]
    nodes: Option<u64>,
    #[arg(long, default_value_t = MAX_DEPTH)]
    max_depth: u8,
    #[arg(long, default_value_t = 128)]
    early_parents: usize,
    #[arg(long, default_value_t = 128)]
    middle_parents: usize,
    #[arg(long, default_value_t = 128)]
    late_parents: usize,
    #[arg(long, default_value_t = 0)]
    start: usize,
    #[arg(long)]
    end: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Parent {
    schema: String,
    id: String,
    source_games_sha256: String,
    source_game_index: u64,
    source_pair_index: u64,
    source_opening_index: u64,
    source_opening_group_id: String,
    source_cycle: u64,
    source_result: String,
    source_winner: Option<String>,
    source_engine_to_move: String,
    source_actual_move: String,
    ply: usize,
    phase: String,
    side_to_move: String,
    sfen: String,
    rank_sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChildStaticScore {
    move_: String,
    root_oriented_score: i32,
    terminal: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProbeRow {
    schema: String,
    parent_id: String,
    parent_sfen: String,
    evaluator: String,
    model_sha256: Option<String>,
    node_budget: u64,
    maximum_depth: u8,
    best_move: Option<String>,
    root_score: Option<i32>,
    completed_depth: u8,
    partial_iteration: bool,
    alpha_beta_nodes: u64,
    qsearch_nodes: u64,
    total_nodes: u64,
    elapsed_milliseconds: f64,
    legal_best_move: bool,
    static_parent_score: Option<i32>,
    child_static_scores: Vec<ChildStaticScore>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LabelRow {
    schema: &'static str,
    parent_id: String,
    parent_sfen: String,
    move_: String,
    child_sfen: String,
    completed: bool,
    terminal: bool,
    completed_depth: u8,
    partial_iteration: bool,
    alpha_beta_nodes: u64,
    qsearch_nodes: u64,
    total_nodes: u64,
    raw_child_side_to_move_score: Option<i32>,
    raw_root_oriented_score: Option<i32>,
    bounded_root_oriented_score: Option<i16>,
    root_oriented_mate_class: String,
    mate_distance: Option<i32>,
}

#[derive(Debug)]
struct ChildSearch {
    completed: bool,
    terminal: bool,
    completed_depth: u8,
    partial_iteration: bool,
    alpha_beta_nodes: u64,
    qsearch_nodes: u64,
    total_nodes: u64,
    raw_child_side_to_move_score: Option<i32>,
    raw_root_oriented_score: Option<i32>,
    bounded_root_oriented_score: Option<i16>,
    root_oriented_mate_class: String,
    mate_distance: Option<i32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Artifact {
    path: String,
    bytes: u64,
    sha256: String,
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn sha256_file(path: &Path) -> Result<String> {
    Ok(sha256_bytes(&fs::read(path)?))
}

fn artifact(path: &Path) -> Result<Artifact> {
    Ok(Artifact {
        path: path.display().to_string(),
        bytes: fs::metadata(path)?.len(),
        sha256: sha256_file(path)?,
    })
}

fn now_unix() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn color_name(color: Color) -> &'static str {
    match color {
        Color::Black => "black",
        Color::White => "white",
    }
}

fn phase(ply: usize) -> &'static str {
    if ply < 16 {
        "early"
    } else if ply < 40 {
        "middle"
    } else {
        "late"
    }
}

fn rank(source_sha256: &str, game_index: u64, ply: usize, sfen: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(POOL_DOMAIN);
    hasher.update(source_sha256.as_bytes());
    hasher.update(game_index.to_le_bytes());
    hasher.update((ply as u64).to_le_bytes());
    hasher.update(sfen.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn string(value: &Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .with_context(|| format!("{key} missing"))
}

fn integer(value: &Value, key: &str) -> Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .with_context(|| format!("{key} missing"))
}

fn engine_to_move(a_color: &str, side: Color) -> Result<&'static str> {
    let a_is_black = match a_color {
        "black" => true,
        "white" => false,
        _ => bail!("invalid aColor {a_color}"),
    };
    Ok(if (side == Color::Black) == a_is_black {
        "selected-width256"
    } else {
        "handcrafted"
    })
}

fn run_extract(args: &Args) -> Result<()> {
    ensure!(args.evaluator.is_none() && args.model.is_none() && args.nodes.is_none());
    ensure!(args.max_depth == MAX_DEPTH);
    ensure!(!args.output_dir.exists(), "output directory already exists");
    fs::create_dir_all(&args.output_dir)?;
    let source_sha256 = sha256_file(&args.input)?;
    let reader = BufReader::new(File::open(&args.input)?);
    let mut one_per_game_phase = BTreeMap::<(u64, String), Parent>::new();
    let mut records = 0usize;
    for (line_index, line) in reader.lines().enumerate() {
        let game: Value = serde_json::from_str(&line?)
            .with_context(|| format!("parse source game line {}", line_index + 1))?;
        ensure!(string(&game, "schema")? == "haitaka-self-play-game");
        let game_index = integer(&game, "gameIndex")?;
        let pair_index = integer(&game, "pairIndex")?;
        let a_color = string(&game, "aColor")?;
        let opening = game.get("opening").context("opening missing")?;
        let opening_index = integer(opening, "suiteIndex")?;
        let start_sfen = string(&game, "startSfen")?;
        let moves = game
            .get("moves")
            .and_then(Value::as_array)
            .context("moves missing")?;
        let mut board = Board::from_sfen(&start_sfen).map_err(|err| anyhow!(err))?;
        for (ply, move_value) in moves.iter().enumerate() {
            let move_text = move_value.as_str().context("move is not text")?;
            let move_ = Move::from_str(move_text).map_err(|err| {
                anyhow!("invalid move {move_text} at game {game_index} ply {ply}: {err}")
            })?;
            ensure!(
                board.is_legal(move_),
                "illegal retained game move {move_text}"
            );
            let sfen = board.to_string();
            let rank_sha256 = rank(&source_sha256, game_index, ply, &sfen);
            let phase_name = phase(ply).to_string();
            let parent = Parent {
                schema: "haitaka-strength-first-step4-parent-v1".to_string(),
                id: format!("step4-parent-{}", &rank_sha256[..24]),
                source_games_sha256: source_sha256.clone(),
                source_game_index: game_index,
                source_pair_index: pair_index,
                source_opening_index: opening_index,
                source_opening_group_id: format!("anhoku-v3-{:03}", opening_index + 1),
                source_cycle: pair_index / 64,
                source_result: string(&game, "result")?,
                source_winner: game
                    .get("winner")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                source_engine_to_move: engine_to_move(&a_color, board.side_to_move())?.to_string(),
                source_actual_move: move_text.to_string(),
                ply,
                phase: phase_name.clone(),
                side_to_move: color_name(board.side_to_move()).to_string(),
                sfen,
                rank_sha256: rank_sha256.clone(),
            };
            let key = (game_index, phase_name);
            if one_per_game_phase
                .get(&key)
                .is_none_or(|existing| parent.rank_sha256 < existing.rank_sha256)
            {
                one_per_game_phase.insert(key, parent);
            }
            board.play_unchecked(move_);
            records += 1;
        }
    }
    let quotas = [
        ("early", args.early_parents),
        ("middle", args.middle_parents),
        ("late", args.late_parents),
    ];
    let mut selected = Vec::new();
    let mut selected_sfens = BTreeSet::new();
    for (phase_name, quota) in quotas {
        let mut values = one_per_game_phase
            .values()
            .filter(|parent| parent.phase == phase_name)
            .cloned()
            .collect::<Vec<_>>();
        values.sort_by(|a, b| a.rank_sha256.cmp(&b.rank_sha256));
        for parent in values {
            if selected_sfens.insert(parent.sfen.clone()) {
                selected.push(parent);
                if selected
                    .iter()
                    .filter(|row| row.phase == phase_name)
                    .count()
                    == quota
                {
                    break;
                }
            }
        }
        ensure!(
            selected
                .iter()
                .filter(|row| row.phase == phase_name)
                .count()
                == quota,
            "{phase_name} pool quota cannot be filled after board deduplication"
        );
    }
    selected.sort_by(|a, b| a.id.cmp(&b.id));
    let unique_ids = selected.iter().map(|row| &row.id).collect::<BTreeSet<_>>();
    let unique_sfens = selected
        .iter()
        .map(|row| &row.sfen)
        .collect::<BTreeSet<_>>();
    ensure!(unique_ids.len() == selected.len(), "duplicate parent id");
    ensure!(
        unique_sfens.len() == selected.len(),
        "duplicate parent SFEN"
    );
    let output = args.output_dir.join("parents.jsonl");
    let mut writer = BufWriter::new(File::create(&output)?);
    for row in &selected {
        serde_json::to_writer(&mut writer, row)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    let summary = json!({
        "schema": "haitaka-strength-first-step4-parent-pool-v1",
        "createdAtUnixSeconds": now_unix()?,
        "source": artifact(&args.input)?,
        "candidatePositions": records,
        "onePerGamePhasePositions": one_per_game_phase.len(),
        "phaseDefinition": {"early": "ply < 16", "middle": "16 <= ply < 40", "late": "ply >= 40"},
        "phaseCounts": {"early": args.early_parents, "middle": args.middle_parents, "late": args.late_parents},
        "parents": artifact(&output)?,
        "passed": true
    });
    fs::write(
        args.output_dir.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    Ok(())
}

fn load_parents(path: &Path) -> Result<Vec<Parent>> {
    let reader = BufReader::new(File::open(path)?);
    let mut rows = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line?;
        if !line.trim().is_empty() {
            rows.push(
                serde_json::from_str(&line)
                    .with_context(|| format!("parent line {}", index + 1))?,
            );
        }
    }
    ensure!(!rows.is_empty(), "parent input is empty");
    Ok(rows)
}

fn legal_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|batch| {
        moves.extend(batch.into_iter().filter(|move_| board.is_legal(*move_)));
        false
    });
    moves.sort_by_key(ToString::to_string);
    moves.dedup_by_key(|move_| move_.to_string());
    moves
}

fn child_static_scores(board: &Board, model: &NnueModel) -> Vec<ChildStaticScore> {
    legal_moves(board)
        .into_iter()
        .map(|move_| {
            let mut child = board.clone();
            child.play_unchecked(move_);
            let terminal = child.status() == GameStatus::Won;
            ChildStaticScore {
                move_: move_.to_string(),
                root_oriented_score: if terminal {
                    MATE_BASE
                } else {
                    -model.evaluate(&child)
                },
                terminal,
            }
        })
        .collect()
}

fn run_probe(args: &Args) -> Result<()> {
    let evaluator = args.evaluator.context("--evaluator is required")?;
    let nodes = args.nodes.context("--nodes is required")?;
    ensure!(nodes > 0 && args.max_depth > 0);
    ensure!(!args.output_dir.exists(), "output directory already exists");
    fs::create_dir_all(&args.output_dir)?;
    let parents = load_parents(&args.input)?;
    let end = args.end.unwrap_or(parents.len());
    ensure!(
        args.start < end && end <= parents.len(),
        "invalid probe range"
    );
    let model = match evaluator {
        Evaluator::Handcrafted => {
            ensure!(
                args.model.is_none(),
                "handcrafted probe cannot take --model"
            );
            None
        }
        Evaluator::Nnue => {
            let path = args.model.as_ref().context("NNUE probe requires --model")?;
            let bytes = fs::read(path)?;
            Some((
                Arc::new(
                    NnueModel::from_bytes(&bytes)
                        .map_err(|err| anyhow!(err.to_string()))?
                        .with_evaluation_multiplier(args.nnue_multiplier)
                        .map_err(|err| anyhow!(err))?,
                ),
                sha256_bytes(&bytes),
            ))
        }
    };
    let evaluator_name = args.model_label.clone().unwrap_or_else(|| match evaluator {
        Evaluator::Handcrafted => "handcrafted".to_string(),
        Evaluator::Nnue => "nnue".to_string(),
    });
    let output = args.output_dir.join("probe.jsonl");
    let mut writer = BufWriter::new(File::create(&output)?);
    let started_all = Instant::now();
    let mut total_nodes = 0u64;
    let mut incomplete = 0usize;
    for parent in &parents[args.start..end] {
        let board = Board::from_sfen(&parent.sfen).map_err(|err| anyhow!(err))?;
        let mut workspace = SearchWorkspace::new();
        let started = Instant::now();
        let summary = match (&model, evaluator) {
            (None, Evaluator::Handcrafted) => {
                search_board_impl_handcrafted_with_node_budget_in_workspace(
                    &board,
                    nodes,
                    args.max_depth,
                    &mut workspace,
                )
            }
            (Some((model, _)), Evaluator::Nnue) => {
                search_board_impl_with_eval_mode_and_node_budget_in_workspace(
                    &board,
                    nodes,
                    args.max_depth,
                    Arc::clone(model),
                    SearchEvalMode::Incremental,
                    &mut workspace,
                )
            }
            _ => unreachable!(),
        }
        .map_err(|err| anyhow!(err))?;
        let best = summary
            .best_move
            .as_deref()
            .and_then(|text| Move::from_str(text).ok());
        let legal_best = best.is_some_and(|move_| board.is_legal(move_));
        let static_parent = model.as_ref().map(|(model, _)| model.evaluate(&board));
        let child_scores = model
            .as_ref()
            .map_or_else(Vec::new, |(model, _)| child_static_scores(&board, model));
        let row = ProbeRow {
            schema: "haitaka-strength-first-step4-root-probe-v1".to_string(),
            parent_id: parent.id.clone(),
            parent_sfen: parent.sfen.clone(),
            evaluator: evaluator_name.clone(),
            model_sha256: model.as_ref().map(|(_, sha)| sha.clone()),
            node_budget: nodes,
            maximum_depth: args.max_depth,
            best_move: summary.best_move,
            root_score: summary.best_score,
            completed_depth: summary.completed_depth,
            partial_iteration: summary.incomplete_iterations > 0,
            alpha_beta_nodes: summary.alpha_beta_nodes,
            qsearch_nodes: summary.qsearch_nodes,
            total_nodes: summary.total_nodes,
            elapsed_milliseconds: started.elapsed().as_secs_f64() * 1000.0,
            legal_best_move: legal_best,
            static_parent_score: static_parent,
            child_static_scores: child_scores,
        };
        ensure!(row.total_nodes <= nodes, "root probe exceeded node budget");
        ensure!(row.alpha_beta_nodes.checked_add(row.qsearch_nodes) == Some(row.total_nodes));
        incomplete += usize::from(
            row.root_score.is_none() || row.completed_depth == 0 || !row.legal_best_move,
        );
        total_nodes += row.total_nodes;
        serde_json::to_writer(&mut writer, &row)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    let receipt = json!({
        "schema": "haitaka-strength-first-step4-root-probe-receipt-v1",
        "createdAtUnixSeconds": now_unix()?,
        "input": artifact(&args.input)?,
        "output": artifact(&output)?,
        "range": {"start": args.start, "end": end},
        "evaluator": evaluator_name,
        "kind": match evaluator { Evaluator::Handcrafted => "handcrafted", Evaluator::Nnue => "nnue" },
        "model": match &args.model { Some(path) => Some(artifact(path)?), None => None },
        "nodeBudget": nodes,
        "maximumDepth": args.max_depth,
        "rows": end - args.start,
        "incompleteRows": incomplete,
        "totalNodes": total_nodes,
        "elapsedSeconds": started_all.elapsed().as_secs_f64(),
        "complete": incomplete == 0,
        "passed": true
    });
    fs::write(
        args.output_dir.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    Ok(())
}

fn mate_fields(score: Option<i32>) -> (String, Option<i32>, Option<i16>) {
    match score {
        Some(value) if value.abs() >= MATE_THRESHOLD => (
            if value > 0 { "win" } else { "loss" }.to_string(),
            Some((MATE_BASE - value.abs()).max(0)),
            Some((value.signum() * SCORE_CLIP) as i16),
        ),
        Some(value) => (
            "none".to_string(),
            None,
            Some(value.clamp(-SCORE_CLIP, SCORE_CLIP) as i16),
        ),
        None => ("unknown/incomplete".to_string(), None, None),
    }
}

fn search_child(board: &Board, nodes: u64, max_depth: u8) -> Result<ChildSearch> {
    if board.status() == GameStatus::Won {
        let raw_root = MATE_BASE;
        let (class, distance, bounded) = mate_fields(Some(raw_root));
        return Ok(ChildSearch {
            completed: true,
            terminal: true,
            completed_depth: 1,
            partial_iteration: false,
            alpha_beta_nodes: 0,
            qsearch_nodes: 0,
            total_nodes: 0,
            raw_child_side_to_move_score: Some(-MATE_BASE),
            raw_root_oriented_score: Some(raw_root),
            bounded_root_oriented_score: bounded,
            root_oriented_mate_class: class,
            mate_distance: distance,
        });
    }
    let mut workspace = SearchWorkspace::new();
    let summary = search_board_impl_handcrafted_with_node_budget_in_workspace(
        board,
        nodes,
        max_depth,
        &mut workspace,
    )
    .map_err(|err| anyhow!(err))?;
    let raw_root = summary.best_score.map(|score| -score);
    let (class, distance, bounded) = mate_fields(raw_root);
    Ok(ChildSearch {
        completed: summary.best_score.is_some() && summary.completed_depth > 0,
        terminal: false,
        completed_depth: summary.completed_depth,
        partial_iteration: summary.incomplete_iterations > 0,
        alpha_beta_nodes: summary.alpha_beta_nodes,
        qsearch_nodes: summary.qsearch_nodes,
        total_nodes: summary.total_nodes,
        raw_child_side_to_move_score: summary.best_score,
        raw_root_oriented_score: raw_root,
        bounded_root_oriented_score: bounded,
        root_oriented_mate_class: class,
        mate_distance: distance,
    })
}

fn run_label(args: &Args) -> Result<()> {
    ensure!(args.evaluator == Some(Evaluator::Handcrafted));
    ensure!(args.model.is_none());
    let nodes = args.nodes.context("--nodes is required")?;
    ensure!(nodes > 0 && args.max_depth > 0);
    ensure!(!args.output_dir.exists(), "output directory already exists");
    fs::create_dir_all(&args.output_dir)?;
    let values = BufReader::new(File::open(&args.input)?)
        .lines()
        .enumerate()
        .map(|(index, line)| -> Result<Value> {
            serde_json::from_str(&line?).with_context(|| format!("union plan line {}", index + 1))
        })
        .collect::<Result<Vec<_>>>()?;
    let end = args.end.unwrap_or(values.len());
    ensure!(
        args.start < end && end <= values.len(),
        "invalid label range"
    );
    let output = args.output_dir.join("labels.jsonl");
    let mut writer = BufWriter::new(File::create(&output)?);
    let started = Instant::now();
    let mut rows = 0usize;
    let mut incomplete = 0usize;
    let mut terminals = 0usize;
    let mut total_nodes = 0u64;
    for value in &values[args.start..end] {
        let parent_id = string(value, "id")?;
        let parent_sfen = string(value, "sfen")?;
        let board = Board::from_sfen(&parent_sfen).map_err(|err| anyhow!(err))?;
        let moves = value
            .get("unionMoves")
            .and_then(Value::as_array)
            .context("unionMoves missing")?;
        ensure!(
            !moves.is_empty() && moves.len() <= 4,
            "union must contain one through four moves"
        );
        let mut unique = BTreeSet::new();
        for move_value in moves {
            let move_text = move_value.as_str().context("union move is not text")?;
            ensure!(unique.insert(move_text), "duplicate union move");
            let move_ = Move::from_str(move_text)
                .map_err(|err| anyhow!("invalid union move {move_text}: {err}"))?;
            ensure!(board.is_legal(move_), "illegal union move {move_text}");
            let mut child = board.clone();
            child.play_unchecked(move_);
            let child_sfen = child.to_string();
            let search = search_child(&child, nodes, args.max_depth)?;
            ensure!(
                search.total_nodes <= nodes,
                "child label exceeded node budget"
            );
            ensure!(
                search.alpha_beta_nodes.checked_add(search.qsearch_nodes)
                    == Some(search.total_nodes)
            );
            incomplete += usize::from(!search.completed);
            terminals += usize::from(search.terminal);
            total_nodes += search.total_nodes;
            rows += 1;
            let row = LabelRow {
                schema: "haitaka-strength-first-step4-restricted-child-label-v1",
                parent_id: parent_id.clone(),
                parent_sfen: parent_sfen.clone(),
                move_: move_text.to_string(),
                child_sfen,
                completed: search.completed,
                terminal: search.terminal,
                completed_depth: search.completed_depth,
                partial_iteration: search.partial_iteration,
                alpha_beta_nodes: search.alpha_beta_nodes,
                qsearch_nodes: search.qsearch_nodes,
                total_nodes: search.total_nodes,
                raw_child_side_to_move_score: search.raw_child_side_to_move_score,
                raw_root_oriented_score: search.raw_root_oriented_score,
                bounded_root_oriented_score: search.bounded_root_oriented_score,
                root_oriented_mate_class: search.root_oriented_mate_class,
                mate_distance: search.mate_distance,
            };
            serde_json::to_writer(&mut writer, &row)?;
            writer.write_all(b"\n")?;
        }
    }
    writer.flush()?;
    let receipt = json!({
        "schema": "haitaka-strength-first-step4-restricted-label-receipt-v1",
        "createdAtUnixSeconds": now_unix()?,
        "input": artifact(&args.input)?,
        "output": artifact(&output)?,
        "range": {"start": args.start, "end": end},
        "parentRows": end - args.start,
        "labelRows": rows,
        "incompleteLabels": incomplete,
        "terminalLabels": terminals,
        "nodeBudgetPerChild": nodes,
        "maximumDepth": args.max_depth,
        "totalNodes": total_nodes,
        "elapsedSeconds": started.elapsed().as_secs_f64(),
        "passed": true
    });
    fs::write(
        args.output_dir.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    Ok(())
}

fn feature_row(id: &str, parent_id: &str, move_text: Option<&str>, board: &Board) -> Value {
    let black = r1_donor_single_active_feature_indices(board, Color::Black);
    let white = r1_donor_single_active_feature_indices(board, Color::White);
    let piece_count = usize::try_from(board.occupied().len()).unwrap_or(0).max(1);
    let output_bucket = (((piece_count - 1) * 8) / 40).min(7);
    json!({
        "schema": "haitaka-strength-first-step4-full-precision-position-v1",
        "id": id,
        "parentId": parent_id,
        "move": move_text,
        "sfen": board.to_string(),
        "outputBucket": output_bucket,
        "black": {"base": black.base, "donor": black.donor},
        "white": {"base": white.base, "donor": white.donor}
    })
}

fn run_features(args: &Args) -> Result<()> {
    ensure!(args.evaluator.is_none() && args.model.is_none() && args.nodes.is_none());
    ensure!(!args.output_dir.exists(), "output directory already exists");
    fs::create_dir_all(&args.output_dir)?;
    let values = BufReader::new(File::open(&args.input)?)
        .lines()
        .enumerate()
        .map(|(index, line)| -> Result<Value> {
            serde_json::from_str(&line?).with_context(|| format!("union plan line {}", index + 1))
        })
        .collect::<Result<Vec<_>>>()?;
    let output = args.output_dir.join("positions.jsonl");
    let mut writer = BufWriter::new(File::create(&output)?);
    let mut rows = 0usize;
    let mut ids = BTreeSet::new();
    for value in &values {
        let parent_id = string(value, "id")?;
        let parent_sfen = string(value, "sfen")?;
        let board = Board::from_sfen(&parent_sfen).map_err(|err| anyhow!(err))?;
        let id = format!("{parent_id}:parent");
        ensure!(ids.insert(id.clone()));
        serde_json::to_writer(&mut writer, &feature_row(&id, &parent_id, None, &board))?;
        writer.write_all(b"\n")?;
        rows += 1;
        let moves = value
            .get("unionMoves")
            .and_then(Value::as_array)
            .context("unionMoves missing")?;
        ensure!(!moves.is_empty() && moves.len() <= 4);
        for move_value in moves {
            let move_text = move_value.as_str().context("union move is not text")?;
            let move_ = Move::from_str(move_text)
                .map_err(|err| anyhow!("invalid union move {move_text}: {err}"))?;
            ensure!(board.is_legal(move_), "illegal union move {move_text}");
            let mut child = board.clone();
            child.play_unchecked(move_);
            let id = format!("{parent_id}:{move_text}");
            ensure!(ids.insert(id.clone()));
            serde_json::to_writer(
                &mut writer,
                &feature_row(&id, &parent_id, Some(move_text), &child),
            )?;
            writer.write_all(b"\n")?;
            rows += 1;
        }
    }
    writer.flush()?;
    let receipt = json!({
        "schema": "haitaka-strength-first-step4-full-precision-position-receipt-v1",
        "createdAtUnixSeconds": now_unix()?,
        "input": artifact(&args.input)?,
        "output": artifact(&output)?,
        "parents": values.len(),
        "positions": rows,
        "passed": true
    });
    fs::write(
        args.output_dir.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.mode {
        Mode::Extract => run_extract(&args),
        Mode::Probe => run_probe(&args),
        Mode::Label => run_label(&args),
        Mode::Features => run_features(&args),
    }
}

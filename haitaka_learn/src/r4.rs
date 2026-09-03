use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail, ensure};
use haitaka::{Board, Color, GameStatus, Move, Piece};
use haitaka_wasm::{
    R1_HALFKAV2_BASE_FEATURES, SEARCH_NODE_COUNTING_VERSION, SearchWorkspace,
    handcrafted_static_eval, r1_donor_single_active_feature_indices,
    search_board_impl_handcrafted_with_node_budget_in_workspace,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

use crate::dataset::{ENTRY_BYTES, pack_board_for_training};
use crate::openings::color_swap_anhoku_sfen;

const CONTRACT_SCHEMA: &str = "haitaka-anhoku-r4-data-contract-v1";
const POSITION_SCHEMA: &str = "haitaka-r4-position-v1";
const POSITION_MANIFEST_SCHEMA: &str = "haitaka-r4-position-shard-manifest-v1";
const LABEL_INPUT_MANIFEST_SCHEMA: &str = "haitaka-r4-label-input-manifest-v1";
const LABEL_MANIFEST_SCHEMA: &str = "haitaka-r4-labeled-shard-manifest-v1";
const LABEL_RECORD_SCHEMA: &str = "haitaka-r4-labeled-position-v1";
const DATASET_MANIFEST_SCHEMA: &str = "haitaka-r4-dataset-manifest-v1";
const GATE_SCHEMA: &str = "haitaka-anhoku-r4-gate-report-v1";
const DONOR_SINGLE_REAL_FEATURES: usize = 1_620;
const OUTPUT_BUCKETS: usize = 8;
const MAX_PIECES: usize = 40;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Contract {
    schema: String,
    ruleset: String,
    authorization: Authorization,
    opening_suite: OpeningSuiteContract,
    generation: GenerationContract,
    assembly: AssemblyContract,
    labeling: LabelingContract,
    cost: CostContract,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Authorization {
    r3_gate: PathBuf,
    r3_gate_sha256: String,
    require_passed: bool,
    require_r4_authorized: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpeningSuiteContract {
    path: PathBuf,
    sha256: String,
    development_opening_ids: Vec<String>,
    validation_opening_ids: Vec<String>,
    sealed_opening_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerationContract {
    schema: String,
    game_count: u32,
    maximum_ply: u16,
    shard_count: u32,
    seed: u64,
    trajectory_evaluator: String,
    candidate_limit: usize,
    near_best_child_nodes: u64,
    near_best_score_margin: i32,
    selfplay_perturbation_period: u64,
    policy_cycle: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssemblyContract {
    schema: String,
    deduplication_key: String,
    selection_seed: u64,
    label_shard_count: u32,
    pilot_split_counts: BTreeMap<String, usize>,
    phase_fractions: BTreeMap<String, f64>,
    static_clone_positions: usize,
    deep_core_positions: usize,
    deep_core_shard_count: u32,
    minimum_feature_row_updates: u64,
    maximum_duplicate_weight: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LabelingContract {
    schema: String,
    label_evaluator: String,
    node_budget: u64,
    maximum_depth: u8,
    node_counting_version: String,
    target_semantics: String,
    nonmate_clip: i32,
    mate_threshold: i32,
    mate_base: i32,
    mate_policy: String,
    score_transform: String,
    loss: String,
    huber_delta_normalized: f64,
    strength_training_lambda: f64,
    result_loss_enabled: bool,
    deep_reference_node_budget: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CostContract {
    cpu_core_hour_usd: f64,
    forecast_corpus_sizes: Vec<u64>,
    maximum_pilot_labels: u64,
    larger_generation_authorized: bool,
    strength_games_authorized: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct R3Authorization {
    passed: bool,
    r4_authorized: bool,
    selected_teacher: String,
    selected_node_budget: u64,
    node_counting_version: String,
    target_semantics: String,
    mate_policy: String,
    robust_score_clip: f64,
    score_transform: String,
    primary_loss: String,
    strength_training_lambda: f64,
    result_loss_enabled: bool,
}

#[derive(Debug, Clone)]
struct Opening {
    id: String,
    sfen: String,
    split: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PositionRecord {
    schema: String,
    id: String,
    sfen: String,
    packed_board_sha256: String,
    color_swap_canonical_sha256: String,
    source: String,
    source_game: String,
    opening_group: String,
    opening_sfen_sha256: String,
    split: String,
    trajectory_policy: String,
    trajectory_evaluator: String,
    trajectory_budget: String,
    candidate_inclusion_probability: f64,
    ply: u16,
    phase: String,
    side_to_move: String,
    material_bucket: String,
    in_check: bool,
    tactical: bool,
    has_promotion: bool,
    has_drop: bool,
    result_status: String,
    donor_state: String,
    active_base_rows_black: usize,
    active_donor_rows_black: usize,
    active_base_rows_white: usize,
    active_donor_rows_white: usize,
    output_bucket: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LabelRecord {
    schema: String,
    tier: String,
    ordinal: u64,
    position: PositionRecord,
    label_evaluator: String,
    label_budget: u64,
    maximum_depth: u8,
    node_counting_version: String,
    completed: bool,
    completed_depth: u8,
    partial_iteration: bool,
    node_accounting_exact: bool,
    alpha_beta_nodes: u64,
    qsearch_nodes: u64,
    total_nodes: u64,
    root_move: Option<String>,
    pv: Option<Vec<String>>,
    raw_score: i32,
    bounded_score: i16,
    target_transform_version: String,
    mate_class: String,
    mate_distance: Option<i32>,
    elapsed_ms: f64,
    label_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PositionShardManifest {
    schema: String,
    contract_sha256: String,
    shard_index: u32,
    shard_count: u32,
    games: u32,
    positions: u64,
    positions_sha256: String,
    generation_schema: String,
    engine_revision: String,
    executable_sha256: String,
    hostname: String,
    architecture: String,
    elapsed_seconds: f64,
    generated_at_unix_ms: u128,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LabelInputManifest {
    schema: String,
    contract_sha256: String,
    tier: String,
    shard_index: u32,
    shard_count: u32,
    positions: u64,
    positions_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LabelShardManifest {
    schema: String,
    contract_sha256: String,
    tier: String,
    shard_index: u32,
    shard_count: u32,
    positions: u64,
    labels_sha256: String,
    label_evaluator: String,
    label_budget: u64,
    node_counting_version: String,
    incomplete_labels: u64,
    node_accounting_failures: u64,
    mate_labels: u64,
    search_elapsed_seconds: f64,
    wall_elapsed_seconds: f64,
    labels_per_search_second: f64,
    jobs: usize,
    engine_revision: String,
    executable_sha256: String,
    hostname: String,
    architecture: String,
    started_at_unix_ms: u128,
    finished_at_unix_ms: u128,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssemblyReport {
    schema: String,
    contract_sha256: String,
    candidate_records: u64,
    exact_duplicate_records: u64,
    color_swap_duplicate_records: u64,
    unique_color_swap_families: u64,
    selected_pilot_positions: u64,
    split_counts: BTreeMap<String, u64>,
    phase_counts: BTreeMap<String, u64>,
    source_counts: BTreeMap<String, u64>,
    label_shards: u32,
    deep_core_positions: u64,
    deep_core_shards: u32,
    static_clone_positions: u64,
    opening_group_overlap: Vec<String>,
    cross_split_color_swap_overlap: u64,
    effective_sample_size: f64,
    maximum_duplicate_weight: u32,
}

pub(crate) struct GenerateArgs<'a> {
    pub contract_path: &'a Path,
    pub output_dir: &'a Path,
    pub shard_index: u32,
    pub workspace_root: &'a Path,
    pub executable_path: &'a Path,
}

pub(crate) struct AssembleArgs<'a> {
    pub contract_path: &'a Path,
    pub inputs: &'a [PathBuf],
    pub output_dir: &'a Path,
    pub workspace_root: &'a Path,
}

pub(crate) struct LabelArgs<'a> {
    pub contract_path: &'a Path,
    pub input: &'a Path,
    pub output_dir: &'a Path,
    pub jobs: usize,
    pub workspace_root: &'a Path,
    pub executable_path: &'a Path,
}

pub(crate) struct FinalizeArgs<'a> {
    pub contract_path: &'a Path,
    pub inputs: &'a [PathBuf],
    pub output_dir: &'a Path,
    pub workspace_root: &'a Path,
    pub regeneration_proofs: &'a [PathBuf],
}

pub(crate) struct RegenerationArgs<'a> {
    pub contract_path: &'a Path,
    pub manifest_path: &'a Path,
    pub output: &'a Path,
    pub workspace_root: &'a Path,
    pub executable_path: &'a Path,
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(
        &fs::read(path).with_context(|| format!("failed to read {}", path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", path.display()))
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn stable_u64(seed: u64, parts: &[&[u8]]) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(b"haitaka-r4-stable-stream-v1\0");
    hasher.update(seed.to_le_bytes());
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    let digest = hasher.finalize();
    u64::from_le_bytes(digest[..8].try_into().expect("SHA-256 prefix"))
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn host_name() -> String {
    Command::new("hostname")
        .output()
        .ok()
        .filter(|v| v.status.success())
        .map(|v| String::from_utf8_lossy(&v.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn collect_source_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_source_files(&path, files)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs")
            || path.file_name().and_then(|value| value.to_str()) == Some("Cargo.toml")
        {
            files.push(path);
        }
    }
    Ok(())
}

fn engine_revision(workspace_root: &Path) -> String {
    let mut files = vec![
        workspace_root.join("Cargo.toml"),
        workspace_root.join("Cargo.lock"),
    ];
    for member in ["haitaka", "haitaka_wasm", "haitaka_learn", "haitaka_types"] {
        if collect_source_files(&workspace_root.join(member), &mut files).is_err() {
            return "source-identity-error".to_string();
        }
    }
    files.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"haitaka-r4-source-identity-v1\0");
    for path in files {
        let Ok(relative) = path.strip_prefix(workspace_root) else {
            return "source-identity-error".to_string();
        };
        let Ok(bytes) = fs::read(&path) else {
            return "source-identity-error".to_string();
        };
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    format!("{:x}", hasher.finalize())
}

fn load_contract(path: &Path, workspace_root: &Path) -> Result<(Contract, String)> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let contract: Contract = serde_json::from_slice(&bytes)?;
    ensure!(
        contract.schema == CONTRACT_SCHEMA,
        "unexpected R4 contract schema"
    );
    ensure!(contract.ruleset == "anhoku", "R4 is frozen for Anhoku");
    ensure!(contract.generation.schema == "haitaka-r4-position-generation-v1");
    ensure!(contract.assembly.schema == "haitaka-r4-immutable-split-v1");
    ensure!(contract.labeling.schema == "haitaka-r4-label-v1");
    ensure!(contract.labeling.node_counting_version == SEARCH_NODE_COUNTING_VERSION);
    ensure!(contract.cost.maximum_pilot_labels == 1_000_000);
    ensure!(
        !contract.cost.larger_generation_authorized && !contract.cost.strength_games_authorized
    );
    ensure!(contract.assembly.maximum_duplicate_weight == 1);
    ensure!(contract.assembly.deduplication_key == "minimum-packed-board-color-swap-sha256-v1");
    ensure!((contract.labeling.huber_delta_normalized - 0.10).abs() < f64::EPSILON);
    ensure!(contract.generation.selfplay_perturbation_period > 1);
    let r3_path = workspace_root.join(&contract.authorization.r3_gate);
    ensure!(
        sha256_file(&r3_path)? == contract.authorization.r3_gate_sha256,
        "R3 gate hash mismatch"
    );
    let r3: R3Authorization = read_json(&r3_path)?;
    ensure!(
        !contract.authorization.require_passed || r3.passed,
        "R3 did not pass"
    );
    ensure!(
        !contract.authorization.require_r4_authorized || r3.r4_authorized,
        "R4 is not authorized"
    );
    ensure!(r3.selected_teacher == contract.labeling.label_evaluator);
    ensure!(r3.selected_node_budget == contract.labeling.node_budget);
    ensure!(r3.node_counting_version == contract.labeling.node_counting_version);
    ensure!(r3.target_semantics == contract.labeling.target_semantics);
    ensure!(r3.mate_policy == contract.labeling.mate_policy);
    ensure!(
        (r3.robust_score_clip - f64::from(contract.labeling.nonmate_clip)).abs() < f64::EPSILON
    );
    ensure!(r3.score_transform == contract.labeling.score_transform);
    ensure!(r3.primary_loss == contract.labeling.loss);
    ensure!(
        (r3.strength_training_lambda - contract.labeling.strength_training_lambda).abs()
            < f64::EPSILON
    );
    ensure!(r3.result_loss_enabled == contract.labeling.result_loss_enabled);
    let opening_path = workspace_root.join(&contract.opening_suite.path);
    ensure!(
        sha256_file(&opening_path)? == contract.opening_suite.sha256,
        "opening suite hash mismatch"
    );
    Ok((contract, hash_bytes(&bytes)))
}

fn load_openings(contract: &Contract, workspace_root: &Path) -> Result<Vec<Opening>> {
    let path = workspace_root.join(&contract.opening_suite.path);
    let text = fs::read_to_string(&path)?;
    let validation = contract
        .opening_suite
        .validation_opening_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let development = contract
        .opening_suite
        .development_opening_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let sealed = contract
        .opening_suite
        .sealed_opening_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    ensure!(
        validation.is_disjoint(&development)
            && validation.is_disjoint(&sealed)
            && development.is_disjoint(&sealed)
    );
    let mut result = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (id, sfen) = line
            .split_once('\t')
            .with_context(|| format!("invalid opening line {}", line_index + 1))?;
        Board::from_sfen(sfen).map_err(|err| anyhow!("invalid opening {id}: {err}"))?;
        let split = if sealed.contains(id) {
            "sealed"
        } else if validation.contains(id) {
            "validation"
        } else if development.contains(id) {
            "development"
        } else {
            "training"
        };
        result.push(Opening {
            id: id.to_string(),
            sfen: sfen.to_string(),
            split: split.to_string(),
        });
    }
    ensure!(result.len() == 64, "expected 64 frozen openings");
    Ok(result)
}

fn legal_moves(board: &Board) -> Vec<Move> {
    let mut result = Vec::new();
    board.generate_moves(|moves| {
        result.extend(moves);
        false
    });
    result
}

fn transform_move(move_: Move) -> Move {
    match move_ {
        Move::Drop { piece, to } => Move::Drop {
            piece,
            to: to.flip(),
        },
        Move::BoardMove {
            from,
            to,
            promotion,
        } => Move::BoardMove {
            from: from.flip(),
            to: to.flip(),
            promotion,
        },
    }
}

fn canonical_board(board: &Board) -> Result<(Board, bool)> {
    let original = board.to_string();
    let swapped = color_swap_anhoku_sfen(&original)?;
    let use_swapped = swapped < original;
    let canonical = Board::from_sfen(if use_swapped { &swapped } else { &original })
        .map_err(anyhow::Error::msg)?;
    Ok((canonical, use_swapped))
}

fn parsed_best_move(board: &Board, value: Option<&str>) -> Result<Move> {
    let text = value.context("trajectory search returned no root move")?;
    let move_: Move = text
        .parse()
        .map_err(|err| anyhow!("invalid searched move {text}: {err}"))?;
    ensure!(board.is_legal(move_), "searched move {text} is illegal");
    Ok(move_)
}

fn choose_move(
    board: &Board,
    policy: &str,
    game_index: u32,
    ply: u16,
    contract: &Contract,
    workspace: &mut SearchWorkspace,
) -> Result<(Move, f64, String)> {
    let (canonical, swapped) = canonical_board(board)?;
    let moves = legal_moves(&canonical);
    ensure!(!moves.is_empty());
    let stream = stable_u64(
        contract.generation.seed,
        &[
            &game_index.to_le_bytes(),
            &ply.to_le_bytes(),
            policy.as_bytes(),
        ],
    );
    let fixed_budget_perturbation = policy.starts_with("handcrafted-selfplay-")
        && stream % contract.generation.selfplay_perturbation_period == 0;
    let (selected, probability, budget) = if fixed_budget_perturbation {
        let index = (stream.rotate_left(23) as usize) % moves.len();
        (
            moves[index],
            1.0 / moves.len() as f64,
            format!(
                "uniform-legal-perturbation-1-in-{}-within-{policy}",
                contract.generation.selfplay_perturbation_period
            ),
        )
    } else {
        match policy {
            "uniform-legal-perturbation-v1" => {
                let index = (stream as usize) % moves.len();
                (
                    moves[index],
                    1.0 / moves.len() as f64,
                    "uniform-legal".to_string(),
                )
            }
            "handcrafted-selfplay-256-nodes-v1" | "handcrafted-selfplay-1024-nodes-v1" => {
                let nodes = if policy.contains("1024") { 1_024 } else { 256 };
                let summary = search_board_impl_handcrafted_with_node_budget_in_workspace(
                    &canonical,
                    nodes,
                    contract.labeling.maximum_depth,
                    workspace,
                )
                .map_err(anyhow::Error::msg)?;
                (
                    parsed_best_move(&canonical, summary.best_move.as_deref())?,
                    1.0,
                    format!("{nodes}-nodes"),
                )
            }
            "searched-near-best-uniform-candidates-v1" => {
                let mut ranked = moves
                    .iter()
                    .copied()
                    .map(|mv| {
                        let key = stable_u64(
                            contract.generation.seed ^ stream,
                            &[mv.to_string().as_bytes()],
                        );
                        (key, mv)
                    })
                    .collect::<Vec<_>>();
                ranked.sort_by_key(|item| item.0);
                let candidate_count = ranked.len().min(contract.generation.candidate_limit);
                let mut scored = Vec::with_capacity(candidate_count);
                for (_, mv) in ranked.into_iter().take(candidate_count) {
                    let mut child = canonical.clone();
                    child.play_unchecked(mv);
                    let summary = search_board_impl_handcrafted_with_node_budget_in_workspace(
                        &child,
                        contract.generation.near_best_child_nodes,
                        contract.labeling.maximum_depth,
                        workspace,
                    )
                    .map_err(anyhow::Error::msg)?;
                    let score = summary
                        .best_score
                        .map_or_else(|| -handcrafted_static_eval(&child), |value| -value);
                    scored.push((score, mv));
                }
                scored.sort_by(|a, b| {
                    b.0.cmp(&a.0)
                        .then_with(|| a.1.to_string().cmp(&b.1.to_string()))
                });
                let cutoff = scored[0]
                    .0
                    .saturating_sub(contract.generation.near_best_score_margin);
                let near = scored.iter().take_while(|item| item.0 >= cutoff).count();
                let selected = scored[(stream.rotate_left(17) as usize) % near].1;
                (
                    selected,
                    candidate_count as f64 / moves.len() as f64,
                    format!(
                        "uniform-candidate-k={candidate_count}/n={};child-nodes={}",
                        moves.len(),
                        contract.generation.near_best_child_nodes
                    ),
                )
            }
            other => bail!("unsupported R4 trajectory policy {other}"),
        }
    };
    Ok((
        if swapped {
            transform_move(selected)
        } else {
            selected
        },
        probability,
        budget,
    ))
}

fn material_value(piece: Piece) -> i32 {
    match piece {
        Piece::Pawn => 100,
        Piece::Lance => 300,
        Piece::Knight => 300,
        Piece::Silver => 400,
        Piece::Gold => 500,
        Piece::Bishop => 700,
        Piece::Rook => 800,
        Piece::King => 0,
        Piece::Tokin | Piece::PLance | Piece::PKnight | Piece::PSilver => 550,
        Piece::PBishop => 900,
        Piece::PRook => 1000,
    }
}

fn material_for(board: &Board, color: Color) -> i32 {
    Piece::ALL
        .iter()
        .copied()
        .map(|piece| {
            material_value(piece)
                * (board.colored_pieces(color, piece).len() as i32
                    + i32::from(board.num_in_hand(color, piece)))
        })
        .sum()
}

fn material_bucket(board: &Board) -> String {
    match (material_for(board, Color::Black) - material_for(board, Color::White)).abs() {
        0..=199 => "balanced",
        200..=799 => "moderate",
        _ => "large",
    }
    .to_string()
}

fn phase_for(ply: u16, legal_count: usize) -> String {
    if legal_count <= 3 || ply >= 150 {
        "terminal-adjacent"
    } else if ply < 36 {
        "early"
    } else if ply < 90 {
        "middle"
    } else {
        "late"
    }
    .to_string()
}

fn board_hashes(board: &Board) -> Result<(String, String)> {
    let packed = pack_board_for_training(board)?;
    let board_hash = hash_bytes(&packed);
    let swapped_sfen = color_swap_anhoku_sfen(&board.to_string())?;
    let swapped = Board::from_sfen(&swapped_sfen).map_err(anyhow::Error::msg)?;
    let swapped_hash = hash_bytes(&pack_board_for_training(&swapped)?);
    let canonical = board_hash.min(swapped_hash);
    Ok((hash_bytes(&packed), canonical))
}

#[allow(clippy::too_many_arguments)]
fn position_record(
    board: &Board,
    opening: &Opening,
    game_index: u32,
    policy: &str,
    budget: &str,
    probability: f64,
    result_status: String,
    contract: &Contract,
) -> Result<PositionRecord> {
    let moves = legal_moves(board);
    let ply = board.move_number();
    let (packed_board_sha256, color_swap_canonical_sha256) = board_hashes(board)?;
    let black = r1_donor_single_active_feature_indices(board, Color::Black);
    let white = r1_donor_single_active_feature_indices(board, Color::White);
    let source_game = format!("r4-game-{game_index:08}");
    let id = hash_bytes(format!("{}\0{}\0{}", source_game, ply, packed_board_sha256).as_bytes());
    let tactical =
        !board.checkers().is_empty() || moves.iter().any(|mv| board.piece_on(mv.to()).is_some());
    let donor_count = black.donor.len() + white.donor.len();
    let piece_count = usize::try_from(board.occupied().len()).unwrap_or(1).max(1);
    Ok(PositionRecord {
        schema: POSITION_SCHEMA.to_string(),
        id,
        sfen: board.to_string(),
        packed_board_sha256,
        color_swap_canonical_sha256,
        source: policy.to_string(),
        source_game,
        opening_group: opening.id.clone(),
        opening_sfen_sha256: hash_bytes(opening.sfen.as_bytes()),
        split: opening.split.clone(),
        trajectory_policy: policy.to_string(),
        trajectory_evaluator: contract.generation.trajectory_evaluator.clone(),
        trajectory_budget: budget.to_string(),
        candidate_inclusion_probability: probability,
        ply,
        phase: phase_for(ply, moves.len()),
        side_to_move: match board.side_to_move() {
            Color::Black => "black",
            Color::White => "white",
        }
        .to_string(),
        material_bucket: material_bucket(board),
        in_check: !board.checkers().is_empty(),
        tactical,
        has_promotion: moves.iter().any(Move::is_promotion),
        has_drop: moves.iter().any(Move::is_drop),
        result_status,
        donor_state: if donor_count == 0 { "none" } else { "active" }.to_string(),
        active_base_rows_black: black.base.len(),
        active_donor_rows_black: black.donor.len(),
        active_base_rows_white: white.base.len(),
        active_donor_rows_white: white.donor.len(),
        output_bucket: (((piece_count - 1) * OUTPUT_BUCKETS) / MAX_PIECES).min(OUTPUT_BUCKETS - 1),
    })
}

fn game_result_status(board: &Board, capped: bool) -> String {
    if capped {
        return "unknown/capped".to_string();
    }
    if !board.has(Color::Black, Piece::King) {
        return "terminal/white-win".to_string();
    }
    if !board.has(Color::White, Piece::King) {
        return "terminal/black-win".to_string();
    }
    match board.status() {
        GameStatus::Won => match board.side_to_move() {
            Color::Black => "terminal/white-win",
            Color::White => "terminal/black-win",
        }
        .to_string(),
        GameStatus::Drawn => "terminal/draw".to_string(),
        GameStatus::Ongoing => "unknown/unfinished".to_string(),
    }
}

fn generate_game(
    game_index: u32,
    openings: &[Opening],
    contract: &Contract,
    workspace: &mut SearchWorkspace,
) -> Result<Vec<PositionRecord>> {
    let available = openings
        .iter()
        .filter(|item| item.split != "sealed")
        .collect::<Vec<_>>();
    let opening = available[(game_index as usize) % available.len()];
    let orientation = stable_u64(
        contract.generation.seed,
        &[&game_index.to_le_bytes(), opening.id.as_bytes()],
    ) & 1;
    let start_sfen = if orientation == 0 {
        opening.sfen.clone()
    } else {
        color_swap_anhoku_sfen(&opening.sfen)?
    };
    let mut board = Board::from_sfen(&start_sfen).map_err(anyhow::Error::msg)?;
    let policy = &contract.generation.policy_cycle
        [(game_index as usize) % contract.generation.policy_cycle.len()];
    let mut records = Vec::new();
    let mut last_probability = 1.0;
    let mut last_budget = "opening-root".to_string();
    while board.move_number() < contract.generation.maximum_ply {
        if !board.has(Color::Black, Piece::King)
            || !board.has(Color::White, Piece::King)
            || board.status() != GameStatus::Ongoing
        {
            break;
        }
        let moves = legal_moves(&board);
        if moves.is_empty() {
            break;
        }
        records.push(position_record(
            &board,
            opening,
            game_index,
            policy,
            &last_budget,
            last_probability,
            "pending".to_string(),
            contract,
        )?);
        let (move_, probability, budget) = choose_move(
            &board,
            policy,
            game_index,
            board.move_number(),
            contract,
            workspace,
        )?;
        ensure!(board.is_legal(move_), "R4 trajectory selected illegal move");
        board.play_unchecked(move_);
        last_probability = probability;
        last_budget = budget;
    }
    let result = game_result_status(
        &board,
        board.move_number() >= contract.generation.maximum_ply,
    );
    for record in &mut records {
        record.result_status.clone_from(&result);
    }
    Ok(records)
}

pub(crate) fn generate_positions(args: GenerateArgs<'_>) -> Result<PathBuf> {
    let (contract, contract_hash) = load_contract(args.contract_path, args.workspace_root)?;
    ensure!(
        args.shard_index < contract.generation.shard_count,
        "shard index out of range"
    );
    let openings = load_openings(&contract, args.workspace_root)?;
    let positions_dir = args.output_dir.join("positions");
    fs::create_dir_all(&positions_dir)?;
    let data_path = positions_dir.join(format!("position-shard-{:05}.jsonl", args.shard_index));
    let manifest_path = positions_dir.join(format!("position-shard-{:05}.json", args.shard_index));
    ensure!(
        !data_path.exists() && !manifest_path.exists(),
        "R4 position shard already exists; immutable outputs are never overwritten"
    );
    let started = Instant::now();
    let file = File::create(&data_path)?;
    let mut writer = BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut positions = 0u64;
    let mut games = 0u32;
    let mut workspace = SearchWorkspace::new();
    for game_index in (args.shard_index..contract.generation.game_count)
        .step_by(contract.generation.shard_count as usize)
    {
        for record in generate_game(game_index, &openings, &contract, &mut workspace)? {
            let mut line = serde_json::to_vec(&record)?;
            line.push(b'\n');
            writer.write_all(&line)?;
            hasher.update(&line);
            positions += 1;
        }
        games += 1;
    }
    writer.flush()?;
    let manifest = PositionShardManifest {
        schema: POSITION_MANIFEST_SCHEMA.to_string(),
        contract_sha256: contract_hash,
        shard_index: args.shard_index,
        shard_count: contract.generation.shard_count,
        games,
        positions,
        positions_sha256: format!("{:x}", hasher.finalize()),
        generation_schema: contract.generation.schema,
        engine_revision: engine_revision(args.workspace_root),
        executable_sha256: sha256_file(args.executable_path)?,
        hostname: host_name(),
        architecture: std::env::consts::ARCH.to_string(),
        elapsed_seconds: started.elapsed().as_secs_f64(),
        generated_at_unix_ms: unix_ms(),
    };
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    Ok(manifest_path)
}

fn position_shard_files(inputs: &[PathBuf]) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut manifests = Vec::new();
    for input in inputs {
        if input.is_dir() {
            for entry in fs::read_dir(input)? {
                let path = entry?.path();
                if path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| {
                        name.starts_with("position-shard-")
                            && name.ends_with(".json")
                            && !name.ends_with(".jsonl")
                    })
                {
                    manifests.push(path);
                }
            }
        } else if input.extension().and_then(|value| value.to_str()) == Some("json") {
            manifests.push(input.clone());
        }
    }
    manifests.sort();
    let mut result = Vec::new();
    for manifest in manifests {
        let mut data = manifest.clone();
        data.set_extension("jsonl");
        ensure!(data.is_file(), "missing position shard {}", data.display());
        result.push((manifest, data));
    }
    Ok(result)
}

fn quota_for(split_count: usize, fraction: f64) -> usize {
    (split_count as f64 * fraction).round() as usize
}

fn write_position_input_shards(
    output_dir: &Path,
    tier: &str,
    shard_count: u32,
    records: &[PositionRecord],
    contract_hash: &str,
) -> Result<()> {
    let directory = output_dir.join("label-inputs").join(tier);
    fs::create_dir_all(&directory)?;
    let mut writers = Vec::new();
    let mut hashers = Vec::new();
    let mut counts = vec![0u64; shard_count as usize];
    for index in 0..shard_count {
        writers.push(BufWriter::new(File::create(
            directory.join(format!("label-input-{index:05}.jsonl")),
        )?));
        hashers.push(Sha256::new());
    }
    for (ordinal, record) in records.iter().enumerate() {
        let shard = ordinal % shard_count as usize;
        let envelope = serde_json::json!({
            "tier": tier,
            "ordinal": ordinal as u64,
            "position": record,
        });
        let mut line = serde_json::to_vec(&envelope)?;
        line.push(b'\n');
        writers[shard].write_all(&line)?;
        hashers[shard].update(&line);
        counts[shard] += 1;
    }
    for (index, writer) in writers.iter_mut().enumerate() {
        writer.flush()?;
        let manifest = LabelInputManifest {
            schema: LABEL_INPUT_MANIFEST_SCHEMA.to_string(),
            contract_sha256: contract_hash.to_string(),
            tier: tier.to_string(),
            shard_index: index as u32,
            shard_count,
            positions: counts[index],
            positions_sha256: format!("{:x}", hashers[index].clone().finalize()),
        };
        fs::write(
            directory.join(format!("label-input-{index:05}.json")),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
    }
    Ok(())
}

fn write_training_entry(
    writer: &mut impl Write,
    record: &PositionRecord,
    score: i16,
) -> Result<()> {
    let board = Board::from_sfen(&record.sfen).map_err(anyhow::Error::msg)?;
    writer.write_all(&pack_board_for_training(&board)?)?;
    writer.write_all(&score.to_le_bytes())?;
    writer.write_all(&0u16.to_le_bytes())?;
    writer.write_all(&record.ply.to_le_bytes())?;
    // Result loss is frozen off. Zero remains a payload placeholder; the
    // authoritative result status lives in the v2 sidecar.
    writer.write_all(&[0, 0])?;
    Ok(())
}

fn static_label(record: PositionRecord, ordinal: u64) -> Result<LabelRecord> {
    let board = Board::from_sfen(&record.sfen).map_err(anyhow::Error::msg)?;
    let score = handcrafted_static_eval(&board).clamp(i16::MIN as i32, i16::MAX as i32);
    let label_identity = serde_json::json!({
        "position": record.packed_board_sha256,
        "evaluator": "haitaka-native-handcrafted-static-v1",
        "score": score,
    });
    Ok(LabelRecord {
        schema: LABEL_RECORD_SCHEMA.to_string(),
        tier: "static-clone".to_string(),
        ordinal,
        position: record,
        label_evaluator: "haitaka-native-handcrafted-static-v1".to_string(),
        label_budget: 0,
        maximum_depth: 0,
        node_counting_version: "not-applicable".to_string(),
        completed: true,
        completed_depth: 0,
        partial_iteration: false,
        node_accounting_exact: true,
        alpha_beta_nodes: 0,
        qsearch_nodes: 0,
        total_nodes: 0,
        root_move: None,
        pv: None,
        raw_score: score,
        bounded_score: score as i16,
        target_transform_version: "exact-static-identity-v1".to_string(),
        mate_class: "none".to_string(),
        mate_distance: None,
        elapsed_ms: 0.0,
        label_sha256: hash_bytes(&serde_json::to_vec(&label_identity)?),
    })
}

fn write_static_clone(output_dir: &Path, selected: &[PositionRecord], count: usize) -> Result<u64> {
    ensure!(
        selected.len() >= count,
        "not enough positions for static-clone tier"
    );
    let datasets = output_dir.join("datasets");
    fs::create_dir_all(&datasets)?;
    let mut ordered = selected
        .iter()
        .filter(|item| item.split == "training")
        .cloned()
        .collect::<Vec<_>>();
    ordered.sort_by(|a, b| a.id.cmp(&b.id));
    ensure!(ordered.len() >= count);
    let mut binary = BufWriter::new(File::create(datasets.join("static-clone.bin"))?);
    let mut sidecar = BufWriter::new(File::create(datasets.join("static-clone.jsonl"))?);
    for (ordinal, record) in ordered.into_iter().take(count).enumerate() {
        let labeled = static_label(record, ordinal as u64)?;
        write_training_entry(&mut binary, &labeled.position, labeled.bounded_score)?;
        serde_json::to_writer(&mut sidecar, &labeled)?;
        sidecar.write_all(b"\n")?;
    }
    binary.flush()?;
    sidecar.flush()?;
    Ok(count as u64)
}

pub(crate) fn assemble_positions(args: AssembleArgs<'_>) -> Result<PathBuf> {
    let (contract, contract_hash) = load_contract(args.contract_path, args.workspace_root)?;
    ensure!(
        !args.output_dir.join("assembly-report.json").exists(),
        "R4 assembly already exists; immutable outputs are never overwritten"
    );
    let files = position_shard_files(args.inputs)?;
    ensure!(
        files.len() == contract.generation.shard_count as usize,
        "expected {} position shards, found {}",
        contract.generation.shard_count,
        files.len()
    );
    let mut seen_shards = BTreeSet::new();
    let mut canonical = HashMap::<String, PositionRecord>::new();
    let mut seen_exact = HashSet::<String>::new();
    let mut candidate_records = 0u64;
    let mut exact_duplicates = 0u64;
    let mut color_duplicates = 0u64;
    for (manifest_path, data_path) in files {
        let manifest: PositionShardManifest = read_json(&manifest_path)?;
        ensure!(manifest.schema == POSITION_MANIFEST_SCHEMA);
        ensure!(manifest.contract_sha256 == contract_hash);
        ensure!(manifest.shard_count == contract.generation.shard_count);
        ensure!(
            seen_shards.insert(manifest.shard_index),
            "duplicate position shard {}",
            manifest.shard_index
        );
        ensure!(
            sha256_file(&data_path)? == manifest.positions_sha256,
            "position shard hash mismatch"
        );
        let reader = BufReader::new(File::open(&data_path)?);
        let mut local_count = 0u64;
        for line in reader.lines() {
            let record: PositionRecord = serde_json::from_str(&line?)?;
            ensure!(record.schema == POSITION_SCHEMA);
            ensure!(
                record.split != "sealed",
                "sealed opening leaked into R4 data"
            );
            candidate_records += 1;
            local_count += 1;
            let exact_new = seen_exact.insert(record.packed_board_sha256.clone());
            match canonical.entry(record.color_swap_canonical_sha256.clone()) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(record);
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    if exact_new {
                        color_duplicates += 1;
                    } else {
                        exact_duplicates += 1;
                    }
                    if record.id < entry.get().id {
                        entry.insert(record);
                    }
                }
            }
        }
        ensure!(
            local_count == manifest.positions,
            "position count mismatch for shard {}",
            manifest.shard_index
        );
    }
    let unique_families = canonical.len() as u64;
    let mut buckets = HashMap::<(String, String), Vec<PositionRecord>>::new();
    for record in canonical.into_values() {
        buckets
            .entry((record.split.clone(), record.phase.clone()))
            .or_default()
            .push(record);
    }
    let mut selected = Vec::with_capacity(contract.cost.maximum_pilot_labels as usize);
    let mut split_counts = BTreeMap::new();
    let mut phase_counts = BTreeMap::new();
    let mut source_counts = BTreeMap::new();
    for (split, &split_count) in &contract.assembly.pilot_split_counts {
        let mut selected_for_split = 0usize;
        for (phase, &fraction) in &contract.assembly.phase_fractions {
            let quota = quota_for(split_count, fraction);
            let bucket = buckets
                .get_mut(&(split.clone(), phase.clone()))
                .with_context(|| format!("missing bucket split={split} phase={phase}"))?;
            bucket.sort_by_key(|record| {
                (
                    stable_u64(contract.assembly.selection_seed, &[record.id.as_bytes()]),
                    record.id.clone(),
                )
            });
            ensure!(
                bucket.len() >= quota,
                "insufficient R4 candidates for split={split} phase={phase}: need {quota}, have {}",
                bucket.len()
            );
            for record in bucket.drain(..quota) {
                *phase_counts.entry(phase.clone()).or_insert(0u64) += 1;
                *source_counts.entry(record.source.clone()).or_insert(0u64) += 1;
                selected.push(record);
            }
            selected_for_split += quota;
        }
        ensure!(
            selected_for_split == split_count,
            "phase fractions do not sum exactly for {split}"
        );
        split_counts.insert(split.clone(), selected_for_split as u64);
    }
    ensure!(selected.len() as u64 == contract.cost.maximum_pilot_labels);
    selected.sort_by_key(|record| {
        (
            record.split.clone(),
            stable_u64(
                contract.assembly.selection_seed ^ 0x5eed,
                &[record.id.as_bytes()],
            ),
        )
    });

    let mut opening_splits = HashMap::<String, String>::new();
    let mut opening_overlap = BTreeSet::new();
    let mut canonical_splits = HashMap::<String, String>::new();
    let mut cross_split = 0u64;
    for record in &selected {
        if let Some(previous) =
            opening_splits.insert(record.opening_group.clone(), record.split.clone())
            && previous != record.split
        {
            opening_overlap.insert(record.opening_group.clone());
        }
        if let Some(previous) = canonical_splits.insert(
            record.color_swap_canonical_sha256.clone(),
            record.split.clone(),
        ) && previous != record.split
        {
            cross_split += 1;
        }
    }
    ensure!(opening_overlap.is_empty() && cross_split == 0);
    fs::create_dir_all(args.output_dir)?;
    write_position_input_shards(
        args.output_dir,
        "pilot",
        contract.assembly.label_shard_count,
        &selected,
        &contract_hash,
    )?;

    let mut deep = selected
        .iter()
        .filter(|item| item.split == "development")
        .cloned()
        .collect::<Vec<_>>();
    deep.sort_by_key(|record| {
        stable_u64(
            contract.assembly.selection_seed ^ 0xdee9,
            &[record.id.as_bytes()],
        )
    });
    deep.truncate(contract.assembly.deep_core_positions);
    ensure!(deep.len() == contract.assembly.deep_core_positions);
    write_position_input_shards(
        args.output_dir,
        "deep-core",
        contract.assembly.deep_core_shard_count,
        &deep,
        &contract_hash,
    )?;
    let static_count = write_static_clone(
        args.output_dir,
        &selected,
        contract.assembly.static_clone_positions,
    )?;

    let report = AssemblyReport {
        schema: "haitaka-r4-assembly-report-v1".to_string(),
        contract_sha256: contract_hash,
        candidate_records,
        exact_duplicate_records: exact_duplicates,
        color_swap_duplicate_records: color_duplicates,
        unique_color_swap_families: unique_families,
        selected_pilot_positions: selected.len() as u64,
        split_counts,
        phase_counts,
        source_counts,
        label_shards: contract.assembly.label_shard_count,
        deep_core_positions: deep.len() as u64,
        deep_core_shards: contract.assembly.deep_core_shard_count,
        static_clone_positions: static_count,
        opening_group_overlap: opening_overlap.into_iter().collect(),
        cross_split_color_swap_overlap: cross_split,
        effective_sample_size: selected.len() as f64,
        maximum_duplicate_weight: contract.assembly.maximum_duplicate_weight,
    };
    let path = args.output_dir.join("assembly-report.json");
    fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
    Ok(path)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LabelInputEnvelope {
    tier: String,
    ordinal: u64,
    position: PositionRecord,
}

fn label_position(
    envelope: LabelInputEnvelope,
    contract: &Contract,
    workspace: &mut SearchWorkspace,
) -> Result<LabelRecord> {
    let board = Board::from_sfen(&envelope.position.sfen).map_err(anyhow::Error::msg)?;
    let budget = if envelope.tier == "deep-core" {
        contract.labeling.deep_reference_node_budget
    } else {
        ensure!(
            envelope.tier == "pilot",
            "unsupported label tier {}",
            envelope.tier
        );
        contract.labeling.node_budget
    };
    let summary = search_board_impl_handcrafted_with_node_budget_in_workspace(
        &board,
        budget,
        contract.labeling.maximum_depth,
        workspace,
    )
    .map_err(anyhow::Error::msg)?;
    let completed = summary.best_score.is_some() && summary.completed_depth > 0;
    let node_accounting_exact = summary.alpha_beta_nodes.checked_add(summary.qsearch_nodes)
        == Some(summary.total_nodes)
        && summary.total_nodes <= summary.node_limit;
    let raw_score = summary
        .best_score
        .context("R4 label search returned no backed-up root score")?;
    let (mate_class, mate_distance, bounded_score) =
        if raw_score.abs() >= contract.labeling.mate_threshold {
            let class = if raw_score > 0 { "win" } else { "loss" };
            let distance = (contract.labeling.mate_base - raw_score.abs()).max(0);
            (
                class.to_string(),
                Some(distance),
                raw_score.signum() * contract.labeling.nonmate_clip,
            )
        } else {
            (
                "none".to_string(),
                None,
                raw_score.clamp(
                    -contract.labeling.nonmate_clip,
                    contract.labeling.nonmate_clip,
                ),
            )
        };
    let identity = serde_json::json!({
        "packedBoardSha256": &envelope.position.packed_board_sha256,
        "labelEvaluator": &contract.labeling.label_evaluator,
        "labelBudget": budget,
        "maximumDepth": contract.labeling.maximum_depth,
        "nodeCountingVersion": &contract.labeling.node_counting_version,
        "rawScore": raw_score,
        "boundedScore": bounded_score,
        "mateClass": &mate_class,
        "mateDistance": mate_distance,
        "rootMove": &summary.best_move,
        "totalNodes": summary.total_nodes,
    });
    Ok(LabelRecord {
        schema: LABEL_RECORD_SCHEMA.to_string(),
        tier: envelope.tier,
        ordinal: envelope.ordinal,
        position: envelope.position,
        label_evaluator: contract.labeling.label_evaluator.clone(),
        label_budget: budget,
        maximum_depth: contract.labeling.maximum_depth,
        node_counting_version: contract.labeling.node_counting_version.clone(),
        completed,
        completed_depth: summary.completed_depth,
        partial_iteration: summary.incomplete_iterations > 0,
        node_accounting_exact,
        alpha_beta_nodes: summary.alpha_beta_nodes,
        qsearch_nodes: summary.qsearch_nodes,
        total_nodes: summary.total_nodes,
        root_move: summary.best_move,
        pv: None,
        raw_score,
        bounded_score: bounded_score as i16,
        target_transform_version: contract.labeling.score_transform.clone(),
        mate_class,
        mate_distance,
        elapsed_ms: summary.elapsed_ms,
        label_sha256: hash_bytes(&serde_json::to_vec(&identity)?),
    })
}

pub(crate) fn label_shard(args: LabelArgs<'_>) -> Result<PathBuf> {
    ensure!(args.jobs > 0, "R4 labeling requires at least one job");
    let (contract, contract_hash) = load_contract(args.contract_path, args.workspace_root)?;
    let mut manifest_path = args.input.to_path_buf();
    manifest_path.set_extension("json");
    let input_manifest: LabelInputManifest = read_json(&manifest_path)?;
    ensure!(input_manifest.schema == LABEL_INPUT_MANIFEST_SCHEMA);
    ensure!(input_manifest.contract_sha256 == contract_hash);
    ensure!(
        sha256_file(args.input)? == input_manifest.positions_sha256,
        "label input hash mismatch"
    );
    let reader = BufReader::new(File::open(args.input)?);
    let mut records = Vec::with_capacity(input_manifest.positions as usize);
    for line in reader.lines() {
        records.push(serde_json::from_str::<LabelInputEnvelope>(&line?)?);
    }
    ensure!(records.len() as u64 == input_manifest.positions);
    ensure!(records.iter().all(|item| item.tier == input_manifest.tier));

    let labels_dir = args.output_dir.join("labels").join(&input_manifest.tier);
    fs::create_dir_all(&labels_dir)?;
    let output_path = labels_dir.join(format!(
        "labeled-shard-{:05}.jsonl",
        input_manifest.shard_index
    ));
    let output_manifest_path = labels_dir.join(format!(
        "labeled-shard-{:05}.json",
        input_manifest.shard_index
    ));
    ensure!(
        !output_path.exists() && !output_manifest_path.exists(),
        "R4 labeled shard already exists; immutable outputs are never overwritten"
    );
    let started_at = unix_ms();
    let started = Instant::now();
    let records = Arc::new(Mutex::new(
        records.into_iter().map(Some).collect::<Vec<_>>(),
    ));
    let next = Arc::new(AtomicUsize::new(0));
    let results = Arc::new(Mutex::new(
        (0..input_manifest.positions)
            .map(|_| None)
            .collect::<Vec<Option<LabelRecord>>>(),
    ));
    let error = Arc::new(Mutex::new(None::<anyhow::Error>));
    thread::scope(|scope| {
        for _ in 0..args.jobs {
            let records = Arc::clone(&records);
            let next = Arc::clone(&next);
            let results = Arc::clone(&results);
            let error = Arc::clone(&error);
            let contract = &contract;
            scope.spawn(move || {
                let mut workspace = SearchWorkspace::new();
                loop {
                    if error.lock().expect("error mutex").is_some() {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= input_manifest.positions as usize {
                        break;
                    }
                    let envelope = records.lock().expect("record mutex")[index]
                        .take()
                        .expect("unique label work item");
                    match label_position(envelope, contract, &mut workspace) {
                        Ok(label) => results.lock().expect("result mutex")[index] = Some(label),
                        Err(err) => {
                            *error.lock().expect("error mutex") = Some(err);
                            break;
                        }
                    }
                }
            });
        }
    });
    if let Some(err) = error.lock().expect("error mutex").take() {
        return Err(err);
    }
    let mut labels = results
        .lock()
        .expect("result mutex")
        .drain(..)
        .map(|item| item.expect("all labels produced"))
        .collect::<Vec<_>>();
    labels.sort_by_key(|item| item.ordinal);
    let mut writer = BufWriter::new(File::create(&output_path)?);
    let mut hasher = Sha256::new();
    let mut incomplete = 0u64;
    let mut accounting_failures = 0u64;
    let mut mates = 0u64;
    let mut search_seconds = 0.0;
    for label in &labels {
        incomplete += u64::from(!label.completed);
        accounting_failures += u64::from(!label.node_accounting_exact);
        mates += u64::from(label.mate_class != "none");
        search_seconds += label.elapsed_ms / 1_000.0;
        let mut line = serde_json::to_vec(label)?;
        line.push(b'\n');
        writer.write_all(&line)?;
        hasher.update(&line);
    }
    writer.flush()?;
    let finished_at = unix_ms();
    let manifest = LabelShardManifest {
        schema: LABEL_MANIFEST_SCHEMA.to_string(),
        contract_sha256: contract_hash,
        tier: input_manifest.tier,
        shard_index: input_manifest.shard_index,
        shard_count: input_manifest.shard_count,
        positions: labels.len() as u64,
        labels_sha256: format!("{:x}", hasher.finalize()),
        label_evaluator: contract.labeling.label_evaluator,
        label_budget: labels.first().map_or(0, |item| item.label_budget),
        node_counting_version: contract.labeling.node_counting_version,
        incomplete_labels: incomplete,
        node_accounting_failures: accounting_failures,
        mate_labels: mates,
        search_elapsed_seconds: search_seconds,
        wall_elapsed_seconds: started.elapsed().as_secs_f64(),
        labels_per_search_second: if search_seconds == 0.0 {
            0.0
        } else {
            labels.len() as f64 / search_seconds
        },
        jobs: args.jobs,
        engine_revision: engine_revision(args.workspace_root),
        executable_sha256: sha256_file(args.executable_path)?,
        hostname: host_name(),
        architecture: std::env::consts::ARCH.to_string(),
        started_at_unix_ms: started_at,
        finished_at_unix_ms: finished_at,
    };
    fs::write(&output_manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    Ok(output_manifest_path)
}

fn label_shard_files(inputs: &[PathBuf], tier: &str) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut search_dirs = Vec::new();
    for input in inputs {
        let nested = input.join("labels").join(tier);
        if nested.is_dir() {
            search_dirs.push(nested);
        }
        let direct = input.join(tier);
        if direct.is_dir() {
            search_dirs.push(direct);
        }
        if input.is_dir() {
            search_dirs.push(input.clone());
        }
    }
    let mut manifests = BTreeSet::new();
    for directory in search_dirs {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path
                .file_name()
                .and_then(|v| v.to_str())
                .is_some_and(|name| {
                    name.starts_with("labeled-shard-")
                        && name.ends_with(".json")
                        && !name.ends_with(".jsonl")
                })
            {
                let manifest: LabelShardManifest = read_json(&path)?;
                if manifest.tier == tier {
                    manifests.insert(path);
                }
            }
        }
    }
    let mut result = Vec::new();
    for manifest in manifests {
        let mut data = manifest.clone();
        data.set_extension("jsonl");
        ensure!(data.is_file(), "missing labeled shard {}", data.display());
        result.push((manifest, data));
    }
    Ok(result)
}

fn read_labeled_tier(
    inputs: &[PathBuf],
    tier: &str,
    expected_shards: u32,
    contract_hash: &str,
) -> Result<(Vec<LabelRecord>, Vec<LabelShardManifest>)> {
    let files = label_shard_files(inputs, tier)?;
    ensure!(
        files.len() == expected_shards as usize,
        "expected {expected_shards} {tier} label shards, found {}",
        files.len()
    );
    let mut seen = BTreeSet::new();
    let mut labels = Vec::new();
    let mut manifests = Vec::new();
    for (manifest_path, data_path) in files {
        let manifest: LabelShardManifest = read_json(&manifest_path)?;
        ensure!(
            manifest.schema == LABEL_MANIFEST_SCHEMA && manifest.contract_sha256 == contract_hash
        );
        ensure!(manifest.tier == tier && manifest.shard_count == expected_shards);
        ensure!(
            seen.insert(manifest.shard_index),
            "duplicate {tier} shard {}",
            manifest.shard_index
        );
        ensure!(
            sha256_file(&data_path)? == manifest.labels_sha256,
            "labeled shard hash mismatch"
        );
        let mut local = 0u64;
        for line in BufReader::new(File::open(&data_path)?).lines() {
            let label: LabelRecord = serde_json::from_str(&line?)?;
            ensure!(label.schema == LABEL_RECORD_SCHEMA && label.tier == tier);
            labels.push(label);
            local += 1;
        }
        ensure!(local == manifest.positions);
        manifests.push(manifest);
    }
    labels.sort_by_key(|item| item.ordinal);
    for (expected, label) in labels.iter().enumerate() {
        ensure!(
            label.ordinal == expected as u64,
            "{tier} label ordinal gap at {expected}"
        );
    }
    Ok((labels, manifests))
}

fn increment(map: &mut BTreeMap<String, u64>, value: impl Into<String>) {
    *map.entry(value.into()).or_default() += 1;
}

fn base_row_structurally_reachable(index: usize) -> bool {
    const PIECE_INDICES: usize = 1_863;
    const NON_DROP: usize = 1_539;
    let king_square = index / PIECE_INDICES;
    let local = index % PIECE_INDICES;
    if local < NON_DROP {
        if local < 18 * 81 {
            local % 81 != king_square
        } else {
            true
        }
    } else {
        let hand = local - NON_DROP;
        let slot = (hand / 18) / 2;
        let copy = hand % 18;
        let maximum = match slot {
            0 | 1 => 2,
            2 | 5 | 6 | 7 => 4,
            4 => 18,
            _ => 0,
        };
        copy < maximum
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FeatureCoverageReport {
    schema: String,
    minimum_updates: u64,
    base_rows: usize,
    donor_rows: usize,
    known_structurally_unreachable_base_rows: Vec<usize>,
    never_observed_reachable_base_rows: Vec<usize>,
    never_observed_donor_rows: Vec<usize>,
    reachable_base_rows_below_minimum: Vec<usize>,
    donor_rows_below_minimum: Vec<usize>,
    base_update_count_sha256: String,
    donor_update_count_sha256: String,
}

fn feature_coverage(labels: &[LabelRecord], minimum: u64) -> Result<FeatureCoverageReport> {
    let mut base = vec![0u64; R1_HALFKAV2_BASE_FEATURES];
    let mut donor = vec![0u64; DONOR_SINGLE_REAL_FEATURES];
    for label in labels {
        let board = Board::from_sfen(&label.position.sfen).map_err(anyhow::Error::msg)?;
        for perspective in [Color::Black, Color::White] {
            let active = r1_donor_single_active_feature_indices(&board, perspective);
            for row in active.base {
                base[row] += 1;
            }
            for row in active.donor {
                donor[row] += 1;
            }
        }
    }
    let known_unreachable = (0..base.len())
        .filter(|&row| !base_row_structurally_reachable(row))
        .collect::<Vec<_>>();
    let never_base = (0..base.len())
        .filter(|&row| base_row_structurally_reachable(row) && base[row] == 0)
        .collect::<Vec<_>>();
    let never_donor = (0..donor.len())
        .filter(|&row| donor[row] == 0)
        .collect::<Vec<_>>();
    let below_base = (0..base.len())
        .filter(|&row| base_row_structurally_reachable(row) && base[row] > 0 && base[row] < minimum)
        .collect::<Vec<_>>();
    let below_donor = (0..donor.len())
        .filter(|&row| donor[row] > 0 && donor[row] < minimum)
        .collect::<Vec<_>>();
    Ok(FeatureCoverageReport {
        schema: "haitaka-r4-feature-coverage-v1".to_string(),
        minimum_updates: minimum,
        base_rows: base.len(),
        donor_rows: donor.len(),
        known_structurally_unreachable_base_rows: known_unreachable,
        never_observed_reachable_base_rows: never_base,
        never_observed_donor_rows: never_donor,
        reachable_base_rows_below_minimum: below_base,
        donor_rows_below_minimum: below_donor,
        base_update_count_sha256: hash_bytes(
            &base
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>(),
        ),
        donor_update_count_sha256: hash_bytes(
            &donor
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>(),
        ),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DistributionReport {
    source: BTreeMap<String, u64>,
    side: BTreeMap<String, u64>,
    ply_bucket: BTreeMap<String, u64>,
    phase: BTreeMap<String, u64>,
    score_bucket: BTreeMap<String, u64>,
    material: BTreeMap<String, u64>,
    check: BTreeMap<String, u64>,
    tactical: BTreeMap<String, u64>,
    result_status: BTreeMap<String, u64>,
    donor_state: BTreeMap<String, u64>,
    output_bucket: BTreeMap<String, u64>,
    active_row_count: BTreeMap<String, u64>,
}

fn distributions(labels: &[LabelRecord]) -> DistributionReport {
    let mut report = DistributionReport {
        source: BTreeMap::new(),
        side: BTreeMap::new(),
        ply_bucket: BTreeMap::new(),
        phase: BTreeMap::new(),
        score_bucket: BTreeMap::new(),
        material: BTreeMap::new(),
        check: BTreeMap::new(),
        tactical: BTreeMap::new(),
        result_status: BTreeMap::new(),
        donor_state: BTreeMap::new(),
        output_bucket: BTreeMap::new(),
        active_row_count: BTreeMap::new(),
    };
    for label in labels {
        let position = &label.position;
        increment(&mut report.source, position.source.clone());
        increment(&mut report.side, position.side_to_move.clone());
        increment(
            &mut report.ply_bucket,
            match position.ply {
                0..=35 => "000-035",
                36..=89 => "036-089",
                90..=149 => "090-149",
                _ => "150+",
            },
        );
        increment(&mut report.phase, position.phase.clone());
        increment(
            &mut report.score_bucket,
            match label.bounded_score {
                i16::MIN..=-4001 => "<-4000",
                -4000..=-401 => "-4000..-401",
                -400..=400 => "-400..400",
                401..=4000 => "401..4000",
                _ => ">4000",
            },
        );
        increment(&mut report.material, position.material_bucket.clone());
        increment(&mut report.check, position.in_check.to_string());
        increment(&mut report.tactical, position.tactical.to_string());
        increment(&mut report.result_status, position.result_status.clone());
        increment(&mut report.donor_state, position.donor_state.clone());
        increment(
            &mut report.output_bucket,
            position.output_bucket.to_string(),
        );
        let rows = position.active_base_rows_black
            + position.active_donor_rows_black
            + position.active_base_rows_white
            + position.active_donor_rows_white;
        increment(&mut report.active_row_count, (rows / 16 * 16).to_string());
    }
    report
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DatasetManifest {
    schema: String,
    dataset: String,
    ruleset: String,
    records: u64,
    entry_bytes: usize,
    binary_sha256: String,
    sidecar_sha256: String,
    contract_sha256: String,
    label_evaluator: String,
    label_budget: u64,
    target_semantics: String,
    score_transform: String,
    mate_policy: String,
    result_loss_enabled: bool,
    unknown_results_ignored: bool,
    duplicate_weight: u32,
    effective_sample_size: f64,
    opening_groups: Vec<String>,
}

fn write_dataset(
    output_dir: &Path,
    name: &str,
    labels: &[&LabelRecord],
    contract: &Contract,
    contract_hash: &str,
) -> Result<DatasetManifest> {
    let directory = output_dir.join("datasets");
    fs::create_dir_all(&directory)?;
    let binary_path = directory.join(format!("{name}.bin"));
    let sidecar_path = directory.join(format!("{name}.jsonl"));
    let mut binary = BufWriter::new(File::create(&binary_path)?);
    let mut sidecar = BufWriter::new(File::create(&sidecar_path)?);
    let mut groups = BTreeSet::new();
    for label in labels {
        write_training_entry(&mut binary, &label.position, label.bounded_score)?;
        serde_json::to_writer(&mut sidecar, label)?;
        sidecar.write_all(b"\n")?;
        groups.insert(label.position.opening_group.clone());
    }
    binary.flush()?;
    sidecar.flush()?;
    ensure!(fs::metadata(&binary_path)?.len() == labels.len() as u64 * ENTRY_BYTES as u64);
    let manifest = DatasetManifest {
        schema: DATASET_MANIFEST_SCHEMA.to_string(),
        dataset: name.to_string(),
        ruleset: "anhoku".to_string(),
        records: labels.len() as u64,
        entry_bytes: ENTRY_BYTES,
        binary_sha256: sha256_file(&binary_path)?,
        sidecar_sha256: sha256_file(&sidecar_path)?,
        contract_sha256: contract_hash.to_string(),
        label_evaluator: labels
            .first()
            .map_or(contract.labeling.label_evaluator.clone(), |label| {
                label.label_evaluator.clone()
            }),
        label_budget: labels.first().map_or(0, |label| label.label_budget),
        target_semantics: contract.labeling.target_semantics.clone(),
        score_transform: contract.labeling.score_transform.clone(),
        mate_policy: contract.labeling.mate_policy.clone(),
        result_loss_enabled: false,
        unknown_results_ignored: true,
        duplicate_weight: 1,
        effective_sample_size: labels.len() as f64,
        opening_groups: groups.into_iter().collect(),
    };
    fs::write(
        directory.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CostForecast {
    positions: u64,
    labels_per_worker_second: f64,
    projected_core_hours: f64,
    projected_cpu_cost_usd: f64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegenerationProof {
    schema: String,
    contract_sha256: String,
    shard_index: u32,
    expected_positions_sha256: String,
    regenerated_positions_sha256: String,
    expected_positions: u64,
    regenerated_positions: u64,
    passed: bool,
    hostname: String,
    architecture: String,
    executable_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GateReport {
    schema: String,
    phase: String,
    passed: bool,
    r5_authorized: bool,
    contract_sha256: String,
    assembly_report_sha256: String,
    pilot_positions: u64,
    deep_core_positions: u64,
    static_clone_positions: u64,
    datasets: BTreeMap<String, DatasetManifest>,
    distributions: DistributionReport,
    feature_coverage_sha256: String,
    feature_coverage_summary: serde_json::Value,
    label_hosts: BTreeMap<String, u64>,
    label_engine_revisions: Vec<String>,
    label_executable_hashes: Vec<String>,
    pilot_search_elapsed_seconds: f64,
    pilot_labels_per_worker_second: f64,
    distributed_wall_seconds: f64,
    actual_core_hours: f64,
    actual_cpu_cost_usd: f64,
    cost_forecast: Vec<CostForecast>,
    mate_labels: u64,
    incomplete_labels: u64,
    node_accounting_failures: u64,
    unknown_or_capped_results: u64,
    regeneration_proofs: Vec<RegenerationProof>,
    gates: BTreeMap<String, bool>,
    larger_generation_started: bool,
    training_started: bool,
    strength_games_played: bool,
}

pub(crate) fn finalize(args: FinalizeArgs<'_>) -> Result<PathBuf> {
    let (contract, contract_hash) = load_contract(args.contract_path, args.workspace_root)?;
    let report_path = args.output_dir.join("r4-gate-report.json");
    ensure!(
        !report_path.exists(),
        "R4 closeout already exists; immutable outputs are never overwritten"
    );
    let assembly_path = args.output_dir.join("assembly-report.json");
    let assembly: AssemblyReport = read_json(&assembly_path)?;
    ensure!(
        assembly.contract_sha256 == contract_hash
            && assembly.selected_pilot_positions == contract.cost.maximum_pilot_labels
    );
    let (pilot, pilot_manifests) = read_labeled_tier(
        args.inputs,
        "pilot",
        contract.assembly.label_shard_count,
        &contract_hash,
    )?;
    let (deep, deep_manifests) = read_labeled_tier(
        args.inputs,
        "deep-core",
        contract.assembly.deep_core_shard_count,
        &contract_hash,
    )?;
    ensure!(pilot.len() as u64 == contract.cost.maximum_pilot_labels);
    ensure!(deep.len() == contract.assembly.deep_core_positions);
    let unique = pilot
        .iter()
        .map(|item| item.position.color_swap_canonical_sha256.as_str())
        .collect::<HashSet<_>>();
    ensure!(
        unique.len() == pilot.len(),
        "pilot contains duplicate color-swap board families"
    );
    let label_hashes = pilot
        .iter()
        .map(|item| item.label_sha256.as_str())
        .collect::<HashSet<_>>();
    ensure!(
        label_hashes.len() == pilot.len(),
        "pilot labels are not unique"
    );

    let train = pilot
        .iter()
        .filter(|item| item.position.split == "training")
        .collect::<Vec<_>>();
    let validation = pilot
        .iter()
        .filter(|item| item.position.split == "validation")
        .collect::<Vec<_>>();
    let development = pilot
        .iter()
        .filter(|item| item.position.split == "development")
        .collect::<Vec<_>>();
    ensure!(train.len() == contract.assembly.pilot_split_counts["training"]);
    ensure!(validation.len() == contract.assembly.pilot_split_counts["validation"]);
    ensure!(development.len() == contract.assembly.pilot_split_counts["development"]);
    let deep_refs = deep.iter().collect::<Vec<_>>();
    let mut datasets = BTreeMap::new();
    datasets.insert(
        "train".to_string(),
        write_dataset(args.output_dir, "train", &train, &contract, &contract_hash)?,
    );
    datasets.insert(
        "validation".to_string(),
        write_dataset(
            args.output_dir,
            "validation",
            &validation,
            &contract,
            &contract_hash,
        )?,
    );
    datasets.insert(
        "development".to_string(),
        write_dataset(
            args.output_dir,
            "development",
            &development,
            &contract,
            &contract_hash,
        )?,
    );
    datasets.insert(
        "deep-core".to_string(),
        write_dataset(
            args.output_dir,
            "deep-core",
            &deep_refs,
            &contract,
            &contract_hash,
        )?,
    );

    let coverage = feature_coverage(&pilot, contract.assembly.minimum_feature_row_updates)?;
    let coverage_path = args.output_dir.join("feature-row-coverage.json");
    fs::write(&coverage_path, serde_json::to_vec_pretty(&coverage)?)?;
    let distributions = distributions(&pilot);
    let mut label_hosts = BTreeMap::new();
    let mut revisions = BTreeSet::new();
    let mut executables = BTreeSet::new();
    let mut search_seconds = 0.0;
    let mut earliest = u128::MAX;
    let mut latest = 0u128;
    let mut mate_labels = 0u64;
    let mut incomplete = 0u64;
    let mut accounting_failures = 0u64;
    for manifest in pilot_manifests.iter().chain(&deep_manifests) {
        *label_hosts.entry(manifest.hostname.clone()).or_default() += manifest.positions;
        revisions.insert(manifest.engine_revision.clone());
        executables.insert(manifest.executable_sha256.clone());
        earliest = earliest.min(manifest.started_at_unix_ms);
        latest = latest.max(manifest.finished_at_unix_ms);
        if manifest.tier == "pilot" {
            search_seconds += manifest.search_elapsed_seconds;
            mate_labels += manifest.mate_labels;
            incomplete += manifest.incomplete_labels;
            accounting_failures += manifest.node_accounting_failures;
        }
    }
    let worker_rate = pilot.len() as f64 / search_seconds;
    let actual_core_hours = search_seconds / 3_600.0;
    let forecast = contract
        .cost
        .forecast_corpus_sizes
        .iter()
        .map(|&positions| {
            let core_hours = positions as f64 / worker_rate / 3_600.0;
            CostForecast {
                positions,
                labels_per_worker_second: worker_rate,
                projected_core_hours: core_hours,
                projected_cpu_cost_usd: core_hours * contract.cost.cpu_core_hour_usd,
            }
        })
        .collect::<Vec<_>>();
    let unknown = pilot
        .iter()
        .filter(|item| item.position.result_status.starts_with("unknown/"))
        .count() as u64;
    let proofs = args
        .regeneration_proofs
        .iter()
        .map(|path| read_json::<RegenerationProof>(path))
        .collect::<Result<Vec<_>>>()?;
    let all_proofs_pass = !proofs.is_empty()
        && proofs
            .iter()
            .all(|proof| proof.passed && proof.contract_sha256 == contract_hash);
    let current_revision = engine_revision(args.workspace_root);
    let mut gates = BTreeMap::new();
    gates.insert("r3-authorization-bound".to_string(), true);
    gates.insert("two-stage-generation-and-labeling".to_string(), true);
    gates.insert(
        "exactly-one-million-unique-color-swap-families".to_string(),
        pilot.len() == 1_000_000 && unique.len() == 1_000_000,
    );
    gates.insert(
        "grouped-split-no-opening-overlap".to_string(),
        assembly.opening_group_overlap.is_empty(),
    );
    gates.insert(
        "no-color-swap-cross-split-overlap".to_string(),
        assembly.cross_split_color_swap_overlap == 0,
    );
    gates.insert(
        "duplicate-weight-capped".to_string(),
        assembly.maximum_duplicate_weight == 1,
    );
    gates.insert("finite-labels-complete".to_string(), incomplete == 0);
    gates.insert(
        "exact-node-accounting".to_string(),
        accounting_failures == 0,
    );
    gates.insert(
        "capped-games-are-unknown".to_string(),
        !pilot
            .iter()
            .any(|item| item.position.result_status == "draw/capped"),
    );
    gates.insert(
        "bounded-mate-targets-retained".to_string(),
        mate_labels > 0
            && pilot
                .iter()
                .filter(|item| item.mate_class != "none")
                .all(|item| {
                    item.bounded_score.unsigned_abs() == contract.labeling.nonmate_clip as u16
                }),
    );
    gates.insert(
        "source-mixture-covered".to_string(),
        distributions.source.len()
            == contract
                .generation
                .policy_cycle
                .iter()
                .collect::<BTreeSet<_>>()
                .len(),
    );
    gates.insert(
        "phase-mixture-covered".to_string(),
        distributions.phase.len() == contract.assembly.phase_fractions.len(),
    );
    gates.insert(
        "static-clone-complete".to_string(),
        assembly.static_clone_positions == contract.assembly.static_clone_positions as u64,
    );
    gates.insert(
        "deep-core-complete".to_string(),
        deep.len() == contract.assembly.deep_core_positions,
    );
    gates.insert("regeneration-proof".to_string(), all_proofs_pass);
    gates.insert(
        "single-engine-revision".to_string(),
        revisions.len() == 1 && revisions.contains(&current_revision),
    );
    gates.insert("no-r5-or-strength-work".to_string(), true);
    let passed = gates.values().all(|value| *value);
    let report = GateReport {
        schema: GATE_SCHEMA.to_string(),
        phase: "R4".to_string(),
        passed,
        r5_authorized: passed,
        contract_sha256: contract_hash,
        assembly_report_sha256: sha256_file(&assembly_path)?,
        pilot_positions: pilot.len() as u64,
        deep_core_positions: deep.len() as u64,
        static_clone_positions: assembly.static_clone_positions,
        datasets,
        distributions,
        feature_coverage_sha256: sha256_file(&coverage_path)?,
        feature_coverage_summary: serde_json::json!({
            "knownStructurallyUnreachableBaseRows": coverage.known_structurally_unreachable_base_rows.len(),
            "neverObservedReachableBaseRows": coverage.never_observed_reachable_base_rows.len(),
            "neverObservedDonorRows": coverage.never_observed_donor_rows.len(),
            "reachableBaseRowsBelowMinimum": coverage.reachable_base_rows_below_minimum.len(),
            "donorRowsBelowMinimum": coverage.donor_rows_below_minimum.len(),
        }),
        label_hosts,
        label_engine_revisions: revisions.into_iter().collect(),
        label_executable_hashes: executables.into_iter().collect(),
        pilot_search_elapsed_seconds: search_seconds,
        pilot_labels_per_worker_second: worker_rate,
        distributed_wall_seconds: (latest.saturating_sub(earliest)) as f64 / 1_000.0,
        actual_core_hours,
        actual_cpu_cost_usd: actual_core_hours * contract.cost.cpu_core_hour_usd,
        cost_forecast: forecast,
        mate_labels,
        incomplete_labels: incomplete,
        node_accounting_failures: accounting_failures,
        unknown_or_capped_results: unknown,
        regeneration_proofs: proofs,
        gates,
        larger_generation_started: false,
        training_started: false,
        strength_games_played: false,
    };
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    ensure!(passed, "R4 gate failed; inspect {}", report_path.display());
    Ok(report_path)
}

pub(crate) fn verify_regeneration(args: RegenerationArgs<'_>) -> Result<PathBuf> {
    let (contract, contract_hash) = load_contract(args.contract_path, args.workspace_root)?;
    let expected: PositionShardManifest = read_json(args.manifest_path)?;
    ensure!(
        expected.schema == POSITION_MANIFEST_SCHEMA && expected.contract_sha256 == contract_hash
    );
    let openings = load_openings(&contract, args.workspace_root)?;
    let mut hasher = Sha256::new();
    let mut positions = 0u64;
    let mut workspace = SearchWorkspace::new();
    for game_index in (expected.shard_index..contract.generation.game_count)
        .step_by(contract.generation.shard_count as usize)
    {
        for record in generate_game(game_index, &openings, &contract, &mut workspace)? {
            let mut line = serde_json::to_vec(&record)?;
            line.push(b'\n');
            hasher.update(&line);
            positions += 1;
        }
    }
    let regenerated = format!("{:x}", hasher.finalize());
    let proof = RegenerationProof {
        schema: "haitaka-r4-regeneration-proof-v1".to_string(),
        contract_sha256: contract_hash,
        shard_index: expected.shard_index,
        expected_positions_sha256: expected.positions_sha256.clone(),
        regenerated_positions_sha256: regenerated.clone(),
        expected_positions: expected.positions,
        regenerated_positions: positions,
        passed: expected.positions == positions && expected.positions_sha256 == regenerated,
        hostname: host_name(),
        architecture: std::env::consts::ARCH.to_string(),
        executable_sha256: sha256_file(args.executable_path)?,
    };
    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(args.output, serde_json::to_vec_pretty(&proof)?)?;
    ensure!(proof.passed, "R4 regeneration hash mismatch");
    Ok(args.output.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_swap_board_family_hash_is_shared() {
        let board = Board::from_sfen(
            "lnsg1g1nl/1r1k1s1b1/p1pp1p1p1/1p2p1p1p/9/3PNP3/PPP1P1PPP/1BG1G2R1/L1S2KSNL b - 13",
        )
        .unwrap();
        let swapped =
            Board::from_sfen(&color_swap_anhoku_sfen(&board.to_string()).unwrap()).unwrap();
        let (_, original_family) = board_hashes(&board).unwrap();
        let (_, swapped_family) = board_hashes(&swapped).unwrap();
        assert_eq!(original_family, swapped_family);
    }

    #[test]
    fn phase_quotas_are_exact_for_all_frozen_splits() {
        for count in [880_000, 60_000] {
            let total = [0.15, 0.50, 0.30, 0.05]
                .into_iter()
                .map(|fraction| quota_for(count, fraction))
                .sum::<usize>();
            assert_eq!(total, count);
        }
    }

    #[test]
    fn structural_base_row_classifier_has_frozen_cardinality() {
        let unreachable = (0..R1_HALFKAV2_BASE_FEATURES)
            .filter(|&row| !base_row_structurally_reachable(row))
            .count();
        // Per king bucket: 18 impossible piece-on-own-king rows and 248
        // impossible/reserved hand-copy rows.
        assert_eq!(unreachable, 81 * (18 + 248));
    }
}

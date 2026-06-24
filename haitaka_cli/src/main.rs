use std::fs;
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command as ProcessCommand, Stdio};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};
use haitaka::{Board, Color, GameStatus, Move, SFEN_STARTPOS};
use haitaka_wasm::{NnueModel, SearchEvalMode, UsiSession};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const ENGINE_ID: &str = "haitaka-variants";
const ENGINE_NAME: &str = "Haitaka Variants";
const MANIFEST_FILE: &str = "shogitter-engine.json";
const ENGINE_DIR: &str = "engine";
const WASM_BINDGEN_MODULE: &str = "engine/haitaka_wasm.js";
const WASM_BINDGEN_WASM: &str = "engine/haitaka_wasm_bg.wasm";
const NNUE_ARTIFACT_PATH: &str = "engine/model.nnue";
const ENGINE_ARCHIVE_MANIFEST_FILE: &str = "haitaka-engine-archive.json";
const ENGINE_ARCHIVE_BIN_PATH: &str = "bin/haitaka_cli";
const ENGINE_ARCHIVE_NNUE_PATH: &str = "nnue/model.nnue";
const SELF_PLAY_REPORT_FILE: &str = "self-play-report.json";
const SELF_PLAY_GAMES_FILE: &str = "self-play-games.jsonl";
const REQUIRED_WASM_FILES: [&str; 2] = ["haitaka_wasm.js", "haitaka_wasm_bg.wasm"];
const OPTIONAL_WASM_FILES: [&str; 4] = [
    "haitaka_wasm.d.ts",
    "haitaka_wasm_bg.wasm.d.ts",
    "package.json",
    "README.md",
];
const USI_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const USI_DEPTH_SEARCH_TIMEOUT: Duration = Duration::from_secs(30);
const USI_SEARCH_TIMEOUT_GRACE: Duration = Duration::from_secs(5);
const USI_STDERR_LIMIT: usize = 50;

static SELF_PLAY_INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Parser)]
#[command(name = "haitaka")]
#[command(about = "Launch tools for local play, self-play, and Shogitter packaging")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Play or debug one side against the built-in search.
    Play(PlayArgs),
    /// Run the engine as a minimal USI subprocess.
    Usi(UsiArgs),
    /// Run engine-vs-engine games and report a small-sample rating estimate.
    SelfPlay(SelfPlayArgs),
    /// Create a Shogitter-consumable engine package archive.
    Package(PackageArgs),
    /// Create a native USI engine archive for reproducible self-play.
    ArchiveEngine(ArchiveEngineArgs),
}

#[derive(Debug, Parser)]
struct PlayArgs {
    /// Starting SFEN. Defaults to the ruleset start position.
    #[arg(long)]
    sfen: Option<String>,
    /// Human side. Use "none" to let the engine play one move and exit.
    #[arg(long, default_value = "black")]
    human: HumanSide,
    /// Fixed search depth.
    #[arg(long, default_value_t = 3)]
    depth: u8,
    /// Maximum plies before the session stops.
    #[arg(long, default_value_t = 200)]
    max_plies: u16,
}

#[derive(Debug, Parser)]
struct UsiArgs {
    /// Evaluator used by the USI engine.
    #[arg(long = "eval", value_enum, default_value = "handcrafted")]
    eval: EngineEvalKind,
    /// NNUE file used when --eval nnue is selected.
    #[arg(long)]
    nnue: Option<PathBuf>,
    /// Maximum depth used for `go movetime`.
    #[arg(long, default_value_t = 64)]
    movetime_max_depth: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum HumanSide {
    Black,
    White,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum EngineEvalKind {
    Handcrafted,
    Nnue,
}

#[derive(Debug, Parser)]
struct SelfPlayArgs {
    /// Number of games to run.
    #[arg(long, default_value_t = 2)]
    games: u32,
    /// Number of worker threads. Set to 0 to use available parallelism.
    #[arg(long, default_value_t = 0)]
    threads: usize,
    /// Engine A fixed search depth, or movetime depth cap when --movetime-ms is set.
    #[arg(long = "a-depth")]
    a_depth: Option<u8>,
    /// Engine B fixed search depth, or movetime depth cap when --movetime-ms is set.
    #[arg(long = "b-depth")]
    b_depth: Option<u8>,
    /// Engine A evaluator.
    #[arg(long = "a-eval", value_enum, default_value = "handcrafted")]
    a_eval: EngineEvalKind,
    /// Engine B evaluator.
    #[arg(long = "b-eval", value_enum, default_value = "handcrafted")]
    b_eval: EngineEvalKind,
    /// Shared NNUE file for any side using NNUE without a side-specific override.
    #[arg(long)]
    nnue: Option<PathBuf>,
    /// Engine A NNUE file override.
    #[arg(long = "a-nnue")]
    a_nnue: Option<PathBuf>,
    /// Engine B NNUE file override.
    #[arg(long = "b-nnue")]
    b_nnue: Option<PathBuf>,
    /// External USI engine executable for side A.
    #[arg(long = "a-engine")]
    a_engine: Option<PathBuf>,
    /// Native engine archive for side A.
    #[arg(long = "a-engine-archive")]
    a_engine_archive: Option<PathBuf>,
    /// Argument appended when launching side A's external engine.
    #[arg(long = "a-engine-arg", action = clap::ArgAction::Append, allow_hyphen_values = true)]
    a_engine_args: Vec<String>,
    /// External USI engine executable for side B.
    #[arg(long = "b-engine")]
    b_engine: Option<PathBuf>,
    /// Native engine archive for side B.
    #[arg(long = "b-engine-archive")]
    b_engine_archive: Option<PathBuf>,
    /// Argument appended when launching side B's external engine.
    #[arg(long = "b-engine-arg", action = clap::ArgAction::Append, allow_hyphen_values = true)]
    b_engine_args: Vec<String>,
    /// Shared movetime budget in milliseconds. If set, both sides use movetime.
    #[arg(long)]
    movetime_ms: Option<u32>,
    /// Starting SFEN. Defaults to the ruleset start position.
    #[arg(long)]
    sfen: Option<String>,
    /// Opening suite file. One SFEN per line; blank lines and # comments are ignored.
    #[arg(long)]
    openings: Option<PathBuf>,
    /// Opening suite selection policy.
    #[arg(long = "opening-order", value_enum, default_value = "sequential")]
    opening_order: OpeningOrder,
    /// Number of random plies applied before each paired game to diversify openings.
    #[arg(long, default_value_t = 0)]
    opening_random_plies: u16,
    /// Seed for random opening generation.
    #[arg(long, default_value_t = 0)]
    seed: u64,
    /// Maximum plies per game before declaring a draw.
    #[arg(long, default_value_t = 200)]
    max_plies: u16,
    /// Directory where self-play-report.json and self-play-games.jsonl are written.
    #[arg(long = "report-dir")]
    report_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "kebab-case")]
enum OpeningOrder {
    Sequential,
    Random,
}

#[derive(Debug, Parser)]
struct PackageArgs {
    /// Package output path.
    #[arg(long, default_value = "target/haitaka-variants.tgz")]
    output: PathBuf,
    /// Directory containing wasm-bindgen or wasm-pack output.
    #[arg(long, default_value = "haitaka_wasm/pkg")]
    wasm_dir: PathBuf,
    /// Ruleset name written into metadata.
    #[arg(long, default_value = default_ruleset())]
    ruleset: String,
    /// Shogitter rule ID written into metadata.
    #[arg(long, default_value_t = default_rule_id())]
    rule_id: u32,
    /// Optional NNUE file to include in the package.
    #[arg(long)]
    nnue: Option<PathBuf>,
    /// Allow metadata-only packages when wasm artifacts are not built yet.
    #[arg(long)]
    allow_missing_wasm: bool,
}

#[derive(Debug, Parser)]
struct ArchiveEngineArgs {
    /// Archive output path.
    #[arg(long)]
    output: PathBuf,
    /// Native USI-capable haitaka_cli binary to archive.
    #[arg(long)]
    binary: PathBuf,
    /// Optional NNUE file to include in the archive.
    #[arg(long)]
    nnue: Option<PathBuf>,
    /// Ruleset name written into metadata.
    #[arg(long, default_value = default_ruleset())]
    ruleset: String,
    /// Engine display name written into metadata.
    #[arg(long = "engine-name", default_value = ENGINE_NAME)]
    engine_name: String,
    /// Build profile written into metadata.
    #[arg(long, value_enum)]
    profile: Option<ArchiveBuildProfile>,
    /// Target triple or platform identifier written into metadata.
    #[arg(long)]
    target: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ArchiveBuildProfile {
    Debug,
    Release,
    Custom,
    Unknown,
}

impl ArchiveBuildProfile {
    fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
            Self::Custom => "custom",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Serialize)]
struct EnginePackageManifest {
    schema: &'static str,
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    engine: ManifestEngine,
    runtime: ManifestRuntime,
    capabilities: ManifestCapabilities,
    profiles: Vec<ManifestProfile>,
}

#[derive(Debug, Serialize)]
struct ManifestEngine {
    id: &'static str,
    name: String,
    version: &'static str,
    commit: String,
}

#[derive(Debug, Serialize)]
struct ManifestRuntime {
    kind: &'static str,
    module: &'static str,
    wasm: &'static str,
}

#[derive(Debug, Serialize)]
struct ManifestCapabilities {
    protocols: Vec<&'static str>,
    commands: Vec<&'static str>,
    #[serde(rename = "supportsPonder")]
    supports_ponder: bool,
    #[serde(rename = "supportsMovetime")]
    supports_movetime: bool,
    #[serde(rename = "supportsDepth")]
    supports_depth: bool,
}

#[derive(Debug, Serialize)]
struct ManifestRule {
    #[serde(rename = "ruleId")]
    rule_id: u32,
    variant: String,
    #[serde(rename = "positionFormat")]
    position_format: &'static str,
    #[serde(rename = "moveFormat")]
    move_format: &'static str,
    startpos: &'static str,
}

#[derive(Debug, Serialize)]
struct ManifestProfile {
    id: String,
    name: String,
    rules: Vec<ManifestRule>,
    nnue: Option<NnueArtifact>,
}

#[derive(Debug, Serialize)]
struct NnueArtifact {
    path: &'static str,
    format: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NativeEngineArchiveManifest {
    schema: String,
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    engine: NativeArchiveEngine,
    runtime: NativeArchiveRuntime,
    nnue: Option<NativeArchiveNnue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NativeArchiveEngine {
    name: String,
    version: String,
    commit: String,
    dirty: bool,
    ruleset: String,
    features: Vec<String>,
    #[serde(rename = "buildProfile")]
    build_profile: String,
    target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NativeArchiveRuntime {
    protocol: String,
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
    executable: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NativeArchiveNnue {
    path: String,
    sha256: String,
}

#[derive(Debug)]
struct ArchiveLaunch {
    engine_path: PathBuf,
    engine_args: Vec<String>,
    report_engine_args: Vec<String>,
    extraction_dir: PathBuf,
    source_archive_path: PathBuf,
    manifest: NativeEngineArchiveManifest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seat {
    A,
    B,
}

#[derive(Debug, Clone)]
struct EngineConfig {
    label: &'static str,
    budget: SearchBudget,
    evaluator: EngineEvaluator,
}

#[derive(Debug, Clone)]
enum EngineEvaluator {
    Handcrafted,
    Nnue {
        path: PathBuf,
        model: Arc<NnueModel>,
    },
    External {
        path: PathBuf,
        args: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchBudget {
    Depth(u8),
    Movetime { max_depth: Option<u8>, millis: u32 },
}

#[derive(Debug, Clone, PartialEq)]
struct EngineSearchResult {
    best_move: Option<String>,
    total_nodes: u64,
    elapsed_ms: f64,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq)]
struct SearchBreakdown {
    #[serde(rename = "totalNodes", default)]
    total_nodes: u64,
    #[serde(rename = "totalElapsedMs", default)]
    total_elapsed_ms: f64,
    #[serde(rename = "aggregateNps", default)]
    aggregate_nps: f64,
}

#[derive(Debug, Default, Clone)]
struct MatchStats {
    a_wins: u32,
    b_wins: u32,
    draws: u32,
    total_nodes: u64,
    total_elapsed_ms: f64,
    total_plies: u64,
    a_breakdown: SearchBreakdown,
    b_breakdown: SearchBreakdown,
}

#[derive(Debug, Clone)]
struct GameResult {
    a_color: Color,
    winner: Option<Seat>,
    plies: u16,
    total_nodes: u64,
    total_elapsed_ms: f64,
    a_breakdown: SearchBreakdown,
    b_breakdown: SearchBreakdown,
    start_sfen: String,
    opening: OpeningRecord,
    moves: Vec<String>,
}

#[derive(Debug, Clone)]
struct OpeningPosition {
    suite_index: usize,
    sfen: String,
}

#[derive(Debug, Clone, Serialize)]
struct OpeningRecord {
    source: String,
    #[serde(rename = "suiteIndex", skip_serializing_if = "Option::is_none")]
    suite_index: Option<usize>,
    #[serde(rename = "baseSfen")]
    base_sfen: String,
    #[serde(rename = "randomPlies")]
    random_plies: u16,
    #[serde(rename = "randomSeed", skip_serializing_if = "Option::is_none")]
    random_seed: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct GameJsonRecord {
    #[serde(rename = "schema")]
    schema: &'static str,
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    #[serde(rename = "gameIndex")]
    game_index: u32,
    #[serde(rename = "pairIndex")]
    pair_index: u32,
    #[serde(rename = "aColor")]
    a_color: String,
    #[serde(rename = "bColor")]
    b_color: String,
    opening: OpeningRecord,
    #[serde(rename = "startSfen")]
    start_sfen: String,
    moves: Vec<String>,
    result: String,
    winner: Option<String>,
    plies: u16,
    #[serde(rename = "totalNodes")]
    total_nodes: u64,
    #[serde(rename = "totalElapsedMs")]
    total_elapsed_ms: f64,
    #[serde(rename = "aBreakdown")]
    a_breakdown: SearchBreakdown,
    #[serde(rename = "bBreakdown")]
    b_breakdown: SearchBreakdown,
    #[serde(rename = "failureState")]
    failure_state: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SelfPlayReport {
    schema: &'static str,
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    #[serde(rename = "generatedAtUnixSeconds")]
    generated_at_unix_seconds: u64,
    package: ReportPackage,
    git: ReportGit,
    ruleset: String,
    command: ReportCommand,
    engines: Vec<ReportEngine>,
    summary: RatingSummary,
}

#[derive(Debug, Clone, Serialize)]
struct ReportPackage {
    name: &'static str,
    version: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct ReportGit {
    commit: String,
    dirty: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ReportCommand {
    games: u32,
    threads: usize,
    #[serde(rename = "aDepth")]
    a_depth: Option<u8>,
    #[serde(rename = "bDepth")]
    b_depth: Option<u8>,
    #[serde(rename = "movetimeMs")]
    movetime_ms: Option<u32>,
    sfen: Option<String>,
    openings: Option<String>,
    #[serde(rename = "openingOrder")]
    opening_order: OpeningOrder,
    #[serde(rename = "openingRandomPlies")]
    opening_random_plies: u16,
    seed: u64,
    #[serde(rename = "maxPlies")]
    max_plies: u16,
}

#[derive(Debug, Clone, Serialize)]
struct ReportEngine {
    label: &'static str,
    kind: String,
    budget: String,
    command: Option<String>,
    args: Vec<String>,
    nnue: Option<String>,
    #[serde(rename = "archivePath", skip_serializing_if = "Option::is_none")]
    archive_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    archive: Option<NativeEngineArchiveManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct RatingSummary {
    games: u32,
    #[serde(rename = "aWins")]
    a_wins: u32,
    #[serde(rename = "bWins")]
    b_wins: u32,
    draws: u32,
    #[serde(rename = "decidedGames")]
    decided_games: u32,
    #[serde(rename = "aScore")]
    a_score: f64,
    #[serde(rename = "scoreRate")]
    score_rate: f64,
    #[serde(rename = "approxElo")]
    approx_elo: f64,
    #[serde(rename = "approxElo95Ci")]
    approx_elo_95_ci: [f64; 2],
    #[serde(rename = "avgPlies")]
    avg_plies: f64,
    #[serde(rename = "totalNodes")]
    total_nodes: u64,
    #[serde(rename = "totalElapsedMs", default)]
    total_elapsed_ms: f64,
    #[serde(rename = "aggregateNps")]
    aggregate_nps: f64,
    #[serde(rename = "aBreakdown", default)]
    a_breakdown: SearchBreakdown,
    #[serde(rename = "bBreakdown", default)]
    b_breakdown: SearchBreakdown,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReportConflictAction {
    Abort,
    Merge,
    Overwrite,
}

#[derive(Debug)]
struct SelfPlayReportOutput {
    games_writer: fs::File,
    report_path: PathBuf,
    existing_stats: MatchStats,
    game_index_offset: u32,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Play(args) => play(args),
        Command::Usi(args) => usi(args),
        Command::SelfPlay(args) => self_play(args),
        Command::Package(args) => package(args),
        Command::ArchiveEngine(args) => archive_engine(args),
    }
}

fn default_ruleset() -> &'static str {
    if cfg!(feature = "annan") {
        "annan"
    } else if cfg!(feature = "anhoku") {
        "anhoku"
    } else if cfg!(feature = "antouzai") {
        "antouzai"
    } else if cfg!(feature = "taimen") {
        "taimen"
    } else if cfg!(feature = "haimen") {
        "haimen"
    } else if cfg!(feature = "neko") {
        "neko"
    } else if cfg!(feature = "nekoneko") {
        "nekoneko"
    } else if cfg!(feature = "yokoneko") {
        "yokoneko"
    } else if cfg!(feature = "yokonekoneko") {
        "yokonekoneko"
    } else {
        "standard"
    }
}

fn default_rule_id() -> u32 {
    if cfg!(feature = "annan") {
        26
    } else if cfg!(feature = "anhoku") {
        55
    } else if cfg!(feature = "antouzai") {
        95
    } else if cfg!(feature = "taimen") {
        72
    } else if cfg!(feature = "haimen") {
        74
    } else if cfg!(feature = "neko") {
        130
    } else if cfg!(feature = "nekoneko") {
        131
    } else if cfg!(feature = "yokoneko") {
        132
    } else if cfg!(feature = "yokonekoneko") {
        133
    } else {
        0
    }
}

fn profile_display_ruleset(ruleset: &str) -> String {
    match ruleset {
        "standard" => "Standard".to_string(),
        "annan" => "Annan".to_string(),
        "anhoku" => "Anhoku".to_string(),
        "antouzai" => "Antouzai".to_string(),
        "taimen" => "Taimen".to_string(),
        "haimen" => "Haimen".to_string(),
        other => {
            let mut chars = other.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            let mut display = first.to_ascii_uppercase().to_string();
            display.push_str(chars.as_str());
            display
        }
    }
}

fn package_manifest(args: &PackageArgs, nnue: Option<NnueArtifact>) -> EnginePackageManifest {
    EnginePackageManifest {
        schema: "shogitter-engine-package",
        schema_version: 1,
        engine: ManifestEngine {
            id: ENGINE_ID,
            name: format!("{ENGINE_NAME} ({})", args.ruleset),
            version: env!("CARGO_PKG_VERSION"),
            commit: git_commit(),
        },
        runtime: ManifestRuntime {
            kind: "wasm-bindgen",
            module: WASM_BINDGEN_MODULE,
            wasm: WASM_BINDGEN_WASM,
        },
        capabilities: ManifestCapabilities {
            protocols: vec!["shogitter-direct-v1", "usi-wasm-v1"],
            commands: vec!["search", "iterative-search", "perft", "dfpn"],
            supports_ponder: false,
            supports_movetime: true,
            supports_depth: true,
        },
        profiles: vec![ManifestProfile {
            id: format!("{}-default", args.ruleset),
            name: format!("{} default", profile_display_ruleset(&args.ruleset)),
            rules: vec![ManifestRule {
                rule_id: args.rule_id,
                variant: args.ruleset.clone(),
                position_format: "sfen",
                move_format: "usi",
                startpos: SFEN_STARTPOS,
            }],
            nnue,
        }],
    }
}

fn parse_board(sfen: Option<&str>) -> Result<Board> {
    let sfen = sfen.unwrap_or(SFEN_STARTPOS);
    Board::from_sfen(sfen).map_err(|err| anyhow!("failed to parse SFEN: {err}"))
}

fn load_nnue_model(path: &Path) -> Result<Arc<NnueModel>> {
    let bytes =
        fs::read(path).with_context(|| format!("failed to read NNUE {}", path.display()))?;
    let model = NnueModel::from_bytes(&bytes)
        .map_err(|err| anyhow!("failed to load NNUE {}: {err}", path.display()))?;
    Ok(Arc::new(model))
}

fn engine_config(
    label: &'static str,
    budget: SearchBudget,
    evaluator: EngineEvalKind,
    shared_nnue: Option<&Path>,
    side_nnue: Option<&Path>,
    external_engine: Option<&Path>,
    external_args: &[String],
) -> Result<EngineConfig> {
    let evaluator = if let Some(path) = external_engine {
        EngineEvaluator::External {
            path: path.to_path_buf(),
            args: external_args.to_vec(),
        }
    } else {
        match evaluator {
            EngineEvalKind::Handcrafted => EngineEvaluator::Handcrafted,
            EngineEvalKind::Nnue => {
                let path = side_nnue
                    .or(shared_nnue)
                    .ok_or_else(|| anyhow!("{label} uses NNUE but no NNUE path was provided"))?;
                EngineEvaluator::Nnue {
                    path: path.to_path_buf(),
                    model: load_nnue_model(path)?,
                }
            }
        }
    };

    Ok(EngineConfig {
        label,
        budget,
        evaluator,
    })
}

fn search_with_engine(board: &Board, engine: &EngineConfig) -> Result<EngineSearchResult> {
    match &engine.evaluator {
        EngineEvaluator::Handcrafted => search_in_process_handcrafted(board, engine.budget)
            .map_err(|err| anyhow!("{} search failed: {err}", engine.label)),
        EngineEvaluator::Nnue { model, .. } => {
            search_in_process_nnue(board, engine.budget, model.clone())
                .map_err(|err| anyhow!("{} NNUE search failed: {err}", engine.label))
        }
        EngineEvaluator::External { .. } => {
            bail!("external engine must be started before searching")
        }
    }
}

fn search_in_process_handcrafted(
    board: &Board,
    budget: SearchBudget,
) -> Result<EngineSearchResult, String> {
    match budget {
        SearchBudget::Depth(depth) => {
            let summary = haitaka_wasm::search_board_impl_handcrafted(board, depth)?;
            Ok(EngineSearchResult {
                best_move: summary.best_move,
                total_nodes: summary.states,
                elapsed_ms: summary.elapsed_ms,
            })
        }
        SearchBudget::Movetime { max_depth, millis } => {
            let summary = haitaka_wasm::search_iterative_deepening_impl(
                &board.to_string(),
                max_depth.unwrap_or(u8::MAX),
                millis,
            )?;
            Ok(EngineSearchResult {
                best_move: summary.best_move,
                total_nodes: summary.states,
                elapsed_ms: summary.elapsed_ms,
            })
        }
    }
}

fn search_in_process_nnue(
    board: &Board,
    budget: SearchBudget,
    model: Arc<NnueModel>,
) -> Result<EngineSearchResult, String> {
    match budget {
        SearchBudget::Depth(depth) => {
            let summary = haitaka_wasm::search_board_impl_with_eval_mode(
                board,
                depth,
                model,
                SearchEvalMode::Incremental,
            )?;
            Ok(EngineSearchResult {
                best_move: summary.best_move,
                total_nodes: summary.states,
                elapsed_ms: summary.elapsed_ms,
            })
        }
        SearchBudget::Movetime { max_depth, millis } => {
            let summary = haitaka_wasm::search_iterative_deepening_impl_with_eval_mode(
                &board.to_string(),
                max_depth.unwrap_or(u8::MAX),
                millis,
                model,
                SearchEvalMode::Incremental,
            )?;
            Ok(EngineSearchResult {
                best_move: summary.best_move,
                total_nodes: summary.states,
                elapsed_ms: summary.elapsed_ms,
            })
        }
    }
}

fn describe_engine(engine: &EngineConfig) -> String {
    match &engine.evaluator {
        EngineEvaluator::Handcrafted => {
            format!(
                "{}: handcrafted {}",
                engine.label,
                describe_budget(engine.budget)
            )
        }
        EngineEvaluator::Nnue { path, .. } => {
            format!(
                "{}: nnue {} model={}",
                engine.label,
                describe_budget(engine.budget),
                path.display()
            )
        }
        EngineEvaluator::External { path, args } => {
            let rendered_args = if args.is_empty() {
                String::new()
            } else {
                format!(" args={}", args.join(" "))
            };
            format!(
                "{}: external {} command={} usi{}",
                engine.label,
                describe_budget(engine.budget),
                path.display(),
                rendered_args
            )
        }
    }
}

fn describe_budget(budget: SearchBudget) -> String {
    match budget {
        SearchBudget::Depth(depth) => format!("depth={depth}"),
        SearchBudget::Movetime { max_depth, millis } => match max_depth {
            Some(max_depth) => format!("movetime_ms={millis} max_depth={max_depth}"),
            None => format!("movetime_ms={millis} max_depth=unlimited"),
        },
    }
}

const DEFAULT_SELF_PLAY_A_DEPTH: u8 = 3;
const DEFAULT_SELF_PLAY_B_DEPTH: u8 = 2;

fn self_play_budget(
    default_depth: u8,
    depth: Option<u8>,
    movetime_ms: Option<u32>,
) -> Result<SearchBudget> {
    match movetime_ms {
        Some(0) => bail!("--movetime-ms must be greater than 0"),
        Some(millis) => Ok(SearchBudget::Movetime {
            max_depth: depth.map(|depth| depth.max(1)),
            millis,
        }),
        None => Ok(SearchBudget::Depth(depth.unwrap_or(default_depth).max(1))),
    }
}

fn resolve_self_play_threads(requested: usize, games: u32) -> usize {
    let available = thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(1);
    let threads = if requested == 0 { available } else { requested };
    threads.max(1).min(games.max(1) as usize)
}

fn format_eta(seconds: f64) -> String {
    if !seconds.is_finite() || seconds <= 0.0 {
        return "0s".to_string();
    }

    let total = seconds.ceil() as u64;
    let hours = total / 3_600;
    let minutes = (total % 3_600) / 60;
    let secs = total % 60;

    if hours > 0 {
        format!("{hours}h{minutes:02}m{secs:02}s")
    } else if minutes > 0 {
        format!("{minutes}m{secs:02}s")
    } else {
        format!("{secs}s")
    }
}

fn opening_seed(seed: u64, pair_index: u32, attempt: u32) -> u64 {
    seed ^ (u64::from(pair_index) << 32) ^ u64::from(attempt).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

fn load_opening_suite(path: &Path) -> Result<Vec<OpeningPosition>> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("read openings {}", path.display()))?;
    let mut openings = Vec::new();
    for (line_index, raw_line) in contents.lines().enumerate() {
        let line = raw_line.split_once('#').map_or(raw_line, |(sfen, _)| sfen);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        Board::from_sfen(line).map_err(|err| {
            anyhow!(
                "failed to parse opening SFEN in {} line {}: {err}",
                path.display(),
                line_index + 1
            )
        })?;
        openings.push(OpeningPosition {
            suite_index: openings.len(),
            sfen: line.to_string(),
        });
    }
    if openings.is_empty() {
        bail!(
            "opening suite {} contains no SFEN positions",
            path.display()
        );
    }
    Ok(openings)
}

fn select_suite_opening<'a>(
    openings: &'a [OpeningPosition],
    order: OpeningOrder,
    seed: u64,
    pair_index: u32,
) -> &'a OpeningPosition {
    match order {
        OpeningOrder::Sequential => &openings[pair_index as usize % openings.len()],
        OpeningOrder::Random => {
            let mut rng = StdRng::seed_from_u64(opening_seed(seed, pair_index, 0));
            &openings[rng.random_range(0..openings.len())]
        }
    }
}

fn generate_opening_board(
    base: &Board,
    opening_random_plies: u16,
    seed: u64,
    pair_index: u32,
) -> Result<Board> {
    if opening_random_plies == 0 {
        return Ok(base.clone());
    }

    for attempt in 0..16 {
        let mut board = base.clone();
        let mut rng = StdRng::seed_from_u64(opening_seed(seed, pair_index, attempt));
        let mut completed = 0;

        while completed < opening_random_plies && board.status() == GameStatus::Ongoing {
            let moves = legal_moves(&board);
            if moves.is_empty() {
                break;
            }
            let mv = moves[rng.random_range(0..moves.len())];
            board.play(mv);
            completed += 1;
        }

        if completed == opening_random_plies && board.status() == GameStatus::Ongoing {
            return Ok(board);
        }
    }

    bail!(
        "failed to generate a non-terminal opening after {} random plies; try reducing --opening-random-plies",
        opening_random_plies
    )
}

fn game_opening(
    base_board: &Board,
    openings: Option<&[OpeningPosition]>,
    args: &SelfPlayArgs,
    pair_index: u32,
) -> Result<(Board, OpeningRecord)> {
    let (base, mut record) = if let Some(openings) = openings {
        let opening = select_suite_opening(openings, args.opening_order, args.seed, pair_index);
        let board = Board::from_sfen(&opening.sfen).map_err(|err| {
            anyhow!(
                "failed to parse selected opening SFEN at suite index {}: {}: {err}",
                opening.suite_index,
                opening.sfen
            )
        })?;
        (
            board,
            OpeningRecord {
                source: "suite".to_string(),
                suite_index: Some(opening.suite_index),
                base_sfen: opening.sfen.clone(),
                random_plies: args.opening_random_plies,
                random_seed: None,
            },
        )
    } else {
        (
            base_board.clone(),
            OpeningRecord {
                source: "sfen".to_string(),
                suite_index: None,
                base_sfen: base_board.to_string(),
                random_plies: args.opening_random_plies,
                random_seed: None,
            },
        )
    };
    if args.opening_random_plies > 0 {
        let random_seed = opening_seed(args.seed, pair_index, 0);
        record.random_seed = Some(random_seed);
        let board =
            generate_opening_board(&base, args.opening_random_plies, args.seed, pair_index)?;
        Ok((board, record))
    } else {
        Ok((base, record))
    }
}

enum GameEngine<'a> {
    InProcess(&'a EngineConfig),
    External(UsiEngineClient),
}

impl<'a> GameEngine<'a> {
    fn start(config: &'a EngineConfig) -> Result<Self> {
        match &config.evaluator {
            EngineEvaluator::External { path, args } => {
                Ok(Self::External(UsiEngineClient::spawn(path, args)?))
            }
            EngineEvaluator::Handcrafted | EngineEvaluator::Nnue { .. } => {
                Ok(Self::InProcess(config))
            }
        }
    }

    fn search(&mut self, board: &Board, budget: SearchBudget) -> Result<EngineSearchResult> {
        match self {
            Self::InProcess(config) => search_with_engine(board, config),
            Self::External(client) => client.search(board, budget),
        }
    }
}

/// Spawns a process, retrying briefly on `ETXTBSY` ("text file busy").
///
/// A just-written executable can transiently fail to launch on Unix with
/// `ETXTBSY`: when another thread in this process `fork()`s (any concurrent
/// `Command::spawn`) it inherits an open write handle to the file until that
/// child `exec`s, and the kernel refuses to `exec` a target that is still open
/// for writing anywhere. This race is common under the parallel test harness and
/// can also occur right after writing/installing an engine binary, so retry a few
/// times before giving up. On non-Unix targets this error does not arise, so we
/// spawn once.
fn spawn_retrying_text_file_busy(command: &mut ProcessCommand) -> std::io::Result<Child> {
    #[cfg(unix)]
    {
        // ETXTBSY is 26 on Linux/macOS/BSD.
        const ETXTBSY: i32 = 26;
        const MAX_ATTEMPTS: u32 = 50;
        const RETRY_DELAY: Duration = Duration::from_millis(10);
        let mut attempts = 0;
        loop {
            match command.spawn() {
                Err(err) if err.raw_os_error() == Some(ETXTBSY) && attempts < MAX_ATTEMPTS => {
                    attempts += 1;
                    thread::sleep(RETRY_DELAY);
                }
                result => return result,
            }
        }
    }
    #[cfg(not(unix))]
    {
        command.spawn()
    }
}

struct UsiEngineClient {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
    stderr_lines: Arc<Mutex<Vec<String>>>,
}

impl UsiEngineClient {
    fn spawn(path: &Path, args: &[String]) -> Result<Self> {
        Self::spawn_with_startup_timeout(path, args, USI_STARTUP_TIMEOUT)
    }

    fn spawn_with_startup_timeout(
        path: &Path,
        args: &[String],
        startup_timeout: Duration,
    ) -> Result<Self> {
        let mut command = ProcessCommand::new(path);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = spawn_retrying_text_file_busy(&mut command)
            .with_context(|| format!("failed to launch external engine {}", path.display()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("external engine stdin was not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("external engine stdout was not piped"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("external engine stderr was not piped"))?;

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(line) = line else {
                    break;
                };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });

        let stderr_lines = Arc::new(Mutex::new(Vec::new()));
        let stderr_slot = Arc::clone(&stderr_lines);
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                let Ok(line) = line else {
                    break;
                };
                let Ok(mut lines) = stderr_slot.lock() else {
                    break;
                };
                if lines.len() == USI_STDERR_LIMIT {
                    lines.remove(0);
                }
                lines.push(line);
            }
        });

        let mut client = Self {
            child,
            stdin,
            lines: rx,
            stderr_lines,
        };
        client.send_command("usi")?;
        client.read_until_exact("usiok", startup_timeout)?;
        client.send_command("isready")?;
        client.read_until_exact("readyok", startup_timeout)?;
        client.send_command("usinewgame")?;
        Ok(client)
    }

    fn search(&mut self, board: &Board, budget: SearchBudget) -> Result<EngineSearchResult> {
        let started_at = Instant::now();
        self.send_command(&format!("position sfen {board}"))?;
        self.send_command(&go_command(budget))?;
        let timeout = search_timeout(budget);
        let bestmove = self.read_bestmove(timeout)?;
        Ok(EngineSearchResult {
            best_move: bestmove,
            total_nodes: 0,
            elapsed_ms: started_at.elapsed().as_secs_f64() * 1_000.0,
        })
    }

    fn send_command(&mut self, command: &str) -> Result<()> {
        writeln!(self.stdin, "{command}").context("failed to write to external engine")?;
        self.stdin
            .flush()
            .context("failed to flush external engine stdin")
    }

    fn read_until_exact(&mut self, expected: &str, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            let line = self.recv_line_until(deadline)?;
            if line == expected {
                return Ok(());
            }
        }
    }

    fn read_bestmove(&mut self, timeout: Duration) -> Result<Option<String>> {
        let deadline = Instant::now() + timeout;
        loop {
            let line = self.recv_line_until(deadline)?;
            if let Some(rest) = line.strip_prefix("bestmove ") {
                let move_text = rest.split_whitespace().next().unwrap_or_default();
                if move_text.is_empty() {
                    bail!("external engine returned empty bestmove");
                }
                if move_text == "resign" {
                    return Ok(None);
                }
                return Ok(Some(move_text.to_string()));
            }
            if line.starts_with("info string invalid position command:")
                || line.starts_with("info string invalid go command:")
            {
                bail!("external engine reported search setup error: {line}");
            }
        }
    }

    fn recv_line_until(&mut self, deadline: Instant) -> Result<String> {
        let now = Instant::now();
        if now >= deadline {
            self.check_child_status()?;
            bail!("external engine timed out{}", self.stderr_context());
        }
        match self.lines.recv_timeout(deadline - now) {
            Ok(line) => Ok(line),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.check_child_status()?;
                bail!("external engine timed out{}", self.stderr_context())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.check_child_status()?;
                bail!("external engine closed stdout{}", self.stderr_context())
            }
        }
    }

    fn check_child_status(&mut self) -> Result<()> {
        if let Some(status) = self
            .child
            .try_wait()
            .context("check external engine status")?
        {
            bail!(
                "external engine exited with status {status}{}",
                self.stderr_context()
            );
        }
        Ok(())
    }

    fn stderr_context(&self) -> String {
        let Ok(lines) = self.stderr_lines.lock() else {
            return String::new();
        };
        if lines.is_empty() {
            String::new()
        } else {
            format!("; recent stderr: {}", lines.join(" | "))
        }
    }
}

impl Drop for UsiEngineClient {
    fn drop(&mut self) {
        let _ = self.send_command("quit");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn go_command(budget: SearchBudget) -> String {
    match budget {
        SearchBudget::Depth(depth) => format!("go depth {}", depth.max(1)),
        SearchBudget::Movetime {
            max_depth: Some(max_depth),
            millis,
        } => {
            format!("go movetime {millis} depth {}", max_depth.max(1))
        }
        SearchBudget::Movetime {
            max_depth: None,
            millis,
        } => format!("go movetime {millis}"),
    }
}

fn search_timeout(budget: SearchBudget) -> Duration {
    match budget {
        SearchBudget::Depth(_) => USI_DEPTH_SEARCH_TIMEOUT,
        SearchBudget::Movetime { millis, .. } => {
            Duration::from_millis(u64::from(millis)) + USI_SEARCH_TIMEOUT_GRACE
        }
    }
}

fn terminal_winner(board: &Board, a_color: Color) -> Option<Seat> {
    (board.status() != GameStatus::Ongoing).then_some(if board.side_to_move() == a_color {
        Seat::B
    } else {
        Seat::A
    })
}

fn play_self_play_game(
    game_index: u32,
    args: &SelfPlayArgs,
    base_board: &Board,
    openings: Option<&[OpeningPosition]>,
    engine_a: &EngineConfig,
    engine_b: &EngineConfig,
) -> Result<GameResult> {
    let pair_index = game_index / 2;
    let (mut board, opening) = game_opening(base_board, openings, args, pair_index)?;
    let start_sfen = board.to_string();
    let a_color = if game_index % 2 == 0 {
        Color::Black
    } else {
        Color::White
    };
    let mut winner = None;
    let mut plies = 0;
    let mut total_nodes = 0;
    let mut total_elapsed_ms = 0.0;
    let mut a_total_nodes = 0;
    let mut a_total_elapsed_ms = 0.0;
    let mut b_total_nodes = 0;
    let mut b_total_elapsed_ms = 0.0;
    let mut moves = Vec::new();
    let mut runtime_a = GameEngine::start(engine_a)
        .map_err(|err| anyhow!("failed to start engine A in game {}: {err}", game_index + 1))?;
    let mut runtime_b = GameEngine::start(engine_b)
        .map_err(|err| anyhow!("failed to start engine B in game {}: {err}", game_index + 1))?;

    for ply in 0..args.max_plies {
        if let Some(seat) = terminal_winner(&board, a_color) {
            winner = Some(seat);
            break;
        }

        let config = if board.side_to_move() == a_color {
            engine_a
        } else {
            engine_b
        };
        let runtime = if board.side_to_move() == a_color {
            &mut runtime_a
        } else {
            &mut runtime_b
        };
        let current_sfen = board.to_string();
        let summary = runtime.search(&board, config.budget).map_err(|err| {
            anyhow!(
                "search failed in game {} on ply {} with {} to move (engine {}, sfen: {}, moves: {}): {err}",
                game_index + 1,
                ply + 1,
                color_name(board.side_to_move()),
                config.label,
                current_sfen,
                moves.join(" ")
            )
        })?;
        total_nodes += summary.total_nodes;
        total_elapsed_ms += summary.elapsed_ms;
        if board.side_to_move() == a_color {
            a_total_nodes += summary.total_nodes;
            a_total_elapsed_ms += summary.elapsed_ms;
        } else {
            b_total_nodes += summary.total_nodes;
            b_total_elapsed_ms += summary.elapsed_ms;
        }
        let Some(best_move) = summary.best_move else {
            winner = Some(if config.label == "A" {
                Seat::B
            } else {
                Seat::A
            });
            break;
        };
        let mv = Move::from_str(&best_move)
            .map_err(|err| anyhow!("engine returned invalid move {best_move}: {err}"))?;
        board.try_play(mv).map_err(|_| {
            anyhow!(
                "engine {} returned illegal move {} in game {} on ply {} with {} to move (sfen: {}, moves: {})",
                config.label,
                best_move,
                game_index + 1,
                ply + 1,
                color_name(board.side_to_move()),
                current_sfen,
                moves.join(" ")
            )
        })?;
        moves.push(best_move);
        plies = ply + 1;
    }
    if winner.is_none() {
        winner = terminal_winner(&board, a_color);
    }

    Ok(GameResult {
        a_color,
        winner,
        plies,
        total_nodes,
        total_elapsed_ms,
        a_breakdown: search_breakdown(a_total_nodes, a_total_elapsed_ms),
        b_breakdown: search_breakdown(b_total_nodes, b_total_elapsed_ms),
        start_sfen,
        opening,
        moves,
    })
}

fn legal_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|piece_moves| {
        moves.extend(piece_moves);
        false
    });
    moves.sort_unstable_by_key(ToString::to_string);
    moves
}

fn color_name(color: Color) -> &'static str {
    match color {
        Color::Black => "black",
        Color::White => "white",
    }
}

fn seat_name(seat: Seat) -> &'static str {
    match seat {
        Seat::A => "A",
        Seat::B => "B",
    }
}

fn result_name(winner: Option<Seat>) -> &'static str {
    match winner {
        Some(Seat::A) => "a-win",
        Some(Seat::B) => "b-win",
        None => "draw",
    }
}

fn score_rate_to_elo(score_rate: f64) -> f64 {
    let score_rate = score_rate.clamp(0.01, 0.99);
    400.0 * (score_rate / (1.0 - score_rate)).log10()
}

fn search_breakdown(total_nodes: u64, total_elapsed_ms: f64) -> SearchBreakdown {
    let aggregate_nps = if total_elapsed_ms > 0.0 {
        total_nodes as f64 / (total_elapsed_ms / 1_000.0)
    } else {
        0.0
    };

    SearchBreakdown {
        total_nodes,
        total_elapsed_ms,
        aggregate_nps,
    }
}

fn rating_summary(stats: &MatchStats, games: u32) -> RatingSummary {
    let a_score = stats.a_wins as f64 + 0.5 * stats.draws as f64;
    let denom = f64::from(games.max(1));
    let score_rate = (a_score / denom).clamp(0.0, 1.0);
    let bounded_rate = score_rate.clamp(0.01, 0.99);
    let se = (bounded_rate * (1.0 - bounded_rate) / denom).sqrt();
    let lower_rate = (bounded_rate - 1.96 * se).clamp(0.01, 0.99);
    let upper_rate = (bounded_rate + 1.96 * se).clamp(0.01, 0.99);
    let total_breakdown = search_breakdown(stats.total_nodes, stats.total_elapsed_ms);
    let mut warnings = Vec::new();
    if games < 30 {
        warnings.push("low sample: Elo and confidence interval are approximate".to_string());
    }
    if stats.draws == games && games > 0 {
        warnings.push("all games were drawn; estimate is uninformative".to_string());
    }

    RatingSummary {
        games,
        a_wins: stats.a_wins,
        b_wins: stats.b_wins,
        draws: stats.draws,
        decided_games: stats.a_wins + stats.b_wins,
        a_score,
        score_rate,
        approx_elo: score_rate_to_elo(bounded_rate),
        approx_elo_95_ci: [score_rate_to_elo(lower_rate), score_rate_to_elo(upper_rate)],
        avg_plies: stats.total_plies as f64 / denom,
        total_nodes: stats.total_nodes,
        total_elapsed_ms: stats.total_elapsed_ms,
        aggregate_nps: total_breakdown.aggregate_nps,
        a_breakdown: search_breakdown(
            stats.a_breakdown.total_nodes,
            stats.a_breakdown.total_elapsed_ms,
        ),
        b_breakdown: search_breakdown(
            stats.b_breakdown.total_nodes,
            stats.b_breakdown.total_elapsed_ms,
        ),
        warnings,
    }
}

fn stats_from_summary(summary: &RatingSummary) -> MatchStats {
    MatchStats {
        a_wins: summary.a_wins,
        b_wins: summary.b_wins,
        draws: summary.draws,
        total_nodes: summary.total_nodes,
        total_elapsed_ms: summary.total_elapsed_ms,
        total_plies: (summary.avg_plies * f64::from(summary.games)).round() as u64,
        a_breakdown: search_breakdown(
            summary.a_breakdown.total_nodes,
            summary.a_breakdown.total_elapsed_ms,
        ),
        b_breakdown: search_breakdown(
            summary.b_breakdown.total_nodes,
            summary.b_breakdown.total_elapsed_ms,
        ),
    }
}

fn normalized_report_command_value(command: serde_json::Value) -> Result<serde_json::Value> {
    let mut object = command
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow!("report command is not a JSON object"))?;
    object.remove("games");
    object.remove("threads");
    Ok(serde_json::Value::Object(object))
}

fn compare_existing_report_field(
    report_path: &Path,
    field_name: &str,
    existing: &serde_json::Value,
    expected: &serde_json::Value,
) -> Result<()> {
    if existing == expected {
        return Ok(());
    }
    let existing_json =
        serde_json::to_string(existing).context("serialize existing report field for error")?;
    let expected_json =
        serde_json::to_string(expected).context("serialize expected report field for error")?;
    bail!(
        "cannot merge {} because existing report {} does not match current self-play configuration (existing={}, current={})",
        report_path.display(),
        field_name,
        existing_json,
        expected_json
    );
}

fn validate_existing_report_merge_compatibility(
    report_path: &Path,
    value: &serde_json::Value,
    expected_ruleset: &str,
    expected_command: &ReportCommand,
    expected_engines: &[ReportEngine],
) -> Result<()> {
    let existing_ruleset = value
        .get("ruleset")
        .ok_or_else(|| anyhow!("existing report {} has no ruleset", report_path.display()))?;
    compare_existing_report_field(
        report_path,
        "ruleset",
        existing_ruleset,
        &serde_json::Value::String(expected_ruleset.to_string()),
    )?;

    let existing_command = value
        .get("command")
        .ok_or_else(|| anyhow!("existing report {} has no command", report_path.display()))?
        .clone();
    let expected_command = normalized_report_command_value(
        serde_json::to_value(expected_command).context("serialize expected report command")?,
    )?;
    let existing_command = normalized_report_command_value(existing_command)?;
    compare_existing_report_field(report_path, "command", &existing_command, &expected_command)?;

    let existing_engines = value
        .get("engines")
        .ok_or_else(|| anyhow!("existing report {} has no engines", report_path.display()))?;
    let expected_engines =
        serde_json::to_value(expected_engines).context("serialize expected report engines")?;
    compare_existing_report_field(report_path, "engines", existing_engines, &expected_engines)?;

    Ok(())
}

fn load_existing_report_stats(
    report_path: &Path,
    expected_ruleset: &str,
    expected_command: &ReportCommand,
    expected_engines: &[ReportEngine],
) -> Result<(MatchStats, u32)> {
    let bytes = fs::read(report_path).with_context(|| format!("read {}", report_path.display()))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse {}", report_path.display()))?;
    validate_existing_report_merge_compatibility(
        report_path,
        &value,
        expected_ruleset,
        expected_command,
        expected_engines,
    )?;
    let summary_value = value
        .get("summary")
        .ok_or_else(|| anyhow!("existing report {} has no summary", report_path.display()))?;
    let summary: RatingSummary = serde_json::from_value(summary_value.clone())
        .with_context(|| format!("parse summary in {}", report_path.display()))?;
    Ok((stats_from_summary(&summary), summary.games))
}

fn prompt_report_conflict_action(report_dir: &Path) -> Result<ReportConflictAction> {
    loop {
        println!(
            "{} already contains a self-play report. What to do?",
            report_dir.display()
        );
        println!("1. Abort");
        println!("2. Self-play more and merge result");
        println!("3. Discard saved and override with new result");
        print!("choice [1/2/3]> ");
        io::stdout().flush()?;

        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            bail!("self-play report already exists and no conflict choice was provided");
        }
        match line.trim() {
            "1" | "abort" | "Abort" => return Ok(ReportConflictAction::Abort),
            "2" | "merge" | "Merge" => return Ok(ReportConflictAction::Merge),
            "3" | "overwrite" | "override" | "Overwrite" | "Override" => {
                return Ok(ReportConflictAction::Overwrite);
            }
            _ => println!("enter 1, 2, or 3"),
        }
    }
}

fn prepare_self_play_report_output(
    report_dir: Option<&Path>,
    expected_ruleset: &str,
    expected_command: &ReportCommand,
    expected_engines: &[ReportEngine],
) -> Result<Option<SelfPlayReportOutput>> {
    let Some(report_dir) = report_dir else {
        return Ok(None);
    };

    let report_path = report_dir.join(SELF_PLAY_REPORT_FILE);
    let games_path = report_dir.join(SELF_PLAY_GAMES_FILE);
    let has_existing_report = report_path.exists();
    let has_existing_games = games_path.exists();
    let action = if has_existing_report || has_existing_games {
        prompt_report_conflict_action(report_dir)?
    } else {
        ReportConflictAction::Overwrite
    };

    match action {
        ReportConflictAction::Abort => bail!("self-play report already exists"),
        ReportConflictAction::Merge => Ok(Some(prepare_self_play_report_merge_output(
            report_dir,
            &report_path,
            &games_path,
            has_existing_report,
            has_existing_games,
            expected_ruleset,
            expected_command,
            expected_engines,
        )?)),
        ReportConflictAction::Overwrite => {
            fs::create_dir_all(report_dir)?;
            let games_writer = fs::File::create(&games_path)
                .with_context(|| format!("create {}", games_path.display()))?;
            Ok(Some(SelfPlayReportOutput {
                games_writer,
                report_path,
                existing_stats: MatchStats::default(),
                game_index_offset: 0,
            }))
        }
    }
}

fn prepare_self_play_report_merge_output(
    report_dir: &Path,
    report_path: &Path,
    games_path: &Path,
    has_existing_report: bool,
    has_existing_games: bool,
    expected_ruleset: &str,
    expected_command: &ReportCommand,
    expected_engines: &[ReportEngine],
) -> Result<SelfPlayReportOutput> {
    if !has_existing_report {
        bail!(
            "cannot merge {} because {} is missing",
            report_dir.display(),
            SELF_PLAY_REPORT_FILE
        );
    }
    if !has_existing_games {
        bail!(
            "cannot merge {} because {} is missing",
            report_dir.display(),
            SELF_PLAY_GAMES_FILE
        );
    }

    fs::create_dir_all(report_dir)?;
    let (existing_stats, game_index_offset) = load_existing_report_stats(
        report_path,
        expected_ruleset,
        expected_command,
        expected_engines,
    )?;
    let games_writer = fs::OpenOptions::new()
        .append(true)
        .open(games_path)
        .with_context(|| format!("open {}", games_path.display()))?;
    Ok(SelfPlayReportOutput {
        games_writer,
        report_path: report_path.to_path_buf(),
        existing_stats,
        game_index_offset,
    })
}

fn game_json_record(game_index: u32, result: &GameResult) -> GameJsonRecord {
    GameJsonRecord {
        schema: "haitaka-self-play-game",
        schema_version: 1,
        game_index: game_index + 1,
        pair_index: game_index / 2,
        a_color: color_name(result.a_color).to_string(),
        b_color: color_name(!result.a_color).to_string(),
        opening: result.opening.clone(),
        start_sfen: result.start_sfen.clone(),
        moves: result.moves.clone(),
        result: result_name(result.winner).to_string(),
        winner: result.winner.map(seat_name).map(str::to_string),
        plies: result.plies,
        total_nodes: result.total_nodes,
        total_elapsed_ms: result.total_elapsed_ms,
        a_breakdown: result.a_breakdown,
        b_breakdown: result.b_breakdown,
        failure_state: None,
    }
}

fn report_engine(engine: &EngineConfig, archive: Option<&ArchiveLaunch>) -> ReportEngine {
    match &engine.evaluator {
        EngineEvaluator::Handcrafted => ReportEngine {
            label: engine.label,
            kind: "handcrafted".to_string(),
            budget: describe_budget(engine.budget),
            command: None,
            args: Vec::new(),
            nnue: None,
            archive_path: None,
            archive: None,
        },
        EngineEvaluator::Nnue { path, .. } => ReportEngine {
            label: engine.label,
            kind: "nnue".to_string(),
            budget: describe_budget(engine.budget),
            command: None,
            args: Vec::new(),
            nnue: Some(path.display().to_string()),
            archive_path: None,
            archive: None,
        },
        EngineEvaluator::External { path, args } => {
            if let Some(archive) = archive {
                ReportEngine {
                    label: engine.label,
                    kind: "archive-usi".to_string(),
                    budget: describe_budget(engine.budget),
                    command: Some(archive.manifest.runtime.executable.clone()),
                    args: archive_report_args(archive, args),
                    nnue: None,
                    archive_path: Some(archive.source_archive_path.display().to_string()),
                    archive: Some(archive.manifest.clone()),
                }
            } else {
                ReportEngine {
                    label: engine.label,
                    kind: "external-usi".to_string(),
                    budget: describe_budget(engine.budget),
                    command: Some(path.display().to_string()),
                    args: args.clone(),
                    nnue: None,
                    archive_path: None,
                    archive: None,
                }
            }
        }
    }
}

fn archive_report_args(archive: &ArchiveLaunch, launch_args: &[String]) -> Vec<String> {
    let user_args = launch_args
        .strip_prefix(archive.engine_args.as_slice())
        .unwrap_or_default();
    archive
        .report_engine_args
        .iter()
        .chain(user_args)
        .cloned()
        .collect()
}

fn report_command(args: &SelfPlayArgs, threads: usize) -> ReportCommand {
    ReportCommand {
        games: args.games,
        threads,
        a_depth: args.a_depth,
        b_depth: args.b_depth,
        movetime_ms: args.movetime_ms,
        sfen: args.sfen.clone(),
        openings: args
            .openings
            .as_ref()
            .map(|path| path.display().to_string()),
        opening_order: args.opening_order,
        opening_random_plies: args.opening_random_plies,
        seed: args.seed,
        max_plies: args.max_plies,
    }
}

fn unix_timestamp_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?
        .as_secs())
}

fn self_play_report(
    args: &SelfPlayArgs,
    threads: usize,
    engines: Vec<ReportEngine>,
    summary: RatingSummary,
) -> Result<SelfPlayReport> {
    Ok(SelfPlayReport {
        schema: "haitaka-self-play-report",
        schema_version: 1,
        generated_at_unix_seconds: unix_timestamp_seconds()?,
        package: ReportPackage {
            name: env!("CARGO_PKG_NAME"),
            version: env!("CARGO_PKG_VERSION"),
        },
        git: ReportGit {
            commit: git_commit(),
            dirty: git_dirty(),
        },
        ruleset: default_ruleset().to_string(),
        command: report_command(args, threads),
        engines,
        summary,
    })
}

fn write_json_file<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut file = fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
    serde_json::to_writer_pretty(&mut file, value)
        .with_context(|| format!("write {}", path.display()))?;
    writeln!(file).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn append_jsonl<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<()> {
    serde_json::to_writer(&mut *writer, value).context("write game JSONL record")?;
    writeln!(writer).context("write game JSONL newline")?;
    Ok(())
}

fn play(args: PlayArgs) -> Result<()> {
    let mut board = parse_board(args.sfen.as_deref())?;
    for ply in 0..args.max_plies {
        println!();
        println!("ply: {} side: {:?}", ply + 1, board.side_to_move());
        println!("sfen: {board}");

        if board.status() != GameStatus::Ongoing {
            println!("game over: side to move has no legal moves");
            return Ok(());
        }

        let human_to_move = match args.human {
            HumanSide::Black => board.side_to_move() == Color::Black,
            HumanSide::White => board.side_to_move() == Color::White,
            HumanSide::None => false,
        };

        if human_to_move {
            let mv = read_human_move(&board)?;
            board
                .try_play(mv)
                .map_err(|_| anyhow!("illegal move: {mv}"))?;
        } else {
            let summary = haitaka_wasm::search_board_impl_handcrafted(&board, args.depth)
                .map_err(|err| anyhow!("search failed: {err}"))?;
            let Some(best_move) = summary.best_move else {
                println!("engine has no legal move");
                return Ok(());
            };
            println!(
                "engine: move={} score={:?} depth={} nodes={} nps={:.0} elapsed_ms={:.3}",
                best_move,
                summary.best_score,
                args.depth,
                summary.states,
                summary.nps,
                summary.elapsed_ms
            );
            let mv = Move::from_str(&best_move)
                .map_err(|err| anyhow!("engine returned invalid move {best_move}: {err}"))?;
            board.play(mv);
            if args.human == HumanSide::None {
                println!("sfen: {board}");
                return Ok(());
            }
        }
    }

    println!("stopped after {} plies", args.max_plies);
    Ok(())
}

fn read_human_move(board: &Board) -> Result<Move> {
    let moves = legal_moves(board);
    println!(
        "legal moves: {}",
        moves
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    );
    loop {
        print!("move> ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let line = line.trim();
        if line == "quit" || line == "exit" {
            bail!("session stopped by user");
        }
        let Ok(mv) = Move::from_str(line) else {
            println!("enter a USI move such as 7g7f or P*5e");
            continue;
        };
        if board.is_legal(mv) {
            return Ok(mv);
        }
        println!("illegal move: {mv}");
    }
}

fn usi(args: UsiArgs) -> Result<()> {
    let mut session = UsiSession::new(ENGINE_NAME, args.movetime_max_depth);
    if args.eval == EngineEvalKind::Nnue {
        let path = args
            .nnue
            .as_ref()
            .ok_or_else(|| anyhow!("USI engine uses NNUE but no NNUE path was provided"))?;
        let bytes =
            fs::read(path).with_context(|| format!("failed to read NNUE {}", path.display()))?;
        session
            .load_nnue(&bytes)
            .map_err(|err| anyhow!("failed to load NNUE {}: {err}", path.display()))?;
    }
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line.context("failed to read USI command")?;
        let command = line.trim();
        if command == "quit" {
            break;
        }
        for output in session.handle_line(command) {
            writeln!(stdout, "{output}")?;
        }
        stdout.flush()?;
    }

    Ok(())
}

/// Number of lines in the live status block, used to move the cursor back up.
const STATUS_LINES: usize = 8;

/// Redraw the status block in place. On every call after the first, the cursor
/// is moved up over the previous block so the same lines are overwritten.
///
/// When stdout is not a terminal (redirected to a file or running under CI), the
/// in-place redraw is skipped and the block is printed as plain lines, so logs
/// stay readable and are not corrupted by ANSI cursor/clear escapes.
fn render_status(block: &str, first: bool) {
    if !io::stdout().is_terminal() {
        println!("{block}");
        return;
    }
    let mut out = String::new();
    if !first {
        out.push_str(&format!("\x1b[{STATUS_LINES}A"));
    }
    for line in block.lines() {
        // Clear the whole line before rewriting so shorter lines don't leave
        // stale characters behind.
        out.push_str("\x1b[2K");
        out.push_str(line);
        out.push('\n');
    }
    print!("{out}");
    let _ = io::stdout().flush();
}

fn self_play(args: SelfPlayArgs) -> Result<()> {
    let mut cleanup_dirs = Vec::new();
    let result = self_play_inner(args, &mut cleanup_dirs);
    let cleanup = cleanup_archive_dirs(&cleanup_dirs);
    if result.is_ok() {
        cleanup?;
    } else {
        let _ = cleanup;
    }
    result
}

#[cfg(unix)]
extern "C" fn handle_sigint(_: libc::c_int) {
    SELF_PLAY_INTERRUPTED.store(true, Ordering::SeqCst);
}

#[cfg(unix)]
fn install_self_play_interrupt_handler() -> Result<()> {
    SELF_PLAY_INTERRUPTED.store(false, Ordering::SeqCst);
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handle_sigint as *const () as libc::sighandler_t;
        action.sa_flags = 0;
        libc::sigemptyset(&mut action.sa_mask);
        if libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut()) != 0 {
            bail!("failed to install SIGINT handler");
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn install_self_play_interrupt_handler() -> Result<()> {
    SELF_PLAY_INTERRUPTED.store(false, Ordering::SeqCst);
    Ok(())
}

fn self_play_inner(args: SelfPlayArgs, cleanup_dirs: &mut Vec<PathBuf>) -> Result<()> {
    let base_board = parse_board(args.sfen.as_deref())?;
    let openings = args
        .openings
        .as_ref()
        .map(|path| load_opening_suite(path))
        .transpose()?;
    validate_engine_source("A", args.a_engine.as_ref(), args.a_engine_archive.as_ref())?;
    validate_engine_source("B", args.b_engine.as_ref(), args.b_engine_archive.as_ref())?;
    let a_archive = args
        .a_engine_archive
        .as_ref()
        .map(|path| extract_engine_archive(path))
        .transpose()?;
    let b_archive = args
        .b_engine_archive
        .as_ref()
        .map(|path| extract_engine_archive(path))
        .transpose()?;
    if let Some(archive) = a_archive.as_ref() {
        cleanup_dirs.push(archive.extraction_dir.clone());
    }
    if let Some(archive) = b_archive.as_ref() {
        cleanup_dirs.push(archive.extraction_dir.clone());
    }
    let mut a_engine_args = args.a_engine_args.clone();
    let a_engine_path = if let Some(archive) = a_archive.as_ref() {
        let mut archive_args = archive.engine_args.clone();
        archive_args.extend(a_engine_args);
        a_engine_args = archive_args;
        Some(archive.engine_path.as_path())
    } else {
        args.a_engine.as_deref()
    };
    let mut b_engine_args = args.b_engine_args.clone();
    let b_engine_path = if let Some(archive) = b_archive.as_ref() {
        let mut archive_args = archive.engine_args.clone();
        archive_args.extend(b_engine_args);
        b_engine_args = archive_args;
        Some(archive.engine_path.as_path())
    } else {
        args.b_engine.as_deref()
    };
    let a_budget = self_play_budget(DEFAULT_SELF_PLAY_A_DEPTH, args.a_depth, args.movetime_ms)?;
    let b_budget = self_play_budget(DEFAULT_SELF_PLAY_B_DEPTH, args.b_depth, args.movetime_ms)?;
    let engine_a = engine_config(
        "A",
        a_budget,
        args.a_eval,
        args.nnue.as_deref(),
        args.a_nnue.as_deref(),
        a_engine_path,
        &a_engine_args,
    )?;
    let engine_b = engine_config(
        "B",
        b_budget,
        args.b_eval,
        args.nnue.as_deref(),
        args.b_nnue.as_deref(),
        b_engine_path,
        &b_engine_args,
    )?;
    let report_engines = vec![
        report_engine(&engine_a, a_archive.as_ref()),
        report_engine(&engine_b, b_archive.as_ref()),
    ];
    let threads = resolve_self_play_threads(args.threads, args.games);
    let expected_report_command = report_command(&args, threads);
    let mut report_output = prepare_self_play_report_output(
        args.report_dir.as_deref(),
        default_ruleset(),
        &expected_report_command,
        &report_engines,
    )?;
    let game_index_offset = report_output
        .as_ref()
        .map_or(0, |output| output.game_index_offset);
    if game_index_offset > u32::MAX - args.games {
        bail!("merged game count would overflow u32");
    }
    let total_target_games = game_index_offset + args.games;
    let mut stats = report_output
        .as_ref()
        .map_or_else(MatchStats::default, |output| output.existing_stats.clone());
    let start = Instant::now();

    println!("{}", describe_engine(&engine_a));
    println!("{}", describe_engine(&engine_b));
    println!("self-play threads={threads}");
    if let Some(path) = args.report_dir.as_ref() {
        println!(
            "report dir={} files={}, {}",
            path.display(),
            SELF_PLAY_REPORT_FILE,
            SELF_PLAY_GAMES_FILE
        );
        if game_index_offset > 0 {
            println!("merging with existing games={game_index_offset}");
        }
    }
    if let Some(path) = args.openings.as_ref() {
        println!(
            "opening suite={} positions={} order={:?}",
            path.display(),
            openings.as_ref().map_or(0, Vec::len),
            args.opening_order
        );
    }
    if args.opening_random_plies > 0 {
        println!(
            "paired random opening plies={} seed={}",
            args.opening_random_plies, args.seed
        );
    }

    install_self_play_interrupt_handler()?;
    let next_game = AtomicU32::new(0);
    let (tx, rx) = mpsc::channel();
    let mut completed = 0_u32;

    thread::scope(|scope| -> Result<()> {
        for _ in 0..threads {
            let tx = tx.clone();
            let args = &args;
            let base_board = &base_board;
            let openings = openings.as_deref();
            let engine_a = &engine_a;
            let engine_b = &engine_b;
            let next_game = &next_game;
            let game_index_offset = game_index_offset;

            scope.spawn(move || {
                loop {
                    if SELF_PLAY_INTERRUPTED.load(Ordering::SeqCst) {
                        break;
                    }
                    let game_index = next_game.fetch_add(1, Ordering::Relaxed);
                    if game_index >= args.games {
                        break;
                    }
                    let effective_game_index = game_index_offset + game_index;
                    let result = play_self_play_game(
                        effective_game_index,
                        args,
                        base_board,
                        openings,
                        engine_a,
                        engine_b,
                    );
                    if tx.send((effective_game_index, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        while completed < args.games {
            let (game_index, result) = match rx.recv() {
                Ok(message) => message,
                Err(err) => {
                    if SELF_PLAY_INTERRUPTED.load(Ordering::SeqCst) {
                        break;
                    }
                    return Err(anyhow!(
                        "self-play worker exited before reporting all games: {err}"
                    ));
                }
            };
            let result = match result {
                Ok(result) => result,
                Err(err) => {
                    SELF_PLAY_INTERRUPTED.store(true, Ordering::SeqCst);
                    return Err(err);
                }
            };
            let outcome = match result.winner {
                Some(Seat::A) => "A win",
                Some(Seat::B) => "B win",
                None => "draw",
            };

            completed += 1;
            stats.total_nodes += result.total_nodes;
            stats.total_elapsed_ms += result.total_elapsed_ms;
            stats.total_plies += u64::from(result.plies);
            stats.a_breakdown.total_nodes += result.a_breakdown.total_nodes;
            stats.a_breakdown.total_elapsed_ms += result.a_breakdown.total_elapsed_ms;
            stats.b_breakdown.total_nodes += result.b_breakdown.total_nodes;
            stats.b_breakdown.total_elapsed_ms += result.b_breakdown.total_elapsed_ms;
            match result.winner {
                Some(Seat::A) => stats.a_wins += 1,
                Some(Seat::B) => stats.b_wins += 1,
                None => stats.draws += 1,
            }
            if let Some(output) = report_output.as_mut() {
                append_jsonl(
                    &mut output.games_writer,
                    &game_json_record(game_index, &result),
                )?;
            }

            let elapsed = start.elapsed().as_secs_f64();
            let remaining = args.games.saturating_sub(completed);
            let eta = if completed == 0 {
                0.0
            } else {
                elapsed * f64::from(remaining) / f64::from(completed)
            };

            let total_completed = game_index_offset + completed;
            let summary = rating_summary(&stats, total_completed);

            let block = format!(
                "game ({game}) done (new {completed}/{new_total}): A({a_color:?}) vs B({b_color:?}) \
                 plies={plies} result={outcome} eta={eta}\n\
                 games: {total_completed}/{total_target} (new {completed}/{new_total})\n\
                 score: A {a_wins} - B {b_wins} - draws {draws}\n\
                 decided games: {decided}\n\
                 approx elo A-B: {elo:.1} (95% CI {elo_low:.1}..{elo_high:.1})\n\
                 avg plies: {avg:.1}\n\
                total nodes: {nodes} (A {a_nodes}, B {b_nodes})\n\
                 aggregate nps: {nps:.0} (A {a_nps:.0}, B {b_nps:.0})",
                game = game_index + 1,
                a_color = result.a_color,
                b_color = !result.a_color,
                plies = result.plies,
                eta = format_eta(eta),
                total_completed = total_completed,
                total_target = total_target_games,
                new_total = args.games,
                a_wins = stats.a_wins,
                b_wins = stats.b_wins,
                draws = stats.draws,
                decided = summary.decided_games,
                elo = summary.approx_elo,
                elo_low = summary.approx_elo_95_ci[0],
                elo_high = summary.approx_elo_95_ci[1],
                avg = summary.avg_plies,
                nodes = summary.total_nodes,
                nps = summary.aggregate_nps,
                a_nodes = summary.a_breakdown.total_nodes,
                b_nodes = summary.b_breakdown.total_nodes,
                a_nps = summary.a_breakdown.aggregate_nps,
                b_nps = summary.b_breakdown.aggregate_nps,
            );
            render_status(&block, completed == 1);
        }

        Ok(())
    })?;

    let final_completed = game_index_offset + completed;
    if let Some(output) = report_output.as_mut() {
        output.games_writer.flush().context("flush game JSONL")?;
        let mut summary = rating_summary(&stats, final_completed);
        if SELF_PLAY_INTERRUPTED.load(Ordering::SeqCst) {
            summary.warnings.push(format!(
                "self-play interrupted after {completed} newly completed games; requested {}",
                args.games
            ));
        }
        let report = self_play_report(&args, threads, report_engines, summary)?;
        write_json_file(&output.report_path, &report)?;
    }
    if SELF_PLAY_INTERRUPTED.load(Ordering::SeqCst) {
        bail!(
            "self-play interrupted after {completed}/{} newly requested games; wrote partial report when --report-dir was set",
            args.games
        );
    }

    Ok(())
}

fn archive_engine(args: ArchiveEngineArgs) -> Result<()> {
    let staging = unique_target_dir("engine-archive-staging")?;
    let result = archive_engine_in_staging(args, &staging);
    if staging.exists() {
        let cleanup =
            fs::remove_dir_all(&staging).with_context(|| format!("remove {}", staging.display()));
        if result.is_ok() {
            cleanup?;
        }
    }
    result
}

fn archive_engine_in_staging(args: ArchiveEngineArgs, staging: &Path) -> Result<()> {
    if staging.exists() {
        fs::remove_dir_all(staging).with_context(|| format!("remove {}", staging.display()))?;
    }
    fs::create_dir_all(staging.join("bin"))?;

    let archive_binary = staging.join(ENGINE_ARCHIVE_BIN_PATH);
    fs::copy(&args.binary, &archive_binary)
        .with_context(|| format!("copy {}", args.binary.display()))?;
    if let Ok(metadata) = fs::metadata(&args.binary) {
        let _ = fs::set_permissions(&archive_binary, metadata.permissions());
    }
    let executable_sha256 = file_sha256(&archive_binary)?;

    let nnue = if let Some(path) = args.nnue.as_ref() {
        fs::create_dir_all(staging.join("nnue"))?;
        fs::copy(path, staging.join(ENGINE_ARCHIVE_NNUE_PATH))
            .with_context(|| format!("copy {}", path.display()))?;
        Some(NativeArchiveNnue {
            path: ENGINE_ARCHIVE_NNUE_PATH.to_string(),
            sha256: file_sha256(path)?,
        })
    } else {
        None
    };

    let manifest = archive_engine_manifest(&args, executable_sha256, nnue);
    fs::write(
        staging.join(ENGINE_ARCHIVE_MANIFEST_FILE),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    fs::write(
        staging.join("README.txt"),
        "Haitaka native USI engine archive. See haitaka-engine-archive.json for metadata.\n",
    )?;

    if let Some(parent) = args.output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let output = args.output.canonicalize().unwrap_or(args.output);
    let status = ProcessCommand::new("tar")
        .arg("-czf")
        .arg(&output)
        .arg("-C")
        .arg(staging)
        .arg(".")
        .status()
        .context("failed to run tar")?;
    if !status.success() {
        bail!("tar failed with status {status}");
    }

    println!("wrote {}", output.display());
    Ok(())
}

fn archive_engine_manifest(
    args: &ArchiveEngineArgs,
    executable_sha256: String,
    nnue: Option<NativeArchiveNnue>,
) -> NativeEngineArchiveManifest {
    NativeEngineArchiveManifest {
        schema: "haitaka-engine-archive".to_string(),
        schema_version: 1,
        engine: NativeArchiveEngine {
            name: args.engine_name.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            commit: git_commit(),
            dirty: git_dirty(),
            ruleset: args.ruleset.clone(),
            features: active_feature_names(),
            build_profile: args
                .profile
                .unwrap_or_else(|| infer_build_profile(&args.binary))
                .as_str()
                .to_string(),
            target: args
                .target
                .clone()
                .unwrap_or_else(default_target_identifier),
        },
        runtime: NativeArchiveRuntime {
            protocol: "usi".to_string(),
            protocol_version: "minimal-v1".to_string(),
            executable: ENGINE_ARCHIVE_BIN_PATH.to_string(),
            sha256: Some(executable_sha256),
        },
        nnue,
    }
}

fn extract_engine_archive(path: &Path) -> Result<ArchiveLaunch> {
    let extraction_dir = unique_target_dir("engine-archive-extract")?;
    let result = extract_engine_archive_in_dir(path, &extraction_dir);
    if result.is_err() && extraction_dir.exists() {
        let _ = fs::remove_dir_all(&extraction_dir);
    }
    result
}

fn extract_engine_archive_in_dir(path: &Path, extraction_dir: &Path) -> Result<ArchiveLaunch> {
    fs::create_dir_all(extraction_dir)?;
    let status = ProcessCommand::new("tar")
        .arg("-xzf")
        .arg(path)
        .arg("-C")
        .arg(extraction_dir)
        .status()
        .with_context(|| format!("failed to extract engine archive {}", path.display()))?;
    if !status.success() {
        bail!(
            "tar failed extracting {} with status {status}",
            path.display()
        );
    }

    let manifest_path = extraction_dir.join(ENGINE_ARCHIVE_MANIFEST_FILE);
    let manifest_bytes = fs::read(&manifest_path)
        .with_context(|| format!("read archive manifest {}", manifest_path.display()))?;
    let manifest: NativeEngineArchiveManifest = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("parse archive manifest {}", manifest_path.display()))?;
    if manifest.schema != "haitaka-engine-archive" || manifest.schema_version != 1 {
        bail!(
            "unsupported engine archive schema {} v{}",
            manifest.schema,
            manifest.schema_version
        );
    }
    if manifest.runtime.protocol != "usi" {
        bail!(
            "unsupported engine archive protocol {}",
            manifest.runtime.protocol
        );
    }

    let engine_path = extraction_dir.join(&manifest.runtime.executable);
    if !engine_path.is_file() {
        bail!(
            "archive executable {} does not exist",
            manifest.runtime.executable
        );
    }
    let expected_executable_sha256 = manifest.runtime.sha256.as_deref().ok_or_else(|| {
        anyhow!(
            "archive executable {} has no sha256 in manifest",
            manifest.runtime.executable
        )
    })?;
    let actual_executable_sha256 = file_sha256(&engine_path)?;
    if actual_executable_sha256 != expected_executable_sha256 {
        bail!(
            "archive executable {} sha256 mismatch: manifest has {}, extracted file has {}",
            manifest.runtime.executable,
            expected_executable_sha256,
            actual_executable_sha256
        );
    }

    let mut engine_args = vec!["usi".to_string()];
    let mut report_engine_args = vec!["usi".to_string()];
    if let Some(nnue) = manifest.nnue.as_ref() {
        let nnue_path = extraction_dir.join(&nnue.path);
        if !nnue_path.is_file() {
            bail!("archive NNUE {} does not exist", nnue.path);
        }
        let actual_sha256 = file_sha256(&nnue_path)?;
        if actual_sha256 != nnue.sha256 {
            bail!(
                "archive NNUE {} sha256 mismatch: manifest has {}, extracted file has {}",
                nnue.path,
                nnue.sha256,
                actual_sha256
            );
        }
        engine_args.extend([
            "--eval".to_string(),
            "nnue".to_string(),
            "--nnue".to_string(),
            nnue_path.display().to_string(),
        ]);
        report_engine_args.extend([
            "--eval".to_string(),
            "nnue".to_string(),
            "--nnue".to_string(),
            nnue.path.clone(),
        ]);
    }

    Ok(ArchiveLaunch {
        engine_path,
        engine_args,
        report_engine_args,
        extraction_dir: extraction_dir.to_path_buf(),
        source_archive_path: path.to_path_buf(),
        manifest,
    })
}

fn validate_engine_source(
    label: &str,
    raw_engine: Option<&PathBuf>,
    archive: Option<&PathBuf>,
) -> Result<()> {
    if raw_engine.is_some() && archive.is_some() {
        bail!(
            "{label} cannot use both --{label_lower}-engine and --{label_lower}-engine-archive",
            label_lower = label.to_ascii_lowercase()
        );
    }
    Ok(())
}

fn cleanup_archive_dirs(dirs: &[PathBuf]) -> Result<()> {
    for dir in dirs {
        if dir.exists() {
            fs::remove_dir_all(dir).with_context(|| format!("remove {}", dir.display()))?;
        }
    }
    Ok(())
}

fn unique_target_dir(prefix: &str) -> Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?
        .as_nanos();
    Ok(PathBuf::from(format!(
        "target/haitaka_cli/{}-{}-{nonce}",
        prefix,
        std::process::id()
    )))
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("read {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn git_dirty() -> bool {
    ProcessCommand::new("git")
        .arg("status")
        .arg("--porcelain")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| !output.stdout.is_empty())
}

fn active_feature_names() -> Vec<String> {
    // Derived from `default_ruleset()` so a native archive's advertised `features`
    // can never disagree with its `ruleset`. The variant features are mutually
    // exclusive, so this is exactly the one active rule (or "standard").
    vec![default_ruleset().to_string()]
}

fn infer_build_profile(binary: &Path) -> ArchiveBuildProfile {
    if binary
        .components()
        .any(|component| component.as_os_str() == "release")
    {
        ArchiveBuildProfile::Release
    } else if binary
        .components()
        .any(|component| component.as_os_str() == "debug")
    {
        ArchiveBuildProfile::Debug
    } else {
        ArchiveBuildProfile::Unknown
    }
}

fn default_target_identifier() -> String {
    format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
}

fn package(args: PackageArgs) -> Result<()> {
    let staging = package_staging_dir()?;
    let result = package_in_staging(args, &staging);
    if staging.exists() {
        let cleanup =
            fs::remove_dir_all(&staging).with_context(|| format!("remove {}", staging.display()));
        if result.is_ok() {
            cleanup?;
        }
    }
    result
}

fn package_in_staging(args: PackageArgs, staging: &Path) -> Result<()> {
    if staging.exists() {
        fs::remove_dir_all(&staging).with_context(|| format!("remove {}", staging.display()))?;
    }
    fs::create_dir_all(staging.join(ENGINE_DIR))?;

    copy_wasm_pack_artifacts(
        &args.wasm_dir,
        &staging.join(ENGINE_DIR),
        args.allow_missing_wasm,
    )?;

    let nnue = if let Some(path) = args.nnue.as_ref() {
        fs::copy(path, staging.join(NNUE_ARTIFACT_PATH))
            .with_context(|| format!("copy {}", path.display()))?;
        Some(NnueArtifact {
            path: NNUE_ARTIFACT_PATH,
            format: "nnue",
        })
    } else {
        None
    };

    let manifest = package_manifest(&args, nnue);
    fs::write(
        staging.join(MANIFEST_FILE),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    fs::write(
        staging.join("README.txt"),
        "Haitaka Variants Shogitter Engine Package v1. See shogitter-engine.json for metadata.\n",
    )?;

    if let Some(parent) = args.output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let output = args.output.canonicalize().unwrap_or(args.output);
    let status = ProcessCommand::new("tar")
        .arg("-czf")
        .arg(&output)
        .arg("-C")
        .arg(&staging)
        .arg(".")
        .status()
        .context("failed to run tar")?;
    if !status.success() {
        bail!("tar failed with status {status}");
    }

    println!("wrote {}", output.display());
    Ok(())
}

fn package_staging_dir() -> Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?
        .as_nanos();
    Ok(PathBuf::from(format!(
        "target/haitaka_cli/package-staging-{}-{nonce}",
        std::process::id()
    )))
}

fn copy_wasm_pack_artifacts(
    wasm_dir: &Path,
    engine_dir: &Path,
    allow_missing_wasm: bool,
) -> Result<()> {
    let missing_required = REQUIRED_WASM_FILES
        .iter()
        .filter(|file| !wasm_dir.join(file).is_file())
        .copied()
        .collect::<Vec<_>>();

    if !missing_required.is_empty() {
        if allow_missing_wasm {
            eprintln!(
                "warning: creating metadata-only package; missing required wasm artifacts in {}: {}",
                wasm_dir.display(),
                missing_required.join(", ")
            );
            return Ok(());
        }
        bail!(
            "missing required wasm artifacts in {}: {}; run wasm-pack build haitaka_wasm --target web --out-dir pkg --release",
            wasm_dir.display(),
            missing_required.join(", ")
        );
    }

    for file in REQUIRED_WASM_FILES.into_iter().chain(OPTIONAL_WASM_FILES) {
        let source = wasm_dir.join(file);
        if source.is_file() {
            fs::copy(&source, engine_dir.join(file))
                .with_context(|| format!("copy {}", source.display()))?;
        }
    }
    Ok(())
}

fn git_commit() -> String {
    ProcessCommand::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|commit| commit.trim().to_string())
        .filter(|commit| !commit.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_package_args(wasm_dir: PathBuf, output: PathBuf) -> PackageArgs {
        PackageArgs {
            output,
            wasm_dir,
            ruleset: default_ruleset().to_string(),
            rule_id: default_rule_id(),
            nnue: None,
            allow_missing_wasm: false,
        }
    }

    fn test_archive_args(
        binary: PathBuf,
        output: PathBuf,
        nnue: Option<PathBuf>,
    ) -> ArchiveEngineArgs {
        ArchiveEngineArgs {
            output,
            binary,
            nnue,
            ruleset: default_ruleset().to_string(),
            engine_name: ENGINE_NAME.to_string(),
            profile: Some(ArchiveBuildProfile::Debug),
            target: Some("test-target".to_string()),
        }
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("haitaka-cli-{name}-{}-{nonce}", std::process::id()))
    }

    fn test_report_command() -> ReportCommand {
        ReportCommand {
            games: 8,
            threads: 4,
            a_depth: Some(2),
            b_depth: Some(3),
            movetime_ms: None,
            sfen: Some(SFEN_STARTPOS.to_string()),
            openings: Some("fixtures/openings.sfen".to_string()),
            opening_order: OpeningOrder::Random,
            opening_random_plies: 2,
            seed: 123,
            max_plies: 200,
        }
    }

    fn test_report_engines() -> Vec<ReportEngine> {
        vec![
            ReportEngine {
                label: "A",
                kind: "handcrafted".to_string(),
                budget: "depth=2".to_string(),
                command: None,
                args: Vec::new(),
                nnue: None,
                archive_path: None,
                archive: None,
            },
            ReportEngine {
                label: "B",
                kind: "external-usi".to_string(),
                budget: "depth=3".to_string(),
                command: Some("/tmp/engine-b".to_string()),
                args: vec!["--fast".to_string()],
                nnue: None,
                archive_path: None,
                archive: None,
            },
        ]
    }

    fn test_native_archive_manifest() -> NativeEngineArchiveManifest {
        NativeEngineArchiveManifest {
            schema: "haitaka-engine-archive".to_string(),
            schema_version: 1,
            engine: NativeArchiveEngine {
                name: "Haitaka Test".to_string(),
                version: "0.1.0".to_string(),
                commit: "abc123".to_string(),
                dirty: false,
                ruleset: default_ruleset().to_string(),
                features: active_feature_names(),
                build_profile: "debug".to_string(),
                target: "test-target".to_string(),
            },
            runtime: NativeArchiveRuntime {
                protocol: "usi".to_string(),
                protocol_version: "minimal-v1".to_string(),
                executable: ENGINE_ARCHIVE_BIN_PATH.to_string(),
                sha256: Some("binary-sha256".to_string()),
            },
            nnue: Some(NativeArchiveNnue {
                path: ENGINE_ARCHIVE_NNUE_PATH.to_string(),
                sha256: "abc".to_string(),
            }),
        }
    }

    fn write_existing_self_play_report(
        report_path: &Path,
        ruleset: &str,
        command: &ReportCommand,
        engines: &[ReportEngine],
    ) {
        let report = serde_json::json!({
            "schema": "haitaka-self-play-report",
            "schemaVersion": 1,
            "ruleset": ruleset,
            "command": serde_json::to_value(command).expect("serialize report command"),
            "engines": serde_json::to_value(engines).expect("serialize report engines"),
            "summary": {
                "games": 4,
                "aWins": 2,
                "bWins": 1,
                "draws": 1,
                "decidedGames": 3,
                "aScore": 2.5,
                "scoreRate": 0.625,
                "approxElo": 66.7,
                "approxElo95Ci": [-100.0, 200.0],
                "avgPlies": 10.0,
                "totalNodes": 1000,
                "totalElapsedMs": 500.0,
                "aggregateNps": 2000.0,
                "warnings": []
            }
        });
        fs::write(
            report_path,
            serde_json::to_vec_pretty(&report).expect("serialize report"),
        )
        .expect("write report");
    }

    #[test]
    fn default_variant_metadata_matches_active_build_feature() {
        let (expected_ruleset, expected_rule_id) = if cfg!(feature = "annan") {
            ("annan", 26)
        } else if cfg!(feature = "anhoku") {
            ("anhoku", 55)
        } else if cfg!(feature = "antouzai") {
            ("antouzai", 95)
        } else if cfg!(feature = "taimen") {
            ("taimen", 72)
        } else if cfg!(feature = "haimen") {
            ("haimen", 74)
        } else if cfg!(feature = "neko") {
            ("neko", 130)
        } else if cfg!(feature = "nekoneko") {
            ("nekoneko", 131)
        } else if cfg!(feature = "yokoneko") {
            ("yokoneko", 132)
        } else if cfg!(feature = "yokonekoneko") {
            ("yokonekoneko", 133)
        } else {
            ("standard", 0)
        };

        assert_eq!(default_ruleset(), expected_ruleset);
        assert_eq!(default_rule_id(), expected_rule_id);
        // A native archive's advertised features must agree with its ruleset.
        assert_eq!(active_feature_names(), vec![expected_ruleset.to_string()]);
    }

    fn write_fake_wasm_pack_output(dir: &Path) {
        fs::create_dir_all(dir).expect("create fake wasm-pack dir");
        fs::write(
            dir.join("haitaka_wasm.js"),
            "export default function init() {}\n",
        )
        .expect("write fake js");
        fs::write(dir.join("haitaka_wasm_bg.wasm"), b"\0asm").expect("write fake wasm");
        fs::write(
            dir.join("haitaka_wasm.d.ts"),
            "export default function init(): void;\n",
        )
        .expect("write fake d.ts");
        fs::write(
            dir.join("haitaka_wasm_bg.wasm.d.ts"),
            "export const memory: WebAssembly.Memory;\n",
        )
        .expect("write fake wasm d.ts");
        fs::write(dir.join("package.json"), "{}\n").expect("write fake package.json");
        fs::write(dir.join("README.md"), "# fake wasm package\n").expect("write fake README");
    }

    #[cfg(unix)]
    fn write_executable_script(path: &Path, body: &str) {
        let temp_path = path.with_extension("tmp");
        {
            let mut file = fs::File::create(&temp_path).expect("create executable script temp");
            file.write_all(body.as_bytes())
                .expect("write executable script temp");
            file.flush().expect("flush executable script temp");
            file.sync_all().expect("sync executable script temp");
        }
        let mut permissions = fs::metadata(&temp_path)
            .expect("script temp metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&temp_path, permissions).expect("set script executable");
        fs::rename(&temp_path, path).expect("rename executable script into place");
    }

    #[test]
    fn manifest_serializes_shogitter_engine_package_v1() {
        let args = test_package_args(PathBuf::from("haitaka_wasm/pkg"), PathBuf::from("out.tgz"));
        let manifest = package_manifest(&args, None);
        let json = serde_json::to_value(&manifest).expect("serialize manifest");

        assert_eq!(json["schema"], "shogitter-engine-package");
        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["engine"]["id"], ENGINE_ID);
        assert_eq!(
            json["engine"]["name"],
            format!("{ENGINE_NAME} ({})", default_ruleset())
        );
        assert_eq!(json["engine"]["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(json["runtime"]["kind"], "wasm-bindgen");
        assert_eq!(json["runtime"]["module"], WASM_BINDGEN_MODULE);
        assert_eq!(json["runtime"]["wasm"], WASM_BINDGEN_WASM);
        assert_eq!(json["capabilities"]["protocols"][0], "shogitter-direct-v1");
        assert_eq!(json["capabilities"]["protocols"][1], "usi-wasm-v1");
        assert_eq!(json["capabilities"]["commands"][0], "search");
        assert_eq!(json["capabilities"]["supportsPonder"], false);
        assert_eq!(json["capabilities"]["supportsMovetime"], true);
        assert_eq!(json["capabilities"]["supportsDepth"], true);
        assert!(json.get("rules").is_none());
        assert!(json.get("artifacts").is_none());
        assert_eq!(json["profiles"].as_array().unwrap().len(), 1);
        assert_eq!(
            json["profiles"][0]["id"],
            format!("{}-default", default_ruleset())
        );
        assert_eq!(
            json["profiles"][0]["name"],
            format!("{} default", profile_display_ruleset(default_ruleset()))
        );
        assert_eq!(json["profiles"][0]["rules"][0]["ruleId"], default_rule_id());
        assert_eq!(
            json["profiles"][0]["rules"][0]["variant"],
            default_ruleset()
        );
        assert_eq!(json["profiles"][0]["rules"][0]["positionFormat"], "sfen");
        assert_eq!(json["profiles"][0]["rules"][0]["moveFormat"], "usi");
        assert_eq!(json["profiles"][0]["rules"][0]["startpos"], SFEN_STARTPOS);
        assert!(json["profiles"][0]["nnue"].is_null());
    }

    #[test]
    fn manifest_serializes_nnue_artifact() {
        let args = test_package_args(PathBuf::from("haitaka_wasm/pkg"), PathBuf::from("out.tgz"));
        let manifest = package_manifest(
            &args,
            Some(NnueArtifact {
                path: NNUE_ARTIFACT_PATH,
                format: "nnue",
            }),
        );
        let json = serde_json::to_value(&manifest).expect("serialize manifest");

        assert_eq!(json["profiles"][0]["nnue"]["path"], NNUE_ARTIFACT_PATH);
        assert_eq!(json["profiles"][0]["nnue"]["format"], "nnue");
    }

    #[test]
    fn manifest_serializes_explicit_anhoku_profile() {
        let mut args =
            test_package_args(PathBuf::from("haitaka_wasm/pkg"), PathBuf::from("out.tgz"));
        args.ruleset = "anhoku".to_string();
        args.rule_id = 55;

        let manifest = package_manifest(&args, None);
        let json = serde_json::to_value(&manifest).expect("serialize manifest");

        assert_eq!(json["profiles"][0]["id"], "anhoku-default");
        assert_eq!(json["profiles"][0]["name"], "Anhoku default");
        assert_eq!(json["profiles"][0]["rules"][0]["ruleId"], 55);
        assert_eq!(json["profiles"][0]["rules"][0]["variant"], "anhoku");
    }

    #[test]
    fn archive_manifest_serializes_native_engine_archive_v1() {
        let args = test_archive_args(
            PathBuf::from("target/debug/haitaka_cli"),
            PathBuf::from("out.tgz"),
            None,
        );
        let manifest = archive_engine_manifest(
            &args,
            "def456".to_string(),
            Some(NativeArchiveNnue {
                path: ENGINE_ARCHIVE_NNUE_PATH.to_string(),
                sha256: "abc123".to_string(),
            }),
        );
        let json = serde_json::to_value(&manifest).expect("serialize manifest");

        assert_eq!(json["schema"], "haitaka-engine-archive");
        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["engine"]["name"], ENGINE_NAME);
        assert_eq!(json["engine"]["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(json["engine"]["ruleset"], default_ruleset());
        assert_eq!(json["engine"]["buildProfile"], "debug");
        assert_eq!(json["engine"]["target"], "test-target");
        assert_eq!(json["runtime"]["protocol"], "usi");
        assert_eq!(json["runtime"]["protocolVersion"], "minimal-v1");
        assert_eq!(json["runtime"]["executable"], ENGINE_ARCHIVE_BIN_PATH);
        assert_eq!(json["runtime"]["sha256"], "def456");
        assert_eq!(json["nnue"]["path"], ENGINE_ARCHIVE_NNUE_PATH);
        assert_eq!(json["nnue"]["sha256"], "abc123");
    }

    #[test]
    fn file_sha256_hashes_small_file() {
        let temp = unique_temp_dir("sha256");
        fs::create_dir_all(&temp).expect("create temp dir");
        let path = temp.join("tiny.txt");
        fs::write(&path, b"abc").expect("write test file");

        assert_eq!(
            file_sha256(&path).expect("hash file"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        fs::remove_dir_all(temp).expect("clean temp dir");
    }

    #[test]
    fn git_helpers_return_fallback_safe_values() {
        assert!(!git_commit().is_empty());
        let _ = git_dirty();
    }

    #[test]
    fn self_play_requires_nnue_path_for_nnue_side() {
        let err = engine_config(
            "A",
            SearchBudget::Depth(3),
            EngineEvalKind::Nnue,
            None,
            None,
            None,
            &[],
        )
        .expect_err("NNUE side should require a model path");
        assert!(
            err.to_string()
                .contains("A uses NNUE but no NNUE path was provided"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn cli_parses_self_play_nnue_flags() {
        let cli = Cli::try_parse_from([
            "haitaka",
            "self-play",
            "--games",
            "8",
            "--a-depth",
            "4",
            "--b-depth",
            "4",
            "--a-eval",
            "nnue",
            "--b-eval",
            "handcrafted",
            "--nnue",
            "model.nnue",
            "--b-nnue",
            "other.nnue",
            "--a-engine",
            "old-haitaka",
            "--b-engine-archive",
            "new-haitaka.tgz",
            "--a-engine-arg=--eval",
            "--a-engine-arg=nnue",
            "--movetime-ms",
            "100",
            "--opening-random-plies",
            "4",
            "--openings",
            "openings.sfen",
            "--opening-order",
            "random",
            "--seed",
            "7",
            "--report-dir",
            "reports/run-1",
        ])
        .expect("CLI args should parse");

        match cli.command {
            Command::SelfPlay(args) => {
                assert_eq!(args.games, 8);
                assert_eq!(args.threads, 0);
                assert_eq!(args.a_depth, Some(4));
                assert_eq!(args.b_depth, Some(4));
                assert_eq!(args.a_eval, EngineEvalKind::Nnue);
                assert_eq!(args.b_eval, EngineEvalKind::Handcrafted);
                assert_eq!(args.nnue, Some(PathBuf::from("model.nnue")));
                assert_eq!(args.b_nnue, Some(PathBuf::from("other.nnue")));
                assert_eq!(args.a_engine, Some(PathBuf::from("old-haitaka")));
                assert_eq!(
                    args.b_engine_archive,
                    Some(PathBuf::from("new-haitaka.tgz"))
                );
                assert_eq!(args.a_engine_args, ["--eval", "nnue"]);
                assert_eq!(args.movetime_ms, Some(100));
                assert_eq!(args.openings, Some(PathBuf::from("openings.sfen")));
                assert_eq!(args.opening_order, OpeningOrder::Random);
                assert_eq!(args.opening_random_plies, 4);
                assert_eq!(args.seed, 7);
                assert_eq!(args.report_dir, Some(PathBuf::from("reports/run-1")));
            }
            other => panic!("expected self-play command, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_archive_engine_flags() {
        let cli = Cli::try_parse_from([
            "haitaka",
            "archive-engine",
            "--output",
            "engine.tgz",
            "--binary",
            "target/release/haitaka_cli",
            "--nnue",
            "model.nnue",
            "--ruleset",
            "annan",
            "--engine-name",
            "Archived Haitaka",
            "--profile",
            "release",
            "--target",
            "aarch64-apple-darwin",
        ])
        .expect("CLI args should parse");

        match cli.command {
            Command::ArchiveEngine(args) => {
                assert_eq!(args.output, PathBuf::from("engine.tgz"));
                assert_eq!(args.binary, PathBuf::from("target/release/haitaka_cli"));
                assert_eq!(args.nnue, Some(PathBuf::from("model.nnue")));
                assert_eq!(args.ruleset, "annan");
                assert_eq!(args.engine_name, "Archived Haitaka");
                assert_eq!(args.profile, Some(ArchiveBuildProfile::Release));
                assert_eq!(args.target.as_deref(), Some("aarch64-apple-darwin"));
            }
            other => panic!("expected archive-engine command, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_self_play_threads_flag() {
        let cli = Cli::try_parse_from(["haitaka", "self-play", "--threads", "6"])
            .expect("CLI args should parse");

        match cli.command {
            Command::SelfPlay(args) => assert_eq!(args.threads, 6),
            other => panic!("expected self-play command, got {other:?}"),
        }
    }

    #[test]
    fn cli_leaves_self_play_depths_unset_when_omitted() {
        let cli = Cli::try_parse_from(["haitaka", "self-play", "--movetime-ms", "100"])
            .expect("CLI args should parse");

        match cli.command {
            Command::SelfPlay(args) => {
                assert_eq!(args.a_depth, None);
                assert_eq!(args.b_depth, None);
                assert_eq!(args.movetime_ms, Some(100));
            }
            other => panic!("expected self-play command, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_usi_flags() {
        let cli = Cli::try_parse_from([
            "haitaka",
            "usi",
            "--eval",
            "nnue",
            "--nnue",
            "model.nnue",
            "--movetime-max-depth",
            "8",
        ])
        .expect("CLI args should parse");

        match cli.command {
            Command::Usi(args) => {
                assert_eq!(args.eval, EngineEvalKind::Nnue);
                assert_eq!(args.nnue, Some(PathBuf::from("model.nnue")));
                assert_eq!(args.movetime_max_depth, 8);
            }
            other => panic!("expected usi command, got {other:?}"),
        }
    }

    #[test]
    fn self_play_budget_prefers_movetime_when_set() {
        assert_eq!(
            self_play_budget(DEFAULT_SELF_PLAY_A_DEPTH, None, None).expect("depth budget"),
            SearchBudget::Depth(DEFAULT_SELF_PLAY_A_DEPTH)
        );
        assert_eq!(
            self_play_budget(DEFAULT_SELF_PLAY_A_DEPTH, Some(3), Some(100))
                .expect("movetime budget with cap"),
            SearchBudget::Movetime {
                max_depth: Some(3),
                millis: 100
            }
        );
        assert_eq!(
            self_play_budget(DEFAULT_SELF_PLAY_A_DEPTH, None, Some(100))
                .expect("movetime budget without cap"),
            SearchBudget::Movetime {
                max_depth: None,
                millis: 100
            }
        );
    }

    #[test]
    fn self_play_budget_rejects_zero_movetime() {
        let err = self_play_budget(DEFAULT_SELF_PLAY_A_DEPTH, None, Some(0))
            .expect_err("zero movetime should be rejected");
        assert!(
            err.to_string()
                .contains("--movetime-ms must be greater than 0")
        );
    }

    #[test]
    fn go_command_includes_movetime_depth_cap_when_set() {
        assert_eq!(
            go_command(SearchBudget::Movetime {
                max_depth: Some(5),
                millis: 100
            }),
            "go movetime 100 depth 5"
        );
        assert_eq!(
            go_command(SearchBudget::Movetime {
                max_depth: None,
                millis: 100
            }),
            "go movetime 100"
        );
    }

    #[test]
    fn self_play_thread_count_uses_available_parallelism_when_zero() {
        let available = thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1);

        assert_eq!(resolve_self_play_threads(0, 2), available.min(2));
        assert_eq!(resolve_self_play_threads(99, 2), 2);
        assert_eq!(resolve_self_play_threads(0, 0), 1);
    }

    #[test]
    fn format_eta_renders_compact_durations() {
        assert_eq!(format_eta(-1.0), "0s");
        assert_eq!(format_eta(0.4), "1s");
        assert_eq!(format_eta(65.0), "1m05s");
        assert_eq!(format_eta(3_661.0), "1h01m01s");
    }

    #[test]
    fn random_opening_generation_is_reproducible() {
        let base = Board::from_sfen(SFEN_STARTPOS).expect("startpos should parse");
        let opening_a =
            generate_opening_board(&base, 2, 123, 0).expect("opening generation should succeed");
        let opening_b =
            generate_opening_board(&base, 2, 123, 0).expect("opening generation should succeed");

        assert_eq!(opening_a.to_string(), opening_b.to_string());
        assert_ne!(opening_a.to_string(), base.to_string());
    }

    #[test]
    fn opening_suite_parser_ignores_comments_and_blanks() {
        let temp = unique_temp_dir("openings");
        fs::create_dir_all(&temp).expect("create temp dir");
        let path = temp.join("suite.sfen");
        fs::write(
            &path,
            format!(
                "\n# comment\n{} # inline comment\n\n",
                haitaka::SFEN_STARTPOS
            ),
        )
        .expect("write suite");

        let openings = load_opening_suite(&path).expect("openings should parse");

        assert_eq!(openings.len(), 1);
        assert_eq!(openings[0].suite_index, 0);
        assert_eq!(openings[0].sfen, haitaka::SFEN_STARTPOS);

        fs::remove_dir_all(temp).expect("clean temp dir");
    }

    #[test]
    fn opening_suite_parser_rejects_empty_files() {
        let temp = unique_temp_dir("empty-openings");
        fs::create_dir_all(&temp).expect("create temp dir");
        let path = temp.join("suite.sfen");
        fs::write(&path, "# no openings\n\n").expect("write suite");

        let err = load_opening_suite(&path).expect_err("empty suite should fail");
        assert!(err.to_string().contains("contains no SFEN positions"));

        fs::remove_dir_all(temp).expect("clean temp dir");
    }

    #[test]
    fn rating_summary_reports_score_elo_and_low_sample_warning() {
        let stats = MatchStats {
            a_wins: 2,
            b_wins: 1,
            draws: 1,
            total_nodes: 1_000,
            total_elapsed_ms: 500.0,
            total_plies: 40,
            a_breakdown: search_breakdown(600, 200.0),
            b_breakdown: search_breakdown(400, 300.0),
        };

        let summary = rating_summary(&stats, 4);

        assert_eq!(summary.games, 4);
        assert_eq!(summary.decided_games, 3);
        assert_eq!(summary.a_score, 2.5);
        assert_eq!(summary.score_rate, 0.625);
        assert!(summary.approx_elo > 0.0);
        assert_eq!(summary.avg_plies, 10.0);
        assert_eq!(summary.total_elapsed_ms, 500.0);
        assert_eq!(summary.aggregate_nps, 2_000.0);
        assert_eq!(summary.a_breakdown.total_nodes, 600);
        assert_eq!(summary.a_breakdown.aggregate_nps, 3_000.0);
        assert_eq!(summary.b_breakdown.total_nodes, 400);
        assert!((summary.b_breakdown.aggregate_nps - 1_333.3333333333335).abs() < 1e-9);
        assert!(
            summary
                .warnings
                .iter()
                .any(|warning| warning.contains("low sample"))
        );
    }

    #[test]
    fn existing_report_stats_loads_summary_for_merge() {
        let temp = unique_temp_dir("existing-report");
        fs::create_dir_all(&temp).expect("create temp dir");
        let report_path = temp.join(SELF_PLAY_REPORT_FILE);
        let expected_command = test_report_command();
        let expected_engines = test_report_engines();
        let mut existing_command = expected_command.clone();
        existing_command.games = 4;
        existing_command.threads = 1;
        write_existing_self_play_report(
            &report_path,
            default_ruleset(),
            &existing_command,
            &expected_engines,
        );

        let (stats, games) = load_existing_report_stats(
            &report_path,
            default_ruleset(),
            &expected_command,
            &expected_engines,
        )
        .expect("load summary");

        assert_eq!(games, 4);
        assert_eq!(stats.a_wins, 2);
        assert_eq!(stats.b_wins, 1);
        assert_eq!(stats.draws, 1);
        assert_eq!(stats.total_nodes, 1000);
        assert_eq!(stats.total_elapsed_ms, 500.0);
        assert_eq!(stats.total_plies, 40);

        fs::remove_dir_all(temp).expect("clean temp dir");
    }

    #[test]
    fn existing_report_stats_rejects_incompatible_engine_metadata_for_merge() {
        let temp = unique_temp_dir("existing-report-mismatch");
        fs::create_dir_all(&temp).expect("create temp dir");
        let report_path = temp.join(SELF_PLAY_REPORT_FILE);
        let expected_command = test_report_command();
        let expected_engines = test_report_engines();
        let mut incompatible_engines = expected_engines.clone();
        incompatible_engines[1].command = Some("/tmp/other-engine".to_string());
        write_existing_self_play_report(
            &report_path,
            default_ruleset(),
            &expected_command,
            &incompatible_engines,
        );

        let err = load_existing_report_stats(
            &report_path,
            default_ruleset(),
            &expected_command,
            &expected_engines,
        )
        .expect_err("merge should reject mismatched engine metadata");

        assert!(
            err.to_string()
                .contains("existing report engines does not match"),
            "unexpected error: {err}"
        );

        fs::remove_dir_all(temp).expect("clean temp dir");
    }

    #[test]
    fn archive_report_engine_uses_stable_archive_metadata() {
        let extraction_dir = PathBuf::from("/tmp/extracted-engine-123");
        let archive_path = PathBuf::from("target/engines/haitaka-native.tgz");
        let manifest = test_native_archive_manifest();
        let archive = ArchiveLaunch {
            engine_path: extraction_dir.join(ENGINE_ARCHIVE_BIN_PATH),
            engine_args: vec![
                "usi".to_string(),
                "--eval".to_string(),
                "nnue".to_string(),
                "--nnue".to_string(),
                extraction_dir
                    .join(ENGINE_ARCHIVE_NNUE_PATH)
                    .display()
                    .to_string(),
            ],
            report_engine_args: vec![
                "usi".to_string(),
                "--eval".to_string(),
                "nnue".to_string(),
                "--nnue".to_string(),
                ENGINE_ARCHIVE_NNUE_PATH.to_string(),
            ],
            extraction_dir,
            source_archive_path: archive_path.clone(),
            manifest,
        };
        let mut launch_args = archive.engine_args.clone();
        launch_args.extend(["--movetime-max-depth".to_string(), "8".to_string()]);
        let engine = EngineConfig {
            label: "A",
            budget: SearchBudget::Movetime {
                max_depth: None,
                millis: 100,
            },
            evaluator: EngineEvaluator::External {
                path: archive.engine_path.clone(),
                args: launch_args,
            },
        };

        let report = report_engine(&engine, Some(&archive));

        assert_eq!(report.kind, "archive-usi");
        assert_eq!(report.command.as_deref(), Some(ENGINE_ARCHIVE_BIN_PATH));
        assert_eq!(
            report.archive_path,
            Some(archive_path.display().to_string())
        );
        assert_eq!(
            report.args,
            vec![
                "usi",
                "--eval",
                "nnue",
                "--nnue",
                ENGINE_ARCHIVE_NNUE_PATH,
                "--movetime-max-depth",
                "8"
            ]
        );
        let encoded = serde_json::to_string(&report).expect("serialize report engine");
        assert!(
            !encoded.contains("/tmp/extracted-engine-123"),
            "report should not contain transient extraction paths: {encoded}"
        );
    }

    #[test]
    fn merge_rejects_missing_existing_game_log() {
        let temp = unique_temp_dir("existing-report-missing-games");
        fs::create_dir_all(&temp).expect("create temp dir");
        let report_path = temp.join(SELF_PLAY_REPORT_FILE);
        let games_path = temp.join(SELF_PLAY_GAMES_FILE);
        let expected_command = test_report_command();
        let expected_engines = test_report_engines();
        write_existing_self_play_report(
            &report_path,
            default_ruleset(),
            &expected_command,
            &expected_engines,
        );

        let err = prepare_self_play_report_merge_output(
            &temp,
            &report_path,
            &games_path,
            true,
            false,
            default_ruleset(),
            &expected_command,
            &expected_engines,
        )
        .expect_err("merge should reject a missing game log");

        assert!(
            err.to_string()
                .contains(&format!("{SELF_PLAY_GAMES_FILE} is missing")),
            "unexpected error: {err}"
        );

        fs::remove_dir_all(temp).expect("clean temp dir");
    }

    #[test]
    fn game_json_record_serializes_opening_and_moves() {
        let result = GameResult {
            a_color: Color::Black,
            winner: Some(Seat::A),
            plies: 2,
            total_nodes: 10,
            total_elapsed_ms: 1.5,
            a_breakdown: search_breakdown(6, 0.5),
            b_breakdown: search_breakdown(4, 1.0),
            start_sfen: haitaka::SFEN_STARTPOS.to_string(),
            opening: OpeningRecord {
                source: "suite".to_string(),
                suite_index: Some(3),
                base_sfen: haitaka::SFEN_STARTPOS.to_string(),
                random_plies: 0,
                random_seed: None,
            },
            moves: vec!["7g7f".to_string(), "3c3d".to_string()],
        };

        let json = serde_json::to_value(game_json_record(6, &result)).expect("serialize record");

        assert_eq!(json["schema"], "haitaka-self-play-game");
        assert_eq!(json["gameIndex"], 7);
        assert_eq!(json["pairIndex"], 3);
        assert_eq!(json["aColor"], "black");
        assert_eq!(json["bColor"], "white");
        assert_eq!(json["opening"]["suiteIndex"], 3);
        assert_eq!(json["moves"][0], "7g7f");
        assert_eq!(json["result"], "a-win");
        assert_eq!(json["winner"], "A");
        assert_eq!(json["aBreakdown"]["totalNodes"], 6);
        assert_eq!(json["bBreakdown"]["totalNodes"], 4);
        assert!(json["failureState"].is_null());
    }

    #[test]
    fn package_command_writes_v1_archive_from_fake_wasm_pack_output() {
        let temp = unique_temp_dir("package");
        let wasm_dir = temp.join("pkg");
        let output = temp.join("haitaka-variants.tgz");
        write_fake_wasm_pack_output(&wasm_dir);

        package(test_package_args(wasm_dir, output.clone())).expect("package should succeed");

        let list_output = ProcessCommand::new("tar")
            .arg("-tzf")
            .arg(&output)
            .output()
            .expect("run tar list");
        assert!(
            list_output.status.success(),
            "tar list failed: {}",
            String::from_utf8_lossy(&list_output.stderr)
        );
        let listing = String::from_utf8(list_output.stdout).expect("tar listing should be utf-8");
        assert!(listing.contains("./shogitter-engine.json"));
        assert!(listing.contains("./engine/haitaka_wasm.js"));
        assert!(listing.contains("./engine/haitaka_wasm_bg.wasm"));
        assert!(listing.contains("./engine/haitaka_wasm.d.ts"));
        assert!(!listing.contains("haitaka-package.json"));
        assert!(!listing.contains("engine/wasm"));

        let manifest_output = ProcessCommand::new("tar")
            .arg("-xOzf")
            .arg(&output)
            .arg("./shogitter-engine.json")
            .output()
            .expect("run tar extract manifest");
        assert!(
            manifest_output.status.success(),
            "tar manifest extract failed: {}",
            String::from_utf8_lossy(&manifest_output.stderr)
        );
        let manifest: Value =
            serde_json::from_slice(&manifest_output.stdout).expect("manifest should parse");
        assert_eq!(manifest["runtime"]["kind"], "wasm-bindgen");
        assert_eq!(manifest["runtime"]["module"], WASM_BINDGEN_MODULE);
        assert_eq!(manifest["runtime"]["wasm"], WASM_BINDGEN_WASM);
        assert_eq!(
            manifest["profiles"][0]["rules"][0]["positionFormat"],
            "sfen"
        );
        assert_eq!(manifest["profiles"][0]["rules"][0]["moveFormat"], "usi");
        assert!(manifest.get("rules").is_none());
        assert!(manifest.get("artifacts").is_none());

        fs::remove_dir_all(temp).expect("clean temp package dir");
    }

    #[test]
    fn archive_engine_command_writes_native_engine_archive() {
        let temp = unique_temp_dir("engine-archive");
        fs::create_dir_all(&temp).expect("create temp dir");
        let binary = temp.join("haitaka_cli");
        let nnue = temp.join("model.nnue");
        let output = temp.join("haitaka-native.tgz");
        fs::write(&binary, b"fake executable").expect("write fake binary");
        fs::write(&nnue, b"abc").expect("write fake nnue");
        let expected_binary_sha256 = file_sha256(&binary).expect("hash fake binary");

        archive_engine(test_archive_args(binary, output.clone(), Some(nnue)))
            .expect("archive should succeed");

        let list_output = ProcessCommand::new("tar")
            .arg("-tzf")
            .arg(&output)
            .output()
            .expect("run tar list");
        assert!(
            list_output.status.success(),
            "tar list failed: {}",
            String::from_utf8_lossy(&list_output.stderr)
        );
        let listing = String::from_utf8(list_output.stdout).expect("tar listing should be utf-8");
        assert!(listing.contains("./haitaka-engine-archive.json"));
        assert!(listing.contains("./bin/haitaka_cli"));
        assert!(listing.contains("./nnue/model.nnue"));
        assert!(listing.contains("./README.txt"));

        let manifest_output = ProcessCommand::new("tar")
            .arg("-xOzf")
            .arg(&output)
            .arg("./haitaka-engine-archive.json")
            .output()
            .expect("run tar extract manifest");
        assert!(
            manifest_output.status.success(),
            "tar manifest extract failed: {}",
            String::from_utf8_lossy(&manifest_output.stderr)
        );
        let manifest: Value =
            serde_json::from_slice(&manifest_output.stdout).expect("manifest should parse");
        assert_eq!(manifest["schema"], "haitaka-engine-archive");
        assert_eq!(manifest["runtime"]["protocol"], "usi");
        assert_eq!(manifest["runtime"]["executable"], ENGINE_ARCHIVE_BIN_PATH);
        assert_eq!(manifest["runtime"]["sha256"], expected_binary_sha256);
        assert_eq!(manifest["nnue"]["path"], ENGINE_ARCHIVE_NNUE_PATH);
        assert_eq!(
            manifest["nnue"]["sha256"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        fs::remove_dir_all(temp).expect("clean temp archive dir");
    }

    #[test]
    fn extract_engine_archive_rejects_executable_sha256_mismatch() {
        let temp = unique_temp_dir("engine-archive-binary-mismatch");
        fs::create_dir_all(&temp).expect("create temp dir");
        let binary = temp.join("haitaka_cli");
        let archive = temp.join("haitaka-native.tgz");
        let unpacked = temp.join("unpacked");
        let extraction_dir = temp.join("extracted");
        let repacked = temp.join("tampered.tgz");
        fs::write(&binary, b"fake executable").expect("write fake binary");
        let original_sha256 = file_sha256(&binary).expect("hash original binary");

        archive_engine(test_archive_args(binary, archive.clone(), None))
            .expect("archive should succeed");

        fs::create_dir_all(&unpacked).expect("create unpacked dir");
        let status = ProcessCommand::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&unpacked)
            .status()
            .expect("extract original archive");
        assert!(status.success(), "archive extract failed: {status}");

        let tampered_binary = unpacked.join(ENGINE_ARCHIVE_BIN_PATH);
        fs::write(&tampered_binary, b"tampered executable")
            .expect("overwrite executable with tampered bytes");
        let tampered_sha256 = file_sha256(&tampered_binary).expect("hash tampered binary");

        let status = ProcessCommand::new("tar")
            .arg("-czf")
            .arg(&repacked)
            .arg("-C")
            .arg(&unpacked)
            .arg(".")
            .status()
            .expect("create tampered archive");
        assert!(status.success(), "archive repack failed: {status}");

        let err = extract_engine_archive_in_dir(&repacked, &extraction_dir)
            .expect_err("tampered archive should fail");
        let message = err.to_string();
        assert!(
            message.contains("archive executable bin/haitaka_cli sha256 mismatch"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains(&original_sha256),
            "unexpected error: {message}"
        );
        assert!(
            message.contains(&tampered_sha256),
            "unexpected error: {message}"
        );

        fs::remove_dir_all(temp).expect("clean temp archive dir");
    }

    #[test]
    fn extract_engine_archive_rejects_nnue_sha256_mismatch() {
        let temp = unique_temp_dir("engine-archive-nnue-mismatch");
        fs::create_dir_all(&temp).expect("create temp dir");
        let binary = temp.join("haitaka_cli");
        let nnue = temp.join("model.nnue");
        let archive = temp.join("haitaka-native.tgz");
        let unpacked = temp.join("unpacked");
        let extraction_dir = temp.join("extracted");
        let repacked = temp.join("tampered.tgz");
        fs::write(&binary, b"fake executable").expect("write fake binary");
        fs::write(&nnue, b"abc").expect("write fake nnue");

        archive_engine(test_archive_args(binary, archive.clone(), Some(nnue)))
            .expect("archive should succeed");

        fs::create_dir_all(&unpacked).expect("create unpacked dir");
        let status = ProcessCommand::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&unpacked)
            .status()
            .expect("extract original archive");
        assert!(status.success(), "archive extract failed: {status}");

        let tampered_nnue = unpacked.join(ENGINE_ARCHIVE_NNUE_PATH);
        fs::write(&tampered_nnue, b"xyz").expect("overwrite nnue with tampered bytes");
        let tampered_sha256 = file_sha256(&tampered_nnue).expect("hash tampered nnue");

        let status = ProcessCommand::new("tar")
            .arg("-czf")
            .arg(&repacked)
            .arg("-C")
            .arg(&unpacked)
            .arg(".")
            .status()
            .expect("create tampered archive");
        assert!(status.success(), "archive repack failed: {status}");

        let err = extract_engine_archive_in_dir(&repacked, &extraction_dir)
            .expect_err("tampered archive should fail");
        let message = err.to_string();
        assert!(
            message.contains("archive NNUE nnue/model.nnue sha256 mismatch"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains(&tampered_sha256),
            "unexpected error: {message}"
        );

        fs::remove_dir_all(temp).expect("clean temp archive dir");
    }

    #[test]
    fn raw_and_archive_engine_sources_conflict() {
        let err = validate_engine_source(
            "A",
            Some(&PathBuf::from("engine")),
            Some(&PathBuf::from("engine.tgz")),
        )
        .expect_err("raw and archive source should conflict");
        assert!(err.to_string().contains("--a-engine"));
        assert!(err.to_string().contains("--a-engine-archive"));
    }

    #[test]
    fn missing_external_engine_path_fails_clearly() {
        let err = match UsiEngineClient::spawn(Path::new("/definitely/missing/haitaka"), &[]) {
            Ok(_) => panic!("missing engine should fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("failed to launch external engine"));
    }

    #[cfg(unix)]
    #[test]
    fn external_engine_launches_with_exact_user_args() {
        let temp = unique_temp_dir("exact-argv");
        fs::create_dir_all(&temp).expect("create temp dir");
        let script = temp.join("engine.sh");
        fs::write(
            &script,
            "#!/bin/sh\nif [ \"$#\" -ne 1 ] || [ \"$1\" != \"--expected\" ]; then echo \"bad argv: $*\" >&2; exit 7; fi\nwhile IFS= read -r line; do\ncase \"$line\" in\n  usi) echo usiok ;;\n  isready) echo readyok ;;\nesac\ndone\n",
        )
        .expect("write engine script");

        let mut client = UsiEngineClient::spawn_with_startup_timeout(
            Path::new("/bin/sh"),
            &[script.display().to_string(), "--expected".to_string()],
            Duration::from_secs(5),
        )
        .expect("engine should receive exact args and start");
        client.send_command("quit").expect("send quit");

        fs::remove_dir_all(temp).expect("clean temp dir");
    }

    #[cfg(unix)]
    #[test]
    fn external_engine_startup_timeout_fails_clearly() {
        let temp = unique_temp_dir("startup-timeout");
        fs::create_dir_all(&temp).expect("create temp dir");
        let script = temp.join("engine.sh");
        fs::write(&script, "sleep 2\n").expect("write silent engine script");

        let err = match UsiEngineClient::spawn_with_startup_timeout(
            Path::new("/bin/sh"),
            &[script.display().to_string()],
            Duration::from_millis(50),
        ) {
            Ok(_) => panic!("silent engine should time out"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("external engine timed out"));

        fs::remove_dir_all(temp).expect("clean temp dir");
    }

    #[cfg(unix)]
    #[test]
    fn external_engine_closed_stdout_fails_clearly() {
        let temp = unique_temp_dir("closed-stdout");
        fs::create_dir_all(&temp).expect("create temp dir");
        let script = temp.join("engine.sh");
        write_executable_script(&script, "#!/bin/sh\nexec 1>&-\nsleep 1\n");

        let err = match UsiEngineClient::spawn_with_startup_timeout(
            Path::new("/bin/sh"),
            &[script.display().to_string()],
            Duration::from_millis(500),
        ) {
            Ok(_) => panic!("exited engine should fail"),
            Err(err) => err,
        };
        let message = err.to_string();
        assert!(
            message.contains("external engine closed stdout")
                || message.contains("external engine exited")
                || message.contains("external engine timed out"),
            "unexpected error: {message}"
        );

        fs::remove_dir_all(temp).expect("clean temp dir");
    }

    #[cfg(unix)]
    #[test]
    fn external_engine_search_timeout_fails_clearly() {
        let temp = unique_temp_dir("search-timeout");
        fs::create_dir_all(&temp).expect("create temp dir");
        let script = temp.join("engine.sh");
        write_executable_script(
            &script,
            "#!/bin/sh\nwhile IFS= read -r line; do\ncase \"$line\" in\n  usi) echo usiok ;;\n  isready) echo readyok ;;\n  go*) sleep 2 ;;\nesac\ndone\n",
        );
        let mut client =
            UsiEngineClient::spawn_with_startup_timeout(&script, &[], Duration::from_secs(5))
                .expect("engine should start");

        client
            .send_command(&format!("position sfen {}", haitaka::SFEN_STARTPOS))
            .expect("send position");
        client.send_command("go depth 1").expect("send go");
        let err = client
            .read_bestmove(Duration::from_millis(50))
            .expect_err("search should time out");
        assert!(err.to_string().contains("external engine timed out"));

        fs::remove_dir_all(temp).expect("clean temp dir");
    }

    #[cfg(unix)]
    #[test]
    fn malformed_bestmove_fails_clearly() {
        let temp = unique_temp_dir("malformed-bestmove");
        fs::create_dir_all(&temp).expect("create temp dir");
        let script = temp.join("engine.sh");
        write_executable_script(
            &script,
            "#!/bin/sh\nwhile IFS= read -r line; do\ncase \"$line\" in\n  usi) echo usiok ;;\n  isready) echo readyok ;;\n  go*) printf 'bestmove \\n' ;;\nesac\ndone\n",
        );
        let mut client =
            UsiEngineClient::spawn_with_startup_timeout(&script, &[], Duration::from_secs(5))
                .expect("engine should start");
        client
            .send_command(&format!("position sfen {}", haitaka::SFEN_STARTPOS))
            .expect("send position");
        client.send_command("go depth 1").expect("send go");

        let err = client
            .read_bestmove(Duration::from_millis(500))
            .expect_err("empty bestmove should fail");
        assert!(
            err.to_string().contains("empty bestmove"),
            "unexpected error: {err}"
        );

        fs::remove_dir_all(temp).expect("clean temp dir");
    }

    #[cfg(unix)]
    #[test]
    fn illegal_bestmove_fails_clearly() {
        let temp = unique_temp_dir("illegal-bestmove");
        fs::create_dir_all(&temp).expect("create temp dir");
        let script = temp.join("engine.sh");
        write_executable_script(
            &script,
            "#!/bin/sh\nwhile IFS= read -r line; do\ncase \"$line\" in\n  usi) echo usiok ;;\n  isready) echo readyok ;;\n  go*) echo 'bestmove 1a1b' ;;\nesac\ndone\n",
        );
        let args = SelfPlayArgs {
            games: 1,
            threads: 1,
            a_depth: Some(1),
            b_depth: Some(1),
            a_eval: EngineEvalKind::Handcrafted,
            b_eval: EngineEvalKind::Handcrafted,
            nnue: None,
            a_nnue: None,
            b_nnue: None,
            a_engine: None,
            a_engine_archive: None,
            a_engine_args: Vec::new(),
            b_engine: None,
            b_engine_archive: None,
            b_engine_args: Vec::new(),
            movetime_ms: None,
            sfen: None,
            openings: None,
            opening_order: OpeningOrder::Sequential,
            opening_random_plies: 0,
            seed: 0,
            max_plies: 4,
            report_dir: None,
        };
        let base = Board::from_sfen(haitaka::SFEN_STARTPOS).expect("startpos should parse");
        let engine_a = EngineConfig {
            label: "A",
            budget: SearchBudget::Depth(1),
            evaluator: EngineEvaluator::External {
                path: script,
                args: Vec::new(),
            },
        };
        let engine_b = EngineConfig {
            label: "B",
            budget: SearchBudget::Depth(1),
            evaluator: EngineEvaluator::Handcrafted,
        };

        let err = play_self_play_game(0, &args, &base, None, &engine_a, &engine_b)
            .expect_err("illegal bestmove should fail");
        assert!(err.to_string().contains("illegal move"));

        fs::remove_dir_all(temp).expect("clean temp dir");
    }

    #[cfg(unix)]
    #[test]
    fn terminal_position_on_last_allowed_ply_is_reported_as_a_win() {
        let temp = unique_temp_dir("last-ply-terminal");
        fs::create_dir_all(&temp).expect("create temp dir");
        let script = temp.join("engine.sh");
        let base = Board::from_sfen("8k/6G2/7B1/9/9/9/9/9/K8 b R 1")
            .expect("one-ply mate position should parse");
        let mating_move = legal_moves(&base)
            .into_iter()
            .find(|mv| {
                let mut next = base.clone();
                next.try_play(*mv).is_ok() && next.status() != GameStatus::Ongoing
            })
            .expect("position should contain a terminal move")
            .to_string();
        write_executable_script(
            &script,
            &format!(
                "#!/bin/sh\nwhile IFS= read -r line; do\ncase \"$line\" in\n  usi) echo usiok ;;\n  isready) echo readyok ;;\n  go*) echo 'bestmove {mating_move}' ;;\nesac\ndone\n"
            ),
        );

        let args = SelfPlayArgs {
            games: 1,
            threads: 1,
            a_depth: Some(1),
            b_depth: Some(1),
            a_eval: EngineEvalKind::Handcrafted,
            b_eval: EngineEvalKind::Handcrafted,
            nnue: None,
            a_nnue: None,
            b_nnue: None,
            a_engine: None,
            a_engine_archive: None,
            a_engine_args: Vec::new(),
            b_engine: None,
            b_engine_archive: None,
            b_engine_args: Vec::new(),
            movetime_ms: None,
            sfen: Some(base.to_string()),
            openings: None,
            opening_order: OpeningOrder::Sequential,
            opening_random_plies: 0,
            seed: 0,
            max_plies: 1,
            report_dir: None,
        };
        let engine_a = EngineConfig {
            label: "A",
            budget: SearchBudget::Depth(1),
            evaluator: EngineEvaluator::External {
                path: PathBuf::from("/bin/sh"),
                args: vec![script.display().to_string()],
            },
        };
        let engine_b = EngineConfig {
            label: "B",
            budget: SearchBudget::Depth(1),
            evaluator: EngineEvaluator::Handcrafted,
        };

        let result =
            play_self_play_game(0, &args, &base, None, &engine_a, &engine_b).expect("game");
        assert_eq!(result.plies, 1);
        assert_eq!(result.winner, Some(Seat::A));

        fs::remove_dir_all(temp).expect("clean temp dir");
    }

    #[test]
    fn package_requires_wasm_bindgen_artifacts_by_default() {
        let temp = unique_temp_dir("missing-wasm");
        let wasm_dir = temp.join("pkg");
        fs::create_dir_all(&wasm_dir).expect("create fake wasm-pack dir");
        fs::write(
            wasm_dir.join("haitaka_wasm.js"),
            "export default function init() {}\n",
        )
        .expect("write fake js");

        let err = package(test_package_args(wasm_dir, temp.join("out.tgz")))
            .expect_err("missing wasm should fail");
        assert!(
            err.to_string().contains("haitaka_wasm_bg.wasm"),
            "error should name missing wasm file: {err}"
        );

        fs::remove_dir_all(temp).expect("clean temp package dir");
    }

    #[test]
    fn wasm_usi_future_work_plan_documents_deferred_items() {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plans/wasm-usi-future-work.md");
        let contents = fs::read_to_string(path).expect("future work plan should exist");

        for expected in [
            "Async search and `stop`",
            "Ponder",
            "Full time controls",
            "Multi-PV",
            "USI options",
            "Web Worker harness",
            "Browser self-play UI",
            "Native archive workflow",
            "Rating and report improvements",
            "Cross-runtime rating policy",
        ] {
            assert!(
                contents.contains(expected),
                "future work plan should mention {expected}"
            );
        }
    }

    #[test]
    fn strength_measurement_future_work_plan_documents_deferred_items() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../plans/strength-measurement-future-work.md");
        let contents = fs::read_to_string(path).expect("future work plan should exist");

        for expected in [
            "SPRT",
            "STC/LTC",
            "Distributed workers",
            "PGN or CSA",
            "Engine crash forfeits",
            "Multi-engine tournaments",
            "Cross-runtime rating pools",
            "Browser rating webapp",
            "Resumable matches",
        ] {
            assert!(
                contents
                    .to_ascii_lowercase()
                    .contains(&expected.to_ascii_lowercase()),
                "future work plan should mention {expected}"
            );
        }
    }

    #[cfg(feature = "annan")]
    #[test]
    fn annan_manifest_defaults_to_rule_26_usi_sfen() {
        let args = test_package_args(PathBuf::from("haitaka_wasm/pkg"), PathBuf::from("out.tgz"));
        let manifest = package_manifest(&args, None);
        let json = serde_json::to_value(&manifest).expect("serialize manifest");

        assert_eq!(json["runtime"]["kind"], "wasm-bindgen");
        assert_eq!(json["engine"]["name"], "Haitaka Variants (annan)");
        assert_eq!(json["profiles"][0]["id"], "annan-default");
        assert_eq!(json["profiles"][0]["name"], "Annan default");
        assert_eq!(json["profiles"][0]["rules"][0]["ruleId"], 26);
        assert_eq!(json["profiles"][0]["rules"][0]["variant"], "annan");
        assert_eq!(json["profiles"][0]["rules"][0]["positionFormat"], "sfen");
        assert_eq!(json["profiles"][0]["rules"][0]["moveFormat"], "usi");
        assert_eq!(
            json["profiles"][0]["rules"][0]["startpos"],
            "lnsgkgsnl/1r5b1/p1ppppp1p/1p5p1/9/1P5P1/P1PPPPP1P/1B5R1/LNSGKGSNL b - 1"
        );
    }
}

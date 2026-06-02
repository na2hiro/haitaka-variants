use std::fs;
use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command as ProcessCommand, Stdio};
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};
use haitaka::{Board, Color, GameStatus, Move, SFEN_STARTPOS};
use haitaka_wasm::{NnueModel, SearchEvalMode};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::Serialize;

const ENGINE_ID: &str = "haitaka-variants";
const ENGINE_NAME: &str = "Haitaka Variants";
const MANIFEST_FILE: &str = "shogitter-engine.json";
const ENGINE_DIR: &str = "engine";
const WASM_BINDGEN_MODULE: &str = "engine/haitaka_wasm.js";
const WASM_BINDGEN_WASM: &str = "engine/haitaka_wasm_bg.wasm";
const NNUE_ARTIFACT_PATH: &str = "engine/model.nnue";
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
    /// Engine A fixed search depth.
    #[arg(long = "a-depth", default_value_t = 3)]
    a_depth: u8,
    /// Engine B fixed search depth.
    #[arg(long = "b-depth", default_value_t = 2)]
    b_depth: u8,
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
    /// Argument appended after `usi` when launching side A's external engine.
    #[arg(long = "a-engine-arg", action = clap::ArgAction::Append, allow_hyphen_values = true)]
    a_engine_args: Vec<String>,
    /// External USI engine executable for side B.
    #[arg(long = "b-engine")]
    b_engine: Option<PathBuf>,
    /// Argument appended after `usi` when launching side B's external engine.
    #[arg(long = "b-engine-arg", action = clap::ArgAction::Append, allow_hyphen_values = true)]
    b_engine_args: Vec<String>,
    /// Shared movetime budget in milliseconds. If set, both sides use movetime.
    #[arg(long)]
    movetime_ms: Option<u32>,
    /// Starting SFEN. Defaults to the ruleset start position.
    #[arg(long)]
    sfen: Option<String>,
    /// Number of random plies applied before each paired game to diversify openings.
    #[arg(long, default_value_t = 0)]
    opening_random_plies: u16,
    /// Seed for random opening generation.
    #[arg(long, default_value_t = 0)]
    seed: u64,
    /// Maximum plies per game before declaring a draw.
    #[arg(long, default_value_t = 200)]
    max_plies: u16,
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
    Movetime { max_depth: u8, millis: u32 },
}

#[derive(Debug, Clone, PartialEq)]
struct EngineSearchResult {
    best_move: Option<String>,
    total_nodes: u64,
    elapsed_ms: f64,
}

#[derive(Debug, Default)]
struct MatchStats {
    a_wins: u32,
    b_wins: u32,
    draws: u32,
    total_nodes: u64,
    total_elapsed_ms: f64,
    total_plies: u64,
}

#[derive(Debug, Clone)]
struct GameResult {
    a_color: Color,
    winner: Option<Seat>,
    plies: u16,
    total_nodes: u64,
    total_elapsed_ms: f64,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Play(args) => play(args),
        Command::Usi(args) => usi(args),
        Command::SelfPlay(args) => self_play(args),
        Command::Package(args) => package(args),
    }
}

fn default_ruleset() -> &'static str {
    if cfg!(feature = "annan") {
        "annan"
    } else {
        "standard"
    }
}

fn default_rule_id() -> u32 {
    if cfg!(feature = "annan") { 26 } else { 0 }
}

fn profile_display_ruleset(ruleset: &str) -> String {
    match ruleset {
        "standard" => "Standard".to_string(),
        "annan" => "Annan".to_string(),
        "anhoku" => "Anhoku".to_string(),
        "antouzai" => "Antouzai".to_string(),
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
            protocols: vec!["shogitter-direct-v1"],
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
                max_depth,
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
                max_depth,
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
        SearchBudget::Movetime { max_depth, millis } => {
            format!("movetime_ms={millis} max_depth={max_depth}")
        }
    }
}

fn self_play_budget(depth: u8, movetime_ms: Option<u32>) -> SearchBudget {
    match movetime_ms {
        Some(millis) => SearchBudget::Movetime {
            max_depth: depth.max(1),
            millis,
        },
        None => SearchBudget::Depth(depth.max(1)),
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

struct UsiEngineClient {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
    stderr_lines: Arc<Mutex<Vec<String>>>,
}

impl UsiEngineClient {
    fn spawn(path: &Path, args: &[String]) -> Result<Self> {
        let mut child = ProcessCommand::new(path)
            .arg("usi")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
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
        client.read_until_exact("usiok", USI_STARTUP_TIMEOUT)?;
        client.send_command("isready")?;
        client.read_until_exact("readyok", USI_STARTUP_TIMEOUT)?;
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
        SearchBudget::Movetime { millis, .. } => format!("go movetime {millis}"),
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

fn play_self_play_game(
    game_index: u32,
    args: &SelfPlayArgs,
    base_board: &Board,
    engine_a: &EngineConfig,
    engine_b: &EngineConfig,
) -> Result<GameResult> {
    let pair_index = game_index / 2;
    let mut board =
        generate_opening_board(base_board, args.opening_random_plies, args.seed, pair_index)?;
    let a_color = if game_index % 2 == 0 {
        Color::Black
    } else {
        Color::White
    };
    let mut winner = None;
    let mut plies = 0;
    let mut total_nodes = 0;
    let mut total_elapsed_ms = 0.0;
    let mut runtime_a = GameEngine::start(engine_a)
        .map_err(|err| anyhow!("failed to start engine A in game {}: {err}", game_index + 1))?;
    let mut runtime_b = GameEngine::start(engine_b)
        .map_err(|err| anyhow!("failed to start engine B in game {}: {err}", game_index + 1))?;

    for ply in 0..args.max_plies {
        if board.status() != GameStatus::Ongoing {
            winner = Some(if board.side_to_move() == a_color {
                Seat::B
            } else {
                Seat::A
            });
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
        let summary = runtime
            .search(&board, config.budget)
            .map_err(|err| anyhow!("search failed in game {}: {err}", game_index + 1))?;
        total_nodes += summary.total_nodes;
        total_elapsed_ms += summary.elapsed_ms;
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
        board
            .try_play(mv)
            .map_err(|_| anyhow!("engine returned illegal move {best_move}"))?;
        plies = ply + 1;
    }

    Ok(GameResult {
        a_color,
        winner,
        plies,
        total_nodes,
        total_elapsed_ms,
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
    let mut engine = engine_config(
        "USI",
        SearchBudget::Depth(1),
        args.eval,
        args.nnue.as_deref(),
        None,
        None,
        &[],
    )?;
    let mut board = Board::from_sfen(SFEN_STARTPOS).expect("startpos should parse");
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line.context("failed to read USI command")?;
        let command = line.trim();
        if command.is_empty() {
            continue;
        }

        match command {
            "quit" => break,
            "usi" => {
                writeln!(stdout, "id name {ENGINE_NAME}")?;
                writeln!(stdout, "usiok")?;
                stdout.flush()?;
            }
            "isready" => {
                writeln!(stdout, "readyok")?;
                stdout.flush()?;
            }
            "usinewgame" => {}
            command if command.starts_with("position ") => match parse_usi_position(command) {
                Ok(parsed) => board = parsed,
                Err(err) => eprintln!("info string invalid position command: {err}"),
            },
            command if command.starts_with("go") => {
                let budget = match parse_usi_go(command, args.movetime_max_depth) {
                    Ok(budget) => budget,
                    Err(err) => {
                        eprintln!("info string invalid go command: {err}");
                        continue;
                    }
                };
                engine.budget = budget;
                if board.status() != GameStatus::Ongoing {
                    writeln!(stdout, "bestmove resign")?;
                    stdout.flush()?;
                    continue;
                }
                match search_with_engine(&board, &engine) {
                    Ok(summary) => {
                        let best_move = summary.best_move.as_deref().unwrap_or("resign");
                        writeln!(stdout, "bestmove {best_move}")?;
                    }
                    Err(err) => {
                        eprintln!("info string search failed: {err}");
                        writeln!(stdout, "bestmove resign")?;
                    }
                }
                stdout.flush()?;
            }
            other => {
                eprintln!("info string unsupported command: {other}");
            }
        }
    }

    Ok(())
}

fn parse_usi_position(command: &str) -> Result<Board> {
    let rest = command
        .strip_prefix("position ")
        .ok_or_else(|| anyhow!("expected position command"))?;
    let tokens = rest.split_whitespace().collect::<Vec<_>>();
    if tokens.is_empty() {
        bail!("missing position body");
    }

    let mut index;
    let mut board = match tokens[0] {
        "startpos" => {
            index = 1;
            Board::from_sfen(SFEN_STARTPOS).expect("startpos should parse")
        }
        "sfen" => {
            index = 1;
            let sfen_start = index;
            while index < tokens.len() && tokens[index] != "moves" {
                index += 1;
            }
            if sfen_start == index {
                bail!("missing SFEN after position sfen");
            }
            Board::from_sfen(&tokens[sfen_start..index].join(" "))
                .map_err(|err| anyhow!("failed to parse SFEN: {err}"))?
        }
        other => bail!("unsupported position source {other}"),
    };

    if index < tokens.len() {
        if tokens[index] != "moves" {
            bail!("unexpected token {}", tokens[index]);
        }
        index += 1;
        for move_text in &tokens[index..] {
            let mv = Move::from_str(move_text)
                .map_err(|err| anyhow!("invalid move {move_text}: {err}"))?;
            board
                .try_play(mv)
                .map_err(|_| anyhow!("illegal move {move_text}"))?;
        }
    }

    Ok(board)
}

fn parse_usi_go(command: &str, movetime_max_depth: u8) -> Result<SearchBudget> {
    let rest = command
        .strip_prefix("go")
        .ok_or_else(|| anyhow!("expected go command"))?;
    let tokens = rest.split_whitespace().collect::<Vec<_>>();
    let mut index = 0;
    let mut depth = None;
    let mut movetime = None;

    while index < tokens.len() {
        match tokens[index] {
            "depth" => {
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| anyhow!("missing depth value"))?;
                depth = Some(
                    value
                        .parse::<u8>()
                        .with_context(|| format!("invalid depth {value}"))?,
                );
            }
            "movetime" => {
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| anyhow!("missing movetime value"))?;
                movetime = Some(
                    value
                        .parse::<u32>()
                        .with_context(|| format!("invalid movetime {value}"))?,
                );
            }
            _ => {}
        }
        index += 1;
    }

    if let Some(depth) = depth {
        return Ok(SearchBudget::Depth(depth.max(1)));
    }
    if let Some(millis) = movetime {
        return Ok(SearchBudget::Movetime {
            max_depth: movetime_max_depth.max(1),
            millis,
        });
    }
    bail!("only go depth N and go movetime N are supported")
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
    let base_board = parse_board(args.sfen.as_deref())?;
    let a_budget = self_play_budget(args.a_depth, args.movetime_ms);
    let b_budget = self_play_budget(args.b_depth, args.movetime_ms);
    let engine_a = engine_config(
        "A",
        a_budget,
        args.a_eval,
        args.nnue.as_deref(),
        args.a_nnue.as_deref(),
        args.a_engine.as_deref(),
        &args.a_engine_args,
    )?;
    let engine_b = engine_config(
        "B",
        b_budget,
        args.b_eval,
        args.nnue.as_deref(),
        args.b_nnue.as_deref(),
        args.b_engine.as_deref(),
        &args.b_engine_args,
    )?;
    let threads = resolve_self_play_threads(args.threads, args.games);
    let mut stats = MatchStats::default();
    let start = Instant::now();

    println!("{}", describe_engine(&engine_a));
    println!("{}", describe_engine(&engine_b));
    println!("self-play threads={threads}");
    if args.opening_random_plies > 0 {
        println!(
            "paired random opening plies={} seed={}",
            args.opening_random_plies, args.seed
        );
    }

    let next_game = AtomicU32::new(0);
    let (tx, rx) = mpsc::channel();
    let mut completed = 0_u32;

    thread::scope(|scope| -> Result<()> {
        for _ in 0..threads {
            let tx = tx.clone();
            let args = &args;
            let base_board = &base_board;
            let engine_a = &engine_a;
            let engine_b = &engine_b;
            let next_game = &next_game;

            scope.spawn(move || {
                loop {
                    let game_index = next_game.fetch_add(1, Ordering::Relaxed);
                    if game_index >= args.games {
                        break;
                    }
                    let result =
                        play_self_play_game(game_index, args, base_board, engine_a, engine_b);
                    if tx.send((game_index, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        for _ in 0..args.games {
            let (game_index, result) = rx.recv().map_err(|err| {
                anyhow!("self-play worker exited before reporting all games: {err}")
            })?;
            let result = result?;
            let outcome = match result.winner {
                Some(Seat::A) => "A win",
                Some(Seat::B) => "B win",
                None => "draw",
            };

            completed += 1;
            stats.total_nodes += result.total_nodes;
            stats.total_elapsed_ms += result.total_elapsed_ms;
            stats.total_plies += u64::from(result.plies);
            match result.winner {
                Some(Seat::A) => stats.a_wins += 1,
                Some(Seat::B) => stats.b_wins += 1,
                None => stats.draws += 1,
            }

            let elapsed = start.elapsed().as_secs_f64();
            let remaining = args.games.saturating_sub(completed);
            let eta = if completed == 0 {
                0.0
            } else {
                elapsed * f64::from(remaining) / f64::from(completed)
            };

            let decided = stats.a_wins + stats.b_wins;
            let a_score = stats.a_wins as f64 + 0.5 * stats.draws as f64;
            let denom = f64::from(completed.max(1));
            let score_rate = (a_score / denom).clamp(0.01, 0.99);
            let elo = -400.0 * (1.0 / score_rate - 1.0).log10();
            let nps = if stats.total_elapsed_ms > 0.0 {
                stats.total_nodes as f64 / (stats.total_elapsed_ms / 1_000.0)
            } else {
                0.0
            };

            let block = format!(
                "game ({game}) done ({completed}/{total}): A({a_color:?}) vs B({b_color:?}) \
                 plies={plies} result={outcome} eta={eta}\n\
                 games: {completed}\n\
                 score: A {a_wins} - B {b_wins} - draws {draws}\n\
                 decided games: {decided}\n\
                 approx elo A-B: {elo:.1} (small sample estimate)\n\
                 avg plies: {avg:.1}\n\
                 total nodes: {nodes}\n\
                 aggregate nps: {nps:.0}",
                game = game_index + 1,
                total = args.games,
                a_color = result.a_color,
                b_color = !result.a_color,
                plies = result.plies,
                eta = format_eta(eta),
                a_wins = stats.a_wins,
                b_wins = stats.b_wins,
                draws = stats.draws,
                avg = stats.total_plies as f64 / denom,
                nodes = stats.total_nodes,
            );
            render_status(&block, completed == 1);
        }

        Ok(())
    })?;

    Ok(())
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

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("haitaka-cli-{name}-{}-{nonce}", std::process::id()))
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
            "--a-engine-arg=--eval",
            "--a-engine-arg=nnue",
            "--movetime-ms",
            "100",
            "--opening-random-plies",
            "4",
            "--seed",
            "7",
        ])
        .expect("CLI args should parse");

        match cli.command {
            Command::SelfPlay(args) => {
                assert_eq!(args.games, 8);
                assert_eq!(args.threads, 0);
                assert_eq!(args.a_depth, 4);
                assert_eq!(args.b_depth, 4);
                assert_eq!(args.a_eval, EngineEvalKind::Nnue);
                assert_eq!(args.b_eval, EngineEvalKind::Handcrafted);
                assert_eq!(args.nnue, Some(PathBuf::from("model.nnue")));
                assert_eq!(args.b_nnue, Some(PathBuf::from("other.nnue")));
                assert_eq!(args.a_engine, Some(PathBuf::from("old-haitaka")));
                assert_eq!(args.a_engine_args, ["--eval", "nnue"]);
                assert_eq!(args.movetime_ms, Some(100));
                assert_eq!(args.opening_random_plies, 4);
                assert_eq!(args.seed, 7);
            }
            other => panic!("expected self-play command, got {other:?}"),
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
        assert_eq!(self_play_budget(3, None), SearchBudget::Depth(3));
        assert_eq!(
            self_play_budget(3, Some(100)),
            SearchBudget::Movetime {
                max_depth: 3,
                millis: 100
            }
        );
    }

    #[test]
    fn parses_usi_position_startpos_and_moves() {
        let board =
            parse_usi_position("position startpos moves 7g7f").expect("position should parse");
        let mut expected = Board::from_sfen(SFEN_STARTPOS).expect("startpos should parse");
        expected.try_play(Move::from_str("7g7f").unwrap()).unwrap();
        assert_eq!(board.to_string(), expected.to_string());
    }

    #[test]
    fn parses_usi_position_sfen() {
        let sfen = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
        let board =
            parse_usi_position(&format!("position sfen {sfen}")).expect("position should parse");
        assert_eq!(board.to_string(), sfen);
    }

    #[test]
    fn rejects_illegal_usi_position_move() {
        let err = parse_usi_position("position startpos moves 1a1b")
            .expect_err("illegal move should fail");
        assert!(err.to_string().contains("illegal move"));
    }

    #[test]
    fn parses_usi_go_budgets() {
        assert_eq!(
            parse_usi_go("go depth 4", 8).unwrap(),
            SearchBudget::Depth(4)
        );
        assert_eq!(
            parse_usi_go("go movetime 100", 8).unwrap(),
            SearchBudget::Movetime {
                max_depth: 8,
                millis: 100
            }
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

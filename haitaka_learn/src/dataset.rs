use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use haitaka::{Board, Color, Move, Piece, Square};
use haitaka_wasm::{
    NnueModel, SearchEvalMode, SearchSummary, search_board_impl_handcrafted,
    search_board_impl_with_eval_mode,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{ArtifactPaths, LoadedConfig, Ruleset};

const PACKED_SFEN_BYTES: usize = 64;
const ENTRY_BYTES: usize = PACKED_SFEN_BYTES + 8;

#[derive(Debug, Clone)]
pub struct DatasetOutput {
    pub output_dir: PathBuf,
    pub train_positions: u64,
    pub validation_positions: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct GenerateOptions {
    pub jobs: Option<u32>,
    pub resume: Option<bool>,
    pub shard_index: Option<u32>,
    pub shard_count: Option<u32>,
}

impl GenerateOptions {
    pub fn from_config(loaded: &LoadedConfig) -> Self {
        Self {
            jobs: Some(loaded.config.data.jobs),
            resume: Some(loaded.config.data.resume),
            shard_index: None,
            shard_count: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct DatasetManifest {
    dataset: String,
    ruleset: Ruleset,
    rule_id: u16,
    opening_sfen: String,
    game_count: u32,
    completed_games: u32,
    sampled_positions: u64,
    search_depth: u8,
    bootstrap_nnue: Option<String>,
    bootstrap_nnue_sha256: Option<String>,
    engine_revision: Option<String>,
    config_hash: String,
    generated_at_unix_ms: u128,
    build_mode: String,
    entry_bytes: usize,
    shard_count: usize,
    jobs: usize,
    resumed_shards: usize,
    generated_shards: usize,
    elapsed_seconds: f64,
    positions_per_second: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ShardManifest {
    dataset: String,
    ruleset: Ruleset,
    rule_id: u16,
    opening_sfen: String,
    game_start: u32,
    game_count: u32,
    sampled_positions: u64,
    search_depth: u8,
    bootstrap_nnue: Option<String>,
    #[serde(default)]
    bootstrap_nnue_sha256: Option<String>,
    engine_revision: Option<String>,
    config_hash: String,
    generated_at_unix_ms: u128,
    build_mode: String,
    entry_bytes: usize,
    shard_index: u32,
}

#[derive(Debug, Clone)]
struct PendingSample {
    board: Board,
    score: i16,
    game_ply: u16,
    side_to_move: Color,
}

#[derive(Debug, Clone, Copy)]
enum GameOutcome {
    Draw,
    Winner(Color),
}

impl GameOutcome {
    fn relative_to(self, side_to_move: Color) -> i8 {
        match self {
            Self::Draw => 0,
            Self::Winner(color) if color == side_to_move => 1,
            Self::Winner(_) => -1,
        }
    }
}

#[derive(Debug, Clone)]
enum Teacher {
    Handcrafted,
    Nnue {
        model: Arc<NnueModel>,
        bootstrap_sha256: String,
    },
}

impl Teacher {
    fn from_config(loaded: &LoadedConfig) -> Result<Self> {
        if let Some(path) = loaded.bootstrap_nnue() {
            if path.exists() {
                let bytes = fs::read(&path)
                    .with_context(|| format!("failed to read bootstrap NNUE {}", path.display()))?;
                let bootstrap_sha256 = hash_bytes_hex(&bytes);
                let model = NnueModel::from_bytes(&bytes).map_err(|err| {
                    anyhow!("failed to load bootstrap NNUE {}: {err}", path.display())
                })?;
                return Ok(Self::Nnue {
                    model: Arc::new(model),
                    bootstrap_sha256,
                });
            }
        }
        Ok(Self::Handcrafted)
    }

    fn describe(&self) -> &'static str {
        match self {
            Self::Handcrafted => "handcrafted",
            Self::Nnue { .. } => "nnue",
        }
    }

    fn bootstrap_sha256(&self) -> Option<&str> {
        match self {
            Self::Handcrafted => None,
            Self::Nnue {
                bootstrap_sha256, ..
            } => Some(bootstrap_sha256),
        }
    }

    fn search(&self, board: &Board, depth: u8) -> Result<SearchSummary> {
        match self {
            Self::Handcrafted => search_board_impl_handcrafted(board, depth)
                .map_err(|err| anyhow!("handcrafted teacher search failed: {err}")),
            Self::Nnue { model, .. } => search_board_impl_with_eval_mode(
                board,
                depth,
                model.clone(),
                SearchEvalMode::Incremental,
            )
            .map_err(|err| anyhow!("NNUE teacher search failed: {err}")),
        }
    }
}

pub fn generate_data(loaded: &LoadedConfig) -> Result<DatasetOutput> {
    generate_data_with_options(loaded, GenerateOptions::from_config(loaded))
}

pub fn generate_data_with_options(
    loaded: &LoadedConfig,
    options: GenerateOptions,
) -> Result<DatasetOutput> {
    loaded.ruleset_requires_matching_engine()?;
    let opening_sfen = loaded.opening_sfen()?;
    let _: Board = Board::from_sfen(&opening_sfen)
        .map_err(|err| anyhow!("invalid opening SFEN in config: {err}"))?;

    let artifacts = loaded.artifact_paths();
    artifacts.ensure_dirs()?;

    let teacher = Teacher::from_config(loaded)?;
    let engine_revision = detect_git_revision(loaded)?;
    let generated_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let shard_selector = ShardSelector::new(options.shard_index, options.shard_count)?;
    let jobs = resolve_jobs(options.jobs.unwrap_or(loaded.config.data.jobs))?;
    let resume = options.resume.unwrap_or(loaded.config.data.resume);
    let started = Instant::now();

    let train_positions = generate_split(
        "train",
        loaded,
        &artifacts,
        &teacher,
        &opening_sfen,
        loaded.config.data.train_games,
        &engine_revision,
        generated_at_unix_ms,
        jobs,
        resume,
        shard_selector,
    )?;
    let validation_positions = generate_split(
        "validation",
        loaded,
        &artifacts,
        &teacher,
        &opening_sfen,
        loaded.config.data.validation_games,
        &engine_revision,
        generated_at_unix_ms,
        jobs,
        resume,
        shard_selector,
    )?;

    println!(
        "generate-data finished elapsed={}",
        format_duration(started.elapsed())
    );

    Ok(DatasetOutput {
        output_dir: artifacts.output_dir,
        train_positions,
        validation_positions,
    })
}

fn generate_split(
    dataset_name: &str,
    loaded: &LoadedConfig,
    artifacts: &ArtifactPaths,
    teacher: &Teacher,
    opening_sfen: &str,
    game_count: u32,
    engine_revision: &Option<String>,
    generated_at_unix_ms: u128,
    jobs: usize,
    resume: bool,
    shard_selector: ShardSelector,
) -> Result<u64> {
    let (bin_path, manifest_path) = match dataset_name {
        "train" => (&artifacts.train_bin, &artifacts.train_manifest),
        "validation" => (&artifacts.validation_bin, &artifacts.validation_manifest),
        _ => bail!("unknown dataset split `{dataset_name}`"),
    };

    let split_started = Instant::now();
    let plans = shard_plans(game_count, loaded.config.data.shard_games, shard_selector);
    let total_games = plans.iter().map(|plan| plan.game_count).sum::<u32>();
    let progress = Arc::new(Mutex::new(Progress::new(
        dataset_name,
        total_games,
        loaded.config.data.progress_every_percent,
    )));
    let queue = Arc::new(Mutex::new(VecDeque::from(plans)));
    let results = Arc::new(Mutex::new(Vec::<ShardResult>::new()));
    let worker_count = jobs.min(queue.lock().unwrap().len().max(1));
    let mut worker_errors = Vec::new();

    thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            let results = Arc::clone(&results);
            let progress = Arc::clone(&progress);
            let teacher = teacher.clone();
            let loaded = loaded.clone();
            let artifacts = artifacts.clone();
            let opening_sfen = opening_sfen.to_string();
            let engine_revision = engine_revision.clone();
            let dataset_name = dataset_name.to_string();

            handles.push(scope.spawn(move || -> Result<()> {
                loop {
                    let Some(plan) = queue.lock().unwrap().pop_front() else {
                        return Ok(());
                    };
                    let result = generate_or_reuse_shard(
                        &dataset_name,
                        &loaded,
                        &artifacts,
                        &teacher,
                        &opening_sfen,
                        &engine_revision,
                        generated_at_unix_ms,
                        plan,
                        resume,
                    )?;
                    progress.lock().unwrap().record(&result);
                    results.lock().unwrap().push(result);
                }
            }));
        }

        for handle in handles {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(err)) => worker_errors.push(err),
                Err(_) => worker_errors.push(anyhow!("dataset worker panicked")),
            }
        }
    });

    if let Some(err) = worker_errors.into_iter().next() {
        return Err(err);
    }

    let mut shard_results = Arc::try_unwrap(results)
        .map_err(|_| anyhow!("failed to unwrap shard results"))?
        .into_inner()
        .map_err(|_| anyhow!("failed to lock shard results"))?;
    shard_results.sort_by_key(|result| result.manifest.game_start);
    let sampled_positions = assemble_shards(&shard_results, bin_path)?;
    let completed_games = shard_results
        .iter()
        .map(|result| result.manifest.game_count)
        .sum::<u32>();
    let resumed_shards = shard_results.iter().filter(|result| result.reused).count();
    let generated_shards = shard_results.len() - resumed_shards;
    let elapsed = split_started.elapsed();
    let positions_per_second = if elapsed.as_secs_f64() > 0.0 {
        sampled_positions as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };

    let manifest = DatasetManifest {
        dataset: dataset_name.to_string(),
        ruleset: loaded.config.rules.ruleset,
        rule_id: loaded.effective_rule_id()?,
        opening_sfen: opening_sfen.to_string(),
        game_count,
        completed_games,
        sampled_positions,
        search_depth: loaded.config.data.search_depth,
        bootstrap_nnue: bootstrap_nnue_path(loaded),
        bootstrap_nnue_sha256: teacher.bootstrap_sha256().map(str::to_string),
        engine_revision: engine_revision.clone(),
        config_hash: loaded.hash_hex.clone(),
        generated_at_unix_ms,
        build_mode: teacher_build_mode(loaded, teacher),
        entry_bytes: ENTRY_BYTES,
        shard_count: shard_results.len(),
        jobs,
        resumed_shards,
        generated_shards,
        elapsed_seconds: elapsed.as_secs_f64(),
        positions_per_second,
    };
    fs::write(manifest_path, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;

    Ok(sampled_positions)
}

pub fn merge_data(loaded: &LoadedConfig, input_dirs: &[PathBuf]) -> Result<DatasetOutput> {
    loaded.ruleset_requires_matching_engine()?;
    if input_dirs.is_empty() {
        bail!("merge-data requires at least one --input directory");
    }

    let artifacts = loaded.artifact_paths();
    artifacts.ensure_dirs()?;
    let opening_sfen = loaded.opening_sfen()?;
    let generated_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let train_positions = merge_split(
        "train",
        loaded,
        &artifacts.train_bin,
        &artifacts.train_manifest,
        input_dirs,
        loaded.config.data.train_games,
        &opening_sfen,
        generated_at_unix_ms,
    )?;
    let validation_positions = merge_split(
        "validation",
        loaded,
        &artifacts.validation_bin,
        &artifacts.validation_manifest,
        input_dirs,
        loaded.config.data.validation_games,
        &opening_sfen,
        generated_at_unix_ms,
    )?;

    Ok(DatasetOutput {
        output_dir: artifacts.output_dir,
        train_positions,
        validation_positions,
    })
}

#[derive(Debug, Clone, Copy)]
struct ShardSelector {
    index: u32,
    count: u32,
}

impl ShardSelector {
    fn new(index: Option<u32>, count: Option<u32>) -> Result<Self> {
        let index = index.unwrap_or(0);
        let count = count.unwrap_or(1);
        if count == 0 {
            bail!("--shard-count must be at least 1");
        }
        if index >= count {
            bail!("--shard-index must be less than --shard-count");
        }
        if count == 1 && index != 0 {
            bail!("--shard-index must be 0 when --shard-count is 1");
        }
        Ok(Self { index, count })
    }

    fn includes(self, shard_index: u32) -> bool {
        shard_index % self.count == self.index
    }
}

#[derive(Debug, Clone, Copy)]
struct ShardPlan {
    shard_index: u32,
    game_start: u32,
    game_count: u32,
}

#[derive(Debug, Clone)]
struct ShardResult {
    bin_path: PathBuf,
    manifest: ShardManifest,
    reused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MergeTeacherIdentity {
    bootstrap_nnue: Option<String>,
    bootstrap_nnue_sha256: Option<String>,
    engine_revision: Option<String>,
    build_mode: String,
}

impl MergeTeacherIdentity {
    fn from_manifest(manifest: &ShardManifest) -> Self {
        Self {
            bootstrap_nnue: manifest.bootstrap_nnue.clone(),
            bootstrap_nnue_sha256: manifest.bootstrap_nnue_sha256.clone(),
            engine_revision: manifest.engine_revision.clone(),
            build_mode: manifest.build_mode.clone(),
        }
    }
}

struct Progress {
    dataset_name: String,
    total_games: u32,
    completed_games: u32,
    sampled_positions: u64,
    started: Instant,
    next_percent: u32,
    every_percent: u32,
}

impl Progress {
    fn new(dataset_name: &str, total_games: u32, every_percent: u32) -> Self {
        Self {
            dataset_name: dataset_name.to_string(),
            total_games,
            completed_games: 0,
            sampled_positions: 0,
            started: Instant::now(),
            next_percent: every_percent.max(1),
            every_percent: every_percent.max(1),
        }
    }

    fn record(&mut self, result: &ShardResult) {
        self.completed_games += result.manifest.game_count;
        self.sampled_positions += result.manifest.sampled_positions;
        let percent = if self.total_games == 0 {
            100
        } else {
            ((u64::from(self.completed_games) * 100) / u64::from(self.total_games)) as u32
        };
        while self.next_percent <= percent.min(100) {
            self.print_line(self.next_percent);
            self.next_percent += self.every_percent;
        }
    }

    fn print_line(&self, percent: u32) {
        let elapsed = self.started.elapsed();
        let games_per_minute = if elapsed.as_secs_f64() > 0.0 {
            f64::from(self.completed_games) / elapsed.as_secs_f64() * 60.0
        } else {
            0.0
        };
        let eta = if self.completed_games == 0 || self.total_games == 0 {
            None
        } else {
            let seconds_per_game = elapsed.as_secs_f64() / f64::from(self.completed_games);
            let remaining_games = self.total_games.saturating_sub(self.completed_games);
            Some(Duration::from_secs_f64(
                seconds_per_game * f64::from(remaining_games),
            ))
        };
        println!(
            "{} {}% games={}/{} positions={} elapsed={} eta={} speed={:.1} games/min",
            self.dataset_name,
            percent,
            self.completed_games,
            self.total_games,
            self.sampled_positions,
            format_duration(elapsed),
            eta.map(format_duration)
                .unwrap_or_else(|| "--:--:--".to_string()),
            games_per_minute,
        );
    }
}

fn resolve_jobs(configured: u32) -> Result<usize> {
    if configured == 0 {
        Ok(thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1))
    } else {
        usize::try_from(configured).context("data.jobs does not fit into usize")
    }
}

fn shard_plans(game_count: u32, shard_games: u32, selector: ShardSelector) -> Vec<ShardPlan> {
    let mut plans = Vec::new();
    let mut game_start = 0;
    let mut shard_index = 0;
    while game_start < game_count {
        let remaining = game_count - game_start;
        let current_games = remaining.min(shard_games);
        if selector.includes(shard_index) {
            plans.push(ShardPlan {
                shard_index,
                game_start,
                game_count: current_games,
            });
        }
        game_start += current_games;
        shard_index += 1;
    }
    plans
}

fn generate_or_reuse_shard(
    dataset_name: &str,
    loaded: &LoadedConfig,
    artifacts: &ArtifactPaths,
    teacher: &Teacher,
    opening_sfen: &str,
    engine_revision: &Option<String>,
    generated_at_unix_ms: u128,
    plan: ShardPlan,
    resume: bool,
) -> Result<ShardResult> {
    let shards_dir = artifacts.datasets_dir.join("shards").join(dataset_name);
    fs::create_dir_all(&shards_dir)
        .with_context(|| format!("failed to create {}", shards_dir.display()))?;
    let bin_path = shards_dir.join(format!("shard-{:06}.bin", plan.shard_index));
    let manifest_path = shards_dir.join(format!("shard-{:06}.json", plan.shard_index));

    if resume {
        if let Some(result) = reusable_shard(
            loaded,
            dataset_name,
            opening_sfen,
            teacher,
            engine_revision,
            plan,
            &bin_path,
            &manifest_path,
        )? {
            return Ok(result);
        }
    }

    let tmp_bin = bin_path.with_extension("bin.tmp");
    let tmp_manifest = manifest_path.with_extension("json.tmp");
    let mut writer = BufWriter::new(
        File::create(&tmp_bin)
            .with_context(|| format!("failed to create {}", tmp_bin.display()))?,
    );
    let mut sampled_positions = 0u64;

    for game_index in plan.game_start..plan.game_start + plan.game_count {
        let entries =
            generate_game_entries(dataset_name, loaded, teacher, opening_sfen, game_index)
                .with_context(|| {
                    format!(
                        "failed to generate {dataset_name} game {game_index} in shard {}",
                        plan.shard_index
                    )
                })?;
        sampled_positions += (entries.len() / ENTRY_BYTES) as u64;
        writer.write_all(&entries)?;
    }
    writer.flush()?;

    let manifest = ShardManifest {
        dataset: dataset_name.to_string(),
        ruleset: loaded.config.rules.ruleset,
        rule_id: loaded.effective_rule_id()?,
        opening_sfen: opening_sfen.to_string(),
        game_start: plan.game_start,
        game_count: plan.game_count,
        sampled_positions,
        search_depth: loaded.config.data.search_depth,
        bootstrap_nnue: bootstrap_nnue_path(loaded),
        bootstrap_nnue_sha256: teacher.bootstrap_sha256().map(str::to_string),
        engine_revision: engine_revision.clone(),
        config_hash: loaded.hash_hex.clone(),
        generated_at_unix_ms,
        build_mode: teacher_build_mode(loaded, teacher),
        entry_bytes: ENTRY_BYTES,
        shard_index: plan.shard_index,
    };
    fs::write(&tmp_manifest, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("failed to write {}", tmp_manifest.display()))?;
    fs::rename(&tmp_bin, &bin_path)
        .with_context(|| format!("failed to rename {}", tmp_bin.display()))?;
    fs::rename(&tmp_manifest, &manifest_path)
        .with_context(|| format!("failed to rename {}", tmp_manifest.display()))?;

    Ok(ShardResult {
        bin_path,
        manifest,
        reused: false,
    })
}

fn reusable_shard(
    loaded: &LoadedConfig,
    dataset_name: &str,
    opening_sfen: &str,
    teacher: &Teacher,
    engine_revision: &Option<String>,
    plan: ShardPlan,
    bin_path: &Path,
    manifest_path: &Path,
) -> Result<Option<ShardResult>> {
    if !bin_path.exists() || !manifest_path.exists() {
        return Ok(None);
    }
    let manifest: ShardManifest = serde_json::from_slice(
        &fs::read(manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    if !shard_manifest_matches(loaded, dataset_name, opening_sfen, plan, &manifest)? {
        return Ok(None);
    }
    if !shard_teacher_matches(loaded, teacher, engine_revision, &manifest) {
        return Ok(None);
    }
    let expected_len = manifest
        .sampled_positions
        .checked_mul(ENTRY_BYTES as u64)
        .ok_or_else(|| anyhow!("shard length overflow"))?;
    let actual_len = fs::metadata(bin_path)
        .with_context(|| format!("failed to stat {}", bin_path.display()))?
        .len();
    if actual_len != expected_len {
        return Ok(None);
    }
    Ok(Some(ShardResult {
        bin_path: bin_path.to_path_buf(),
        manifest,
        reused: true,
    }))
}

fn shard_manifest_matches(
    loaded: &LoadedConfig,
    dataset_name: &str,
    opening_sfen: &str,
    plan: ShardPlan,
    manifest: &ShardManifest,
) -> Result<bool> {
    Ok(manifest.dataset == dataset_name
        && manifest.ruleset == loaded.config.rules.ruleset
        && manifest.rule_id == loaded.effective_rule_id()?
        && manifest.opening_sfen == opening_sfen
        && manifest.game_start == plan.game_start
        && manifest.game_count == plan.game_count
        && manifest.search_depth == loaded.config.data.search_depth
        && manifest.config_hash == loaded.hash_hex
        && manifest.entry_bytes == ENTRY_BYTES
        && manifest.shard_index == plan.shard_index)
}

fn shard_teacher_matches(
    loaded: &LoadedConfig,
    teacher: &Teacher,
    engine_revision: &Option<String>,
    manifest: &ShardManifest,
) -> bool {
    manifest.bootstrap_nnue == bootstrap_nnue_path(loaded)
        && manifest.bootstrap_nnue_sha256 == teacher.bootstrap_sha256().map(str::to_string)
        && manifest.engine_revision == *engine_revision
        && manifest.build_mode == teacher_build_mode(loaded, teacher)
}

fn generate_game_entries(
    dataset_name: &str,
    loaded: &LoadedConfig,
    teacher: &Teacher,
    opening_sfen: &str,
    game_index: u32,
) -> Result<Vec<u8>> {
    let mut rng =
        StdRng::seed_from_u64(game_seed(loaded.config.data.seed, dataset_name, game_index));
    let mut board = Board::from_sfen(opening_sfen)
        .map_err(|err| anyhow!("failed to parse opening SFEN: {err}"))?;
    let mut samples = Vec::new();
    let mut played_plies = 0u16;

    while played_plies < loaded.config.data.max_plies {
        if !has_both_kings(&board) {
            break;
        }
        let legal_moves = collect_legal_moves(&board);
        if legal_moves.is_empty() {
            break;
        }

        let should_sample = played_plies >= loaded.config.data.sample_start_ply
            && (played_plies - loaded.config.data.sample_start_ply)
                % loaded.config.data.sample_every_ply
                == 0
            && samples.len() < usize::from(loaded.config.data.max_positions_per_game);
        let needs_teacher =
            should_sample || played_plies >= loaded.config.data.opening_random_plies;
        let teacher_summary = if needs_teacher {
            Some(teacher.search(&board, loaded.config.data.search_depth)?)
        } else {
            None
        };

        if should_sample {
            let summary = teacher_summary
                .as_ref()
                .ok_or_else(|| anyhow!("teacher search unexpectedly missing"))?;
            let score = summary
                .best_score
                .unwrap_or_else(|| terminal_teacher_score(&board))
                .clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            samples.push(PendingSample {
                board: board.clone(),
                score,
                game_ply: played_plies,
                side_to_move: board.side_to_move(),
            });
        }

        let mv = if played_plies < loaded.config.data.opening_random_plies {
            legal_moves[rng.random_range(0..legal_moves.len())]
        } else {
            let best_move = teacher_summary
                .as_ref()
                .and_then(|summary| summary.best_move.as_deref())
                .ok_or_else(|| anyhow!("teacher search did not return a best move"))?;
            let mv: Move = best_move
                .parse()
                .map_err(|err| anyhow!("failed to parse teacher move `{best_move}`: {err}"))?;
            if !board.is_legal(mv) {
                bail!("teacher move `{best_move}` was not legal for position `{board}`");
            }
            mv
        };

        board.play_unchecked(mv);
        played_plies += 1;
    }

    let outcome = if played_plies >= loaded.config.data.max_plies {
        GameOutcome::Draw
    } else if !board.has(Color::Black, Piece::King) {
        GameOutcome::Winner(Color::White)
    } else if !board.has(Color::White, Piece::King) {
        GameOutcome::Winner(Color::Black)
    } else {
        match board.status() {
            haitaka::GameStatus::Won => GameOutcome::Winner(!board.side_to_move()),
            haitaka::GameStatus::Drawn => GameOutcome::Draw,
            haitaka::GameStatus::Ongoing => GameOutcome::Draw,
        }
    };

    let mut entries = Vec::with_capacity(samples.len() * ENTRY_BYTES);
    for sample in samples {
        let game_result = outcome.relative_to(sample.side_to_move);
        let packed = pack_board_for_training(&sample.board)?;
        write_training_entry(
            &mut entries,
            &packed,
            sample.score,
            0,
            sample.game_ply,
            game_result,
        )?;
    }
    Ok(entries)
}

fn assemble_shards(shard_results: &[ShardResult], bin_path: &Path) -> Result<u64> {
    let mut writer = BufWriter::new(
        File::create(bin_path)
            .with_context(|| format!("failed to create {}", bin_path.display()))?,
    );
    let mut sampled_positions = 0u64;
    let mut buffer = Vec::new();
    for result in shard_results {
        buffer.clear();
        File::open(&result.bin_path)
            .with_context(|| format!("failed to open {}", result.bin_path.display()))?
            .read_to_end(&mut buffer)
            .with_context(|| format!("failed to read {}", result.bin_path.display()))?;
        writer.write_all(&buffer)?;
        sampled_positions += result.manifest.sampled_positions;
    }
    writer.flush()?;
    Ok(sampled_positions)
}

fn merge_split(
    dataset_name: &str,
    loaded: &LoadedConfig,
    bin_path: &Path,
    manifest_path: &Path,
    input_dirs: &[PathBuf],
    game_count: u32,
    opening_sfen: &str,
    generated_at_unix_ms: u128,
) -> Result<u64> {
    let started = Instant::now();
    let mut by_start = BTreeMap::new();
    let mut teacher_identity = None;
    for input_dir in input_dirs {
        let shard_dir = input_dir.join("datasets").join("shards").join(dataset_name);
        if !shard_dir.exists() {
            continue;
        }
        let entries = fs::read_dir(&shard_dir)
            .with_context(|| format!("failed to read shard dir {}", shard_dir.display()))?;
        for entry in entries {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let manifest: ShardManifest = serde_json::from_slice(
                &fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?,
            )
            .with_context(|| format!("failed to parse {}", path.display()))?;
            validate_merge_shard(
                loaded,
                dataset_name,
                opening_sfen,
                &mut teacher_identity,
                &manifest,
            )
            .with_context(|| format!("invalid shard manifest {}", path.display()))?;
            let bin = path.with_extension("bin");
            validate_shard_bin(&bin, &manifest)?;
            let game_start = manifest.game_start;
            let result = ShardResult {
                bin_path: bin,
                manifest,
                reused: false,
            };
            if by_start.insert(game_start, result).is_some() {
                bail!("duplicate {dataset_name} shard starting at game {game_start}");
            }
        }
    }

    let mut expected_start = 0;
    let mut shard_results = Vec::new();
    for (_, result) in by_start {
        if result.manifest.game_start != expected_start {
            bail!(
                "missing {dataset_name} shard range: expected game_start {expected_start}, got {}",
                result.manifest.game_start
            );
        }
        expected_start += result.manifest.game_count;
        shard_results.push(result);
    }
    if expected_start != game_count {
        bail!("incomplete {dataset_name} shards: covered {expected_start}/{game_count} games");
    }

    let sampled_positions = assemble_shards(&shard_results, bin_path)?;
    let elapsed = started.elapsed();
    let positions_per_second = if elapsed.as_secs_f64() > 0.0 {
        sampled_positions as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };
    let manifest = DatasetManifest {
        dataset: dataset_name.to_string(),
        ruleset: loaded.config.rules.ruleset,
        rule_id: loaded.effective_rule_id()?,
        opening_sfen: opening_sfen.to_string(),
        game_count,
        completed_games: expected_start,
        sampled_positions,
        search_depth: loaded.config.data.search_depth,
        bootstrap_nnue: teacher_identity
            .as_ref()
            .and_then(|identity| identity.bootstrap_nnue.clone()),
        bootstrap_nnue_sha256: teacher_identity
            .as_ref()
            .and_then(|identity| identity.bootstrap_nnue_sha256.clone()),
        engine_revision: teacher_identity
            .as_ref()
            .and_then(|identity| identity.engine_revision.clone()),
        config_hash: loaded.hash_hex.clone(),
        generated_at_unix_ms,
        build_mode: format!("{}+merged", loaded.runtime_mode()),
        entry_bytes: ENTRY_BYTES,
        shard_count: shard_results.len(),
        jobs: 1,
        resumed_shards: shard_results.len(),
        generated_shards: 0,
        elapsed_seconds: elapsed.as_secs_f64(),
        positions_per_second,
    };
    fs::write(manifest_path, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;

    println!(
        "merged {dataset_name} games={expected_start}/{game_count} positions={sampled_positions} shards={} elapsed={}",
        shard_results.len(),
        format_duration(elapsed)
    );

    Ok(sampled_positions)
}

fn validate_merge_shard(
    loaded: &LoadedConfig,
    dataset_name: &str,
    opening_sfen: &str,
    teacher_identity: &mut Option<MergeTeacherIdentity>,
    manifest: &ShardManifest,
) -> Result<()> {
    ensure_merge(
        manifest.dataset == dataset_name,
        "dataset name does not match",
    )?;
    ensure_merge(
        manifest.ruleset == loaded.config.rules.ruleset,
        "ruleset does not match",
    )?;
    ensure_merge(
        manifest.rule_id == loaded.effective_rule_id()?,
        "rule_id does not match",
    )?;
    ensure_merge(
        manifest.opening_sfen == opening_sfen,
        "opening_sfen does not match",
    )?;
    ensure_merge(
        manifest.search_depth == loaded.config.data.search_depth,
        "search_depth does not match",
    )?;
    ensure_merge(
        manifest.config_hash == loaded.hash_hex,
        "config_hash does not match",
    )?;
    validate_merge_teacher_identity(teacher_identity, manifest)?;
    ensure_merge(
        manifest.entry_bytes == ENTRY_BYTES,
        "entry_bytes does not match",
    )?;
    Ok(())
}

fn validate_merge_teacher_identity(
    expected: &mut Option<MergeTeacherIdentity>,
    manifest: &ShardManifest,
) -> Result<()> {
    let current = MergeTeacherIdentity::from_manifest(manifest);
    if let Some(expected) = expected.as_ref() {
        ensure_merge(
            current.bootstrap_nnue_sha256 == expected.bootstrap_nnue_sha256,
            "bootstrap_nnue_sha256 does not match",
        )?;
        ensure_merge(
            current.engine_revision == expected.engine_revision,
            "engine_revision does not match",
        )?;
        ensure_merge(
            current.build_mode == expected.build_mode,
            "build_mode does not match",
        )?;
    } else {
        *expected = Some(current);
    }
    Ok(())
}

fn ensure_merge(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        bail!("{message}")
    }
}

fn validate_shard_bin(bin_path: &Path, manifest: &ShardManifest) -> Result<()> {
    let expected_len = manifest
        .sampled_positions
        .checked_mul(ENTRY_BYTES as u64)
        .ok_or_else(|| anyhow!("shard length overflow"))?;
    let actual_len = fs::metadata(bin_path)
        .with_context(|| format!("failed to stat {}", bin_path.display()))?
        .len();
    if actual_len != expected_len {
        bail!(
            "shard {} has {} bytes, expected {}",
            bin_path.display(),
            actual_len,
            expected_len
        );
    }
    Ok(())
}

fn bootstrap_nnue_path(loaded: &LoadedConfig) -> Option<String> {
    loaded
        .bootstrap_nnue()
        .map(|path| path.display().to_string())
}

fn teacher_build_mode(loaded: &LoadedConfig, teacher: &Teacher) -> String {
    format!("{}+teacher:{}", loaded.runtime_mode(), teacher.describe())
}

fn hash_bytes_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn game_seed(base_seed: u64, dataset_name: &str, game_index: u32) -> u64 {
    let split_key = match dataset_name {
        "train" => 0x7472_6169_6e00_0000,
        "validation" => 0x7661_6c69_6400_0000,
        _ => 0x6461_7461_0000_0000,
    };
    splitmix64(base_seed ^ split_key ^ u64::from(game_index))
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn terminal_teacher_score(board: &Board) -> i32 {
    match board.status() {
        haitaka::GameStatus::Won => -30_000,
        haitaka::GameStatus::Drawn => 0,
        haitaka::GameStatus::Ongoing => 0,
    }
}

fn collect_legal_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|piece_moves| {
        moves.extend(piece_moves);
        false
    });
    moves
}

fn has_both_kings(board: &Board) -> bool {
    board.has(Color::Black, Piece::King) && board.has(Color::White, Piece::King)
}

fn detect_git_revision(loaded: &LoadedConfig) -> Result<Option<String>> {
    let Some(repo_root) = find_haitaka_workspace_root(&loaded.path) else {
        return Ok(None);
    };
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok();
    Ok(output
        .filter(|result| result.status.success())
        .map(|result| String::from_utf8_lossy(&result.stdout).trim().to_string()))
}

fn find_haitaka_workspace_root(config_path: &Path) -> Option<&Path> {
    config_path
        .parent()
        .into_iter()
        .flat_map(Path::ancestors)
        .find(|candidate| {
            candidate.join("Cargo.toml").is_file() && candidate.join("haitaka_learn").is_dir()
        })
}

fn write_training_entry(
    writer: &mut impl Write,
    packed_sfen: &[u8; PACKED_SFEN_BYTES],
    score: i16,
    teacher_move: u16,
    game_ply: u16,
    game_result: i8,
) -> Result<()> {
    writer.write_all(packed_sfen)?;
    writer.write_all(&score.to_le_bytes())?;
    writer.write_all(&teacher_move.to_le_bytes())?;
    writer.write_all(&game_ply.to_le_bytes())?;
    writer.write_all(&[game_result as u8])?;
    writer.write_all(&[0])?;
    Ok(())
}

fn pack_board_for_training(board: &Board) -> Result<[u8; PACKED_SFEN_BYTES]> {
    let mut writer = BitWriter::default();
    let trainer_side_to_move = invert_color(board.side_to_move());
    writer.write_one_bit(matches!(trainer_side_to_move, TrainerColor::Black));

    let trainer_white_king = trainer_square_index(board.king(Color::Black));
    let trainer_black_king = trainer_square_index(board.king(Color::White));
    writer.write_n_bits(trainer_white_king as u32, 7);
    writer.write_n_bits(trainer_black_king as u32, 7);

    let mut trainer_board = [None; 81];
    for square_index in 0..Square::NUM {
        let square = Square::index_const(square_index);
        if let Some(colored) = board.colored_piece_on(square) {
            let trainer_square = trainer_square_index(square);
            trainer_board[trainer_square] = Some(TrainerPiece {
                color: invert_color(colored.color),
                piece_type: trainer_piece_type(colored.piece),
            });
        }
    }

    for rank in (0..9).rev() {
        for file in 0..9 {
            let square_index = rank * 9 + file;
            if square_index == trainer_white_king || square_index == trainer_black_king {
                continue;
            }

            match trainer_board[square_index] {
                None => writer.write_huffman_empty(),
                Some(piece) => writer.write_board_piece(piece),
            }
        }
    }

    let hand_counts = trainer_hand_counts(board);
    for trainer_color in [TrainerColor::White, TrainerColor::Black] {
        for piece_type in 0..10 {
            writer.write_n_bits(hand_counts[trainer_color as usize][piece_type] as u32, 5);
        }
    }

    for _ in 0..4 {
        writer.write_one_bit(false);
    }
    writer.write_one_bit(false);

    let fullmove = board.move_number();
    writer.write_n_bits(0, 6);
    writer.write_n_bits(u32::from(fullmove & 0xff), 8);
    writer.write_n_bits(u32::from(fullmove >> 8), 8);
    writer.write_one_bit(false);

    Ok(writer.finish())
}

fn trainer_hand_counts(board: &Board) -> [[u8; 10]; 2] {
    let mut counts = [[0u8; 10]; 2];
    for (color, trainer_color) in [
        (Color::Black, TrainerColor::White),
        (Color::White, TrainerColor::Black),
    ] {
        for piece in [
            Piece::Pawn,
            Piece::Lance,
            Piece::Knight,
            Piece::Silver,
            Piece::Bishop,
            Piece::Rook,
            Piece::Gold,
        ] {
            counts[trainer_color as usize][trainer_piece_type(piece)] =
                board.num_in_hand(color, piece);
        }
    }
    counts
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrainerPiece {
    color: TrainerColor,
    piece_type: usize,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrainerColor {
    White = 0,
    Black = 1,
}

fn invert_color(color: Color) -> TrainerColor {
    match color {
        Color::Black => TrainerColor::White,
        Color::White => TrainerColor::Black,
    }
}

fn trainer_piece_type(piece: Piece) -> usize {
    match piece {
        Piece::Bishop => 0,
        Piece::Rook => 1,
        Piece::Silver => 2,
        Piece::PRook => 3,
        Piece::Pawn => 4,
        Piece::Lance => 5,
        Piece::Knight => 6,
        Piece::Gold | Piece::Tokin | Piece::PLance | Piece::PKnight | Piece::PSilver => 7,
        Piece::PBishop => 8,
        Piece::King => 9,
    }
}

fn trainer_square_index(square: Square) -> usize {
    let file = 8usize - square.file() as usize;
    let rank = 8usize - square.rank() as usize;
    file + rank * 9
}

#[derive(Debug)]
struct BitWriter {
    data: [u8; PACKED_SFEN_BYTES],
    bit_cursor: usize,
}

impl Default for BitWriter {
    fn default() -> Self {
        Self {
            data: [0; PACKED_SFEN_BYTES],
            bit_cursor: 0,
        }
    }
}

impl BitWriter {
    fn write_one_bit(&mut self, bit: bool) {
        if bit {
            self.data[self.bit_cursor / 8] |= 1 << (self.bit_cursor % 8);
        }
        self.bit_cursor += 1;
    }

    fn write_n_bits(&mut self, value: u32, bits: usize) {
        for shift in 0..bits {
            self.write_one_bit(((value >> shift) & 1) != 0);
        }
    }

    fn write_huffman_empty(&mut self) {
        self.write_one_bit(false);
    }

    fn write_board_piece(&mut self, piece: TrainerPiece) {
        let code = 1u32 + 2u32 * (piece.piece_type as u32);
        self.write_n_bits(code, 5);
        self.write_one_bit(matches!(piece.color, TrainerColor::Black));
    }

    fn finish(self) -> [u8; PACKED_SFEN_BYTES] {
        self.data
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::config::LoadedConfig;
    use tempfile::tempdir;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FeatureSignature {
        side_to_move: TrainerColor,
        white_king: usize,
        black_king: usize,
        board: [Option<TrainerPiece>; 81],
        hands: [[u8; 10]; 2],
        fullmove: u16,
    }

    #[test]
    fn packed_entry_size_matches_trainer_layout() {
        assert_eq!(ENTRY_BYTES, 72);
    }

    #[test]
    fn packer_preserves_feature_signature() {
        let board = Board::from_sfen(
            "lnsgkgsnl/1r5b1/pppp1pppp/4p4/4+P4/9/PPPP1PPPP/1B5R1/LNSGKGSNL b - 3",
        )
        .unwrap();
        let packed = pack_board_for_training(&board).unwrap();
        let decoded = decode_signature(&packed);

        assert_eq!(decoded, signature_for_board(&board));
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        not(any(feature = "annan", feature = "anhoku", feature = "antouzai"))
    ))]
    fn generate_data_smoke_test_writes_non_empty_shards() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("haitaka_learn.toml");
        let ruleset = if cfg!(feature = "annan") {
            "annan"
        } else if cfg!(feature = "anhoku") {
            "anhoku"
        } else if cfg!(feature = "antouzai") {
            "antouzai"
        } else {
            "standard"
        };
        fs::write(
            &config_path,
            format!(
                r#"
[rules]
ruleset = "{ruleset}"

[paths]
output_dir = "out"

[data]
train_games = 1
validation_games = 1
max_plies = 8
search_depth = 1
opening_random_plies = 2
sample_start_ply = 0
sample_every_ply = 1
max_positions_per_game = 4
seed = 7

[verify]
run_search_smoke = false
"#,
            ),
        )
        .unwrap();

        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        let output = generate_data(&loaded).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);

        let artifacts = loaded.artifact_paths();
        assert!(artifacts.train_bin.exists());
        assert!(artifacts.validation_bin.exists());
        assert!(fs::metadata(&artifacts.train_bin).unwrap().len() > 0);
        assert!(fs::metadata(&artifacts.validation_bin).unwrap().len() > 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        not(any(feature = "annan", feature = "anhoku", feature = "antouzai"))
    ))]
    fn generation_is_deterministic_across_job_counts() {
        let temp = tempdir().unwrap();
        let ruleset = active_test_ruleset();
        let config_one = temp.path().join("one.toml");
        let config_two = temp.path().join("two.toml");
        fs::write(&config_one, deterministic_test_config(ruleset, "out-one")).unwrap();
        fs::write(&config_two, deterministic_test_config(ruleset, "out-two")).unwrap();

        let one = LoadedConfig::from_path(&config_one).unwrap();
        let two = LoadedConfig::from_path(&config_two).unwrap();
        generate_data_with_options(
            &one,
            GenerateOptions {
                jobs: Some(1),
                resume: Some(false),
                shard_index: None,
                shard_count: None,
            },
        )
        .unwrap();
        generate_data_with_options(
            &two,
            GenerateOptions {
                jobs: Some(2),
                resume: Some(false),
                shard_index: None,
                shard_count: None,
            },
        )
        .unwrap();

        assert_eq!(
            fs::read(one.artifact_paths().train_bin).unwrap(),
            fs::read(two.artifact_paths().train_bin).unwrap()
        );
        assert_eq!(
            fs::read(one.artifact_paths().validation_bin).unwrap(),
            fs::read(two.artifact_paths().validation_bin).unwrap()
        );
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        not(any(feature = "annan", feature = "anhoku", feature = "antouzai"))
    ))]
    fn resume_reuses_completed_shards() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("resume.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        generate_data(&loaded).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(loaded.artifact_paths().train_manifest).unwrap())
                .unwrap();
        assert!(manifest["resumed_shards"].as_u64().unwrap() > 0);
        assert_eq!(manifest["generated_shards"].as_u64().unwrap(), 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        not(any(feature = "annan", feature = "anhoku", feature = "antouzai"))
    ))]
    fn resume_regenerates_shards_when_teacher_identity_changes() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("resume-teacher.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        mutate_first_shard_manifest(&loaded, "train", |manifest| {
            manifest["build_mode"] = serde_json::Value::String("stale-teacher".to_string());
        });
        generate_data(&loaded).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(loaded.artifact_paths().train_manifest).unwrap())
                .unwrap();
        assert!(manifest["generated_shards"].as_u64().unwrap() > 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        not(any(feature = "annan", feature = "anhoku", feature = "antouzai"))
    ))]
    fn merge_data_combines_distributed_shards() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data_with_options(
            &loaded,
            GenerateOptions {
                jobs: Some(1),
                resume: Some(false),
                shard_index: Some(0),
                shard_count: Some(2),
            },
        )
        .unwrap();
        let first_input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &first_input).unwrap();

        generate_data_with_options(
            &loaded,
            GenerateOptions {
                jobs: Some(1),
                resume: Some(false),
                shard_index: Some(1),
                shard_count: Some(2),
            },
        )
        .unwrap();
        let second_input = temp.path().join("machine-b");
        fs::rename(loaded.artifact_paths().output_dir, &second_input).unwrap();

        let output = merge_data(&loaded, &[first_input, second_input]).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
        assert!(loaded.artifact_paths().train_bin.exists());
        assert!(loaded.artifact_paths().validation_bin.exists());
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        not(any(feature = "annan", feature = "anhoku", feature = "antouzai"))
    ))]
    fn merge_treats_empty_shard_lanes_as_empty_inputs() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-empty-lane.toml");
        fs::write(
            &config_path,
            distributed_empty_lane_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data_with_options(
            &loaded,
            GenerateOptions {
                jobs: Some(1),
                resume: Some(false),
                shard_index: Some(0),
                shard_count: Some(2),
            },
        )
        .unwrap();
        let first_input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &first_input).unwrap();

        generate_data_with_options(
            &loaded,
            GenerateOptions {
                jobs: Some(1),
                resume: Some(false),
                shard_index: Some(1),
                shard_count: Some(2),
            },
        )
        .unwrap();
        let second_input = temp.path().join("machine-b");
        fs::rename(loaded.artifact_paths().output_dir, &second_input).unwrap();

        assert!(
            !second_input
                .join("datasets")
                .join("shards")
                .join("validation")
                .exists()
        );

        let output = merge_data(&loaded, &[first_input, second_input]).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        not(any(feature = "annan", feature = "anhoku", feature = "antouzai"))
    ))]
    fn merge_rejects_shards_with_mismatched_teacher_identity() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-teacher.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();
        mutate_first_shard_manifest_in_dir(&input, "train", |manifest| {
            manifest["engine_revision"] = serde_json::Value::String("other-revision".to_string());
        });

        let err = format!("{:?}", merge_data(&loaded, &[input]).unwrap_err());
        assert!(err.contains("engine_revision does not match"));
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        not(any(feature = "annan", feature = "anhoku", feature = "antouzai"))
    ))]
    fn merge_allows_shards_with_different_bootstrap_paths() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-bootstrap-path.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();
        for dataset_name in ["train", "validation"] {
            mutate_first_shard_manifest_in_dir(&input, dataset_name, |manifest| {
                manifest["bootstrap_nnue"] =
                    serde_json::Value::String("/different/machine/bootstrap.nnue".to_string());
            });
        }

        let output = merge_data(&loaded, &[input]).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        not(any(feature = "annan", feature = "anhoku", feature = "antouzai"))
    ))]
    fn merge_does_not_require_local_bootstrap_file() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-missing-bootstrap.toml");
        fs::write(
            &config_path,
            distributed_empty_lane_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();
        for dataset_name in ["train", "validation"] {
            mutate_first_shard_manifest_in_dir(&input, dataset_name, |manifest| {
                manifest["bootstrap_nnue"] =
                    serde_json::Value::String("/worker/bootstrap.nnue".to_string());
                manifest["bootstrap_nnue_sha256"] =
                    serde_json::Value::String("same-bootstrap-hash".to_string());
                manifest["build_mode"] =
                    serde_json::Value::String(format!("{}+teacher:nnue", active_test_ruleset()));
            });
        }

        let output = merge_data(&loaded, &[input]).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    #[cfg(not(any(feature = "annan", feature = "anhoku", feature = "antouzai")))]
    fn handicap_generate_data_smoke_test_writes_non_empty_shards() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("haitaka_learn.toml");
        fs::write(
            &config_path,
            r#"
[rules]
ruleset = "handicap"
handicap = "six-piece"

[paths]
output_dir = "out"

[data]
train_games = 1
validation_games = 1
max_plies = 8
search_depth = 1
opening_random_plies = 2
sample_start_ply = 0
sample_every_ply = 1
max_positions_per_game = 4
seed = 9
"#,
        )
        .unwrap();

        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        let output = generate_data(&loaded).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    fn finds_workspace_root_from_root_config_path() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir.parent().unwrap();
        let config_path = workspace_root.join("haitaka_learn.toml");

        let detected = find_haitaka_workspace_root(&config_path).unwrap();

        assert_eq!(detected, workspace_root);
    }

    fn active_test_ruleset() -> &'static str {
        if cfg!(feature = "annan") {
            "annan"
        } else if cfg!(feature = "anhoku") {
            "anhoku"
        } else if cfg!(feature = "antouzai") {
            "antouzai"
        } else {
            "standard"
        }
    }

    fn deterministic_test_config(ruleset: &str, output_dir: &str) -> String {
        format!(
            r#"
[rules]
ruleset = "{ruleset}"

[paths]
output_dir = "{output_dir}"

[data]
train_games = 4
validation_games = 2
max_plies = 8
search_depth = 1
opening_random_plies = 2
sample_start_ply = 0
sample_every_ply = 1
max_positions_per_game = 4
seed = 7
jobs = 1
shard_games = 1
progress_every_percent = 50
resume = true

[verify]
run_search_smoke = false
"#,
        )
    }

    fn distributed_empty_lane_test_config(ruleset: &str, output_dir: &str) -> String {
        format!(
            r#"
[rules]
ruleset = "{ruleset}"

[paths]
output_dir = "{output_dir}"

[data]
train_games = 2
validation_games = 1
max_plies = 8
search_depth = 1
opening_random_plies = 2
sample_start_ply = 0
sample_every_ply = 1
max_positions_per_game = 4
seed = 7
jobs = 1
shard_games = 100
progress_every_percent = 50
resume = true

[verify]
run_search_smoke = false
"#,
        )
    }

    fn mutate_first_shard_manifest(
        loaded: &LoadedConfig,
        dataset_name: &str,
        mutate: impl FnOnce(&mut serde_json::Value),
    ) {
        mutate_first_shard_manifest_in_dir(
            &loaded.artifact_paths().output_dir,
            dataset_name,
            mutate,
        )
    }

    fn mutate_first_shard_manifest_in_dir(
        output_dir: &Path,
        dataset_name: &str,
        mutate: impl FnOnce(&mut serde_json::Value),
    ) {
        let shard_path = output_dir
            .join("datasets")
            .join("shards")
            .join(dataset_name)
            .join("shard-000000.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&shard_path).unwrap()).unwrap();
        mutate(&mut manifest);
        fs::write(&shard_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    }

    #[test]
    #[cfg(feature = "annan")]
    fn annan_nnue_teacher_handles_live_check_positions_without_sfen_roundtrip() {
        let bootstrap = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("../shogi-878ca61334a7.nnue");
        if !bootstrap.exists() {
            return;
        }

        let temp = tempdir().unwrap();
        let config_path = temp.path().join("haitaka_learn.toml");
        fs::write(
            &config_path,
            format!(
                r#"
[rules]
ruleset = "annan"
rule_id = 26
opening_sfen = "8k/6G2/7B1/9/9/9/9/9/K8 b R 1"

[paths]
output_dir = "out"
bootstrap_nnue = "{}"

[data]
train_games = 1
validation_games = 1
max_plies = 2
search_depth = 1
opening_random_plies = 0
sample_start_ply = 0
sample_every_ply = 1
max_positions_per_game = 2
seed = 5

[verify]
run_search_smoke = false
"#,
                bootstrap.display()
            ),
        )
        .unwrap();

        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        let output = generate_data(&loaded).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    #[cfg(feature = "annan")]
    fn annan_nnue_teacher_handles_king_capture_lines() {
        let bootstrap = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("../shogi-878ca61334a7.nnue");
        if !bootstrap.exists() {
            return;
        }

        let temp = tempdir().unwrap();
        let config_path = temp.path().join("haitaka_learn.toml");
        fs::write(
            &config_path,
            format!(
                r#"
[rules]
ruleset = "annan"
rule_id = 26
opening_sfen = "4R4/9/k8/9/9/4r4/4p4/9/4K4 b - 1"

[paths]
output_dir = "out"
bootstrap_nnue = "{}"

[data]
train_games = 1
validation_games = 1
max_plies = 4
search_depth = 2
opening_random_plies = 0
sample_start_ply = 0
sample_every_ply = 1
max_positions_per_game = 2
seed = 11

[verify]
run_search_smoke = false
"#,
                bootstrap.display()
            ),
        )
        .unwrap();

        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        let output = generate_data(&loaded).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    fn signature_for_board(board: &Board) -> FeatureSignature {
        let mut trainer_board = [None; 81];
        for square_index in 0..Square::NUM {
            let square = Square::index_const(square_index);
            if let Some(colored) = board.colored_piece_on(square) {
                if colored.piece == Piece::King {
                    continue;
                }
                trainer_board[trainer_square_index(square)] = Some(TrainerPiece {
                    color: invert_color(colored.color),
                    piece_type: trainer_piece_type(colored.piece),
                });
            }
        }
        FeatureSignature {
            side_to_move: invert_color(board.side_to_move()),
            white_king: trainer_square_index(board.king(Color::Black)),
            black_king: trainer_square_index(board.king(Color::White)),
            board: trainer_board,
            hands: trainer_hand_counts(board),
            fullmove: board.move_number(),
        }
    }

    fn decode_signature(packed: &[u8; PACKED_SFEN_BYTES]) -> FeatureSignature {
        let mut reader = BitReader::new(packed);
        let side_to_move = if reader.read_one_bit() {
            TrainerColor::Black
        } else {
            TrainerColor::White
        };
        let white_king = reader.read_n_bits(7) as usize;
        let black_king = reader.read_n_bits(7) as usize;
        let mut board = [None; 81];
        for rank in (0..9).rev() {
            for file in 0..9 {
                let square_index = rank * 9 + file;
                if square_index == white_king || square_index == black_king {
                    continue;
                }
                board[square_index] = reader.read_board_piece();
            }
        }

        let mut hands = [[0u8; 10]; 2];
        for color in 0..2 {
            for piece_type in 0..10 {
                hands[color][piece_type] = reader.read_n_bits(5) as u8;
            }
        }
        for _ in 0..4 {
            let _ = reader.read_one_bit();
        }
        let has_ep = reader.read_one_bit();
        assert!(!has_ep);
        let _rule50_low = reader.read_n_bits(6);
        let fullmove_low = reader.read_n_bits(8);
        let fullmove_high = reader.read_n_bits(8);
        let _rule50_high = reader.read_one_bit();
        FeatureSignature {
            side_to_move,
            white_king,
            black_king,
            board,
            hands,
            fullmove: ((fullmove_high << 8) | fullmove_low) as u16,
        }
    }

    struct BitReader<'a> {
        bytes: &'a [u8; PACKED_SFEN_BYTES],
        bit_cursor: usize,
    }

    impl<'a> BitReader<'a> {
        fn new(bytes: &'a [u8; PACKED_SFEN_BYTES]) -> Self {
            Self {
                bytes,
                bit_cursor: 0,
            }
        }

        fn read_one_bit(&mut self) -> bool {
            let bit = ((self.bytes[self.bit_cursor / 8] >> (self.bit_cursor % 8)) & 1) != 0;
            self.bit_cursor += 1;
            bit
        }

        fn read_n_bits(&mut self, bits: usize) -> u32 {
            let mut value = 0u32;
            for shift in 0..bits {
                if self.read_one_bit() {
                    value |= 1 << shift;
                }
            }
            value
        }

        fn read_board_piece(&mut self) -> Option<TrainerPiece> {
            if !self.read_one_bit() {
                return None;
            }

            let mut code = 1u32;
            for shift in 1..5 {
                if self.read_one_bit() {
                    code |= 1 << shift;
                }
            }
            let piece_type = ((code - 1) / 2) as usize;
            let color = if self.read_one_bit() {
                TrainerColor::Black
            } else {
                TrainerColor::White
            };
            Some(TrainerPiece { color, piece_type })
        }
    }
}

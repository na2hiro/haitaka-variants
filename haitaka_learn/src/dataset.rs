use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File};
use std::io::{BufWriter, IsTerminal, Read, Write, stderr, stdin};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use haitaka::{Board, Color, Move, Piece, Square};
use haitaka_wasm::{
    NnueModel, SearchEvalMode, SearchSummary, SearchWorkspace,
    search_board_impl_handcrafted_in_workspace, search_board_impl_with_eval_mode_in_workspace,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{ArtifactPaths, LoadedConfig, Ruleset, SamplingPolicy, TEACHER_MOVE_ENCODING};
use crate::openings::{GameOpeningMetadata, OpeningSource};

const PACKED_SFEN_BYTES: usize = 64;
pub(crate) const ENTRY_BYTES: usize = PACKED_SFEN_BYTES + 8;
#[cfg(all(unix, not(test)))]
const GRACEFUL_STOP_MESSAGE: &[u8] =
    "graceful stop中です。もう一度ctrl-cすることで即座に終了できます\n".as_bytes();

static GRACEFUL_STOP_STATE: AtomicU8 = AtomicU8::new(0);

#[derive(Debug, Clone)]
pub struct DatasetOutput {
    pub output_dir: PathBuf,
    pub train_positions: u64,
    pub validation_positions: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerateOptions {
    pub jobs: Option<u32>,
    pub resume: Option<bool>,
    pub shard_index: Option<u32>,
    pub shard_index_end: Option<u32>,
    pub shard_count: Option<u32>,
    pub ignore_identity_mismatch: bool,
}

impl GenerateOptions {
    pub fn from_config(loaded: &LoadedConfig) -> Self {
        Self {
            jobs: Some(loaded.config.data.jobs),
            resume: Some(loaded.config.data.resume),
            shard_index: None,
            shard_index_end: None,
            shard_count: None,
            ignore_identity_mismatch: false,
        }
    }
}

#[derive(Debug, Serialize)]
struct DatasetManifest {
    dataset: String,
    ruleset: Ruleset,
    rule_id: u16,
    opening_sfen: String,
    opening_policy: String,
    opening_suite_id: Option<String>,
    opening_suite_sha256: Option<String>,
    opening_transformation: String,
    opening_ids: Vec<String>,
    games: Vec<GameOpeningMetadata>,
    game_count: u32,
    completed_games: u32,
    sampled_positions: u64,
    search_depth: u8,
    label_search_depth: u8,
    rollout_search_depth: u8,
    label_searches: u64,
    rollout_searches: u64,
    label_search_states: u64,
    rollout_search_states: u64,
    bootstrap_nnue: Option<String>,
    bootstrap_nnue_sha256: Option<String>,
    engine_revision: Option<String>,
    config_hash: String,
    seed: u64,
    feature_family: String,
    sampling_phase: String,
    sample_after_opening: bool,
    teacher_move_encoding: String,
    opening_random_plies: u16,
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
    #[serde(default = "legacy_opening_policy")]
    opening_policy: String,
    #[serde(default)]
    opening_suite_id: Option<String>,
    #[serde(default)]
    opening_suite_sha256: Option<String>,
    #[serde(default = "legacy_opening_transformation")]
    opening_transformation: String,
    #[serde(default)]
    opening_ids: Vec<String>,
    #[serde(default)]
    games: Vec<GameOpeningMetadata>,
    game_start: u32,
    game_count: u32,
    sampled_positions: u64,
    search_depth: u8,
    #[serde(default)]
    label_search_depth: u8,
    #[serde(default)]
    rollout_search_depth: u8,
    #[serde(default)]
    label_searches: u64,
    #[serde(default)]
    rollout_searches: u64,
    #[serde(default)]
    label_search_states: u64,
    #[serde(default)]
    rollout_search_states: u64,
    bootstrap_nnue: Option<String>,
    #[serde(default)]
    bootstrap_nnue_sha256: Option<String>,
    engine_revision: Option<String>,
    config_hash: String,
    #[serde(default = "legacy_sampling_phase")]
    sampling_phase: String,
    #[serde(default)]
    sample_after_opening: bool,
    #[serde(default = "legacy_teacher_move_encoding")]
    teacher_move_encoding: String,
    generated_at_unix_ms: u128,
    build_mode: String,
    entry_bytes: usize,
    shard_index: u32,
}

fn legacy_sampling_phase() -> String {
    "fixed-phase-legacy".to_string()
}

fn legacy_opening_policy() -> String {
    "uniform-random".to_string()
}

fn legacy_opening_transformation() -> String {
    "none".to_string()
}

fn legacy_teacher_move_encoding() -> String {
    "legacy-ambiguous-u16".to_string()
}

impl ShardManifest {
    fn label_search_depth(&self) -> u8 {
        if self.label_search_depth == 0 {
            self.search_depth
        } else {
            self.label_search_depth
        }
    }

    fn rollout_search_depth(&self) -> u8 {
        if self.rollout_search_depth == 0 {
            self.search_depth
        } else {
            self.rollout_search_depth
        }
    }
}

#[derive(Debug, Clone)]
struct PendingSample {
    board: Board,
    score: i16,
    game_ply: u16,
    side_to_move: Color,
}

#[derive(Debug, Clone)]
struct GameEntries {
    entries: Vec<u8>,
    stats: SearchUseStats,
    opening: GameOpeningMetadata,
}

#[derive(Debug, Clone, Copy, Default)]
struct SearchUseStats {
    label_searches: u64,
    rollout_searches: u64,
    label_search_states: u64,
    rollout_search_states: u64,
}

impl SearchUseStats {
    fn record_label(&mut self, summary: &SearchSummary) {
        self.label_searches += 1;
        self.label_search_states += summary.states;
    }

    fn record_rollout(&mut self, summary: &SearchSummary) {
        self.rollout_searches += 1;
        self.rollout_search_states += summary.states;
    }

    fn add(&mut self, other: Self) {
        self.label_searches += other.label_searches;
        self.rollout_searches += other.rollout_searches;
        self.label_search_states += other.label_search_states;
        self.rollout_search_states += other.rollout_search_states;
    }
}

impl From<&ShardManifest> for SearchUseStats {
    fn from(manifest: &ShardManifest) -> Self {
        Self {
            label_searches: manifest.label_searches,
            rollout_searches: manifest.rollout_searches,
            label_search_states: manifest.label_search_states,
            rollout_search_states: manifest.rollout_search_states,
        }
    }
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

    fn search(
        &self,
        board: &Board,
        depth: u8,
        workspace: &mut SearchWorkspace,
    ) -> Result<SearchSummary> {
        match self {
            Self::Handcrafted => {
                search_board_impl_handcrafted_in_workspace(board, depth, workspace)
                    .map_err(|err| anyhow!("handcrafted teacher search failed: {err}"))
            }
            Self::Nnue { model, .. } => search_board_impl_with_eval_mode_in_workspace(
                board,
                depth,
                model.clone(),
                SearchEvalMode::Incremental,
                workspace,
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
    let _graceful_stop = GracefulStopGuard::install()?;
    loaded.ruleset_requires_matching_engine()?;
    let opening_sfen = loaded.opening_sfen()?;
    let _: Board = Board::from_sfen(&opening_sfen)
        .map_err(|err| anyhow!("invalid opening SFEN in config: {err}"))?;
    let opening_source = OpeningSource::from_config(loaded, &opening_sfen)?;

    let artifacts = loaded.artifact_paths();
    artifacts.ensure_dirs()?;

    let teacher = Teacher::from_config(loaded)?;
    let engine_revision = detect_git_revision(loaded)?;
    let generated_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let shard_selector = ShardSelector::new(
        options.shard_index,
        options.shard_index_end,
        options.shard_count,
    )?;
    let jobs = resolve_jobs(options.jobs.unwrap_or(loaded.config.data.jobs))?;
    let resume = options.resume.unwrap_or(loaded.config.data.resume);
    let allow_identity_mismatch = resolve_identity_mismatch(
        loaded,
        &artifacts,
        &teacher,
        &opening_sfen,
        &opening_source,
        &engine_revision,
        shard_selector,
        resume,
        options.ignore_identity_mismatch,
    )?;
    let started = Instant::now();

    let train_positions = generate_split(
        "train",
        loaded,
        &artifacts,
        &teacher,
        &opening_sfen,
        &opening_source,
        loaded.config.data.train_games,
        &engine_revision,
        generated_at_unix_ms,
        jobs,
        resume,
        shard_selector,
        allow_identity_mismatch,
    )?;
    if graceful_stop_requested() {
        bail!(
            "generate-data stopped gracefully after training split elapsed={}; \
             completed shard files were kept and can be resumed",
            format_duration(started.elapsed())
        );
    }
    let validation_positions = generate_split(
        "validation",
        loaded,
        &artifacts,
        &teacher,
        &opening_sfen,
        &opening_source,
        loaded.config.data.validation_games,
        &engine_revision,
        generated_at_unix_ms,
        jobs,
        resume,
        shard_selector,
        allow_identity_mismatch,
    )?;

    if graceful_stop_requested() {
        bail!(
            "generate-data stopped gracefully elapsed={}; \
             completed shard files were kept and can be resumed",
            format_duration(started.elapsed())
        );
    } else {
        println!(
            "generate-data finished elapsed={}",
            format_duration(started.elapsed())
        );
    }

    Ok(DatasetOutput {
        output_dir: artifacts.output_dir,
        train_positions,
        validation_positions,
    })
}

struct GracefulStopGuard {
    #[cfg(unix)]
    previous_handler: Option<libc::sighandler_t>,
}

impl GracefulStopGuard {
    fn install() -> Result<Self> {
        GRACEFUL_STOP_STATE.store(0, Ordering::SeqCst);
        install_graceful_stop_handler()
    }
}

#[cfg(test)]
fn install_graceful_stop_handler() -> Result<GracefulStopGuard> {
    Ok(GracefulStopGuard {
        #[cfg(unix)]
        previous_handler: None,
    })
}

#[cfg(all(unix, not(test)))]
fn install_graceful_stop_handler() -> Result<GracefulStopGuard> {
    let previous_handler = unsafe {
        let previous = libc::signal(
            libc::SIGINT,
            handle_sigint as *const () as libc::sighandler_t,
        );
        if previous == libc::SIG_ERR {
            bail!("failed to install SIGINT handler");
        }
        previous
    };
    Ok(GracefulStopGuard {
        previous_handler: Some(previous_handler),
    })
}

#[cfg(all(not(unix), not(test)))]
fn install_graceful_stop_handler() -> Result<GracefulStopGuard> {
    Ok(GracefulStopGuard {})
}

#[cfg(unix)]
impl Drop for GracefulStopGuard {
    fn drop(&mut self) {
        if let Some(previous_handler) = self.previous_handler {
            unsafe {
                libc::signal(libc::SIGINT, previous_handler);
            }
        }
    }
}

#[cfg(not(unix))]
impl Drop for GracefulStopGuard {
    fn drop(&mut self) {}
}

#[cfg(all(unix, not(test)))]
unsafe extern "C" fn handle_sigint(_: libc::c_int) {
    if GRACEFUL_STOP_STATE
        .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        unsafe {
            libc::write(
                libc::STDERR_FILENO,
                GRACEFUL_STOP_MESSAGE.as_ptr().cast(),
                GRACEFUL_STOP_MESSAGE.len(),
            );
        }
    } else {
        unsafe {
            libc::_exit(130);
        }
    }
}

fn graceful_stop_requested() -> bool {
    GRACEFUL_STOP_STATE.load(Ordering::SeqCst) != 0
}

fn generate_split(
    dataset_name: &str,
    loaded: &LoadedConfig,
    artifacts: &ArtifactPaths,
    teacher: &Teacher,
    opening_sfen: &str,
    opening_source: &OpeningSource,
    game_count: u32,
    engine_revision: &Option<String>,
    generated_at_unix_ms: u128,
    jobs: usize,
    resume: bool,
    shard_selector: ShardSelector,
    allow_identity_mismatch: bool,
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
            let opening_source = opening_source.clone();
            let engine_revision = engine_revision.clone();
            let dataset_name = dataset_name.to_string();

            handles.push(scope.spawn(move || -> Result<()> {
                loop {
                    let Some(plan) = next_shard_plan(&queue) else {
                        return Ok(());
                    };
                    let result = generate_or_reuse_shard(
                        &dataset_name,
                        &loaded,
                        &artifacts,
                        &teacher,
                        &opening_sfen,
                        &opening_source,
                        &engine_revision,
                        generated_at_unix_ms,
                        plan,
                        resume,
                        allow_identity_mismatch,
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

    if graceful_stop_requested() {
        println!("graceful stop requested; assembling completed {dataset_name} shards");
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
    let generated_positions = shard_results
        .iter()
        .filter(|result| !result.reused)
        .map(|result| result.manifest.sampled_positions)
        .sum::<u64>();
    let search_stats = shard_results
        .iter()
        .fold(SearchUseStats::default(), |mut stats, result| {
            stats.add(SearchUseStats::from(&result.manifest));
            stats
        });
    let games = shard_results
        .iter()
        .flat_map(|result| result.manifest.games.iter().cloned())
        .collect::<Vec<_>>();
    let opening_ids = games
        .iter()
        .map(|game| game.opening_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let elapsed = split_started.elapsed();
    let positions_per_second = if elapsed.as_secs_f64() > 0.0 {
        generated_positions as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };

    let manifest = DatasetManifest {
        dataset: dataset_name.to_string(),
        ruleset: loaded.config.rules.ruleset,
        rule_id: loaded.effective_rule_id()?,
        opening_sfen: opening_sfen.to_string(),
        opening_policy: opening_source.policy().to_string(),
        opening_suite_id: opening_source.suite_id().map(str::to_string),
        opening_suite_sha256: opening_source.suite_sha256().map(str::to_string),
        opening_transformation: opening_source.transformation().to_string(),
        opening_ids,
        games,
        game_count,
        completed_games,
        sampled_positions,
        search_depth: loaded.config.data.search_depth,
        label_search_depth: loaded.config.data.search_depth,
        rollout_search_depth: loaded.config.data.rollout_search_depth,
        label_searches: search_stats.label_searches,
        rollout_searches: search_stats.rollout_searches,
        label_search_states: search_stats.label_search_states,
        rollout_search_states: search_stats.rollout_search_states,
        bootstrap_nnue: bootstrap_nnue_path(loaded),
        bootstrap_nnue_sha256: teacher.bootstrap_sha256().map(str::to_string),
        engine_revision: engine_revision.clone(),
        config_hash: loaded.hash_hex.clone(),
        seed: loaded.config.data.seed,
        feature_family: loaded.training_features().to_string(),
        sampling_phase: loaded
            .config
            .data
            .sampling_policy
            .manifest_name()
            .to_string(),
        sample_after_opening: loaded.config.data.sampling_policy.samples_after_opening(),
        teacher_move_encoding: TEACHER_MOVE_ENCODING.to_string(),
        opening_random_plies: loaded.config.data.opening_random_plies,
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

fn next_shard_plan(queue: &Arc<Mutex<VecDeque<ShardPlan>>>) -> Option<ShardPlan> {
    next_shard_plan_with(queue, graceful_stop_requested)
}

fn next_shard_plan_with(
    queue: &Arc<Mutex<VecDeque<ShardPlan>>>,
    stop_requested: impl Fn() -> bool,
) -> Option<ShardPlan> {
    if stop_requested() {
        return None;
    }
    let mut queue = queue.lock().unwrap();
    if stop_requested() {
        return None;
    }
    queue.pop_front()
}

pub fn merge_data(
    loaded: &LoadedConfig,
    input_dirs: &[PathBuf],
    ignore_identity_mismatch: bool,
) -> Result<DatasetOutput> {
    loaded.ruleset_requires_matching_engine()?;
    if input_dirs.is_empty() {
        bail!("merge-data requires at least one --input directory");
    }

    let artifacts = loaded.artifact_paths();
    artifacts.ensure_dirs()?;
    let opening_sfen = loaded.opening_sfen()?;
    let opening_source = OpeningSource::from_config(loaded, &opening_sfen)?;
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
        &opening_source,
        generated_at_unix_ms,
        ignore_identity_mismatch,
    )?;
    let validation_positions = merge_split(
        "validation",
        loaded,
        &artifacts.validation_bin,
        &artifacts.validation_manifest,
        input_dirs,
        loaded.config.data.validation_games,
        &opening_sfen,
        &opening_source,
        generated_at_unix_ms,
        ignore_identity_mismatch,
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
    index_end: u32,
    count: u32,
}

impl ShardSelector {
    fn new(index: Option<u32>, index_end: Option<u32>, count: Option<u32>) -> Result<Self> {
        let index = index.unwrap_or(0);
        let index_end = index_end.unwrap_or(index);
        let count = count.unwrap_or(1);
        if count == 0 {
            bail!("--shard-count must be at least 1");
        }
        if index >= count {
            bail!("--shard-index must be less than --shard-count");
        }
        if index_end >= count {
            bail!("--shard-index-end must be less than --shard-count");
        }
        if index_end < index {
            bail!("--shard-index-end must be greater than or equal to --shard-index");
        }
        if count == 1 && index != 0 {
            bail!("--shard-index must be 0 when --shard-count is 1");
        }
        Ok(Self {
            index,
            index_end,
            count,
        })
    }

    fn selected_range(self, total_shards: u32) -> Range<u32> {
        let start = partition_boundary(total_shards, self.index, self.count);
        let end = partition_boundary(total_shards, self.index_end + 1, self.count);
        start..end
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
    generated_games: u32,
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
            generated_games: 0,
            started: Instant::now(),
            next_percent: every_percent.max(1),
            every_percent: every_percent.max(1),
        }
    }

    fn record(&mut self, result: &ShardResult) {
        self.completed_games += result.manifest.game_count;
        self.sampled_positions += result.manifest.sampled_positions;
        if !result.reused {
            self.generated_games += result.manifest.game_count;
        }
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
        // Throughput and ETA reflect freshly generated games only; restored
        // (resumed) shards complete instantly and would otherwise inflate them.
        let games_per_minute = if elapsed.as_secs_f64() > 0.0 {
            f64::from(self.generated_games) / elapsed.as_secs_f64() * 60.0
        } else {
            0.0
        };
        let eta = if self.generated_games == 0 || self.total_games == 0 {
            None
        } else {
            let seconds_per_game = elapsed.as_secs_f64() / f64::from(self.generated_games);
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
    let total_shards = game_count.div_ceil(shard_games);
    let selected_shards = selector.selected_range(total_shards);
    let mut game_start = 0;
    let mut shard_index = 0;
    while game_start < game_count {
        let remaining = game_count - game_start;
        let current_games = remaining.min(shard_games);
        if selected_shards.contains(&shard_index) {
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

fn partition_boundary(total_shards: u32, lane_index: u32, lane_count: u32) -> u32 {
    ((u64::from(total_shards) * u64::from(lane_index)) / u64::from(lane_count)) as u32
}

fn generate_or_reuse_shard(
    dataset_name: &str,
    loaded: &LoadedConfig,
    artifacts: &ArtifactPaths,
    teacher: &Teacher,
    opening_sfen: &str,
    opening_source: &OpeningSource,
    engine_revision: &Option<String>,
    generated_at_unix_ms: u128,
    plan: ShardPlan,
    resume: bool,
    allow_identity_mismatch: bool,
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
            opening_source,
            teacher,
            engine_revision,
            plan,
            &bin_path,
            &manifest_path,
            allow_identity_mismatch,
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
    let mut search_stats = SearchUseStats::default();
    let mut games = Vec::with_capacity(plan.game_count as usize);
    let mut search_workspace = SearchWorkspace::default();

    for game_index in plan.game_start..plan.game_start + plan.game_count {
        let shard_index = plan.shard_index;
        let error_context =
            format!("failed to generate {dataset_name} game {game_index} in shard {shard_index}");
        let game = generate_game_entries(
            dataset_name,
            loaded,
            teacher,
            &mut search_workspace,
            opening_source,
            game_index,
        )
        .context(error_context)?;
        sampled_positions += (game.entries.len() / ENTRY_BYTES) as u64;
        search_stats.add(game.stats);
        games.push(game.opening);
        writer.write_all(&game.entries)?;
    }
    writer.flush()?;

    let manifest = ShardManifest {
        dataset: dataset_name.to_string(),
        ruleset: loaded.config.rules.ruleset,
        rule_id: loaded.effective_rule_id()?,
        opening_sfen: opening_sfen.to_string(),
        opening_policy: opening_source.policy().to_string(),
        opening_suite_id: opening_source.suite_id().map(str::to_string),
        opening_suite_sha256: opening_source.suite_sha256().map(str::to_string),
        opening_transformation: opening_source.transformation().to_string(),
        opening_ids: games
            .iter()
            .map(|game| game.opening_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        games,
        game_start: plan.game_start,
        game_count: plan.game_count,
        sampled_positions,
        search_depth: loaded.config.data.search_depth,
        label_search_depth: loaded.config.data.search_depth,
        rollout_search_depth: loaded.config.data.rollout_search_depth,
        label_searches: search_stats.label_searches,
        rollout_searches: search_stats.rollout_searches,
        label_search_states: search_stats.label_search_states,
        rollout_search_states: search_stats.rollout_search_states,
        bootstrap_nnue: bootstrap_nnue_path(loaded),
        bootstrap_nnue_sha256: teacher.bootstrap_sha256().map(str::to_string),
        engine_revision: engine_revision.clone(),
        config_hash: loaded.hash_hex.clone(),
        sampling_phase: loaded
            .config
            .data
            .sampling_policy
            .manifest_name()
            .to_string(),
        sample_after_opening: loaded.config.data.sampling_policy.samples_after_opening(),
        teacher_move_encoding: TEACHER_MOVE_ENCODING.to_string(),
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
    opening_source: &OpeningSource,
    teacher: &Teacher,
    engine_revision: &Option<String>,
    plan: ShardPlan,
    bin_path: &Path,
    manifest_path: &Path,
    allow_identity_mismatch: bool,
) -> Result<Option<ShardResult>> {
    if !bin_path.exists() || !manifest_path.exists() {
        return Ok(None);
    }
    let manifest: ShardManifest = serde_json::from_slice(
        &fs::read(manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    if !shard_manifest_matches(
        loaded,
        dataset_name,
        opening_sfen,
        opening_source,
        plan,
        &manifest,
        allow_identity_mismatch,
    )? {
        return Ok(None);
    }
    if !shard_teacher_matches(
        loaded,
        teacher,
        engine_revision,
        &manifest,
        allow_identity_mismatch,
    ) {
        return Ok(None);
    }
    if !shard_bin_matches(bin_path, &manifest) {
        return Ok(None);
    }
    Ok(Some(ShardResult {
        bin_path: bin_path.to_path_buf(),
        manifest,
        reused: true,
    }))
}

/// Whether `bin_path` exists and has the exact byte length implied by the manifest.
/// A missing, partially written, or unreadable file is treated as not reusable.
fn shard_bin_matches(bin_path: &Path, manifest: &ShardManifest) -> bool {
    let Some(expected_len) = manifest.sampled_positions.checked_mul(ENTRY_BYTES as u64) else {
        return false;
    };
    fs::metadata(bin_path)
        .map(|meta| meta.len() == expected_len)
        .unwrap_or(false)
}

fn shard_manifest_matches(
    loaded: &LoadedConfig,
    dataset_name: &str,
    opening_sfen: &str,
    opening_source: &OpeningSource,
    plan: ShardPlan,
    manifest: &ShardManifest,
    ignore_identity: bool,
) -> Result<bool> {
    Ok(manifest.dataset == dataset_name
        && manifest.ruleset == loaded.config.rules.ruleset
        && manifest.rule_id == loaded.effective_rule_id()?
        && manifest.opening_sfen == opening_sfen
        && (ignore_identity
            || (manifest.opening_policy == opening_source.policy()
                && manifest.opening_suite_id.as_deref() == opening_source.suite_id()
                && manifest.opening_suite_sha256.as_deref() == opening_source.suite_sha256()
                && manifest.opening_transformation == opening_source.transformation()))
        && manifest.game_start == plan.game_start
        && manifest.game_count == plan.game_count
        && manifest.search_depth == loaded.config.data.search_depth
        && manifest.label_search_depth() == loaded.config.data.search_depth
        && manifest.rollout_search_depth() == loaded.config.data.rollout_search_depth
        && (ignore_identity
            || (manifest.sampling_phase == loaded.config.data.sampling_policy.manifest_name()
                && manifest.sample_after_opening
                    == loaded.config.data.sampling_policy.samples_after_opening()
                && manifest.teacher_move_encoding == TEACHER_MOVE_ENCODING))
        && (ignore_identity || manifest.config_hash == loaded.hash_hex)
        && manifest.entry_bytes == ENTRY_BYTES
        && manifest.shard_index == plan.shard_index)
}

fn shard_teacher_matches(
    loaded: &LoadedConfig,
    teacher: &Teacher,
    engine_revision: &Option<String>,
    manifest: &ShardManifest,
    ignore_identity: bool,
) -> bool {
    manifest.bootstrap_nnue == bootstrap_nnue_path(loaded)
        && manifest.bootstrap_nnue_sha256 == teacher.bootstrap_sha256().map(str::to_string)
        && (ignore_identity || manifest.engine_revision == *engine_revision)
        && manifest.build_mode == teacher_build_mode(loaded, teacher)
}

enum MismatchChoice {
    Abort,
    Reuse,
    Regenerate,
}

/// Decides whether resumed shards with a mismatching generation identity may be
/// reused. Returns `true` when such shards should be reused as-is.
#[allow(clippy::too_many_arguments)]
fn resolve_identity_mismatch(
    loaded: &LoadedConfig,
    artifacts: &ArtifactPaths,
    teacher: &Teacher,
    opening_sfen: &str,
    opening_source: &OpeningSource,
    engine_revision: &Option<String>,
    shard_selector: ShardSelector,
    resume: bool,
    ignore_identity_mismatch: bool,
) -> Result<bool> {
    if ignore_identity_mismatch {
        return Ok(true);
    }
    if !resume {
        return Ok(false);
    }
    let (mismatched_games, total_games) = detect_identity_mismatch(
        loaded,
        artifacts,
        teacher,
        opening_sfen,
        opening_source,
        engine_revision,
        shard_selector,
    )?;
    if mismatched_games == 0 {
        return Ok(false);
    }
    let percent = if total_games == 0 {
        0.0
    } else {
        f64::from(mismatched_games) / f64::from(total_games) * 100.0
    };
    match prompt_identity_mismatch_choice(percent)? {
        MismatchChoice::Abort => bail!(
            "aborting: existing shards have a mismatching generation identity (config, engine, opening, sampling, and/or teacher-move contract). \
             Re-run with --ignore-identity-mismatch to reuse them, or with --no-resume to regenerate."
        ),
        MismatchChoice::Reuse => Ok(true),
        MismatchChoice::Regenerate => Ok(false),
    }
}

/// Scans the shards this run would produce (our lane only) and counts how many
/// games sit in shards that would be reusable if and only if the git revision /
/// config hash checks were relaxed. Returns `(mismatched_games, total_games)`.
fn detect_identity_mismatch(
    loaded: &LoadedConfig,
    artifacts: &ArtifactPaths,
    teacher: &Teacher,
    opening_sfen: &str,
    opening_source: &OpeningSource,
    engine_revision: &Option<String>,
    shard_selector: ShardSelector,
) -> Result<(u32, u32)> {
    let mut mismatched_games = 0u32;
    let mut total_games = 0u32;
    for (dataset_name, game_count) in [
        ("train", loaded.config.data.train_games),
        ("validation", loaded.config.data.validation_games),
    ] {
        let shards_dir = artifacts.datasets_dir.join("shards").join(dataset_name);
        let plans = shard_plans(game_count, loaded.config.data.shard_games, shard_selector);
        for plan in plans {
            total_games += plan.game_count;
            let manifest_path = shards_dir.join(format!("shard-{:06}.json", plan.shard_index));
            if !manifest_path.exists() {
                continue;
            }
            let Ok(bytes) = fs::read(&manifest_path) else {
                continue;
            };
            let Ok(manifest) = serde_json::from_slice::<ShardManifest>(&bytes) else {
                continue;
            };
            // A shard is only reusable if its .bin sibling matches the manifest, same as
            // the reuse path; otherwise generate_or_reuse_shard regenerates it regardless
            // of identity, so it must not count toward a mismatch that blocks the run.
            let bin_path = shards_dir.join(format!("shard-{:06}.bin", plan.shard_index));
            if !shard_bin_matches(&bin_path, &manifest) {
                continue;
            }
            let strict =
                shard_manifest_matches(
                    loaded,
                    dataset_name,
                    opening_sfen,
                    opening_source,
                    plan,
                    &manifest,
                    false,
                )? && shard_teacher_matches(loaded, teacher, engine_revision, &manifest, false);
            let relaxed =
                shard_manifest_matches(
                    loaded,
                    dataset_name,
                    opening_sfen,
                    opening_source,
                    plan,
                    &manifest,
                    true,
                )? && shard_teacher_matches(loaded, teacher, engine_revision, &manifest, true);
            if relaxed && !strict {
                mismatched_games += plan.game_count;
            }
        }
    }
    Ok((mismatched_games, total_games))
}

fn prompt_identity_mismatch_choice(percent: f64) -> Result<MismatchChoice> {
    // Write the prompt to stderr so it stays visible even when stdout is redirected
    // to a log, and only prompt when both stdin and the prompt stream are terminals.
    let mut err = stderr();
    let _ = writeln!(
        err,
        "Generation identity mismatch found in existing shards \
         covering {percent:.1}% of this run's data."
    );
    if !stdin().is_terminal() || !err.is_terminal() {
        let _ = writeln!(
            err,
            "  not running interactively; aborting. Re-run with --ignore-identity-mismatch to reuse."
        );
        return Ok(MismatchChoice::Abort);
    }
    let _ = writeln!(err, "  1) Abort");
    let _ = writeln!(err, "  2) Resume, reusing the mismatched shards as-is");
    let _ = writeln!(
        err,
        "  3) Discard the mismatched shards and regenerate them"
    );
    let _ = write!(err, "Choice [1/2/3] (default 1): ");
    let _ = err.flush();
    let mut line = String::new();
    if stdin().read_line(&mut line)? == 0 {
        return Ok(MismatchChoice::Abort);
    }
    Ok(match line.trim() {
        "2" => MismatchChoice::Reuse,
        "3" => MismatchChoice::Regenerate,
        _ => MismatchChoice::Abort,
    })
}

fn generate_game_entries(
    dataset_name: &str,
    loaded: &LoadedConfig,
    teacher: &Teacher,
    search_workspace: &mut SearchWorkspace,
    opening_source: &OpeningSource,
    game_index: u32,
) -> Result<GameEntries> {
    let seed = game_seed(loaded.config.data.seed, dataset_name, game_index);
    let pair_seed = game_seed(loaded.config.data.seed, dataset_name, game_index / 2);
    let selected_opening = opening_source.select(pair_seed, game_index);
    let mut rng = StdRng::seed_from_u64(seed);
    let sample_origin = sampling_origin(
        seed,
        loaded.config.data.sampling_policy,
        loaded.config.data.sample_start_ply,
        loaded.config.data.opening_random_plies,
        loaded.config.data.sample_every_ply,
    );
    let mut board = Board::from_sfen(&selected_opening.sfen)
        .map_err(|err| anyhow!("failed to parse opening SFEN: {err}"))?;
    let mut samples = Vec::new();
    let mut played_plies = 0u16;
    let mut stats = SearchUseStats::default();

    while played_plies < loaded.config.data.max_plies {
        if !has_both_kings(&board) {
            break;
        }
        let legal_moves = collect_legal_moves(&board);
        if legal_moves.is_empty() {
            break;
        }

        let should_sample = played_plies >= sample_origin
            && (played_plies - sample_origin) % loaded.config.data.sample_every_ply == 0
            && samples.len() < usize::from(loaded.config.data.max_positions_per_game);
        let needs_rollout_search =
            played_plies >= loaded.config.data.opening_random_plies && !should_sample;
        let label_summary = if should_sample {
            let summary =
                teacher.search(&board, loaded.config.data.search_depth, search_workspace)?;
            stats.record_label(&summary);
            Some(summary)
        } else {
            None
        };
        let rollout_summary = if needs_rollout_search {
            let summary = teacher.search(
                &board,
                loaded.config.data.rollout_search_depth,
                search_workspace,
            )?;
            stats.record_rollout(&summary);
            Some(summary)
        } else {
            None
        };

        if should_sample {
            let summary = label_summary
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
            let summary = label_summary
                .as_ref()
                .or(rollout_summary.as_ref())
                .ok_or_else(|| anyhow!("rollout search unexpectedly missing"))?;
            searched_best_move(&board, summary)?
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
    Ok(GameEntries {
        entries,
        stats,
        opening: selected_opening.metadata,
    })
}

fn sampling_origin(
    game_seed: u64,
    policy: SamplingPolicy,
    sample_start_ply: u16,
    opening_random_plies: u16,
    sample_every_ply: u16,
) -> u16 {
    match policy {
        SamplingPolicy::PerGameRandomV1 => {
            let base = sample_start_ply.max(opening_random_plies);
            let phase = (splitmix64(game_seed ^ 0x7361_6d70_6c65_7631)
                % u64::from(sample_every_ply)) as u16;
            base.saturating_add(phase)
        }
        SamplingPolicy::FixedPhaseLegacy => sample_start_ply,
    }
}

fn searched_best_move(board: &Board, summary: &SearchSummary) -> Result<Move> {
    let best_move = summary
        .best_move
        .as_deref()
        .ok_or_else(|| anyhow!("teacher search did not return a best move"))?;
    let mv: Move = best_move
        .parse()
        .map_err(|err| anyhow!("failed to parse teacher move `{best_move}`: {err}"))?;
    if !board.is_legal(mv) {
        bail!("teacher move `{best_move}` was not legal for position `{board}`");
    }
    Ok(mv)
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
    opening_source: &OpeningSource,
    generated_at_unix_ms: u128,
    ignore_identity_mismatch: bool,
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
                opening_source,
                &mut teacher_identity,
                &manifest,
                ignore_identity_mismatch,
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
    let search_stats = shard_results
        .iter()
        .fold(SearchUseStats::default(), |mut stats, result| {
            stats.add(SearchUseStats::from(&result.manifest));
            stats
        });
    let games = shard_results
        .iter()
        .flat_map(|result| result.manifest.games.iter().cloned())
        .collect::<Vec<_>>();
    let opening_ids = games
        .iter()
        .map(|game| game.opening_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
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
        opening_policy: opening_source.policy().to_string(),
        opening_suite_id: opening_source.suite_id().map(str::to_string),
        opening_suite_sha256: opening_source.suite_sha256().map(str::to_string),
        opening_transformation: opening_source.transformation().to_string(),
        opening_ids,
        games,
        game_count,
        completed_games: expected_start,
        sampled_positions,
        search_depth: loaded.config.data.search_depth,
        label_search_depth: loaded.config.data.search_depth,
        rollout_search_depth: loaded.config.data.rollout_search_depth,
        label_searches: search_stats.label_searches,
        rollout_searches: search_stats.rollout_searches,
        label_search_states: search_stats.label_search_states,
        rollout_search_states: search_stats.rollout_search_states,
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
        seed: loaded.config.data.seed,
        feature_family: loaded.training_features().to_string(),
        sampling_phase: loaded
            .config
            .data
            .sampling_policy
            .manifest_name()
            .to_string(),
        sample_after_opening: loaded.config.data.sampling_policy.samples_after_opening(),
        teacher_move_encoding: TEACHER_MOVE_ENCODING.to_string(),
        opening_random_plies: loaded.config.data.opening_random_plies,
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
    opening_source: &OpeningSource,
    teacher_identity: &mut Option<MergeTeacherIdentity>,
    manifest: &ShardManifest,
    ignore_identity_mismatch: bool,
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
    if !ignore_identity_mismatch {
        ensure_merge(
            manifest.opening_policy == opening_source.policy(),
            "opening_policy does not match",
        )?;
        ensure_merge(
            manifest.opening_suite_id.as_deref() == opening_source.suite_id(),
            "opening_suite_id does not match",
        )?;
        ensure_merge(
            manifest.opening_suite_sha256.as_deref() == opening_source.suite_sha256(),
            "opening_suite_sha256 does not match",
        )?;
        ensure_merge(
            manifest.opening_transformation == opening_source.transformation(),
            "opening_transformation does not match",
        )?;
    }
    ensure_merge(
        manifest.search_depth == loaded.config.data.search_depth,
        "search_depth does not match",
    )?;
    ensure_merge(
        manifest.label_search_depth() == loaded.config.data.search_depth,
        "label_search_depth does not match",
    )?;
    ensure_merge(
        manifest.rollout_search_depth() == loaded.config.data.rollout_search_depth,
        "rollout_search_depth does not match",
    )?;
    if !ignore_identity_mismatch {
        ensure_merge(
            manifest.sampling_phase == loaded.config.data.sampling_policy.manifest_name(),
            "sampling_phase does not match",
        )?;
        ensure_merge(
            manifest.sample_after_opening
                == loaded.config.data.sampling_policy.samples_after_opening(),
            "sample_after_opening does not match",
        )?;
        ensure_merge(
            manifest.teacher_move_encoding == TEACHER_MOVE_ENCODING,
            "teacher_move_encoding does not match",
        )?;
    }
    if !ignore_identity_mismatch {
        ensure_merge(
            manifest.config_hash == loaded.hash_hex,
            "config_hash does not match. If you're sure to continue merging using mismatching identity, rerun with --ignore-identity-mismatch flag",
        )?;
    }
    validate_merge_teacher_identity(teacher_identity, manifest, ignore_identity_mismatch)?;
    ensure_merge(
        manifest.entry_bytes == ENTRY_BYTES,
        "entry_bytes does not match",
    )?;
    Ok(())
}

fn validate_merge_teacher_identity(
    expected: &mut Option<MergeTeacherIdentity>,
    manifest: &ShardManifest,
    ignore_identity_mismatch: bool,
) -> Result<()> {
    let current = MergeTeacherIdentity::from_manifest(manifest);
    if let Some(expected) = expected.as_ref() {
        ensure_merge(
            current.bootstrap_nnue_sha256 == expected.bootstrap_nnue_sha256,
            "bootstrap_nnue_sha256 does not match",
        )?;
        if !ignore_identity_mismatch {
            ensure_merge(
                current.engine_revision == expected.engine_revision,
                "engine_revision does not match. If you're sure to continue merging using mismatching identity, rerun with --ignore-identity-mismatch flag",
            )?;
        }
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
    fn per_game_sampling_phase_covers_both_parities_and_starts_after_opening() {
        let origins = (0..100)
            .map(|game_index| {
                sampling_origin(
                    game_seed(75, "train", game_index),
                    SamplingPolicy::PerGameRandomV1,
                    8,
                    16,
                    2,
                )
            })
            .collect::<Vec<_>>();
        assert!(origins.iter().all(|&ply| ply >= 16));
        assert!(origins.iter().any(|ply| ply % 2 == 0));
        assert!(origins.iter().any(|ply| ply % 2 == 1));

        let repeated = (0..100)
            .map(|game_index| {
                sampling_origin(
                    game_seed(75, "train", game_index),
                    SamplingPolicy::PerGameRandomV1,
                    8,
                    16,
                    2,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(origins, repeated);
    }

    #[test]
    fn legacy_sampling_requires_the_explicit_fixed_phase_policy() {
        assert_eq!(
            sampling_origin(123, SamplingPolicy::FixedPhaseLegacy, 8, 16, 2),
            8
        );
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
    fn trainer_packing_maps_pair_donor_slots_to_overlay_order() {
        let square = Square::E5;
        assert_eq!(
            packed_delta(square, square.try_offset(1, 0).unwrap()),
            (-1, 0)
        );
        assert_eq!(
            packed_delta(square, square.try_offset(-1, 0).unwrap()),
            (1, 0)
        );
    }

    #[test]
    fn trainer_packing_maps_knight8_runtime_offsets_to_overlay_order() {
        let square = Square::E5;
        let relative_offsets = [
            (1, 2),
            (-1, 2),
            (-2, 1),
            (-2, -1),
            (-1, -2),
            (1, -2),
            (2, -1),
            (2, 1),
        ];

        for color in [Color::Black, Color::White] {
            for (left, forward) in relative_offsets {
                let runtime_donor = runtime_relative_square(color, square, left, forward).unwrap();
                let expected_overlay_delta = overlay_relative_delta(color, left, forward);
                assert_eq!(
                    packed_delta(square, runtime_donor),
                    expected_overlay_delta,
                    "color={color:?}, left={left}, forward={forward}"
                );
            }
        }
    }

    #[test]
    fn graceful_stop_prevents_starting_new_shards() {
        let queue = Arc::new(Mutex::new(VecDeque::from([ShardPlan {
            shard_index: 0,
            game_start: 0,
            game_count: 1,
        }])));

        assert!(next_shard_plan_with(&queue, || true).is_none());
        assert_eq!(queue.lock().unwrap().len(), 1);
    }

    #[test]
    fn shard_lanes_are_contiguous_division_ranges() {
        let fourth_lane = shard_plan_indices(16, 2, Some(3), None, Some(4));
        let seventh_lane = shard_plan_indices(16, 2, Some(6), None, Some(8));
        let eighth_lane = shard_plan_indices(16, 2, Some(7), None, Some(8));

        assert_eq!(fourth_lane, [6, 7]);
        assert_eq!(seventh_lane, [6]);
        assert_eq!(eighth_lane, [7]);
    }

    #[test]
    fn shard_lanes_cover_uneven_division_without_overlap() {
        let mut all_indices = Vec::new();
        for index in 0..3 {
            all_indices.extend(shard_plan_indices(10, 1, Some(index), None, Some(3)));
        }

        assert_eq!(all_indices, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn shard_lane_ranges_cover_multiple_contiguous_lanes() {
        let combined = shard_plan_indices(16, 2, Some(2), Some(4), Some(8));
        let mut separate = shard_plan_indices(16, 2, Some(2), None, Some(8));
        separate.extend(shard_plan_indices(16, 2, Some(3), None, Some(8)));
        separate.extend(shard_plan_indices(16, 2, Some(4), None, Some(8)));

        assert_eq!(combined, separate);
        assert_eq!(combined, [2, 3, 4]);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
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
        } else if cfg!(feature = "tenkyo") {
            "tenkyo"
        } else if cfg!(feature = "tenjiku") {
            "tenjiku"
        } else if cfg!(feature = "anki") {
            "anki"
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
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
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
                shard_index_end: None,
                shard_count: None,
                ignore_identity_mismatch: false,
            },
        )
        .unwrap();
        generate_data_with_options(
            &two,
            GenerateOptions {
                jobs: Some(2),
                resume: Some(false),
                shard_index: None,
                shard_index_end: None,
                shard_count: None,
                ignore_identity_mismatch: false,
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
    #[cfg(feature = "anhoku")]
    fn suite_generation_records_deterministic_color_swapped_pair_metadata() {
        let temp = tempdir().unwrap();
        let suite = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("openings")
            .join("anhoku-v1.tsv");
        let config_path = temp.path().join("suite.toml");
        fs::write(
            &config_path,
            suite_test_config("out", &suite.display().to_string()),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        generate_data(&loaded).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(loaded.artifact_paths().train_manifest).unwrap())
                .unwrap();
        assert_eq!(manifest["opening_policy"], "suite");
        assert_eq!(manifest["opening_suite_id"], "anhoku-v1");
        assert_eq!(
            manifest["opening_transformation"],
            "anhoku-rotate180-color-swap-v1"
        );
        assert_eq!(manifest["games"].as_array().unwrap().len(), 4);
        for pair in manifest["games"].as_array().unwrap().chunks_exact(2) {
            assert_eq!(pair[0]["opening_id"], pair[1]["opening_id"]);
            assert_eq!(pair[0]["color"], "base");
            assert_eq!(pair[1]["color"], "swapped");
            assert_ne!(pair[0]["sfen"], pair[1]["sfen"]);
        }
    }

    #[test]
    #[cfg(feature = "anhoku")]
    fn suite_content_change_invalidates_resume_and_merge_identity() {
        let temp = tempdir().unwrap();
        let source_suite = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("openings")
            .join("anhoku-v1.tsv");
        let suite = temp.path().join("suite.tsv");
        fs::copy(source_suite, &suite).unwrap();
        let config_path = temp.path().join("suite.toml");
        fs::write(
            &config_path,
            suite_test_config("out", &suite.display().to_string()),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        generate_data(&loaded).unwrap();

        let mut changed = fs::read_to_string(&suite).unwrap();
        changed.push_str("# identity change\n");
        fs::write(&suite, changed).unwrap();
        let changed_loaded = LoadedConfig::from_path(&config_path).unwrap();
        let error = format!("{:#}", generate_data(&changed_loaded).unwrap_err());
        assert!(error.contains("--ignore-identity-mismatch"));

        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();
        let error = format!(
            "{:#}",
            merge_data(&changed_loaded, &[input], false).unwrap_err()
        );
        assert!(error.contains("opening_suite_sha256 does not match"));
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
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
    #[cfg(feature = "anhoku")]
    fn resume_rejects_sampling_and_teacher_move_contract_mismatches_without_override() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("resume-contract.toml");
        fs::write(&config_path, deterministic_test_config("anhoku", "out")).unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        generate_data(&loaded).unwrap();

        mutate_first_shard_manifest(&loaded, "train", |manifest| {
            manifest["sampling_phase"] =
                serde_json::Value::String("fixed-phase-legacy".to_string());
        });
        let error = format!("{:#}", generate_data(&loaded).unwrap_err());
        assert!(error.contains("--ignore-identity-mismatch"));

        mutate_first_shard_manifest(&loaded, "train", |manifest| {
            manifest["sampling_phase"] =
                serde_json::Value::String("per-game-random-v1".to_string());
            manifest["teacher_move_encoding"] =
                serde_json::Value::String("legacy-ambiguous-u16".to_string());
        });
        let error = format!("{:#}", generate_data(&loaded).unwrap_err());
        assert!(error.contains("--ignore-identity-mismatch"));
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn resume_regenerates_shards_when_rollout_depth_changes() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("resume-rollout-depth.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        mutate_first_shard_manifest(&loaded, "train", |manifest| {
            manifest["rollout_search_depth"] = serde_json::Value::from(2);
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
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn resume_reuses_legacy_shards_without_explicit_search_depths() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("resume-legacy-depths.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        mutate_first_shard_manifest(&loaded, "train", remove_explicit_search_depths);
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
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
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
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn resume_reuses_mismatched_shards_when_identity_ignored() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("resume-ignore-identity.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        mutate_first_shard_manifest(&loaded, "train", |manifest| {
            manifest["config_hash"] = serde_json::Value::String("stale-config-hash".to_string());
            manifest["engine_revision"] = serde_json::Value::String("other-revision".to_string());
        });
        generate_data_with_options(
            &loaded,
            GenerateOptions {
                jobs: Some(1),
                resume: Some(true),
                shard_index: None,
                shard_index_end: None,
                shard_count: None,
                ignore_identity_mismatch: true,
            },
        )
        .unwrap();

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
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn detect_identity_mismatch_counts_mismatched_games() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("detect-identity.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let artifacts = loaded.artifact_paths();
        let teacher = Teacher::from_config(&loaded).unwrap();
        let opening_sfen = loaded.opening_sfen().unwrap();
        let opening_source = OpeningSource::from_config(&loaded, &opening_sfen).unwrap();
        let engine_revision = detect_git_revision(&loaded).unwrap();
        let selector = ShardSelector::new(None, None, None).unwrap();

        let (before, total) = detect_identity_mismatch(
            &loaded,
            &artifacts,
            &teacher,
            &opening_sfen,
            &opening_source,
            &engine_revision,
            selector,
        )
        .unwrap();
        assert_eq!(before, 0);
        assert!(total > 0);

        mutate_first_shard_manifest(&loaded, "train", |manifest| {
            manifest["config_hash"] = serde_json::Value::String("stale-config-hash".to_string());
        });
        let (after, _) = detect_identity_mismatch(
            &loaded,
            &artifacts,
            &teacher,
            &opening_sfen,
            &opening_source,
            &engine_revision,
            selector,
        )
        .unwrap();
        assert!(after > 0);

        // A mismatched manifest whose .bin is missing/wrong-length is not reusable, so it
        // must not count toward a mismatch that would block a non-interactive resumed run.
        let bin_path = artifacts
            .datasets_dir
            .join("shards")
            .join("train")
            .join("shard-000000.bin");
        fs::write(&bin_path, b"truncated").unwrap();
        let (after_bad_bin, _) = detect_identity_mismatch(
            &loaded,
            &artifacts,
            &teacher,
            &opening_sfen,
            &opening_source,
            &engine_revision,
            selector,
        )
        .unwrap();
        assert_eq!(after_bad_bin, 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
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
                shard_index_end: None,
                shard_count: Some(2),
                ignore_identity_mismatch: false,
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
                shard_index_end: None,
                shard_count: Some(2),
                ignore_identity_mismatch: false,
            },
        )
        .unwrap();
        let second_input = temp.path().join("machine-b");
        fs::rename(loaded.artifact_paths().output_dir, &second_input).unwrap();

        let output = merge_data(&loaded, &[first_input, second_input], false).unwrap();
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
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
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
                shard_index_end: None,
                shard_count: Some(2),
                ignore_identity_mismatch: false,
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
                shard_index_end: None,
                shard_count: Some(2),
                ignore_identity_mismatch: false,
            },
        )
        .unwrap();
        let second_input = temp.path().join("machine-b");
        fs::rename(loaded.artifact_paths().output_dir, &second_input).unwrap();

        assert!(
            !first_input
                .join("datasets")
                .join("shards")
                .join("validation")
                .exists()
        );

        let output = merge_data(&loaded, &[first_input, second_input], false).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
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

        let err = format!("{:?}", merge_data(&loaded, &[input], false).unwrap_err());
        assert!(err.contains("engine_revision does not match"));
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn merge_rejects_shards_with_mismatched_rollout_depth() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-rollout-depth.toml");
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
            manifest["rollout_search_depth"] = serde_json::Value::from(2);
        });

        let err = format!("{:?}", merge_data(&loaded, &[input], false).unwrap_err());
        assert!(err.contains("rollout_search_depth does not match"));
    }

    #[test]
    #[cfg(feature = "anhoku")]
    fn merge_rejects_sampling_and_teacher_move_contract_mismatches() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-contract.toml");
        fs::write(&config_path, deterministic_test_config("anhoku", "out")).unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        generate_data(&loaded).unwrap();
        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();

        mutate_first_shard_manifest_in_dir(&input, "train", |manifest| {
            manifest["sampling_phase"] =
                serde_json::Value::String("fixed-phase-legacy".to_string());
        });
        let error = format!(
            "{:#}",
            merge_data(&loaded, &[input.clone()], false).unwrap_err()
        );
        assert!(error.contains("sampling_phase does not match"));

        mutate_first_shard_manifest_in_dir(&input, "train", |manifest| {
            manifest["sampling_phase"] =
                serde_json::Value::String("per-game-random-v1".to_string());
            manifest["teacher_move_encoding"] =
                serde_json::Value::String("legacy-ambiguous-u16".to_string());
        });
        let error = format!("{:#}", merge_data(&loaded, &[input], false).unwrap_err());
        assert!(error.contains("teacher_move_encoding does not match"));
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn merge_accepts_legacy_shards_without_explicit_search_depths() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-legacy-depths.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();
        mutate_first_shard_manifest_in_dir(&input, "train", remove_explicit_search_depths);
        mutate_first_shard_manifest_in_dir(&input, "validation", remove_explicit_search_depths);

        let output = merge_data(&loaded, &[input], false).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn merge_ignores_identity_mismatch_with_flag() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-ignore-identity.toml");
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
            manifest["config_hash"] = serde_json::Value::String("stale-config-hash".to_string());
        });

        let err = format!(
            "{:?}",
            merge_data(&loaded, &[input.clone()], false).unwrap_err()
        );
        assert!(err.contains("config_hash does not match"));
        assert!(err.contains("--ignore-identity-mismatch"));

        let output = merge_data(&loaded, &[input], true).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
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

        let output = merge_data(&loaded, &[input], false).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
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

        let output = merge_data(&loaded, &[input], false).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko",
        feature = "tenkyo",
        feature = "tenjiku",
        feature = "anki"
    )))]
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
    fn rollout_search_depth_defaults_to_one() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("rollout-default.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();

        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        assert_eq!(loaded.config.data.rollout_search_depth, 1);
    }

    #[test]
    fn rollout_search_depth_must_be_positive() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("rollout-invalid.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out").replace(
                "search_depth = 1",
                "search_depth = 1\nrollout_search_depth = 0",
            ),
        )
        .unwrap();

        let err = format!("{:?}", LoadedConfig::from_path(&config_path).unwrap_err());

        assert!(err.contains("data.rollout_search_depth must be at least 1"));
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn split_search_counters_track_label_and_rollout_searches() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("rollout-counters.toml");
        fs::write(
            &config_path,
            rollout_counter_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(loaded.artifact_paths().train_manifest).unwrap())
                .unwrap();
        assert_eq!(manifest["search_depth"].as_u64().unwrap(), 2);
        assert_eq!(manifest["label_search_depth"].as_u64().unwrap(), 2);
        assert_eq!(manifest["rollout_search_depth"].as_u64().unwrap(), 1);
        assert_eq!(manifest["label_searches"].as_u64().unwrap(), 3);
        assert_eq!(manifest["rollout_searches"].as_u64().unwrap(), 3);
        assert!(manifest["label_search_states"].as_u64().unwrap() > 0);
        assert!(manifest["rollout_search_states"].as_u64().unwrap() > 0);
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
        } else if cfg!(feature = "tenkyo") {
            "tenkyo"
        } else if cfg!(feature = "tenjiku") {
            "tenjiku"
        } else if cfg!(feature = "anki") {
            "anki"
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

    #[cfg(feature = "anhoku")]
    fn suite_test_config(output_dir: &str, suite: &str) -> String {
        format!(
            r#"
[rules]
ruleset = "anhoku"

[paths]
output_dir = "{output_dir}"

[data]
train_games = 4
validation_games = 2
max_plies = 4
search_depth = 1
opening_random_plies = 0
opening_policy = "suite"
opening_suite = "{suite}"
opening_suite_id = "anhoku-v1"
sample_start_ply = 0
sample_every_ply = 1
max_positions_per_game = 2
seed = 75
jobs = 1
shard_games = 2
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

    fn rollout_counter_test_config(ruleset: &str, output_dir: &str) -> String {
        format!(
            r#"
[rules]
ruleset = "{ruleset}"

[paths]
output_dir = "{output_dir}"

[data]
train_games = 1
validation_games = 1
max_plies = 6
search_depth = 2
rollout_search_depth = 1
opening_random_plies = 0
sample_start_ply = 0
sample_every_ply = 2
max_positions_per_game = 8
seed = 7
jobs = 1
shard_games = 1
progress_every_percent = 50
resume = false

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

    fn shard_plan_indices(
        game_count: u32,
        shard_games: u32,
        shard_index: Option<u32>,
        shard_index_end: Option<u32>,
        shard_count: Option<u32>,
    ) -> Vec<u32> {
        let selector = ShardSelector::new(shard_index, shard_index_end, shard_count).unwrap();
        shard_plans(game_count, shard_games, selector)
            .into_iter()
            .map(|plan| plan.shard_index)
            .collect()
    }

    fn remove_explicit_search_depths(manifest: &mut serde_json::Value) {
        let object = manifest.as_object_mut().unwrap();
        object.remove("label_search_depth");
        object.remove("rollout_search_depth");
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

    fn runtime_relative_square(
        color: Color,
        square: Square,
        left: i8,
        forward: i8,
    ) -> Option<Square> {
        let file_delta = match color {
            Color::Black => left,
            Color::White => -left,
        };
        let rank_delta = match color {
            Color::Black => -forward,
            Color::White => forward,
        };
        square.try_offset(file_delta, rank_delta)
    }

    fn overlay_relative_delta(color: Color, left: i8, forward: i8) -> (i8, i8) {
        match color {
            Color::Black => (-left, forward),
            Color::White => (left, -forward),
        }
    }

    fn packed_delta(origin: Square, target: Square) -> (i8, i8) {
        let origin = trainer_square_index(origin);
        let target = trainer_square_index(target);
        (
            (target % 9) as i8 - (origin % 9) as i8,
            (target / 9) as i8 - (origin / 9) as i8,
        )
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

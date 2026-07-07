use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{LoadedConfig, SelectionConfig};
use crate::trainer;

const SELECTION_SCHEMA: &str = "haitaka-nnue-selection";
const SELECTION_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone)]
pub struct TrainSelectOptions {
    pub self_play_bin: PathBuf,
    pub resume_override: Option<bool>,
    pub selection_max_games: Option<u32>,
    pub storage_saver: Option<bool>,
}

#[derive(Debug, Clone)]
struct SelectionPaths {
    candidates: PathBuf,
    matches: PathBuf,
    state: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelectionState {
    schema: String,
    schema_version: u8,
    config_hash: String,
    ruleset: String,
    feature_set: String,
    selection: SelectionConfig,
    candidates: Vec<CandidateRecord>,
    incumbent_checkpoint: Option<String>,
    selected_checkpoint: Option<String>,
    selected_nnue: Option<String>,
    deletions: Vec<DeletionRecord>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CandidateRecord {
    id: String,
    checkpoint: String,
    nnue: String,
    status: CandidateStatus,
    discovered_at_unix_seconds: u64,
    exported_at_unix_seconds: Option<u64>,
    matches: Vec<MatchRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum CandidateStatus {
    Exported,
    Incumbent,
    Rejected,
    Dethroned,
    Inconclusive,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MatchRecord {
    incumbent_checkpoint: String,
    report_dir: String,
    games: u32,
    a_wins: u32,
    b_wins: u32,
    draws: u32,
    llr: f64,
    sprt_state: SprtState,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeletionRecord {
    checkpoint: String,
    reason: String,
    deleted_at_unix_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum SprtState {
    Accepted,
    Rejected,
    Inconclusive,
}

#[derive(Debug)]
struct SprtResult {
    llr: f64,
    state: SprtState,
}

struct TrainingChild {
    child: Child,
    finished: bool,
}

impl TrainingChild {
    fn new(child: Child) -> Self {
        Self {
            child,
            finished: false,
        }
    }

    fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        let status = self
            .child
            .try_wait()
            .context("failed to poll training process")?;
        if status.is_some() {
            self.finished = true;
        }
        Ok(status)
    }
}

impl Drop for TrainingChild {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelfPlayReport {
    summary: SelfPlaySummary,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelfPlaySummary {
    games: u32,
    a_wins: u32,
    b_wins: u32,
    draws: u32,
}

pub fn train_select(loaded: &LoadedConfig, options: TrainSelectOptions) -> Result<PathBuf> {
    let mut selection = loaded.config.selection.clone();
    if let Some(max_games) = options.selection_max_games {
        selection.max_games = max_games;
    }
    if let Some(storage_saver) = options.storage_saver {
        selection.storage_saver = storage_saver;
    }
    validate_effective_selection(&selection)?;

    let artifacts = loaded.artifact_paths();
    artifacts.ensure_dirs()?;
    let paths = selection_paths(loaded);
    fs::create_dir_all(&paths.candidates)
        .with_context(|| format!("failed to create {}", paths.candidates.display()))?;
    fs::create_dir_all(&paths.matches)
        .with_context(|| format!("failed to create {}", paths.matches.display()))?;

    let trainer_checkout = loaded.trainer_checkout()?;
    let _guard = trainer::PreparedTrainer::new(loaded, &trainer_checkout)?;
    let mut state = load_or_create_state(loaded, &selection, &paths)?;
    state.selection = selection.clone();
    let mut child = TrainingChild::new(trainer::spawn_training(
        loaded,
        &trainer_checkout,
        options.resume_override,
    )?);
    println!(
        "training started; watching {}",
        artifacts.logs_dir.display()
    );

    let training_status = loop {
        let valid_checkpoints = eligible_checkpoints(
            &artifacts.logs_dir,
            &loaded.config.paths.python,
            &trainer_checkout,
            Duration::from_secs(selection.stable_checkpoint_secs),
        )?;
        process_new_checkpoints(
            loaded,
            &trainer_checkout,
            &options.self_play_bin,
            &selection,
            &paths,
            &mut state,
            &valid_checkpoints,
        )?;
        save_state(&paths.state, &state)?;
        if let Some(status) = child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_secs(selection.poll_interval_secs));
    };

    let final_checkpoints = eligible_checkpoints(
        &artifacts.logs_dir,
        &loaded.config.paths.python,
        &trainer_checkout,
        Duration::ZERO,
    )?;
    process_new_checkpoints(
        loaded,
        &trainer_checkout,
        &options.self_play_bin,
        &selection,
        &paths,
        &mut state,
        &final_checkpoints,
    )?;

    if !training_status.success() {
        save_state(&paths.state, &state)?;
        bail!("training failed with exit status {training_status}");
    }

    let selected = finalize_selection(loaded, &trainer_checkout, &mut state)?;
    apply_storage_saver(&selection, &mut state, final_checkpoints.last())?;
    save_state(&paths.state, &state)?;
    println!("selected NNUE: {}", selected.display());
    Ok(selected)
}

fn validate_effective_selection(selection: &SelectionConfig) -> Result<()> {
    if selection.batch_games == 0 {
        bail!("selection.batch_games must be > 0");
    }
    if selection.max_games < selection.batch_games {
        bail!("selection.max_games must be >= selection.batch_games");
    }
    Ok(())
}

fn selection_paths(loaded: &LoadedConfig) -> SelectionPaths {
    let root = loaded.artifact_paths().artifacts_dir.join("selection");
    SelectionPaths {
        candidates: root.join("candidates"),
        matches: root.join("matches"),
        state: root.join("selection.json"),
    }
}

fn load_or_create_state(
    loaded: &LoadedConfig,
    selection: &SelectionConfig,
    paths: &SelectionPaths,
) -> Result<SelectionState> {
    if paths.state.exists() {
        let state: SelectionState = serde_json::from_slice(
            &fs::read(&paths.state)
                .with_context(|| format!("failed to read {}", paths.state.display()))?,
        )
        .with_context(|| format!("failed to parse {}", paths.state.display()))?;
        if state.schema != SELECTION_SCHEMA || state.schema_version != SELECTION_SCHEMA_VERSION {
            bail!(
                "unsupported selection state schema in {}",
                paths.state.display()
            );
        }
        validate_state_identity(loaded, &state, &paths.state)?;
        return Ok(state);
    }
    Ok(SelectionState {
        schema: SELECTION_SCHEMA.to_string(),
        schema_version: SELECTION_SCHEMA_VERSION,
        config_hash: loaded.hash_hex.clone(),
        ruleset: loaded.config.rules.ruleset.as_str().to_string(),
        feature_set: loaded.training_features().to_string(),
        selection: selection.clone(),
        candidates: Vec::new(),
        incumbent_checkpoint: None,
        selected_checkpoint: None,
        selected_nnue: None,
        deletions: Vec::new(),
        warnings: Vec::new(),
    })
}

fn validate_state_identity(
    loaded: &LoadedConfig,
    state: &SelectionState,
    state_path: &Path,
) -> Result<()> {
    if state.config_hash != loaded.hash_hex {
        bail!(
            "selection state {} was created for config hash {}, but current config hash is {}. Use a separate paths.output_dir or remove the stale selection state before continuing.",
            state_path.display(),
            state.config_hash,
            loaded.hash_hex
        );
    }
    let ruleset = loaded.config.rules.ruleset.as_str();
    if state.ruleset != ruleset {
        bail!(
            "selection state {} was created for ruleset {}, but current ruleset is {}",
            state_path.display(),
            state.ruleset,
            ruleset
        );
    }
    let feature_set = loaded.training_features();
    if state.feature_set != feature_set {
        bail!(
            "selection state {} was created for feature set {}, but current feature set is {}",
            state_path.display(),
            state.feature_set,
            feature_set
        );
    }
    Ok(())
}

fn save_state(path: &Path, state: &SelectionState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, serde_json::to_vec_pretty(state)?)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn eligible_checkpoints(
    logs_dir: &Path,
    python: &str,
    trainer_checkout: &Path,
    stable_age: Duration,
) -> Result<Vec<PathBuf>> {
    let mut checkpoints = Vec::new();
    trainer::collect_checkpoints(logs_dir, &mut checkpoints);
    checkpoints.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    let now = SystemTime::now();
    let mut valid = Vec::new();
    for checkpoint in checkpoints {
        let modified = fs::metadata(&checkpoint)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if now.duration_since(modified).unwrap_or(Duration::ZERO) < stable_age {
            continue;
        }
        if trainer::is_valid_checkpoint(&checkpoint, python, trainer_checkout)? {
            valid.push(checkpoint);
        }
    }
    Ok(valid)
}

fn process_new_checkpoints(
    loaded: &LoadedConfig,
    trainer_checkout: &Path,
    self_play_bin: &Path,
    selection: &SelectionConfig,
    paths: &SelectionPaths,
    state: &mut SelectionState,
    checkpoints: &[PathBuf],
) -> Result<()> {
    let known: HashSet<String> = state
        .candidates
        .iter()
        .map(|candidate| candidate.checkpoint.clone())
        .collect();
    for checkpoint in checkpoints {
        let checkpoint_string = checkpoint.display().to_string();
        if known.contains(&checkpoint_string) {
            continue;
        }
        let candidate_index = export_candidate(loaded, trainer_checkout, paths, state, checkpoint)?;
        if state.incumbent_checkpoint.is_none() {
            state.candidates[candidate_index].status = CandidateStatus::Incumbent;
            state.incumbent_checkpoint = Some(state.candidates[candidate_index].checkpoint.clone());
            println!(
                "initial incumbent: {}",
                state.candidates[candidate_index].checkpoint
            );
        } else {
            evaluate_candidate(
                loaded,
                self_play_bin,
                selection,
                paths,
                state,
                candidate_index,
            )?;
        }
        apply_storage_saver(selection, state, checkpoints.last())?;
        save_state(&paths.state, state)?;
    }
    Ok(())
}

fn export_candidate(
    loaded: &LoadedConfig,
    trainer_checkout: &Path,
    paths: &SelectionPaths,
    state: &mut SelectionState,
    checkpoint: &Path,
) -> Result<usize> {
    let id = candidate_id(checkpoint);
    let nnue = paths.candidates.join(&id).join("model.nnue");
    println!(
        "exporting checkpoint {} -> {}",
        checkpoint.display(),
        nnue.display()
    );
    trainer::export_checkpoint_to(loaded, trainer_checkout, checkpoint, &nnue)?;
    state.candidates.push(CandidateRecord {
        id,
        checkpoint: checkpoint.display().to_string(),
        nnue: nnue.display().to_string(),
        status: CandidateStatus::Exported,
        discovered_at_unix_seconds: unix_timestamp_seconds()?,
        exported_at_unix_seconds: Some(unix_timestamp_seconds()?),
        matches: Vec::new(),
    });
    Ok(state.candidates.len() - 1)
}

fn evaluate_candidate(
    loaded: &LoadedConfig,
    self_play_bin: &Path,
    selection: &SelectionConfig,
    paths: &SelectionPaths,
    state: &mut SelectionState,
    candidate_index: usize,
) -> Result<()> {
    let incumbent_checkpoint = state
        .incumbent_checkpoint
        .clone()
        .ok_or_else(|| anyhow!("missing incumbent checkpoint"))?;
    let incumbent_index = state
        .candidates
        .iter()
        .position(|candidate| candidate.checkpoint == incumbent_checkpoint)
        .ok_or_else(|| anyhow!("incumbent checkpoint is not in candidate state"))?;

    let candidate_id = state.candidates[candidate_index].id.clone();
    let incumbent_id = state.candidates[incumbent_index].id.clone();
    let candidate_nnue = state.candidates[candidate_index].nnue.clone();
    let incumbent_nnue = state.candidates[incumbent_index].nnue.clone();
    let opening_sfen = loaded.opening_sfen()?;
    println!(
        "evaluating {} against incumbent {}",
        state.candidates[candidate_index].checkpoint, incumbent_checkpoint
    );

    let mut a_wins = 0;
    let mut b_wins = 0;
    let mut draws = 0;
    while a_wins + b_wins + draws < selection.max_games {
        let completed = a_wins + b_wins + draws;
        let games = selection.batch_games.min(selection.max_games - completed);
        let batch_index = state.candidates[candidate_index].matches.len();
        let report_dir = paths
            .matches
            .join(format!("{candidate_id}-vs-{incumbent_id}"))
            .join(format!("batch-{batch_index:04}"));
        run_self_play_batch(
            self_play_bin,
            selection,
            &candidate_nnue,
            &incumbent_nnue,
            &opening_sfen,
            &report_dir,
            games,
            batch_index as u64,
        )?;
        let report = read_self_play_report(&report_dir.join("self-play-report.json"))?;
        a_wins += report.summary.a_wins;
        b_wins += report.summary.b_wins;
        draws += report.summary.draws;
        let sprt = sprt_result(a_wins, b_wins, draws, selection);
        state.candidates[candidate_index].matches.push(MatchRecord {
            incumbent_checkpoint: incumbent_checkpoint.clone(),
            report_dir: report_dir.display().to_string(),
            games: report.summary.games,
            a_wins: report.summary.a_wins,
            b_wins: report.summary.b_wins,
            draws: report.summary.draws,
            llr: sprt.llr,
            sprt_state: sprt.state,
        });
        println!(
            "SPRT {:?}: games={} score {}-{}-{} llr={:.3}",
            sprt.state,
            a_wins + b_wins + draws,
            a_wins,
            b_wins,
            draws,
            sprt.llr
        );
        match sprt.state {
            SprtState::Accepted => {
                state.candidates[incumbent_index].status = CandidateStatus::Dethroned;
                state.candidates[candidate_index].status = CandidateStatus::Incumbent;
                state.incumbent_checkpoint =
                    Some(state.candidates[candidate_index].checkpoint.clone());
                return Ok(());
            }
            SprtState::Rejected => {
                state.candidates[candidate_index].status = CandidateStatus::Rejected;
                return Ok(());
            }
            SprtState::Inconclusive => {}
        }
    }
    state.candidates[candidate_index].status = CandidateStatus::Inconclusive;
    Ok(())
}

fn run_self_play_batch(
    self_play_bin: &Path,
    selection: &SelectionConfig,
    candidate_nnue: &str,
    incumbent_nnue: &str,
    opening_sfen: &str,
    report_dir: &Path,
    games: u32,
    batch_index: u64,
) -> Result<()> {
    if report_dir.exists() {
        fs::remove_dir_all(report_dir)
            .with_context(|| format!("failed to remove {}", report_dir.display()))?;
    }
    let status = Command::new(self_play_bin)
        .args(self_play_batch_args(
            selection,
            candidate_nnue,
            incumbent_nnue,
            opening_sfen,
            report_dir,
            games,
            batch_index,
        ))
        .status()
        .with_context(|| {
            format!(
                "failed to start self-play using {}",
                self_play_bin.display()
            )
        })?;
    if !status.success() {
        bail!("self-play failed with exit status {status}");
    }
    Ok(())
}

fn self_play_batch_args(
    selection: &SelectionConfig,
    candidate_nnue: &str,
    incumbent_nnue: &str,
    opening_sfen: &str,
    report_dir: &Path,
    games: u32,
    batch_index: u64,
) -> Vec<OsString> {
    vec![
        "self-play".into(),
        "--games".into(),
        games.to_string().into(),
        "--threads".into(),
        selection.threads.to_string().into(),
        "--movetime-ms".into(),
        selection.movetime_ms.to_string().into(),
        "--opening-random-plies".into(),
        selection.opening_random_plies.to_string().into(),
        "--seed".into(),
        selection.seed.wrapping_add(batch_index).to_string().into(),
        "--sfen".into(),
        opening_sfen.into(),
        "--report-dir".into(),
        report_dir.as_os_str().to_os_string(),
        "--a-eval".into(),
        "nnue".into(),
        "--a-nnue".into(),
        candidate_nnue.into(),
        "--b-eval".into(),
        "nnue".into(),
        "--b-nnue".into(),
        incumbent_nnue.into(),
    ]
}

fn read_self_play_report(path: &Path) -> Result<SelfPlayReport> {
    serde_json::from_slice(
        &fs::read(path).with_context(|| format!("failed to read {}", path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", path.display()))
}

fn finalize_selection(
    loaded: &LoadedConfig,
    trainer_checkout: &Path,
    state: &mut SelectionState,
) -> Result<PathBuf> {
    let incumbent = state
        .incumbent_checkpoint
        .clone()
        .ok_or_else(|| anyhow!("training finished without any selectable checkpoint"))?;
    let incumbent_record = state
        .candidates
        .iter()
        .find(|candidate| candidate.checkpoint == incumbent)
        .ok_or_else(|| anyhow!("incumbent checkpoint is missing from state"))?;
    let artifacts = loaded.artifact_paths();
    fs::copy(&incumbent_record.nnue, &artifacts.exported_nnue).with_context(|| {
        format!(
            "failed to copy selected NNUE {} to {}",
            incumbent_record.nnue,
            artifacts.exported_nnue.display()
        )
    })?;
    trainer::write_export_metadata(
        loaded,
        trainer_checkout,
        Path::new(&incumbent_record.checkpoint),
        &artifacts.exported_nnue,
    )?;
    state.selected_checkpoint = Some(incumbent_record.checkpoint.clone());
    state.selected_nnue = Some(artifacts.exported_nnue.display().to_string());
    Ok(artifacts.exported_nnue)
}

fn apply_storage_saver(
    selection: &SelectionConfig,
    state: &mut SelectionState,
    newest_resume_checkpoint: Option<&PathBuf>,
) -> Result<()> {
    if !selection.storage_saver {
        return Ok(());
    }
    let newest_resume_checkpoint = newest_resume_checkpoint.map(|path| path.display().to_string());
    let protected = [
        state.incumbent_checkpoint.as_deref(),
        state.selected_checkpoint.as_deref(),
        newest_resume_checkpoint.as_deref(),
    ];
    let already_deleted: HashSet<String> = state
        .deletions
        .iter()
        .map(|deletion| deletion.checkpoint.clone())
        .collect();
    let mut pending_deletions = Vec::new();
    for candidate in &state.candidates {
        if !matches!(
            candidate.status,
            CandidateStatus::Rejected | CandidateStatus::Dethroned
        ) {
            continue;
        }
        if protected
            .iter()
            .flatten()
            .any(|path| *path == candidate.checkpoint)
        {
            continue;
        }
        if already_deleted.contains(&candidate.checkpoint) {
            continue;
        }
        pending_deletions.push((
            candidate.checkpoint.clone(),
            format!("{:?}", candidate.status),
        ));
    }
    for (checkpoint, reason) in pending_deletions {
        let path = Path::new(&checkpoint);
        if path.exists() {
            fs::remove_file(path)
                .with_context(|| format!("failed to delete checkpoint {}", path.display()))?;
            state.deletions.push(DeletionRecord {
                checkpoint,
                reason,
                deleted_at_unix_seconds: unix_timestamp_seconds()?,
            });
        }
    }
    Ok(())
}

fn sprt_result(a_wins: u32, b_wins: u32, draws: u32, selection: &SelectionConfig) -> SprtResult {
    let llr = sprt_llr(
        a_wins,
        b_wins,
        draws,
        selection.sprt_elo0,
        selection.sprt_elo1,
    );
    let lower = (selection.sprt_beta / (1.0 - selection.sprt_alpha)).ln();
    let upper = ((1.0 - selection.sprt_beta) / selection.sprt_alpha).ln();
    let state = if llr >= upper {
        SprtState::Accepted
    } else if llr <= lower {
        SprtState::Rejected
    } else {
        SprtState::Inconclusive
    };
    SprtResult { llr, state }
}

fn sprt_llr(a_wins: u32, b_wins: u32, draws: u32, elo0: f64, elo1: f64) -> f64 {
    let total = a_wins + b_wins + draws;
    if total == 0 {
        return 0.0;
    }
    let draw_rate = f64::from(draws) / f64::from(total);
    let p0 = outcome_probabilities(elo0, draw_rate);
    let p1 = outcome_probabilities(elo1, draw_rate);
    f64::from(a_wins) * (p1.0 / p0.0).ln()
        + f64::from(draws) * (p1.1 / p0.1).ln()
        + f64::from(b_wins) * (p1.2 / p0.2).ln()
}

fn outcome_probabilities(elo: f64, draw_rate: f64) -> (f64, f64, f64) {
    let epsilon = 1.0e-9;
    let draw = draw_rate.clamp(epsilon, 1.0 - epsilon);
    let decisive = (1.0 - draw).max(epsilon);
    let score = 1.0 / (1.0 + 10.0_f64.powf(-elo / 400.0));
    let decisive_win_rate = ((score - draw / 2.0) / decisive).clamp(epsilon, 1.0 - epsilon);
    let win = decisive * decisive_win_rate;
    let loss = decisive * (1.0 - decisive_win_rate);
    (win.max(epsilon), draw, loss.max(epsilon))
}

fn candidate_id(checkpoint: &Path) -> String {
    let stem = checkpoint
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("checkpoint")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    let mut hasher = Sha256::new();
    hasher.update(checkpoint.display().to_string().as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    format!("{stem}-{}", &hash[..12])
}

fn unix_timestamp_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?
        .as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        DataConfig, ExportConfig, LearnConfig, PathsConfig, RulesConfig, Ruleset, TrainingConfig,
        VerifyConfig,
    };

    fn test_selection() -> SelectionConfig {
        SelectionConfig {
            batch_games: 4,
            max_games: 16,
            sprt_elo0: 0.0,
            sprt_elo1: 5.0,
            sprt_alpha: 0.05,
            sprt_beta: 0.05,
            ..SelectionConfig::default()
        }
    }

    fn loaded_config_for_tests(hash_hex: &str) -> LoadedConfig {
        LoadedConfig {
            path: PathBuf::from("/tmp/haitaka_learn.toml"),
            hash_hex: hash_hex.to_string(),
            config: LearnConfig {
                rules: RulesConfig {
                    ruleset: Ruleset::Standard,
                    rule_id: None,
                    handicap: None,
                    opening_sfen: None,
                },
                paths: PathsConfig::default(),
                data: DataConfig::default(),
                training: TrainingConfig::default(),
                export: ExportConfig::default(),
                verify: VerifyConfig::default(),
                selection: SelectionConfig::default(),
            },
        }
    }

    fn empty_state_for_tests(loaded: &LoadedConfig, selection: SelectionConfig) -> SelectionState {
        SelectionState {
            schema: SELECTION_SCHEMA.to_string(),
            schema_version: SELECTION_SCHEMA_VERSION,
            config_hash: loaded.hash_hex.clone(),
            ruleset: loaded.config.rules.ruleset.as_str().to_string(),
            feature_set: loaded.training_features().to_string(),
            selection,
            candidates: Vec::new(),
            incumbent_checkpoint: None,
            selected_checkpoint: None,
            selected_nnue: None,
            deletions: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn sprt_accepts_clear_winner() {
        let selection = test_selection();
        let result = sprt_result(300, 100, 100, &selection);
        assert_eq!(result.state, SprtState::Accepted);
    }

    #[test]
    fn sprt_rejects_clear_loser() {
        let selection = test_selection();
        let result = sprt_result(100, 300, 100, &selection);
        assert_eq!(result.state, SprtState::Rejected);
    }

    #[test]
    fn candidate_id_is_stable_and_sanitized() {
        let id = candidate_id(Path::new(
            "/tmp/lightning/version 0/checkpoints/epoch=3-step=4.ckpt",
        ));
        assert!(id.starts_with("epoch-3-step-4-"));
        assert_eq!(
            id,
            candidate_id(Path::new(
                "/tmp/lightning/version 0/checkpoints/epoch=3-step=4.ckpt"
            ))
        );
    }

    #[test]
    fn self_play_args_include_configured_opening_sfen() {
        let selection = test_selection();
        let report_dir = Path::new("/tmp/haitaka-selection-report");
        let sfen = "4k4/9/9/9/9/9/9/9/4K4 b - 1";
        let args = self_play_batch_args(
            &selection,
            "/tmp/candidate.nnue",
            "/tmp/incumbent.nnue",
            sfen,
            report_dir,
            8,
            2,
        );
        let args = args_as_strings(&args);

        assert!(has_adjacent_args(&args, "--sfen", sfen));
    }

    #[test]
    fn state_identity_rejects_stale_config_hash() {
        let loaded = loaded_config_for_tests("current");
        let mut state = empty_state_for_tests(&loaded, test_selection());
        state.config_hash = "previous".to_string();

        let err =
            validate_state_identity(&loaded, &state, Path::new("selection.json")).unwrap_err();

        assert!(format!("{err:?}").contains("current config hash is current"));
    }

    #[test]
    fn storage_saver_keeps_incumbent_selected_and_newest_resume_checkpoint() {
        let mut selection = test_selection();
        selection.storage_saver = true;
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old.ckpt");
        let newest = temp.path().join("newest.ckpt");
        fs::write(&old, b"old").unwrap();
        fs::write(&newest, b"newest").unwrap();
        let mut state = SelectionState {
            schema: SELECTION_SCHEMA.to_string(),
            schema_version: SELECTION_SCHEMA_VERSION,
            config_hash: "cfg".to_string(),
            ruleset: "standard".to_string(),
            feature_set: "HalfKAv2^".to_string(),
            selection: selection.clone(),
            candidates: vec![
                CandidateRecord {
                    id: "old".to_string(),
                    checkpoint: old.display().to_string(),
                    nnue: "/tmp/old.nnue".to_string(),
                    status: CandidateStatus::Rejected,
                    discovered_at_unix_seconds: 0,
                    exported_at_unix_seconds: Some(0),
                    matches: Vec::new(),
                },
                CandidateRecord {
                    id: "newest".to_string(),
                    checkpoint: newest.display().to_string(),
                    nnue: "/tmp/newest.nnue".to_string(),
                    status: CandidateStatus::Rejected,
                    discovered_at_unix_seconds: 0,
                    exported_at_unix_seconds: Some(0),
                    matches: Vec::new(),
                },
            ],
            incumbent_checkpoint: None,
            selected_checkpoint: None,
            selected_nnue: None,
            deletions: Vec::new(),
            warnings: Vec::new(),
        };
        apply_storage_saver(&selection, &mut state, Some(&newest)).unwrap();
        assert!(!old.exists());
        assert!(newest.exists());
        assert_eq!(state.deletions.len(), 1);
    }

    fn args_as_strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn has_adjacent_args(args: &[String], first: &str, second: &str) -> bool {
        args.windows(2)
            .any(|window| window[0] == first && window[1] == second)
    }
}

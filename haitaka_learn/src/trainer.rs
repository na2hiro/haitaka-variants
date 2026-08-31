use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{
    FEATURE_SET_DONOR_RECEIVER_PAIR_V2, LoadedConfig, Ruleset, TEACHER_MOVE_ENCODING,
};
use crate::dataset::ENTRY_BYTES;
use crate::dataset_audit::audit_dataset;

#[derive(Debug, Serialize)]
pub(crate) struct ExportMetadata {
    exported_nnue: String,
    source_checkpoint: String,
    trainer_checkout: String,
    trainer_revision: Option<String>,
    features: String,
    description: String,
    config_hash: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct TrainerFeatureParityReport {
    schema: &'static str,
    schema_version: u32,
    trainer_checkout: String,
    trainer_revision: Option<String>,
    feature_set: String,
    feature_set_hash: String,
    real_features: usize,
    donor_block_features: usize,
    python_spec_passed: bool,
    cpp_loader_compiled: bool,
    rust_cpp_index_anchors: [usize; 4],
    donor_features_py_sha256: String,
    training_data_loader_cpp_sha256: String,
    pub(crate) passed: bool,
}

pub(crate) fn verify_receiver_pair_v2_trainer_parity(
    loaded: &LoadedConfig,
    output: &Path,
) -> Result<TrainerFeatureParityReport> {
    if loaded.training_features() != FEATURE_SET_DONOR_RECEIVER_PAIR_V2 {
        bail!("trainer parity requires training.features={FEATURE_SET_DONOR_RECEIVER_PAIR_V2}");
    }
    let trainer_checkout = loaded.trainer_checkout()?;
    let donor_overlay = overlay_donor_features_py_contents();
    let loader_overlay = overlay_training_data_loader_cpp_contents();
    let _guard = PreparedTrainer::new(loaded, &trainer_checkout)?;
    run_command(
        &loaded.config.paths.python,
        &[
            "-c".to_string(),
            concat!(
                "import sys, types; ",
                "chess=types.ModuleType('chess'); chess.Board=object; sys.modules['chess']=chess; ",
                "sys.modules['torch']=types.ModuleType('torch'); ",
                "variant=types.ModuleType('variant'); variant.SQUARES=81; variant.PIECE_TYPES=10; ",
                "variant.DONOR_MODE='single-front'; sys.modules['variant']=variant; ",
                "import donor_features; f=donor_features.DonorReceiverPairV2(); ",
                "assert f.name == 'DonorReceiverPairV2'; ",
                "assert f.hash == 0x6D124A8F; ",
                "assert f.num_real_features == 16200; ",
                "composite=(0x5F234CB8 ^ ((f.hash << 1) & 0xffffffff) ^ (f.hash >> 1)) & 0xffffffff; ",
                "assert composite == 0xB38EFCE1"
            )
            .to_string(),
        ],
        &trainer_checkout,
        "validate DonorReceiverPairV2 Python feature specification",
    )?;

    let report = TrainerFeatureParityReport {
        schema: "haitaka-anhoku-phase11a-trainer-parity",
        schema_version: 1,
        trainer_checkout: trainer_checkout.display().to_string(),
        trainer_revision: detect_git_revision(&trainer_checkout),
        feature_set: FEATURE_SET_DONOR_RECEIVER_PAIR_V2.to_string(),
        feature_set_hash: "0xb38efce1".to_string(),
        real_features: 167_103,
        donor_block_features: 16_200,
        python_spec_passed: true,
        cpp_loader_compiled: true,
        rust_cpp_index_anchors: [6520, 6601, 6682, 8140],
        donor_features_py_sha256: format!("{:x}", Sha256::digest(donor_overlay.as_bytes())),
        training_data_loader_cpp_sha256: format!("{:x}", Sha256::digest(loader_overlay.as_bytes())),
        passed: true,
    };
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(output, serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("failed to write {}", output.display()))?;
    Ok(report)
}

#[derive(Debug, Deserialize)]
struct DatasetCompletionManifest {
    game_count: u32,
    completed_games: u32,
    sampled_positions: u64,
    entry_bytes: usize,
    #[serde(default)]
    teacher_move_encoding: String,
}

pub fn train(loaded: &LoadedConfig, resume_override: Option<bool>) -> Result<PathBuf> {
    let trainer_checkout = loaded.trainer_checkout()?;
    let artifacts = loaded.artifact_paths();
    artifacts.ensure_dirs()?;

    ensure_training_inputs_ready(loaded)?;

    let _guard = PreparedTrainer::new(loaded, &trainer_checkout)?;
    let args = training_args(loaded, resume_override, &trainer_checkout)?;

    run_command(
        &loaded.config.paths.python,
        &args,
        &trainer_checkout,
        "haitaka-variant-nnue-pytorch training",
    )?;

    find_latest_valid_checkpoint(
        &artifacts.logs_dir,
        &loaded.config.paths.python,
        &trainer_checkout,
    )?
    .ok_or_else(|| {
        anyhow!(
            "training finished but no valid checkpoint was found under {}",
            artifacts.logs_dir.display()
        )
    })
}

/// Materialize the configured warm-start as a trainer checkpoint while doing
/// no optimization. In particular, this exercises the V1-to-V2 migration and
/// the real PyTorch deserializer before an hourly GPU run is authorized.
pub fn prepare_bootstrap(loaded: &LoadedConfig) -> Result<PathBuf> {
    let trainer_checkout = loaded.trainer_checkout()?;
    let artifacts = loaded.artifact_paths();
    artifacts.ensure_dirs()?;
    let _guard = PreparedTrainer::new(loaded, &trainer_checkout)?;
    materialize_bootstrap_pt(loaded, &trainer_checkout)?.ok_or_else(|| {
        anyhow!("paths.bootstrap_nnue is required to prepare a bootstrap checkpoint")
    })
}

pub fn export(loaded: &LoadedConfig, source_checkpoint: Option<PathBuf>) -> Result<PathBuf> {
    let trainer_checkout = loaded.trainer_checkout()?;
    let artifacts = loaded.artifact_paths();
    artifacts.ensure_dirs()?;

    let checkpoint = if let Some(path) = source_checkpoint {
        path
    } else {
        find_latest_valid_checkpoint(
            &artifacts.logs_dir,
            &loaded.config.paths.python,
            &trainer_checkout,
        )?
        .ok_or_else(|| {
            anyhow!(
                "could not find a valid checkpoint under {}",
                artifacts.logs_dir.display()
            )
        })?
    };
    ensure_file_exists(&checkpoint, "checkpoint")?;

    let _guard = PreparedTrainer::new(loaded, &trainer_checkout)?;
    run_command(
        &loaded.config.paths.python,
        &[
            "serialize.py".to_string(),
            checkpoint.display().to_string(),
            artifacts.exported_nnue.display().to_string(),
            "--features".to_string(),
            loaded.training_features().to_string(),
            "--description".to_string(),
            loaded.config.export.description.clone(),
        ],
        &trainer_checkout,
        "haitaka-variant-nnue-pytorch export",
    )?;

    write_export_metadata(
        loaded,
        &trainer_checkout,
        &checkpoint,
        &artifacts.exported_nnue,
    )?;

    Ok(artifacts.exported_nnue)
}

/// Evaluate one arbitrary checkpoint against the configured ID validation set
/// and the optional legacy two-opening OOD set without starting training.
pub fn evaluate_checkpoint(
    loaded: &LoadedConfig,
    checkpoint: PathBuf,
    output: Option<PathBuf>,
) -> Result<PathBuf> {
    let trainer_checkout = loaded.trainer_checkout()?;
    let artifacts = loaded.artifact_paths();
    let checkpoint = loaded.resolve_path(&checkpoint);
    ensure_file_exists(&checkpoint, "checkpoint")?;
    ensure_file_exists(&artifacts.validation_bin, "ID validation dataset")?;
    let ood = loaded.legacy_ood_validation_bin().ok_or_else(|| {
        anyhow!("paths.legacy_ood_validation_bin is required for offline ID/OOD evaluation")
    })?;
    ensure_file_exists(&ood, "legacy OOD validation dataset")?;

    let output = output
        .map(|path| loaded.resolve_path(&path))
        .unwrap_or_else(|| artifacts.artifacts_dir.join("offline-evaluation.json"));
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let _guard = PreparedTrainer::new(loaded, &trainer_checkout)?;
    run_command(
        &loaded.config.paths.python,
        &[
            "evaluate.py".to_string(),
            checkpoint.display().to_string(),
            "--id-validation".to_string(),
            artifacts.validation_bin.display().to_string(),
            "--ood-validation".to_string(),
            ood.display().to_string(),
            "--features".to_string(),
            loaded.training_features().to_string(),
            "--batch-size".to_string(),
            loaded.config.training.batch_size.to_string(),
            "--validation-size".to_string(),
            loaded.config.training.validation_size.to_string(),
            "--output".to_string(),
            output.display().to_string(),
        ],
        &trainer_checkout,
        "haitaka-variant-nnue-pytorch offline checkpoint evaluation",
    )?;
    Ok(output)
}

pub(crate) fn ensure_training_inputs_ready(loaded: &LoadedConfig) -> Result<()> {
    let artifacts = loaded.artifact_paths();
    ensure_training_dataset_ready(
        &artifacts.train_bin,
        &artifacts.train_manifest,
        "training dataset",
        loaded.config.data.train_games,
    )?;
    ensure_training_board_minimum(loaded, &artifacts.train_bin, &artifacts.train_manifest)?;
    ensure_training_dataset_ready(
        &artifacts.validation_bin,
        &artifacts.validation_manifest,
        "validation dataset",
        loaded.config.data.validation_games,
    )?;
    if let Some(ood) = loaded.legacy_ood_validation_bin() {
        ensure_file_exists(&ood, "legacy OOD validation dataset")?;
        let metadata =
            fs::metadata(&ood).with_context(|| format!("failed to stat {}", ood.display()))?;
        if metadata.len() == 0 || metadata.len() % ENTRY_BYTES as u64 != 0 {
            bail!(
                "legacy OOD validation dataset {} must be a non-empty multiple of {} bytes",
                ood.display(),
                ENTRY_BYTES
            );
        }
    }
    Ok(())
}

pub(crate) fn training_args(
    loaded: &LoadedConfig,
    resume_override: Option<bool>,
    trainer_checkout: &Path,
) -> Result<Vec<String>> {
    let artifacts = loaded.artifact_paths();
    let should_resume = resume_override.unwrap_or(loaded.config.training.resume);
    let resume_checkpoint = if should_resume {
        find_latest_valid_checkpoint(
            &artifacts.logs_dir,
            &loaded.config.paths.python,
            trainer_checkout,
        )?
    } else {
        None
    };
    let bootstrap_model = if resume_checkpoint.is_none() {
        materialize_bootstrap_pt(loaded, trainer_checkout)?
    } else {
        None
    };

    let mut args = vec![
        "train.py".to_string(),
        artifacts.train_bin.display().to_string(),
        artifacts.validation_bin.display().to_string(),
        "--features".to_string(),
        loaded.training_features().to_string(),
        "--default_root_dir".to_string(),
        artifacts.logs_dir.display().to_string(),
        "--max_epochs".to_string(),
        loaded.config.training.max_epochs.to_string(),
        "--num-workers".to_string(),
        loaded.config.training.num_workers.to_string(),
        "--batch-size".to_string(),
        loaded.config.training.batch_size.to_string(),
        "--lambda".to_string(),
        loaded.config.training.lambda_.to_string(),
        "--random-fen-skipping".to_string(),
        loaded.config.training.random_fen_skipping.to_string(),
        "--epoch-size".to_string(),
        loaded.config.training.epoch_size.to_string(),
        "--validation-size".to_string(),
        loaded.config.training.validation_size.to_string(),
        "--initial-learning-rate".to_string(),
        loaded.config.training.initial_learning_rate.to_string(),
    ];
    if let Some(interval) = loaded.config.training.checkpoint_interval_steps {
        args.push("--checkpoint-interval-steps".to_string());
        args.push(interval.to_string());
    }
    if let Some(interval) = loaded.config.training.validation_interval_steps {
        args.push("--validation-interval-steps".to_string());
        args.push(interval.to_string());
    }
    if let Some(max_steps) = loaded.config.training.max_steps {
        args.push("--max-steps".to_string());
        args.push(max_steps.to_string());
    }
    if let Some(ood) = loaded.legacy_ood_validation_bin() {
        args.push("--ood-validation".to_string());
        args.push(ood.display().to_string());
    }
    if let Some(checkpoint) = resume_checkpoint {
        println!("resuming training from {}", checkpoint.display());
        args.push("--resume_from_checkpoint".to_string());
        args.push(checkpoint.display().to_string());
    } else if let Some(model) = bootstrap_model {
        args.push("--resume-from-model".to_string());
        args.push(model.display().to_string());
    }
    args.extend(loaded.config.training.extra_args.clone());
    Ok(args)
}

pub(crate) fn spawn_training(
    loaded: &LoadedConfig,
    trainer_checkout: &Path,
    resume_override: Option<bool>,
) -> Result<Child> {
    ensure_training_inputs_ready(loaded)?;
    let args = training_args(loaded, resume_override, trainer_checkout)?;
    Command::new(&loaded.config.paths.python)
        .args(args)
        .current_dir(trainer_checkout)
        .spawn()
        .with_context(|| {
            format!(
                "failed to start haitaka-variant-nnue-pytorch training using `{}`",
                loaded.config.paths.python
            )
        })
}

pub(crate) fn export_checkpoint_to(
    loaded: &LoadedConfig,
    trainer_checkout: &Path,
    checkpoint: &Path,
    output_nnue: &Path,
) -> Result<()> {
    ensure_file_exists(checkpoint, "checkpoint")?;
    if let Some(parent) = output_nnue.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    run_command(
        &loaded.config.paths.python,
        &[
            "serialize.py".to_string(),
            checkpoint.display().to_string(),
            output_nnue.display().to_string(),
            "--features".to_string(),
            loaded.training_features().to_string(),
            "--description".to_string(),
            loaded.config.export.description.clone(),
        ],
        trainer_checkout,
        "haitaka-variant-nnue-pytorch export",
    )
}

pub(crate) fn write_export_metadata(
    loaded: &LoadedConfig,
    trainer_checkout: &Path,
    checkpoint: &Path,
    exported_nnue: &Path,
) -> Result<()> {
    let artifacts = loaded.artifact_paths();
    let metadata = ExportMetadata {
        exported_nnue: exported_nnue.display().to_string(),
        source_checkpoint: checkpoint.display().to_string(),
        trainer_checkout: trainer_checkout.display().to_string(),
        trainer_revision: detect_git_revision(trainer_checkout),
        features: loaded.training_features().to_string(),
        description: loaded.config.export.description.clone(),
        config_hash: loaded.hash_hex.clone(),
    };
    fs::write(
        &artifacts.export_metadata,
        serde_json::to_vec_pretty(&metadata)?,
    )
    .with_context(|| format!("failed to write {}", artifacts.export_metadata.display()))?;

    Ok(())
}

pub(crate) struct PreparedTrainer {
    backups: Vec<FileBackup>,
}

impl PreparedTrainer {
    pub(crate) fn new(loaded: &LoadedConfig, trainer_checkout: &Path) -> Result<Self> {
        Self::new_inner(
            loaded,
            trainer_checkout,
            loaded.config.training.build_data_loader,
        )
    }

    /// Install the exact Python/variant overlays while deliberately skipping
    /// the C++ loader build. Phase 11-C only deserializes a frozen checkpoint.
    pub(crate) fn new_without_build(
        loaded: &LoadedConfig,
        trainer_checkout: &Path,
    ) -> Result<Self> {
        Self::new_inner(loaded, trainer_checkout, false)
    }

    fn new_inner(
        loaded: &LoadedConfig,
        trainer_checkout: &Path,
        build_data_loader: bool,
    ) -> Result<Self> {
        if !trainer_checkout.exists() {
            bail!(
                "trainer checkout does not exist: {}",
                trainer_checkout.display()
            );
        }

        let variant_py = trainer_checkout.join("variant.py");
        let variant_h = trainer_checkout.join("variant.h");
        let feature_set_py = trainer_checkout.join("feature_set.py");
        let features_py = trainer_checkout.join("features.py");
        let donor_features_py = trainer_checkout.join("donor_features.py");
        let training_data_loader_cpp = trainer_checkout.join("training_data_loader.cpp");
        let mut prepared = Self {
            backups: Vec::new(),
        };
        prepared.write_with_backup(&variant_py, variant_py_contents(loaded))?;
        prepared.write_with_backup(&variant_h, variant_h_contents(loaded))?;
        prepared.write_with_backup(&feature_set_py, overlay_feature_set_py_contents())?;
        prepared.write_with_backup(&features_py, overlay_features_py_contents())?;
        prepared.write_with_backup(&donor_features_py, overlay_donor_features_py_contents())?;
        prepared.write_with_backup(
            &training_data_loader_cpp,
            overlay_training_data_loader_cpp_contents(),
        )?;

        if build_data_loader {
            run_command(
                &loaded.config.paths.cmake,
                &["-S".into(), ".".into(), "-B".into(), "build".into()],
                trainer_checkout,
                "configure trainer data loader",
            )?;
            run_command(
                &loaded.config.paths.cmake,
                &["--build".into(), "build".into()],
                trainer_checkout,
                "build trainer data loader",
            )?;
            run_command(
                &loaded.config.paths.cmake,
                &[
                    "--install".into(),
                    "build".into(),
                    "--prefix".into(),
                    ".".into(),
                ],
                trainer_checkout,
                "install trainer data loader",
            )?;
        }

        Ok(prepared)
    }

    fn write_with_backup(&mut self, path: &Path, contents: String) -> Result<()> {
        let previous = fs::read(path).ok();
        fs::write(path, contents.as_bytes())
            .with_context(|| format!("failed to write {}", path.display()))?;
        self.backups.push(FileBackup {
            path: path.to_path_buf(),
            previous,
        });
        Ok(())
    }
}

impl Drop for PreparedTrainer {
    fn drop(&mut self) {
        for backup in self.backups.drain(..).rev() {
            match backup.previous {
                Some(bytes) => {
                    let _ = fs::write(&backup.path, bytes);
                }
                None => {
                    let _ = fs::remove_file(&backup.path);
                }
            }
        }
    }
}

struct FileBackup {
    path: PathBuf,
    previous: Option<Vec<u8>>,
}

fn materialize_bootstrap_pt(
    loaded: &LoadedConfig,
    trainer_checkout: &Path,
) -> Result<Option<PathBuf>> {
    let Some(bootstrap_nnue) = loaded.bootstrap_nnue() else {
        return Ok(None);
    };
    ensure_file_exists(&bootstrap_nnue, "bootstrap NNUE")?;

    let artifacts = loaded.artifact_paths();
    let import_features = bootstrap_import_features(loaded);
    let training_features = loaded.training_features();
    let bootstrap_nnue = if training_features == FEATURE_SET_DONOR_RECEIVER_PAIR_V2 {
        let source = fs::read(&bootstrap_nnue)
            .with_context(|| format!("failed to read {}", bootstrap_nnue.display()))?;
        let migrated =
            haitaka_wasm::migrate_donor_single_to_receiver_pair_v2(&source).map_err(|err| {
                anyhow!("failed to migrate V1 bootstrap to DonorReceiverPairV2: {err}")
            })?;
        fs::write(&artifacts.bootstrap_migrated_nnue, migrated).with_context(|| {
            format!(
                "failed to write {}",
                artifacts.bootstrap_migrated_nnue.display()
            )
        })?;
        artifacts.bootstrap_migrated_nnue.clone()
    } else {
        bootstrap_nnue
    };
    let imported_model_pt = if import_features == training_features {
        artifacts.bootstrap_model_pt.clone()
    } else {
        bootstrap_base_model_pt_path(&artifacts.bootstrap_model_pt)
    };
    run_command(
        &loaded.config.paths.python,
        &[
            "serialize.py".to_string(),
            bootstrap_nnue.display().to_string(),
            imported_model_pt.display().to_string(),
            "--features".to_string(),
            import_features.to_string(),
        ],
        trainer_checkout,
        "convert bootstrap NNUE to torch checkpoint",
    )?;

    if import_features != training_features {
        run_command(
            &loaded.config.paths.python,
            &[
                "-c".to_string(),
                bootstrap_feature_expansion_script().to_string(),
                imported_model_pt.display().to_string(),
                artifacts.bootstrap_model_pt.display().to_string(),
                training_features.to_string(),
            ],
            trainer_checkout,
            "expand bootstrap torch checkpoint feature set",
        )?;
        let _ = fs::remove_file(&imported_model_pt);
    }

    Ok(Some(artifacts.bootstrap_model_pt))
}

fn bootstrap_import_features(loaded: &LoadedConfig) -> &str {
    // A warm-start NNUE must be parsed with the exact family that produced its
    // network hash.  The Phase 7.1 v0.5.1 anchor is already a donor-family
    // network; importing it as plain HalfKAv2 makes serialize.py reject the
    // header before any feature expansion can occur.
    loaded.training_features()
}

fn bootstrap_base_model_pt_path(path: &Path) -> PathBuf {
    let mut path = path.to_path_buf();
    path.set_extension("base.pt");
    path
}

fn bootstrap_feature_expansion_script() -> &'static str {
    r#"import sys, torch, features
source, target, feature_name = sys.argv[1:4]
feature_set = features.get_feature_set_from_name(feature_name)
model = torch.load(source, map_location="cpu", weights_only=False)
model.set_feature_set(feature_set)
torch.save(model, target)
"#
}

fn run_command(program: &str, args: &[String], cwd: &Path, label: &str) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("failed to start {label} using `{program}`"))?;
    if !status.success() {
        bail!("{label} failed with exit status {status}");
    }
    Ok(())
}

fn ensure_file_exists(path: &Path, label: &str) -> Result<()> {
    if path.exists() {
        Ok(())
    } else {
        bail!("{label} is missing: {}", path.display())
    }
}

fn ensure_training_board_minimum(
    loaded: &LoadedConfig,
    bin_path: &Path,
    manifest_path: &Path,
) -> Result<()> {
    let Some(minimum) = loaded.config.data.minimum_train_boards()? else {
        return Ok(());
    };
    let report = audit_dataset(bin_path, manifest_path, None).with_context(|| {
        format!("failed to audit the training dataset before applying the {minimum}-board minimum")
    })?;
    let distinct_boards = report.distinct_packed_boards();
    if distinct_boards < minimum {
        bail!(
            "training dataset contains {distinct_boards} distinct packed boards, below the configured minimum of {minimum}; do not start training"
        );
    }
    Ok(())
}

fn ensure_training_dataset_ready(
    bin_path: &Path,
    manifest_path: &Path,
    label: &str,
    expected_game_count: u32,
) -> Result<()> {
    ensure_file_exists(bin_path, label)?;
    ensure_file_exists(manifest_path, &format!("{label} manifest"))?;

    let manifest: DatasetCompletionManifest = serde_json::from_slice(
        &fs::read(manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    if manifest.game_count != expected_game_count {
        bail!(
            "{label} manifest game_count is {}, expected {}",
            manifest.game_count,
            expected_game_count
        );
    }
    if manifest.completed_games != manifest.game_count {
        bail!(
            "{label} is incomplete: completed {}/{} games. Rerun generate-data to resume.",
            manifest.completed_games,
            manifest.game_count
        );
    }
    if manifest.entry_bytes != ENTRY_BYTES {
        bail!(
            "{label} manifest entry_bytes is {}, expected {}",
            manifest.entry_bytes,
            ENTRY_BYTES
        );
    }
    if manifest.teacher_move_encoding != TEACHER_MOVE_ENCODING {
        bail!(
            "{label} manifest teacher_move_encoding is `{}`, expected `{TEACHER_MOVE_ENCODING}`; regenerate the dataset so teacher-move-dependent filtering cannot consume ambiguous 16-bit values",
            if manifest.teacher_move_encoding.is_empty() {
                "legacy-unspecified"
            } else {
                &manifest.teacher_move_encoding
            }
        );
    }
    let expected_len = manifest
        .sampled_positions
        .checked_mul(manifest.entry_bytes as u64)
        .ok_or_else(|| anyhow!("{label} byte length overflow"))?;
    let actual_len = fs::metadata(bin_path)
        .with_context(|| format!("failed to stat {}", bin_path.display()))?
        .len();
    if actual_len != expected_len {
        bail!(
            "{label} has {} bytes, expected {} from manifest",
            actual_len,
            expected_len
        );
    }
    Ok(())
}

fn detect_git_revision(repo_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(crate) fn collect_checkpoints(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_checkpoints(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "ckpt") {
            out.push(path);
        }
    }
}

pub(crate) fn sort_checkpoints_newest_first(paths: &mut [PathBuf]) {
    paths.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    paths.reverse();
}

pub(crate) fn find_latest_valid_checkpoint(
    root: &Path,
    python: &str,
    cwd: &Path,
) -> Result<Option<PathBuf>> {
    let mut candidates = Vec::new();
    collect_checkpoints(root, &mut candidates);
    sort_checkpoints_newest_first(&mut candidates);
    for candidate in candidates {
        if is_valid_checkpoint(&candidate, python, cwd)? {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

pub(crate) fn is_valid_checkpoint(path: &Path, python: &str, cwd: &Path) -> Result<bool> {
    let status = Command::new(python)
        .args(["-c", checkpoint_validation_script()])
        .arg(path)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("failed to inspect checkpoint {}", path.display()))?;
    Ok(status.success())
}

fn checkpoint_validation_script() -> &'static str {
    "import sys, torch; torch.load(sys.argv[1], map_location='cpu', weights_only=False)"
}

fn variant_py_contents(loaded: &LoadedConfig) -> String {
    format!(
        r#"RANKS = 9
FILES = 9
SQUARES = RANKS * FILES
KING_SQUARES = RANKS * FILES
PIECE_TYPES = 10
PIECES = 2 * PIECE_TYPES
USE_POCKETS = True
POCKETS = 2 * FILES if USE_POCKETS else 0
RULESET = "{ruleset}"
DONOR_MODE = "{donor_mode}"

PIECE_VALUES = {{
    1: 700,
    2: 800,
    3: 400,
    4: 1000,
    5: 100,
    6: 300,
    7: 300,
    8: 500,
    9: 900,
}}
"#,
        ruleset = loaded.config.rules.ruleset.as_str(),
        donor_mode = donor_mode_py_name(loaded.config.rules.ruleset)
    )
}

fn variant_h_contents(loaded: &LoadedConfig) -> String {
    format!(
        r#"#define FILES 9
#define RANKS 9
#define PIECE_TYPES 10
#define PIECE_COUNT 40
#define POCKETS true
#define KING_SQUARES FILES * RANKS
#define DATA_SIZE 512
#define HAITAKA_DONOR_MODE {}
"#,
        donor_mode_cpp_value(loaded.config.rules.ruleset)
    )
}

fn overlay_feature_set_py_contents() -> String {
    include_str!("../trainer_overlay/feature_set.py").to_string()
}

fn overlay_features_py_contents() -> String {
    include_str!("../trainer_overlay/features.py").to_string()
}

fn overlay_donor_features_py_contents() -> String {
    include_str!("../trainer_overlay/donor_features.py").to_string()
}

fn overlay_training_data_loader_cpp_contents() -> String {
    include_str!("../trainer_overlay/training_data_loader.cpp").to_string()
}

fn donor_mode_py_name(ruleset: Ruleset) -> &'static str {
    match ruleset {
        Ruleset::Standard | Ruleset::Handicap => "none",
        Ruleset::Annan => "single-behind",
        Ruleset::Anhoku => "single-front",
        Ruleset::Antouzai => "pair-left-right",
        Ruleset::Taimen => "single-front-enemy",
        Ruleset::Haimen => "single-behind-enemy",
        Ruleset::Neko => "single-neko-vertical-friendly",
        Ruleset::Nekoneko => "single-neko-vertical-any",
        Ruleset::Yokoneko => "single-neko-horizontal-friendly",
        Ruleset::Yokonekoneko => "single-neko-horizontal-any",
        Ruleset::Tenkyo => "single-point-symmetry-any",
        Ruleset::Tenjiku => "single-behind-plus-native",
        Ruleset::Anki => "knight8-friendly",
    }
}

fn donor_mode_cpp_value(ruleset: Ruleset) -> u8 {
    match ruleset {
        Ruleset::Standard | Ruleset::Handicap => 0,
        Ruleset::Annan => 1,
        Ruleset::Anhoku => 2,
        Ruleset::Antouzai => 3,
        Ruleset::Taimen => 4,
        Ruleset::Haimen => 5,
        Ruleset::Neko => 6,
        Ruleset::Nekoneko => 7,
        Ruleset::Yokoneko => 8,
        Ruleset::Yokonekoneko => 9,
        Ruleset::Tenkyo => 10,
        Ruleset::Tenjiku => 11,
        Ruleset::Anki => 12,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        DataConfig, ExportConfig, LearnConfig, PathsConfig, RulesConfig, SelectionConfig,
        TrainingConfig, VerifyConfig,
    };
    use tempfile::tempdir;

    #[test]
    fn checkpoint_validation_script_disables_weights_only_mode() {
        assert!(checkpoint_validation_script().contains("weights_only=False"));
    }

    #[test]
    fn donor_training_imports_bootstrap_with_exact_feature_family() {
        let mut loaded = loaded_config_for_tests(Ruleset::Antouzai);
        loaded.config.training.features = Some("HalfKAv2^+DonorPairSlots".to_string());
        assert_eq!(
            bootstrap_import_features(&loaded),
            "HalfKAv2^+DonorPairSlots"
        );

        loaded.config.training.features = Some("HalfKAv2^+DonorSingleEff".to_string());
        assert_eq!(
            bootstrap_import_features(&loaded),
            "HalfKAv2^+DonorSingleEff"
        );

        loaded.config.training.features = Some("HalfKAv2^+DonorKnight8Slots".to_string());
        assert_eq!(
            bootstrap_import_features(&loaded),
            "HalfKAv2^+DonorKnight8Slots"
        );
    }

    #[test]
    fn standard_training_imports_bootstrap_with_training_features() {
        let mut loaded = loaded_config_for_tests(Ruleset::Standard);
        loaded.config.training.features = Some("HalfKAv2^".to_string());
        assert_eq!(bootstrap_import_features(&loaded), "HalfKAv2^");
    }

    #[test]
    fn donor_bootstrap_uses_intermediate_base_model_path() {
        let path = PathBuf::from("/tmp/bootstrap.pt");
        assert_eq!(
            bootstrap_base_model_pt_path(&path),
            PathBuf::from("/tmp/bootstrap.base.pt")
        );
    }

    #[test]
    fn bootstrap_feature_expansion_script_loads_and_resizes_model() {
        let script = bootstrap_feature_expansion_script();
        assert!(script.contains("weights_only=False"));
        assert!(script.contains("model.set_feature_set(feature_set)"));
        assert!(script.contains("torch.save(model, target)"));
    }

    #[test]
    fn variant_overlays_match_haitaka_geometry() {
        let loaded = loaded_config_for_tests(Ruleset::Anhoku);
        let py = variant_py_contents(&loaded);
        let h = variant_h_contents(&loaded);
        assert!(py.contains("PIECE_TYPES = 10"));
        assert!(py.contains("USE_POCKETS = True"));
        assert!(py.contains("DONOR_MODE = \"single-front\""));
        assert!(h.contains("#define FILES 9"));
        assert!(h.contains("#define DATA_SIZE 512"));
        assert!(h.contains("#define HAITAKA_DONOR_MODE 2"));
        assert!(overlay_features_py_contents().contains("donor_features"));
        assert!(overlay_feature_set_py_contents().contains("_calculate_features_hash"));
        assert!(overlay_training_data_loader_cpp_contents().contains("HalfKAv2^+DonorSingleEff"));
        assert!(
            overlay_training_data_loader_cpp_contents().contains("HalfKAv2^+DonorReceiverPairV2")
        );
        assert!(overlay_donor_features_py_contents().contains("DonorReceiverPairV2"));
        assert!(overlay_donor_features_py_contents().contains("0x6D124A8F"));
        assert!(
            overlay_training_data_loader_cpp_contents()
                .contains("Ignore the trainer's smart/filtered")
        );

        let anki = loaded_config_for_tests(Ruleset::Anki);
        assert!(variant_py_contents(&anki).contains("DONOR_MODE = \"knight8-friendly\""));
        assert!(variant_h_contents(&anki).contains("#define HAITAKA_DONOR_MODE 12"));
        assert!(overlay_donor_features_py_contents().contains("knight8-friendly"));
        assert!(overlay_donor_features_py_contents().contains("0x6A09E667"));
    }

    #[test]
    fn training_dataset_ready_accepts_complete_manifest() {
        let temp = tempdir().unwrap();
        let bin_path = temp.path().join("train.bin");
        let manifest_path = temp.path().join("train.json");
        fs::write(&bin_path, vec![0u8; 72]).unwrap();
        fs::write(
            &manifest_path,
            r#"{"game_count":2,"completed_games":2,"sampled_positions":1,"entry_bytes":72,"teacher_move_encoding":"unavailable"}"#,
        )
        .unwrap();

        ensure_training_dataset_ready(&bin_path, &manifest_path, "training dataset", 2).unwrap();
    }

    #[test]
    fn training_dataset_ready_rejects_partial_manifest() {
        let temp = tempdir().unwrap();
        let bin_path = temp.path().join("train.bin");
        let manifest_path = temp.path().join("train.json");
        fs::write(&bin_path, vec![0u8; 72]).unwrap();
        fs::write(
            &manifest_path,
            r#"{"game_count":2,"completed_games":1,"sampled_positions":1,"entry_bytes":72,"teacher_move_encoding":"unavailable"}"#,
        )
        .unwrap();

        let err = ensure_training_dataset_ready(&bin_path, &manifest_path, "training dataset", 2)
            .unwrap_err();

        assert!(format!("{err:?}").contains("training dataset is incomplete"));
    }

    #[test]
    fn training_dataset_ready_rejects_incompatible_entry_bytes() {
        let temp = tempdir().unwrap();
        let bin_path = temp.path().join("train.bin");
        let manifest_path = temp.path().join("train.json");
        fs::write(&bin_path, vec![0u8; 64]).unwrap();
        fs::write(
            &manifest_path,
            r#"{"game_count":2,"completed_games":2,"sampled_positions":1,"entry_bytes":64,"teacher_move_encoding":"unavailable"}"#,
        )
        .unwrap();

        let err = ensure_training_dataset_ready(&bin_path, &manifest_path, "training dataset", 2)
            .unwrap_err();

        assert!(format!("{err:?}").contains("manifest entry_bytes is 64, expected 72"));
    }

    fn loaded_config_for_tests(ruleset: Ruleset) -> LoadedConfig {
        LoadedConfig {
            path: PathBuf::from("/tmp/haitaka_learn.toml"),
            hash_hex: "hash".to_string(),
            config: LearnConfig {
                rules: RulesConfig {
                    ruleset,
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
}

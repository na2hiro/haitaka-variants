use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;

use crate::config::LoadedConfig;

#[derive(Debug, Serialize)]
struct ExportMetadata {
    exported_nnue: String,
    source_checkpoint: String,
    trainer_checkout: String,
    trainer_revision: Option<String>,
    features: String,
    description: String,
    config_hash: String,
}

pub fn train(loaded: &LoadedConfig) -> Result<PathBuf> {
    let trainer_checkout = loaded.trainer_checkout()?;
    let artifacts = loaded.artifact_paths();
    artifacts.ensure_dirs()?;

    ensure_file_exists(&artifacts.train_bin, "training dataset")?;
    ensure_file_exists(&artifacts.validation_bin, "validation dataset")?;

    let _guard = PreparedTrainer::new(loaded, &trainer_checkout)?;
    let bootstrap_model = materialize_bootstrap_pt(loaded, &trainer_checkout)?;

    let mut args = vec![
        "train.py".to_string(),
        artifacts.train_bin.display().to_string(),
        artifacts.validation_bin.display().to_string(),
        "--features".to_string(),
        loaded.config.training.features.clone(),
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
    ];
    if let Some(model) = bootstrap_model {
        args.push("--resume-from-model".to_string());
        args.push(model.display().to_string());
    }
    args.extend(loaded.config.training.extra_args.clone());

    run_command(
        &loaded.config.paths.python,
        &args,
        &trainer_checkout,
        "variant-nnue-pytorch training",
    )?;

    find_latest_checkpoint(&artifacts.logs_dir).ok_or_else(|| {
        anyhow!(
            "training finished but no checkpoint was found under {}",
            artifacts.logs_dir.display()
        )
    })
}

pub fn export(loaded: &LoadedConfig, source_checkpoint: Option<PathBuf>) -> Result<PathBuf> {
    let trainer_checkout = loaded.trainer_checkout()?;
    let artifacts = loaded.artifact_paths();
    artifacts.ensure_dirs()?;

    let checkpoint = if let Some(path) = source_checkpoint {
        path
    } else {
        find_latest_checkpoint(&artifacts.logs_dir).ok_or_else(|| {
            anyhow!(
                "could not find a checkpoint under {}",
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
            loaded.config.training.features.clone(),
            "--description".to_string(),
            loaded.config.export.description.clone(),
        ],
        &trainer_checkout,
        "variant-nnue-pytorch export",
    )?;

    let metadata = ExportMetadata {
        exported_nnue: artifacts.exported_nnue.display().to_string(),
        source_checkpoint: checkpoint.display().to_string(),
        trainer_checkout: trainer_checkout.display().to_string(),
        trainer_revision: detect_git_revision(&trainer_checkout),
        features: loaded.config.training.features.clone(),
        description: loaded.config.export.description.clone(),
        config_hash: loaded.hash_hex.clone(),
    };
    fs::write(
        &artifacts.export_metadata,
        serde_json::to_vec_pretty(&metadata)?,
    )
    .with_context(|| format!("failed to write {}", artifacts.export_metadata.display()))?;

    Ok(artifacts.exported_nnue)
}

struct PreparedTrainer {
    backups: Vec<FileBackup>,
}

impl PreparedTrainer {
    fn new(loaded: &LoadedConfig, trainer_checkout: &Path) -> Result<Self> {
        if !trainer_checkout.exists() {
            bail!(
                "trainer checkout does not exist: {}",
                trainer_checkout.display()
            );
        }

        let variant_py = trainer_checkout.join("variant.py");
        let variant_h = trainer_checkout.join("variant.h");
        let mut prepared = Self {
            backups: Vec::new(),
        };
        prepared.write_with_backup(&variant_py, variant_py_contents())?;
        prepared.write_with_backup(&variant_h, variant_h_contents())?;

        if loaded.config.training.build_data_loader {
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
    run_command(
        &loaded.config.paths.python,
        &[
            "serialize.py".to_string(),
            bootstrap_nnue.display().to_string(),
            artifacts.bootstrap_model_pt.display().to_string(),
            "--features".to_string(),
            loaded.config.training.features.clone(),
        ],
        trainer_checkout,
        "convert bootstrap NNUE to torch checkpoint",
    )?;

    Ok(Some(artifacts.bootstrap_model_pt))
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

fn find_latest_checkpoint(root: &Path) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    collect_checkpoints(root, &mut candidates);
    candidates.into_iter().max_by_key(|path| {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    })
}

fn collect_checkpoints(root: &Path, out: &mut Vec<PathBuf>) {
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

fn variant_py_contents() -> String {
    r#"RANKS = 9
FILES = 9
SQUARES = RANKS * FILES
KING_SQUARES = RANKS * FILES
PIECE_TYPES = 10
PIECES = 2 * PIECE_TYPES
USE_POCKETS = True
POCKETS = 2 * FILES if USE_POCKETS else 0

PIECE_VALUES = {
    1: 700,
    2: 800,
    3: 400,
    4: 1000,
    5: 100,
    6: 300,
    7: 300,
    8: 500,
    9: 900,
}
"#
    .to_string()
}

fn variant_h_contents() -> String {
    r#"#define FILES 9
#define RANKS 9
#define PIECE_TYPES 10
#define PIECE_COUNT 40
#define POCKETS true
#define KING_SQUARES FILES * RANKS
#define DATA_SIZE 512
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_overlays_match_haitaka_geometry() {
        let py = variant_py_contents();
        let h = variant_h_contents();
        assert!(py.contains("PIECE_TYPES = 10"));
        assert!(py.contains("USE_POCKETS = True"));
        assert!(h.contains("#define FILES 9"));
        assert!(h.contains("#define DATA_SIZE 512"));
    }
}

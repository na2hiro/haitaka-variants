use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;

use crate::config::{
    FEATURE_SET_DONOR_KNIGHT8, FEATURE_SET_DONOR_PAIR, FEATURE_SET_DONOR_SINGLE,
    FEATURE_SET_HALFKAV2, LoadedConfig, Ruleset,
};

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

pub fn train(loaded: &LoadedConfig, resume_override: Option<bool>) -> Result<PathBuf> {
    let trainer_checkout = loaded.trainer_checkout()?;
    let artifacts = loaded.artifact_paths();
    artifacts.ensure_dirs()?;

    ensure_file_exists(&artifacts.train_bin, "training dataset")?;
    ensure_file_exists(&artifacts.validation_bin, "validation dataset")?;

    let _guard = PreparedTrainer::new(loaded, &trainer_checkout)?;
    let should_resume = resume_override.unwrap_or(loaded.config.training.resume);
    let resume_checkpoint = if should_resume {
        find_latest_valid_checkpoint(
            &artifacts.logs_dir,
            &loaded.config.paths.python,
            &trainer_checkout,
        )?
    } else {
        None
    };
    let bootstrap_model = if resume_checkpoint.is_none() {
        materialize_bootstrap_pt(loaded, &trainer_checkout)?
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
    ];
    if let Some(checkpoint) = resume_checkpoint {
        println!("resuming training from {}", checkpoint.display());
        args.push("--resume_from_checkpoint".to_string());
        args.push(checkpoint.display().to_string());
    } else if let Some(model) = bootstrap_model {
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
        "variant-nnue-pytorch export",
    )?;

    let metadata = ExportMetadata {
        exported_nnue: artifacts.exported_nnue.display().to_string(),
        source_checkpoint: checkpoint.display().to_string(),
        trainer_checkout: trainer_checkout.display().to_string(),
        trainer_revision: detect_git_revision(&trainer_checkout),
        features: loaded.training_features().to_string(),
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
    let import_features = bootstrap_import_features(loaded);
    let training_features = loaded.training_features();
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
    match loaded.training_features() {
        FEATURE_SET_DONOR_SINGLE | FEATURE_SET_DONOR_PAIR | FEATURE_SET_DONOR_KNIGHT8 => {
            FEATURE_SET_HALFKAV2
        }
        features => features,
    }
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

fn sort_checkpoints_newest_first(paths: &mut [PathBuf]) {
    paths.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    paths.reverse();
}

fn find_latest_valid_checkpoint(root: &Path, python: &str, cwd: &Path) -> Result<Option<PathBuf>> {
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

fn is_valid_checkpoint(path: &Path, python: &str, cwd: &Path) -> Result<bool> {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        DataConfig, ExportConfig, LearnConfig, PathsConfig, RulesConfig, TrainingConfig,
        VerifyConfig,
    };

    #[test]
    fn checkpoint_validation_script_disables_weights_only_mode() {
        assert!(checkpoint_validation_script().contains("weights_only=False"));
    }

    #[test]
    fn donor_training_imports_standard_bootstrap_with_base_features() {
        let mut loaded = loaded_config_for_tests(Ruleset::Antouzai);
        loaded.config.training.features = Some(FEATURE_SET_DONOR_PAIR.to_string());
        assert_eq!(bootstrap_import_features(&loaded), FEATURE_SET_HALFKAV2);

        loaded.config.training.features = Some(FEATURE_SET_DONOR_SINGLE.to_string());
        assert_eq!(bootstrap_import_features(&loaded), FEATURE_SET_HALFKAV2);

        loaded.config.training.features = Some(FEATURE_SET_DONOR_KNIGHT8.to_string());
        assert_eq!(bootstrap_import_features(&loaded), FEATURE_SET_HALFKAV2);
    }

    #[test]
    fn standard_training_imports_bootstrap_with_training_features() {
        let mut loaded = loaded_config_for_tests(Ruleset::Standard);
        loaded.config.training.features = Some(FEATURE_SET_HALFKAV2.to_string());
        assert_eq!(bootstrap_import_features(&loaded), FEATURE_SET_HALFKAV2);
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
            },
        }
    }
}

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const STANDARD_STARTPOS_SFEN: &str =
    "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
const ANNAN_STARTPOS_SFEN: &str =
    "lnsgkgsnl/1r5b1/p1ppppp1p/1p5p1/9/1P5P1/P1PPPPP1P/1B5R1/LNSGKGSNL b - 1";

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub path: PathBuf,
    pub hash_hex: String,
    pub config: LearnConfig,
}

impl LoadedConfig {
    pub fn from_path(path: &Path) -> Result<Self> {
        let raw_toml = fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        let config: LearnConfig =
            toml::from_str(&raw_toml).context("failed to parse haitaka_learn TOML")?;
        config.validate()?;

        let hash_hex = {
            let mut hasher = Sha256::new();
            hasher.update(raw_toml.as_bytes());
            format!("{:x}", hasher.finalize())
        };

        Ok(Self {
            path: path.to_path_buf(),
            hash_hex,
            config,
        })
    }

    pub fn config_dir(&self) -> &Path {
        self.path.parent().unwrap_or_else(|| Path::new("."))
    }

    pub fn resolve_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.config_dir().join(path)
        }
    }

    pub fn runtime_mode(&self) -> &'static str {
        if cfg!(feature = "annan") {
            "annan"
        } else {
            "standard"
        }
    }

    pub fn ruleset_requires_matching_engine(&self) -> Result<()> {
        match self.config.rules.ruleset {
            Ruleset::Annan if !cfg!(feature = "annan") => bail!(
                "ruleset=annan requires building haitaka_learn with `--features annan`"
            ),
            Ruleset::Standard | Ruleset::Handicap if cfg!(feature = "annan") => bail!(
                "standard and handicap self-play data generation should use the default build without `--features annan`"
            ),
            _ => Ok(()),
        }
    }

    pub fn opening_sfen(&self) -> Result<String> {
        match self.config.rules.ruleset {
            Ruleset::Standard => {
                self.ruleset_requires_matching_engine()?;
                Ok(STANDARD_STARTPOS_SFEN.to_string())
            }
            Ruleset::Annan => {
                self.ruleset_requires_matching_engine()?;
                Ok(ANNAN_STARTPOS_SFEN.to_string())
            }
            Ruleset::Handicap => {
                self.ruleset_requires_matching_engine()?;
                if let Some(sfen) = &self.config.rules.opening_sfen {
                    Ok(sfen.clone())
                } else {
                    let preset = self
                        .config
                        .rules
                        .handicap
                        .ok_or_else(|| anyhow!("rules.handicap must be set for ruleset=handicap"))?;
                    let sfen = match preset {
                        HandicapPreset::TwoPiece => haitaka::SFEN_2PIECE_HANDICAP,
                        HandicapPreset::FourPiece => haitaka::SFEN_4PIECE_HANDICAP,
                        HandicapPreset::SixPiece => haitaka::SFEN_6PIECE_HANDICAP,
                    };
                    Ok(sfen.to_string())
                }
            }
        }
    }

    pub fn artifact_paths(&self) -> ArtifactPaths {
        ArtifactPaths::new(self)
    }

    pub fn trainer_checkout(&self) -> Result<PathBuf> {
        let checkout = self
            .config
            .paths
            .trainer_checkout
            .as_ref()
            .ok_or_else(|| anyhow!("paths.trainer_checkout is required for train/export/pipeline"))?;
        Ok(self.resolve_path(checkout))
    }

    pub fn bootstrap_nnue(&self) -> Option<PathBuf> {
        self.config
            .paths
            .bootstrap_nnue
            .as_ref()
            .map(|path| self.resolve_path(path))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactPaths {
    pub output_dir: PathBuf,
    pub datasets_dir: PathBuf,
    pub artifacts_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub train_bin: PathBuf,
    pub validation_bin: PathBuf,
    pub train_manifest: PathBuf,
    pub validation_manifest: PathBuf,
    pub bootstrap_model_pt: PathBuf,
    pub export_metadata: PathBuf,
    pub verify_report: PathBuf,
    pub exported_nnue: PathBuf,
}

impl ArtifactPaths {
    fn new(loaded: &LoadedConfig) -> Self {
        let output_dir = loaded.resolve_path(&loaded.config.paths.output_dir);
        let datasets_dir = output_dir.join("datasets");
        let artifacts_dir = output_dir.join("artifacts");
        let logs_dir = output_dir.join("logs");
        Self {
            train_bin: datasets_dir.join("train.bin"),
            validation_bin: datasets_dir.join("validation.bin"),
            train_manifest: datasets_dir.join("train.json"),
            validation_manifest: datasets_dir.join("validation.json"),
            bootstrap_model_pt: artifacts_dir.join("bootstrap.pt"),
            export_metadata: artifacts_dir.join("export.json"),
            verify_report: artifacts_dir.join("verify.json"),
            exported_nnue: artifacts_dir.join(&loaded.config.export.output_name),
            output_dir,
            datasets_dir,
            artifacts_dir,
            logs_dir,
        }
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.datasets_dir)
            .with_context(|| format!("failed to create {}", self.datasets_dir.display()))?;
        fs::create_dir_all(&self.artifacts_dir)
            .with_context(|| format!("failed to create {}", self.artifacts_dir.display()))?;
        fs::create_dir_all(&self.logs_dir)
            .with_context(|| format!("failed to create {}", self.logs_dir.display()))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LearnConfig {
    pub rules: RulesConfig,
    #[serde(default)]
    pub paths: PathsConfig,
    #[serde(default)]
    pub data: DataConfig,
    #[serde(default)]
    pub training: TrainingConfig,
    #[serde(default)]
    pub export: ExportConfig,
    #[serde(default)]
    pub verify: VerifyConfig,
}

impl LearnConfig {
    fn validate(&self) -> Result<()> {
        ensure!(self.data.train_games > 0, "data.train_games must be > 0");
        ensure!(
            self.data.validation_games > 0,
            "data.validation_games must be > 0"
        );
        ensure!(self.data.max_plies > 0, "data.max_plies must be > 0");
        ensure!(
            self.data.search_depth > 0,
            "data.search_depth must be at least 1"
        );
        ensure!(
            self.data.sample_every_ply > 0,
            "data.sample_every_ply must be at least 1"
        );
        ensure!(
            self.data.max_positions_per_game > 0,
            "data.max_positions_per_game must be > 0"
        );
        ensure!(
            self.training.features == "HalfKAv2^",
            "training.features must stay `HalfKAv2^` for Haitaka/Fairy-Stockfish compatibility"
        );
        if self.rules.ruleset == Ruleset::Handicap {
            ensure!(
                self.rules.handicap.is_some() || self.rules.opening_sfen.is_some(),
                "ruleset=handicap requires either rules.handicap or rules.opening_sfen"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RulesConfig {
    pub ruleset: Ruleset,
    #[serde(default)]
    pub rule_id: Option<u16>,
    #[serde(default)]
    pub handicap: Option<HandicapPreset>,
    #[serde(default)]
    pub opening_sfen: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ruleset {
    Standard,
    Handicap,
    Annan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HandicapPreset {
    TwoPiece,
    FourPiece,
    SixPiece,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PathsConfig {
    #[serde(default = "default_output_dir")]
    pub output_dir: PathBuf,
    #[serde(default)]
    pub trainer_checkout: Option<PathBuf>,
    #[serde(default)]
    pub bootstrap_nnue: Option<PathBuf>,
    #[serde(default = "default_python")]
    pub python: String,
    #[serde(default = "default_cmake")]
    pub cmake: String,
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            output_dir: default_output_dir(),
            trainer_checkout: None,
            bootstrap_nnue: None,
            python: default_python(),
            cmake: default_cmake(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DataConfig {
    #[serde(default = "default_train_games")]
    pub train_games: u32,
    #[serde(default = "default_validation_games")]
    pub validation_games: u32,
    #[serde(default = "default_max_plies")]
    pub max_plies: u16,
    #[serde(default = "default_search_depth")]
    pub search_depth: u8,
    #[serde(default = "default_opening_random_plies")]
    pub opening_random_plies: u16,
    #[serde(default)]
    pub sample_start_ply: u16,
    #[serde(default = "default_sample_every_ply")]
    pub sample_every_ply: u16,
    #[serde(default = "default_max_positions_per_game")]
    pub max_positions_per_game: u16,
    #[serde(default = "default_seed")]
    pub seed: u64,
}

impl Default for DataConfig {
    fn default() -> Self {
        Self {
            train_games: default_train_games(),
            validation_games: default_validation_games(),
            max_plies: default_max_plies(),
            search_depth: default_search_depth(),
            opening_random_plies: default_opening_random_plies(),
            sample_start_ply: 0,
            sample_every_ply: default_sample_every_ply(),
            max_positions_per_game: default_max_positions_per_game(),
            seed: default_seed(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrainingConfig {
    #[serde(default = "default_features")]
    pub features: String,
    #[serde(default = "default_num_workers")]
    pub num_workers: u32,
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,
    #[serde(rename = "lambda", default = "default_lambda")]
    pub lambda_: f32,
    #[serde(default = "default_random_fen_skipping")]
    pub random_fen_skipping: u32,
    #[serde(default = "default_epoch_size")]
    pub epoch_size: u32,
    #[serde(default = "default_validation_size")]
    pub validation_size: u32,
    #[serde(default = "default_max_epochs")]
    pub max_epochs: u32,
    #[serde(default = "default_build_data_loader")]
    pub build_data_loader: bool,
    #[serde(default)]
    pub extra_args: Vec<String>,
}

impl Default for TrainingConfig {
    fn default() -> Self {
        Self {
            features: default_features(),
            num_workers: default_num_workers(),
            batch_size: default_batch_size(),
            lambda_: default_lambda(),
            random_fen_skipping: default_random_fen_skipping(),
            epoch_size: default_epoch_size(),
            validation_size: default_validation_size(),
            max_epochs: default_max_epochs(),
            build_data_loader: default_build_data_loader(),
            extra_args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExportConfig {
    #[serde(default = "default_output_name")]
    pub output_name: String,
    #[serde(default = "default_export_description")]
    pub description: String,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            output_name: default_output_name(),
            description: default_export_description(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct VerifyConfig {
    #[serde(default = "default_verify_search_depth")]
    pub search_depth: u8,
    #[serde(default = "default_run_search_smoke")]
    pub run_search_smoke: bool,
}

impl Default for VerifyConfig {
    fn default() -> Self {
        Self {
            search_depth: default_verify_search_depth(),
            run_search_smoke: default_run_search_smoke(),
        }
    }
}

fn default_output_dir() -> PathBuf {
    PathBuf::from("haitaka_learn-out")
}

fn default_python() -> String {
    "python3".to_string()
}

fn default_cmake() -> String {
    "cmake".to_string()
}

fn default_train_games() -> u32 {
    8
}

fn default_validation_games() -> u32 {
    2
}

fn default_max_plies() -> u16 {
    160
}

fn default_search_depth() -> u8 {
    2
}

fn default_opening_random_plies() -> u16 {
    8
}

fn default_sample_every_ply() -> u16 {
    2
}

fn default_max_positions_per_game() -> u16 {
    24
}

fn default_seed() -> u64 {
    42
}

fn default_features() -> String {
    "HalfKAv2^".to_string()
}

fn default_num_workers() -> u32 {
    1
}

fn default_batch_size() -> u32 {
    16_384
}

fn default_lambda() -> f32 {
    1.0
}

fn default_random_fen_skipping() -> u32 {
    3
}

fn default_epoch_size() -> u32 {
    200_000
}

fn default_validation_size() -> u32 {
    20_000
}

fn default_max_epochs() -> u32 {
    1
}

fn default_build_data_loader() -> bool {
    true
}

fn default_output_name() -> String {
    "haitaka.nnue".to_string()
}

fn default_export_description() -> String {
    "Haitaka network trained with variant-nnue-pytorch".to_string()
}

fn default_verify_search_depth() -> u8 {
    2
}

fn default_run_search_smoke() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let raw = r#"
[rules]
ruleset = "standard"

[paths]
output_dir = "out"

[data]
train_games = 1
validation_games = 1
"#;
        let config: LearnConfig = toml::from_str(raw).unwrap();
        config.validate().unwrap();
        assert_eq!(config.training.features, "HalfKAv2^");
        assert_eq!(config.export.output_name, "haitaka.nnue");
    }
}

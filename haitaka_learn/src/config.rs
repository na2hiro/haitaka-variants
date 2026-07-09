use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const STANDARD_STARTPOS_SFEN: &str =
    "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
const ANNAN_STARTPOS_SFEN: &str =
    "lnsgkgsnl/1r5b1/p1ppppp1p/1p5p1/9/1P5P1/P1PPPPP1P/1B5R1/LNSGKGSNL b - 1";
pub const FEATURE_SET_HALFKAV2: &str = "HalfKAv2^";
pub const FEATURE_SET_DONOR_SINGLE: &str = "HalfKAv2^+DonorSingleEff";
pub const FEATURE_SET_DONOR_PAIR: &str = "HalfKAv2^+DonorPairSlots";
#[allow(dead_code)]
pub const FEATURE_SET_DONOR_KNIGHT8: &str = "HalfKAv2^+DonorKnight8Slots";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RulesetSpec {
    pub ruleset: Ruleset,
    pub required_feature: Option<&'static str>,
    pub default_rule_id: u16,
    pub default_opening_sfen: &'static str,
    pub verification_name: &'static str,
}

pub const DEFAULT_RULESET_SPECS: [RulesetSpec; 13] = [
    RulesetSpec {
        ruleset: Ruleset::Standard,
        required_feature: None,
        default_rule_id: 0,
        default_opening_sfen: STANDARD_STARTPOS_SFEN,
        verification_name: "standard_startpos",
    },
    RulesetSpec {
        ruleset: Ruleset::Annan,
        required_feature: Some("annan"),
        default_rule_id: 26,
        default_opening_sfen: ANNAN_STARTPOS_SFEN,
        verification_name: "annan_startpos",
    },
    RulesetSpec {
        ruleset: Ruleset::Anhoku,
        required_feature: Some("anhoku"),
        default_rule_id: 55,
        default_opening_sfen: STANDARD_STARTPOS_SFEN,
        verification_name: "anhoku_startpos",
    },
    RulesetSpec {
        ruleset: Ruleset::Antouzai,
        required_feature: Some("antouzai"),
        default_rule_id: 95,
        default_opening_sfen: STANDARD_STARTPOS_SFEN,
        verification_name: "antouzai_startpos",
    },
    RulesetSpec {
        ruleset: Ruleset::Taimen,
        required_feature: Some("taimen"),
        default_rule_id: 72,
        default_opening_sfen: STANDARD_STARTPOS_SFEN,
        verification_name: "taimen_startpos",
    },
    RulesetSpec {
        ruleset: Ruleset::Haimen,
        required_feature: Some("haimen"),
        default_rule_id: 74,
        default_opening_sfen: STANDARD_STARTPOS_SFEN,
        verification_name: "haimen_startpos",
    },
    RulesetSpec {
        ruleset: Ruleset::Neko,
        required_feature: Some("neko"),
        default_rule_id: 130,
        default_opening_sfen: STANDARD_STARTPOS_SFEN,
        verification_name: "neko_startpos",
    },
    RulesetSpec {
        ruleset: Ruleset::Nekoneko,
        required_feature: Some("nekoneko"),
        default_rule_id: 131,
        default_opening_sfen: STANDARD_STARTPOS_SFEN,
        verification_name: "nekoneko_startpos",
    },
    RulesetSpec {
        ruleset: Ruleset::Yokoneko,
        required_feature: Some("yokoneko"),
        default_rule_id: 132,
        default_opening_sfen: STANDARD_STARTPOS_SFEN,
        verification_name: "yokoneko_startpos",
    },
    RulesetSpec {
        ruleset: Ruleset::Yokonekoneko,
        required_feature: Some("yokonekoneko"),
        default_rule_id: 133,
        default_opening_sfen: STANDARD_STARTPOS_SFEN,
        verification_name: "yokonekoneko_startpos",
    },
    RulesetSpec {
        ruleset: Ruleset::Tenkyo,
        required_feature: Some("tenkyo"),
        default_rule_id: 151,
        default_opening_sfen: STANDARD_STARTPOS_SFEN,
        verification_name: "tenkyo_startpos",
    },
    RulesetSpec {
        ruleset: Ruleset::Tenjiku,
        required_feature: Some("tenjiku"),
        default_rule_id: 56,
        default_opening_sfen: STANDARD_STARTPOS_SFEN,
        verification_name: "tenjiku_startpos",
    },
    RulesetSpec {
        ruleset: Ruleset::Anki,
        required_feature: Some("anki"),
        default_rule_id: 94,
        default_opening_sfen: STANDARD_STARTPOS_SFEN,
        verification_name: "anki_startpos",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerificationFixture {
    pub name: &'static str,
    pub sfen: &'static str,
}

const HANDICAP_6PIECE_FIXTURE: VerificationFixture = VerificationFixture {
    name: "handicap_6piece",
    sfen: haitaka::SFEN_6PIECE_HANDICAP,
};

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub path: PathBuf,
    pub hash_hex: String,
    pub config: LearnConfig,
}

impl LoadedConfig {
    pub fn from_path(path: &Path) -> Result<Self> {
        let canonical_path = fs::canonicalize(path)
            .with_context(|| format!("failed to resolve config {}", path.display()))?;
        let raw_toml = fs::read_to_string(&canonical_path)
            .with_context(|| format!("failed to read config {}", canonical_path.display()))?;
        let config: LearnConfig =
            toml::from_str(&raw_toml).context("failed to parse haitaka_learn TOML")?;
        config.validate()?;

        let hash_hex = {
            let mut hasher = Sha256::new();
            hasher.update(raw_toml.as_bytes());
            format!("{:x}", hasher.finalize())
        };

        Ok(Self {
            path: canonical_path,
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
        active_variant_feature().unwrap_or("standard")
    }

    pub fn training_features(&self) -> &str {
        self.config
            .training
            .features
            .as_deref()
            .unwrap_or_else(|| recommended_feature_set(self.config.rules.ruleset))
    }

    pub fn ruleset_requires_matching_engine(&self) -> Result<()> {
        match self.config.rules.ruleset {
            Ruleset::Handicap | Ruleset::Standard => {
                if active_variant_feature().is_some() {
                    bail!(
                        "ruleset={} requires the default haitaka_learn build without variant features",
                        self.config.rules.ruleset.as_str()
                    );
                }
                Ok(())
            }
            ruleset => {
                let required_feature = ruleset
                    .spec()
                    .and_then(|spec| spec.required_feature)
                    .expect("non-standard variant rulesets should have a required feature");
                if active_variant_feature() == Some(required_feature) {
                    Ok(())
                } else {
                    bail!(
                        "ruleset={} requires building haitaka_learn with `--features {required_feature}`",
                        ruleset.as_str()
                    );
                }
            }
        }
    }

    pub fn opening_sfen(&self) -> Result<String> {
        self.ruleset_requires_matching_engine()?;
        if let Some(sfen) = &self.config.rules.opening_sfen {
            return Ok(sfen.clone());
        }

        match self.config.rules.ruleset {
            Ruleset::Standard
            | Ruleset::Annan
            | Ruleset::Anhoku
            | Ruleset::Antouzai
            | Ruleset::Taimen
            | Ruleset::Haimen
            | Ruleset::Neko
            | Ruleset::Nekoneko
            | Ruleset::Yokoneko
            | Ruleset::Yokonekoneko
            | Ruleset::Tenkyo
            | Ruleset::Tenjiku
            | Ruleset::Anki => self
                .config
                .rules
                .ruleset
                .spec()
                .map(|spec| spec.default_opening_sfen.to_string())
                .ok_or_else(|| {
                    anyhow!(
                        "missing ruleset spec for {}",
                        self.config.rules.ruleset.as_str()
                    )
                }),
            Ruleset::Handicap => {
                let preset =
                    self.config.rules.handicap.ok_or_else(|| {
                        anyhow!("rules.handicap must be set for ruleset=handicap")
                    })?;
                let sfen = match preset {
                    HandicapPreset::TwoPiece => haitaka::SFEN_2PIECE_HANDICAP,
                    HandicapPreset::FourPiece => haitaka::SFEN_4PIECE_HANDICAP,
                    HandicapPreset::SixPiece => haitaka::SFEN_6PIECE_HANDICAP,
                };
                Ok(sfen.to_string())
            }
        }
    }

    pub fn effective_rule_id(&self) -> Result<u16> {
        if let Some(rule_id) = self.config.rules.rule_id {
            return Ok(rule_id);
        }

        match self.config.rules.ruleset {
            Ruleset::Handicap => match self.config.rules.handicap {
                Some(HandicapPreset::SixPiece) => Ok(6),
                Some(HandicapPreset::FourPiece) => Ok(4),
                Some(HandicapPreset::TwoPiece) => Ok(2),
                None => bail!(
                    "rules.rule_id must be set when ruleset=handicap uses a custom opening_sfen without a named handicap preset"
                ),
            },
            ruleset => ruleset
                .spec()
                .map(|spec| spec.default_rule_id)
                .ok_or_else(|| anyhow!("missing ruleset spec for {}", ruleset.as_str())),
        }
    }

    pub fn verification_fixtures(&self) -> Vec<VerificationFixture> {
        let mut fixtures = Vec::with_capacity(DEFAULT_RULESET_SPECS.len() + 1);
        fixtures.extend(
            DEFAULT_RULESET_SPECS
                .iter()
                .map(|spec| VerificationFixture {
                    name: spec.verification_name,
                    sfen: spec.default_opening_sfen,
                }),
        );
        fixtures.push(HANDICAP_6PIECE_FIXTURE);
        fixtures
    }

    pub fn artifact_paths(&self) -> ArtifactPaths {
        ArtifactPaths::new(self)
    }

    pub fn trainer_checkout(&self) -> Result<PathBuf> {
        let checkout = self.config.paths.trainer_checkout.as_ref().ok_or_else(|| {
            anyhow!("paths.trainer_checkout is required for train/export/pipeline")
        })?;
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
    #[serde(default)]
    pub selection: SelectionConfig,
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
            self.data.rollout_search_depth > 0,
            "data.rollout_search_depth must be at least 1"
        );
        ensure!(
            self.data.sample_every_ply > 0,
            "data.sample_every_ply must be at least 1"
        );
        ensure!(
            self.data.max_positions_per_game > 0,
            "data.max_positions_per_game must be > 0"
        );
        ensure!(self.data.shard_games > 0, "data.shard_games must be > 0");
        ensure!(
            (1..=100).contains(&self.data.progress_every_percent),
            "data.progress_every_percent must be between 1 and 100"
        );
        let recommended = recommended_feature_set(self.rules.ruleset);
        let configured = self.training.features.as_deref().unwrap_or(recommended);
        let allowed = allowed_feature_sets(self.rules.ruleset);
        ensure!(
            allowed.contains(&configured),
            "training.features=`{configured}` is not valid for ruleset={}; expected one of: {}",
            self.rules.ruleset.as_str(),
            allowed.join(", ")
        );
        ensure!(
            self.selection.poll_interval_secs > 0,
            "selection.poll_interval_secs must be > 0"
        );
        ensure!(
            self.selection.batch_games > 0,
            "selection.batch_games must be > 0"
        );
        ensure!(
            self.selection.max_games >= self.selection.batch_games,
            "selection.max_games must be >= selection.batch_games"
        );
        ensure!(
            self.selection.movetime_ms > 0,
            "selection.movetime_ms must be > 0"
        );
        ensure!(
            self.selection.sprt_alpha > 0.0 && self.selection.sprt_alpha < 1.0,
            "selection.sprt_alpha must be between 0 and 1"
        );
        ensure!(
            self.selection.sprt_beta > 0.0 && self.selection.sprt_beta < 1.0,
            "selection.sprt_beta must be between 0 and 1"
        );
        ensure!(
            self.selection.sprt_elo1 > self.selection.sprt_elo0,
            "selection.sprt_elo1 must be greater than selection.sprt_elo0"
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
    Anhoku,
    Antouzai,
    Taimen,
    Haimen,
    Neko,
    Nekoneko,
    Yokoneko,
    Yokonekoneko,
    Tenkyo,
    Tenjiku,
    Anki,
}

impl Ruleset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Handicap => "handicap",
            Self::Annan => "annan",
            Self::Anhoku => "anhoku",
            Self::Antouzai => "antouzai",
            Self::Taimen => "taimen",
            Self::Haimen => "haimen",
            Self::Neko => "neko",
            Self::Nekoneko => "nekoneko",
            Self::Yokoneko => "yokoneko",
            Self::Yokonekoneko => "yokonekoneko",
            Self::Tenkyo => "tenkyo",
            Self::Tenjiku => "tenjiku",
            Self::Anki => "anki",
        }
    }

    pub fn spec(self) -> Option<&'static RulesetSpec> {
        DEFAULT_RULESET_SPECS
            .iter()
            .find(|spec| spec.ruleset == self)
    }
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
    #[serde(default = "default_rollout_search_depth")]
    pub rollout_search_depth: u8,
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
    #[serde(default = "default_jobs")]
    pub jobs: u32,
    #[serde(default = "default_shard_games")]
    pub shard_games: u32,
    #[serde(default = "default_progress_every_percent")]
    pub progress_every_percent: u32,
    #[serde(default = "default_resume")]
    pub resume: bool,
}

impl Default for DataConfig {
    fn default() -> Self {
        Self {
            train_games: default_train_games(),
            validation_games: default_validation_games(),
            max_plies: default_max_plies(),
            search_depth: default_search_depth(),
            rollout_search_depth: default_rollout_search_depth(),
            opening_random_plies: default_opening_random_plies(),
            sample_start_ply: 0,
            sample_every_ply: default_sample_every_ply(),
            max_positions_per_game: default_max_positions_per_game(),
            seed: default_seed(),
            jobs: default_jobs(),
            shard_games: default_shard_games(),
            progress_every_percent: default_progress_every_percent(),
            resume: default_resume(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrainingConfig {
    #[serde(default = "default_features")]
    pub features: Option<String>,
    #[serde(default = "default_training_resume")]
    pub resume: bool,
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
            resume: default_training_resume(),
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

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SelectionConfig {
    #[serde(default = "default_selection_poll_interval_secs")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_selection_stable_checkpoint_secs")]
    pub stable_checkpoint_secs: u64,
    #[serde(default = "default_selection_batch_games")]
    pub batch_games: u32,
    #[serde(default = "default_selection_max_games")]
    pub max_games: u32,
    #[serde(default = "default_selection_threads")]
    pub threads: usize,
    #[serde(default = "default_selection_movetime_ms")]
    pub movetime_ms: u32,
    #[serde(default = "default_selection_opening_random_plies")]
    pub opening_random_plies: u16,
    #[serde(default = "default_selection_seed")]
    pub seed: u64,
    #[serde(default = "default_selection_sprt_elo0")]
    pub sprt_elo0: f64,
    #[serde(default = "default_selection_sprt_elo1")]
    pub sprt_elo1: f64,
    #[serde(default = "default_selection_sprt_alpha")]
    pub sprt_alpha: f64,
    #[serde(default = "default_selection_sprt_beta")]
    pub sprt_beta: f64,
    #[serde(default)]
    pub storage_saver: bool,
}

impl Default for SelectionConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: default_selection_poll_interval_secs(),
            stable_checkpoint_secs: default_selection_stable_checkpoint_secs(),
            batch_games: default_selection_batch_games(),
            max_games: default_selection_max_games(),
            threads: default_selection_threads(),
            movetime_ms: default_selection_movetime_ms(),
            opening_random_plies: default_selection_opening_random_plies(),
            seed: default_selection_seed(),
            sprt_elo0: default_selection_sprt_elo0(),
            sprt_elo1: default_selection_sprt_elo1(),
            sprt_alpha: default_selection_sprt_alpha(),
            sprt_beta: default_selection_sprt_beta(),
            storage_saver: false,
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

fn default_rollout_search_depth() -> u8 {
    1
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

fn default_jobs() -> u32 {
    0
}

fn default_shard_games() -> u32 {
    100
}

fn default_progress_every_percent() -> u32 {
    1
}

fn default_resume() -> bool {
    true
}

fn default_features() -> Option<String> {
    None
}

fn default_training_resume() -> bool {
    true
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

fn default_selection_poll_interval_secs() -> u64 {
    15
}

fn default_selection_stable_checkpoint_secs() -> u64 {
    10
}

fn default_selection_batch_games() -> u32 {
    64
}

fn default_selection_max_games() -> u32 {
    1024
}

fn default_selection_threads() -> usize {
    0
}

fn default_selection_movetime_ms() -> u32 {
    100
}

fn default_selection_opening_random_plies() -> u16 {
    4
}

fn default_selection_seed() -> u64 {
    1
}

fn default_selection_sprt_elo0() -> f64 {
    0.0
}

fn default_selection_sprt_elo1() -> f64 {
    5.0
}

fn default_selection_sprt_alpha() -> f64 {
    0.05
}

fn default_selection_sprt_beta() -> f64 {
    0.05
}

fn active_variant_feature() -> Option<&'static str> {
    if cfg!(feature = "annan") {
        Some("annan")
    } else if cfg!(feature = "anhoku") {
        Some("anhoku")
    } else if cfg!(feature = "antouzai") {
        Some("antouzai")
    } else if cfg!(feature = "taimen") {
        Some("taimen")
    } else if cfg!(feature = "haimen") {
        Some("haimen")
    } else if cfg!(feature = "neko") {
        Some("neko")
    } else if cfg!(feature = "nekoneko") {
        Some("nekoneko")
    } else if cfg!(feature = "yokoneko") {
        Some("yokoneko")
    } else if cfg!(feature = "yokonekoneko") {
        Some("yokonekoneko")
    } else if cfg!(feature = "tenkyo") {
        Some("tenkyo")
    } else if cfg!(feature = "tenjiku") {
        Some("tenjiku")
    } else if cfg!(feature = "anki") {
        Some("anki")
    } else {
        None
    }
}

pub fn recommended_feature_set(ruleset: Ruleset) -> &'static str {
    match ruleset {
        Ruleset::Standard | Ruleset::Handicap => FEATURE_SET_HALFKAV2,
        Ruleset::Annan
        | Ruleset::Anhoku
        | Ruleset::Taimen
        | Ruleset::Haimen
        | Ruleset::Neko
        | Ruleset::Nekoneko
        | Ruleset::Yokoneko
        | Ruleset::Yokonekoneko
        | Ruleset::Tenkyo
        | Ruleset::Tenjiku => FEATURE_SET_DONOR_SINGLE,
        Ruleset::Antouzai => FEATURE_SET_DONOR_PAIR,
        Ruleset::Anki => FEATURE_SET_DONOR_KNIGHT8,
    }
}

fn allowed_feature_sets(ruleset: Ruleset) -> Vec<&'static str> {
    match ruleset {
        Ruleset::Standard | Ruleset::Handicap => vec![FEATURE_SET_HALFKAV2],
        Ruleset::Annan
        | Ruleset::Anhoku
        | Ruleset::Taimen
        | Ruleset::Haimen
        | Ruleset::Neko
        | Ruleset::Nekoneko
        | Ruleset::Yokoneko
        | Ruleset::Yokonekoneko
        | Ruleset::Tenkyo
        | Ruleset::Tenjiku => vec![FEATURE_SET_DONOR_SINGLE],
        Ruleset::Antouzai => vec![FEATURE_SET_DONOR_PAIR],
        Ruleset::Anki => vec![FEATURE_SET_DONOR_KNIGHT8],
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use tempfile::tempdir;

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
        assert_eq!(config.training.features, None);
        assert_eq!(config.export.output_name, "haitaka.nnue");
        assert_eq!(config.data.jobs, 0);
        assert_eq!(config.data.shard_games, 100);
        assert_eq!(config.data.progress_every_percent, 1);
        assert!(config.data.resume);
    }

    #[test]
    fn ruleset_specs_cover_supported_training_rules() {
        assert_eq!(Ruleset::Standard.spec().unwrap().default_rule_id, 0);
        assert_eq!(Ruleset::Annan.spec().unwrap().default_rule_id, 26);
        assert_eq!(Ruleset::Anhoku.spec().unwrap().default_rule_id, 55);
        assert_eq!(Ruleset::Antouzai.spec().unwrap().default_rule_id, 95);
        assert_eq!(Ruleset::Taimen.spec().unwrap().default_rule_id, 72);
        assert_eq!(Ruleset::Haimen.spec().unwrap().default_rule_id, 74);
        assert_eq!(Ruleset::Tenkyo.spec().unwrap().default_rule_id, 151);
        assert_eq!(Ruleset::Tenjiku.spec().unwrap().default_rule_id, 56);
        assert_eq!(Ruleset::Anki.spec().unwrap().default_rule_id, 94);
        assert_eq!(
            Ruleset::Anhoku.spec().unwrap().required_feature,
            Some("anhoku")
        );
        assert_eq!(
            Ruleset::Antouzai.spec().unwrap().required_feature,
            Some("antouzai")
        );
        assert_eq!(
            Ruleset::Taimen.spec().unwrap().required_feature,
            Some("taimen")
        );
        assert_eq!(
            Ruleset::Haimen.spec().unwrap().required_feature,
            Some("haimen")
        );
        assert!(Ruleset::Handicap.spec().is_none());
    }

    #[test]
    fn resolves_recommended_feature_sets_by_ruleset() {
        assert_eq!(
            recommended_feature_set(Ruleset::Standard),
            FEATURE_SET_HALFKAV2
        );
        assert_eq!(
            recommended_feature_set(Ruleset::Handicap),
            FEATURE_SET_HALFKAV2
        );
        assert_eq!(
            recommended_feature_set(Ruleset::Annan),
            FEATURE_SET_DONOR_SINGLE
        );
        assert_eq!(
            recommended_feature_set(Ruleset::Anhoku),
            FEATURE_SET_DONOR_SINGLE
        );
        assert_eq!(
            recommended_feature_set(Ruleset::Antouzai),
            FEATURE_SET_DONOR_PAIR
        );
        assert_eq!(
            recommended_feature_set(Ruleset::Taimen),
            FEATURE_SET_DONOR_SINGLE
        );
        assert_eq!(
            recommended_feature_set(Ruleset::Haimen),
            FEATURE_SET_DONOR_SINGLE
        );
        assert_eq!(
            recommended_feature_set(Ruleset::Tenkyo),
            FEATURE_SET_DONOR_SINGLE
        );
        assert_eq!(
            recommended_feature_set(Ruleset::Tenjiku),
            FEATURE_SET_DONOR_SINGLE
        );
        assert_eq!(
            recommended_feature_set(Ruleset::Anki),
            FEATURE_SET_DONOR_KNIGHT8
        );
    }

    #[test]
    fn rejects_mismatched_feature_family_for_ruleset() {
        let config: LearnConfig = toml::from_str(
            r#"
[rules]
ruleset = "anhoku"

[training]
features = "HalfKAv2^"

[data]
train_games = 1
validation_games = 1
"#,
        )
        .unwrap();
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("training.features=`HalfKAv2^`"));
        assert!(err.contains(FEATURE_SET_DONOR_SINGLE));
    }

    #[test]
    fn explicit_opening_override_applies_to_non_handicap_rulesets() {
        let config: LearnConfig = toml::from_str(
            r#"
[rules]
ruleset = "anhoku"
opening_sfen = "4k4/9/9/9/9/9/9/9/4K4 b - 1"

[data]
train_games = 1
validation_games = 1
"#,
        )
        .unwrap();
        let loaded = LoadedConfig {
            path: PathBuf::from("/tmp/haitaka_learn.toml"),
            hash_hex: "hash".to_string(),
            config,
        };

        assert_eq!(
            loaded.config.rules.opening_sfen.as_deref(),
            Some("4k4/9/9/9/9/9/9/9/4K4 b - 1")
        );
        if cfg!(feature = "anhoku") {
            assert_eq!(
                loaded.opening_sfen().unwrap(),
                "4k4/9/9/9/9/9/9/9/4K4 b - 1"
            );
        }
    }

    #[test]
    fn effective_rule_id_uses_registry_defaults() {
        for (ruleset, expected) in [
            (Ruleset::Standard, 0),
            (Ruleset::Annan, 26),
            (Ruleset::Anhoku, 55),
            (Ruleset::Antouzai, 95),
            (Ruleset::Taimen, 72),
            (Ruleset::Haimen, 74),
            (Ruleset::Tenkyo, 151),
            (Ruleset::Tenjiku, 56),
            (Ruleset::Anki, 94),
        ] {
            let loaded = LoadedConfig {
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
            };
            assert_eq!(loaded.effective_rule_id().unwrap(), expected);
        }
    }

    #[test]
    fn custom_handicap_opening_requires_explicit_rule_id() {
        let loaded = LoadedConfig {
            path: PathBuf::from("/tmp/haitaka_learn.toml"),
            hash_hex: "hash".to_string(),
            config: LearnConfig {
                rules: RulesConfig {
                    ruleset: Ruleset::Handicap,
                    rule_id: None,
                    handicap: None,
                    opening_sfen: Some("4k4/9/9/9/9/9/9/9/4K4 b - 1".to_string()),
                },
                paths: PathsConfig::default(),
                data: DataConfig::default(),
                training: TrainingConfig::default(),
                export: ExportConfig::default(),
                verify: VerifyConfig::default(),
                selection: SelectionConfig::default(),
            },
        };

        let err = loaded.effective_rule_id().unwrap_err().to_string();
        assert!(err.contains("rules.rule_id must be set"));
    }

    #[test]
    fn loaded_config_canonicalizes_relative_path() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("haitaka_learn.toml");
        fs::write(
            &config_path,
            r#"
[rules]
ruleset = "standard"

[paths]
output_dir = "out"

[data]
train_games = 1
validation_games = 1
"#,
        )
        .unwrap();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp.path()).unwrap();
        let loaded = LoadedConfig::from_path(Path::new("haitaka_learn.toml")).unwrap();
        std::env::set_current_dir(original_dir).unwrap();

        assert!(loaded.path.is_absolute());
        assert_eq!(loaded.path, config_path.canonicalize().unwrap());
    }
}

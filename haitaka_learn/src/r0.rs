use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{
    ArtifactPaths, EvaluatorKind, InitialCheckpoint, LoadedGenerationConfig, LoadedTrainingConfig,
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

impl ArtifactRef {
    fn from_path(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        })
    }

    fn validate(&self, root: &Path) -> Result<()> {
        ensure_sha256(&self.sha256, "artifact sha256")?;
        let path = root.join(&self.path);
        let bytes = fs::read(&path)
            .with_context(|| format!("failed to read frozen artifact {}", path.display()))?;
        ensure!(
            bytes.len() as u64 == self.bytes,
            "frozen artifact {} has {} bytes, expected {}",
            path.display(),
            bytes.len(),
            self.bytes
        );
        let actual = format!("{:x}", Sha256::digest(&bytes));
        ensure!(
            actual == self.sha256,
            "frozen artifact {} has sha256 {}, expected {}",
            path.display(),
            actual,
            self.sha256
        );
        Ok(())
    }
}

pub fn write_generation_manifest(
    loaded: &LoadedGenerationConfig,
    artifacts: &ArtifactPaths,
) -> Result<PathBuf> {
    let evaluator = |kind: EvaluatorKind,
                     model: Option<&Path>,
                     model_sha256: Option<&str>,
                     search: String|
     -> Result<EvaluatorIdentity> {
        let model = match (kind, model) {
            (EvaluatorKind::Handcrafted, None) => None,
            (EvaluatorKind::Nnue, Some(path)) => {
                let resolved = loaded.resolve_path(path);
                let artifact = ArtifactRef::from_path(&resolved)?;
                ensure!(
                    model_sha256 == Some(artifact.sha256.as_str()),
                    "evaluator model hash changed before manifest publication"
                );
                Some(artifact)
            }
            _ => bail!("invalid evaluator model identity"),
        };
        Ok(EvaluatorIdentity {
            kind: match kind {
                EvaluatorKind::Handcrafted => "handcrafted",
                EvaluatorKind::Nnue => "nnue",
            }
            .to_string(),
            model,
            search,
        })
    };
    let trajectory = loaded.trajectory_evaluator();
    let label = loaded.label_evaluator();
    let manifest = CombinedGenerationManifestV1 {
        schema: "haitaka-combined-generation-v1".to_string(),
        executable: ArtifactRef::from_path(&std::env::current_exe()?)?,
        config_source_sha256: loaded.source_hash_hex.clone(),
        config_canonical_sha256: loaded.hash_hex.clone(),
        trajectory_evaluator: evaluator(
            trajectory.evaluator.kind,
            trajectory.evaluator.model.as_deref(),
            trajectory.evaluator.model_sha256.as_deref(),
            format!("depth={}", trajectory.search_depth),
        )?,
        label_evaluator: evaluator(
            label.evaluator.kind,
            label.evaluator.model.as_deref(),
            label.evaluator.model_sha256.as_deref(),
            match label.search_budget()? {
                crate::config::LabelSearchBudget::Depth { depth } => format!("depth={depth}"),
                crate::config::LabelSearchBudget::Nodes { nodes, max_depth } => {
                    format!("nodes={nodes},max-depth={max_depth}")
                }
            },
        )?,
        label_target_semantics: label.target_semantics.clone(),
        score_transform_version: label.score_transform_version.clone(),
        record_format: loaded.record_format().to_string(),
        seeds: vec![
            loaded.config.data.seed,
            loaded.config.data.split_seed,
            loaded.config.data.shuffle_seed,
        ],
        output_shards: [
            &artifacts.train_bin,
            &artifacts.validation_bin,
            &artifacts.train_manifest,
            &artifacts.validation_manifest,
        ]
        .into_iter()
        .map(|path| ArtifactRef::from_path(path))
        .collect::<Result<Vec<_>>>()?,
    };
    let path = artifacts
        .artifacts_dir
        .join("combined-generation-stage-v1.json");
    fs::write(&path, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

pub fn write_training_manifest(
    loaded: &LoadedTrainingConfig,
    checkpoint: &Path,
    export: Option<&Path>,
) -> Result<PathBuf> {
    let artifacts = loaded.artifact_paths();
    let initialization = match &loaded.config.training.initial_checkpoint {
        InitialCheckpoint::Scratch => "scratch".to_string(),
        InitialCheckpoint::FullPrecision { state_policy, .. } => {
            format!("full-precision:{state_policy:?}")
        }
        InitialCheckpoint::QuantizedImportDiagnostic {
            import_transform_version,
            ..
        } => format!("quantized-import-diagnostic:{import_transform_version}"),
    };
    let trainer = loaded.trainer_checkout()?.join("train.py");
    let manifest = TrainingManifestV1 {
        schema: "haitaka-training-stage-v1".to_string(),
        trainer_executable: ArtifactRef::from_path(&trainer)?,
        config_source_sha256: loaded.source_hash_hex.clone(),
        config_canonical_sha256: loaded.canonical_hash_hex.clone(),
        input_dataset_manifests: [&artifacts.train_manifest, &artifacts.validation_manifest]
            .into_iter()
            .map(|path| ArtifactRef::from_path(path))
            .collect::<Result<Vec<_>>>()?,
        initialization,
        optimizer_state: "checkpoint-defined; R2 will require full deterministic state".to_string(),
        checkpoints: vec![ArtifactRef::from_path(checkpoint)?],
        export: export.map(ArtifactRef::from_path).transpose()?,
    };
    let path = artifacts.artifacts_dir.join("training-stage-v1.json");
    fs::write(&path, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestRef {
    pub stage: StageKind,
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StageKind {
    CombinedGeneration,
    Training,
    Evaluation,
    Verification,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorIdentity {
    pub kind: String,
    pub model: Option<ArtifactRef>,
    pub search: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CombinedGenerationManifestV1 {
    pub schema: String,
    pub executable: ArtifactRef,
    pub config_source_sha256: String,
    pub config_canonical_sha256: String,
    pub trajectory_evaluator: EvaluatorIdentity,
    pub label_evaluator: EvaluatorIdentity,
    pub label_target_semantics: String,
    pub score_transform_version: String,
    pub record_format: String,
    pub seeds: Vec<u64>,
    pub output_shards: Vec<ArtifactRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingManifestV1 {
    pub schema: String,
    pub trainer_executable: ArtifactRef,
    pub config_source_sha256: String,
    pub config_canonical_sha256: String,
    pub input_dataset_manifests: Vec<ArtifactRef>,
    pub initialization: String,
    pub optimizer_state: String,
    pub checkpoints: Vec<ArtifactRef>,
    pub export: Option<ArtifactRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // Emitted by the evaluation harness introduced after the R0 boundary.
pub struct EvaluationManifestV1 {
    pub schema: String,
    pub model: ArtifactRef,
    pub harness: ArtifactRef,
    pub openings: ArtifactRef,
    pub execution_environment: ExecutionEnvironment,
    pub raw_results: Vec<ArtifactRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub struct ExecutionEnvironment {
    pub resolved_threads: u32,
    pub cpu: String,
    pub affinity: String,
    pub compiler: String,
    pub cold_warm_protocol: String,
    pub concurrency: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct R0IndependenceVerificationManifestV1 {
    pub schema: String,
    pub generation_config: ArtifactRef,
    pub test_source: ArtifactRef,
    pub test_name: String,
    pub command: String,
    pub trajectory_evaluator_identity: String,
    pub label_evaluator_identity: String,
    pub training_initialization_identities: Vec<String>,
    pub assertions: Vec<String>,
    pub result: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GateRegistration {
    pub metric: String,
    pub direction: MetricDirection,
    pub baseline: String,
    pub minimum_effect_or_margin: f64,
    pub uncertainty_rule: String,
    pub multiplicity_handling: String,
    pub decision_rule: String,
    pub cost_ceiling: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MetricDirection {
    HigherIsBetter,
    LowerIsBetter,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentPreregistration {
    pub id: String,
    pub registered_at: String,
    pub hypothesis: String,
    pub changed_variable: String,
    pub controls: Vec<String>,
    pub config_stage: StageKind,
    pub config: ArtifactRef,
    pub gate: GateRegistration,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentOutcome {
    pub preregistration_id: String,
    pub recorded_at: String,
    pub stage_manifests: Vec<ManifestRef>,
    pub trajectory_evaluator_identity: String,
    pub label_evaluator_identity: String,
    pub initialization_identity: String,
    pub outcome: String,
    pub proves: Vec<String>,
    pub does_not_prove: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentRegistryV1 {
    pub schema: String,
    pub preregistrations: Vec<ExperimentPreregistration>,
    pub outcomes: Vec<ExperimentOutcome>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalEvidenceV1 {
    pub schema: String,
    pub records: Vec<HistoricalEvidenceRecord>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalEvidenceRecord {
    pub id: String,
    pub artifacts: Vec<ArtifactRef>,
    pub valid_claims: Vec<String>,
    pub invalid_claims: Vec<String>,
    pub classification: EvidenceClassification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContaminationKind {
    NoMoveLossPath,
    EmergencyFallback,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalContaminationCount {
    pub id: String,
    pub kind: ContaminationKind,
    pub source_artifacts: Vec<ArtifactRef>,
    pub affected_games: u64,
    pub total_games: u64,
    pub fallback_moves: Option<u64>,
    pub counting_rule: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalContaminationReportV1 {
    pub schema: String,
    pub counts: Vec<HistoricalContaminationCount>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct R1FixtureSpecificationsV1 {
    pub schema: String,
    pub deterministic_parity_corpus: R1ParityCorpusSpec,
    pub sentinel_network: R1SentinelNetworkSpec,
    pub interruption_safe_search: R1InterruptionSafeSearchSpec,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct R1ParityCorpusSpec {
    pub schema: String,
    pub deterministic_order: String,
    pub required_position_classes: Vec<String>,
    pub required_move_transitions: Vec<String>,
    pub required_oracles: Vec<String>,
    pub pass_rule: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct R1SentinelNetworkSpec {
    pub schema: String,
    pub construction: String,
    pub required_patterns: Vec<String>,
    pub required_exports: Vec<String>,
    pub pass_rule: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct R1InterruptionSafeSearchSpec {
    pub schema: String,
    pub required_cases: Vec<String>,
    pub required_result_fields: Vec<String>,
    pub pass_rule: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceClassification {
    HistoricalOnly,
    DiagnosticOnly,
    NonDecisional,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionExecutionSpecV1 {
    pub schema: String,
    pub interface: String,
    pub complete_search_path: String,
    pub concurrent_games: u32,
    pub supported_host_classes: Vec<String>,
    pub supported_device_classes: Vec<String>,
    pub clock_policy: String,
    pub cold_warm_protocol: String,
    pub target_move_time_ms: u64,
    pub max_serialized_bytes: u64,
    pub max_peak_memory_bytes: u64,
    pub max_load_time_ms: u64,
    pub max_per_move_latency_ms: u64,
}

pub fn validate_bundle(bundle_dir: &Path, workspace_root: &Path) -> Result<()> {
    let history: HistoricalEvidenceV1 = read_json(&bundle_dir.join("historical-evidence.json"))?;
    ensure!(
        history.schema == "haitaka-historical-evidence-v1",
        "wrong history schema"
    );
    ensure!(!history.records.is_empty(), "historical evidence is empty");
    for record in &history.records {
        ensure!(
            !record.valid_claims.is_empty(),
            "{} has no valid claims",
            record.id
        );
        ensure!(
            !record.invalid_claims.is_empty(),
            "{} has no invalid claims",
            record.id
        );
        for artifact in &record.artifacts {
            artifact.validate(workspace_root)?;
        }
    }

    let registry: ExperimentRegistryV1 = read_json(&bundle_dir.join("experiment-registry.json"))?;
    validate_registry(&registry, workspace_root)?;

    let contamination: HistoricalContaminationReportV1 =
        read_json(&bundle_dir.join("historical-contamination-counts.json"))?;
    validate_contamination_report(&contamination, workspace_root)?;

    let r1_fixtures: R1FixtureSpecificationsV1 =
        read_json(&bundle_dir.join("r1-fixture-specifications.json"))?;
    validate_r1_fixture_specifications(&r1_fixtures)?;

    let spec: ProductionExecutionSpecV1 =
        read_json(&bundle_dir.join("production-execution-spec.json"))?;
    validate_production_spec(&spec)?;
    Ok(())
}

fn validate_registry(registry: &ExperimentRegistryV1, root: &Path) -> Result<()> {
    ensure!(
        registry.schema == "haitaka-experiment-registry-v1",
        "wrong registry schema"
    );
    ensure!(
        !registry.preregistrations.is_empty(),
        "experiment registry has no preregistrations"
    );
    ensure!(
        !registry.outcomes.is_empty(),
        "experiment registry has no outcomes"
    );
    ensure!(
        registry
            .outcomes
            .iter()
            .all(|outcome| !outcome.stage_manifests.is_empty()),
        "experiment registry has an outcome without stage-manifest links"
    );
    let mut ids = BTreeSet::new();
    for prereg in &registry.preregistrations {
        ensure!(
            ids.insert(&prereg.id),
            "duplicate preregistration {}",
            prereg.id
        );
        prereg.config.validate(root)?;
        if prereg.config_stage == StageKind::CombinedGeneration {
            let config_path = root.join(&prereg.config.path);
            let loaded = LoadedGenerationConfig::from_path(&config_path).with_context(|| {
                format!(
                    "preregistration {} does not reference a strict generation config",
                    prereg.id
                )
            })?;
            ensure!(
                loaded.source_hash_hex == prereg.config.sha256,
                "preregistration {} config hash is not bound to {}",
                prereg.id,
                prereg.config.path.display()
            );
        }
        ensure!(
            !prereg.gate.metric.trim().is_empty()
                && !prereg.gate.baseline.trim().is_empty()
                && !prereg.gate.uncertainty_rule.trim().is_empty()
                && !prereg.gate.multiplicity_handling.trim().is_empty()
                && !prereg.gate.decision_rule.trim().is_empty()
                && !prereg.gate.cost_ceiling.trim().is_empty(),
            "preregistration {} has an incomplete machine-decisive gate",
            prereg.id
        );
    }
    let mut outcome_ids = BTreeSet::new();
    for outcome in &registry.outcomes {
        ensure!(
            outcome_ids.insert(&outcome.preregistration_id),
            "duplicate outcome for preregistration {}",
            outcome.preregistration_id
        );
        ensure!(
            ids.contains(&outcome.preregistration_id),
            "outcome references missing preregistration {}",
            outcome.preregistration_id
        );
        let preregistration = registry
            .preregistrations
            .iter()
            .find(|prereg| prereg.id == outcome.preregistration_id)
            .expect("membership was checked above");
        ensure!(
            !outcome.proves.is_empty() && !outcome.does_not_prove.is_empty(),
            "outcome {} has incomplete attribution",
            outcome.preregistration_id
        );
        ensure!(
            !outcome.trajectory_evaluator_identity.trim().is_empty()
                && !outcome.label_evaluator_identity.trim().is_empty()
                && !outcome.initialization_identity.trim().is_empty(),
            "outcome {} does not link trajectory evaluator, label evaluator, and initialization independently",
            outcome.preregistration_id
        );
        ensure!(
            !outcome.stage_manifests.is_empty(),
            "outcome {} has no stage-manifest links",
            outcome.preregistration_id
        );
        let mut independently_linked = false;
        for manifest in &outcome.stage_manifests {
            ensure_sha256(&manifest.sha256, "stage manifest sha256")?;
            let bytes = fs::read(root.join(&manifest.path)).with_context(|| {
                format!("failed to read stage manifest {}", manifest.path.display())
            })?;
            ensure!(
                format!("{:x}", Sha256::digest(&bytes)) == manifest.sha256,
                "stage manifest hash mismatch for {}",
                manifest.path.display()
            );
            match manifest.stage {
                StageKind::CombinedGeneration => {
                    validate_combined_generation_manifest(
                        &serde_json::from_slice(&bytes).with_context(|| {
                            format!("failed to parse {}", manifest.path.display())
                        })?,
                        root,
                    )?;
                }
                StageKind::Training => {
                    validate_training_manifest(
                        &serde_json::from_slice(&bytes).with_context(|| {
                            format!("failed to parse {}", manifest.path.display())
                        })?,
                        root,
                    )?;
                }
                StageKind::Evaluation => {
                    validate_evaluation_manifest(
                        &serde_json::from_slice(&bytes).with_context(|| {
                            format!("failed to parse {}", manifest.path.display())
                        })?,
                        root,
                    )?;
                }
                StageKind::Verification => {
                    let verification: R0IndependenceVerificationManifestV1 =
                        serde_json::from_slice(&bytes).with_context(|| {
                            format!("failed to parse {}", manifest.path.display())
                        })?;
                    validate_r0_verification_manifest(
                        &verification,
                        preregistration,
                        outcome,
                        root,
                    )?;
                    independently_linked = true;
                }
            }
        }
        ensure!(
            independently_linked,
            "outcome {} lacks a manifest that independently links both evaluators and initialization",
            outcome.preregistration_id
        );
    }
    ensure!(
        outcome_ids == ids,
        "every preregistration must have exactly one outcome"
    );
    Ok(())
}

fn validate_combined_generation_manifest(
    manifest: &CombinedGenerationManifestV1,
    root: &Path,
) -> Result<()> {
    ensure!(
        manifest.schema == "haitaka-combined-generation-v1",
        "wrong combined-generation manifest schema"
    );
    manifest.executable.validate(root)?;
    ensure_sha256(
        &manifest.config_source_sha256,
        "generation config source sha256",
    )?;
    ensure_sha256(
        &manifest.config_canonical_sha256,
        "generation config canonical sha256",
    )?;
    for evaluator in [&manifest.trajectory_evaluator, &manifest.label_evaluator] {
        ensure!(
            !evaluator.kind.trim().is_empty(),
            "evaluator kind is missing"
        );
        ensure!(
            !evaluator.search.trim().is_empty(),
            "evaluator search is missing"
        );
        if let Some(model) = &evaluator.model {
            model.validate(root)?;
        }
    }
    ensure!(
        !manifest.output_shards.is_empty(),
        "generation manifest has no output shards"
    );
    for artifact in &manifest.output_shards {
        artifact.validate(root)?;
    }
    Ok(())
}

fn validate_training_manifest(manifest: &TrainingManifestV1, root: &Path) -> Result<()> {
    ensure!(
        manifest.schema == "haitaka-training-stage-v1",
        "wrong training manifest schema"
    );
    manifest.trainer_executable.validate(root)?;
    ensure!(
        !manifest.input_dataset_manifests.is_empty(),
        "training manifest has no inputs"
    );
    ensure!(
        !manifest.checkpoints.is_empty(),
        "training manifest has no checkpoints"
    );
    for artifact in manifest
        .input_dataset_manifests
        .iter()
        .chain(manifest.checkpoints.iter())
        .chain(manifest.export.iter())
    {
        artifact.validate(root)?;
    }
    Ok(())
}

fn validate_evaluation_manifest(manifest: &EvaluationManifestV1, root: &Path) -> Result<()> {
    ensure!(
        manifest.schema == "haitaka-evaluation-stage-v1",
        "wrong evaluation manifest schema"
    );
    manifest.model.validate(root)?;
    manifest.harness.validate(root)?;
    manifest.openings.validate(root)?;
    ensure!(
        manifest.execution_environment.concurrency == 1,
        "evaluation concurrency must be one"
    );
    ensure!(
        !manifest.raw_results.is_empty(),
        "evaluation manifest has no raw results"
    );
    for artifact in &manifest.raw_results {
        artifact.validate(root)?;
    }
    Ok(())
}

fn validate_r0_verification_manifest(
    manifest: &R0IndependenceVerificationManifestV1,
    preregistration: &ExperimentPreregistration,
    outcome: &ExperimentOutcome,
    root: &Path,
) -> Result<()> {
    ensure!(
        manifest.schema == "haitaka-r0-independence-verification-v1",
        "wrong R0 verification manifest schema"
    );
    manifest.generation_config.validate(root)?;
    manifest.test_source.validate(root)?;
    ensure!(
        preregistration.config_stage == StageKind::CombinedGeneration
            && preregistration.config == manifest.generation_config,
        "R0 verification generation config does not match its preregistration"
    );
    let loaded = LoadedGenerationConfig::from_path(&root.join(&manifest.generation_config.path))?;
    ensure!(
        loaded.source_hash_hex == manifest.generation_config.sha256,
        "R0 verification generation config hash is not bound to its file"
    );
    ensure!(
        manifest.training_initialization_identities.len() >= 2,
        "R0 verification must compare at least two training initializations"
    );
    ensure!(
        !manifest.assertions.is_empty(),
        "R0 verification has no assertions"
    );
    ensure!(manifest.result == "pass", "R0 verification did not pass");
    ensure!(
        manifest.trajectory_evaluator_identity == outcome.trajectory_evaluator_identity
            && manifest.label_evaluator_identity == outcome.label_evaluator_identity
            && manifest.training_initialization_identities.join(" versus ")
                == outcome.initialization_identity,
        "R0 outcome identities do not match its verification manifest"
    );
    Ok(())
}

fn validate_contamination_report(
    report: &HistoricalContaminationReportV1,
    root: &Path,
) -> Result<()> {
    ensure!(
        report.schema == "haitaka-historical-contamination-v1",
        "wrong historical contamination schema"
    );
    ensure!(
        !report.counts.is_empty(),
        "historical contamination report is empty"
    );
    let expected = [
        (
            "phase8d-candidate-vs-handcrafted",
            ContaminationKind::NoMoveLossPath,
            330,
            1_024,
            None,
        ),
        (
            "phase8d-c16-retention",
            ContaminationKind::NoMoveLossPath,
            381,
            1_024,
            None,
        ),
        (
            "phase8d-phase8b-retention",
            ContaminationKind::NoMoveLossPath,
            1_771,
            4_096,
            None,
        ),
        (
            "phase8d-anchored-checkpoint-ranking",
            ContaminationKind::NoMoveLossPath,
            1_795,
            4_096,
            None,
        ),
        (
            "phase11-v2-vs-v1",
            ContaminationKind::NoMoveLossPath,
            1_822,
            4_096,
            None,
        ),
        (
            "phase8r-equal-node-fallbacks",
            ContaminationKind::EmergencyFallback,
            42,
            2_048,
            Some(342),
        ),
    ];
    ensure!(
        report.counts.len() == expected.len(),
        "historical contamination report must contain every frozen affected run"
    );
    for (id, kind, affected_games, total_games, fallback_moves) in expected {
        let count = report
            .counts
            .iter()
            .find(|count| count.id == id)
            .with_context(|| format!("historical contamination report is missing {id}"))?;
        ensure!(
            count.kind == kind
                && count.affected_games == affected_games
                && count.total_games == total_games
                && count.fallback_moves == fallback_moves,
            "{id} does not match the frozen contamination count"
        );
    }
    let mut ids = BTreeSet::new();
    for count in &report.counts {
        ensure!(
            ids.insert(&count.id),
            "duplicate contamination count {}",
            count.id
        );
        ensure!(
            !count.source_artifacts.is_empty(),
            "{} has no source artifacts",
            count.id
        );
        let mut total_games = 0_u64;
        let mut affected_games = 0_u64;
        let mut fallback_moves = 0_u64;
        for artifact in &count.source_artifacts {
            artifact.validate(root)?;
            let file = fs::File::open(root.join(&artifact.path))?;
            for line in BufReader::new(file).lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let game: serde_json::Value = serde_json::from_str(&line)?;
                total_games += 1;
                match count.kind {
                    ContaminationKind::NoMoveLossPath => {
                        let plies = game["plies"].as_u64().context("game is missing plies")?;
                        let incomplete = game["incompleteIterations"]
                            .as_u64()
                            .context("game is missing incompleteIterations")?;
                        affected_games += u64::from(incomplete == plies + 1);
                    }
                    ContaminationKind::EmergencyFallback => {
                        let fallbacks = game["fallbacks"]
                            .as_u64()
                            .context("game is missing fallbacks")?;
                        affected_games += u64::from(fallbacks > 0);
                        fallback_moves += fallbacks;
                    }
                }
            }
        }
        ensure!(
            total_games == count.total_games && affected_games == count.affected_games,
            "{} contamination count changed: observed {affected_games}/{total_games}, expected {}/{}",
            count.id,
            count.affected_games,
            count.total_games
        );
        match count.kind {
            ContaminationKind::NoMoveLossPath => ensure!(
                count.counting_rule == "incompleteIterations == plies + 1"
                    && count.fallback_moves.is_none(),
                "{} has the wrong no-move counting contract",
                count.id
            ),
            ContaminationKind::EmergencyFallback => ensure!(
                count.counting_rule == "fallbacks > 0; fallback_moves = sum(fallbacks)"
                    && count.fallback_moves == Some(fallback_moves),
                "{} fallback count changed: observed {fallback_moves}",
                count.id
            ),
        }
    }
    Ok(())
}

fn validate_r1_fixture_specifications(spec: &R1FixtureSpecificationsV1) -> Result<()> {
    ensure!(
        spec.schema == "haitaka-r1-fixture-specifications-v1",
        "wrong R1 fixture specification schema"
    );
    ensure!(
        spec.deterministic_parity_corpus.schema == "haitaka-r1-parity-corpus-v1",
        "wrong R1 parity corpus schema"
    );
    ensure!(
        spec.sentinel_network.schema == "haitaka-r1-sentinel-network-v1",
        "wrong R1 sentinel network schema"
    );
    ensure!(
        spec.interruption_safe_search.schema == "haitaka-r1-interruption-safe-search-v1",
        "wrong R1 interruption schema"
    );
    ensure!(
        !spec
            .deterministic_parity_corpus
            .required_position_classes
            .is_empty()
            && !spec
                .deterministic_parity_corpus
                .required_move_transitions
                .is_empty()
            && !spec.deterministic_parity_corpus.required_oracles.is_empty()
            && !spec.sentinel_network.required_patterns.is_empty()
            && !spec.sentinel_network.required_exports.is_empty()
            && !spec.interruption_safe_search.required_cases.is_empty()
            && !spec
                .interruption_safe_search
                .required_result_fields
                .is_empty(),
        "R1 fixture specifications are incomplete"
    );
    for (value, label) in [
        (
            spec.deterministic_parity_corpus
                .deterministic_order
                .as_str(),
            "R1 parity deterministic order",
        ),
        (
            spec.deterministic_parity_corpus.pass_rule.as_str(),
            "R1 parity pass rule",
        ),
        (
            spec.sentinel_network.construction.as_str(),
            "R1 sentinel construction",
        ),
        (
            spec.sentinel_network.pass_rule.as_str(),
            "R1 sentinel pass rule",
        ),
        (
            spec.interruption_safe_search.pass_rule.as_str(),
            "R1 interruption pass rule",
        ),
    ] {
        ensure_resolved_policy(value, label)?;
    }
    for (values, label) in [
        (
            spec.deterministic_parity_corpus
                .required_position_classes
                .as_slice(),
            "R1 position class",
        ),
        (
            spec.deterministic_parity_corpus
                .required_move_transitions
                .as_slice(),
            "R1 move transition",
        ),
        (
            spec.deterministic_parity_corpus.required_oracles.as_slice(),
            "R1 parity oracle",
        ),
        (
            spec.sentinel_network.required_patterns.as_slice(),
            "R1 sentinel pattern",
        ),
        (
            spec.sentinel_network.required_exports.as_slice(),
            "R1 sentinel export",
        ),
        (
            spec.interruption_safe_search.required_cases.as_slice(),
            "R1 interruption case",
        ),
        (
            spec.interruption_safe_search
                .required_result_fields
                .as_slice(),
            "R1 interruption result field",
        ),
    ] {
        for value in values {
            ensure_resolved_policy(value, label)?;
        }
    }
    Ok(())
}

fn validate_production_spec(spec: &ProductionExecutionSpecV1) -> Result<()> {
    ensure!(
        spec.schema == "haitaka-production-execution-v1",
        "wrong production spec schema"
    );
    ensure!(
        spec.concurrent_games == 1,
        "production concurrency must be one game"
    );
    ensure_resolved_policy(&spec.interface, "production interface")?;
    ensure_resolved_policy(&spec.complete_search_path, "complete search path")?;
    ensure_resolved_policy(&spec.clock_policy, "clock policy")?;
    ensure_resolved_policy(&spec.cold_warm_protocol, "cold/warm protocol")?;
    ensure!(
        !spec.supported_host_classes.is_empty(),
        "supported host classes are missing"
    );
    ensure!(
        !spec.supported_device_classes.is_empty(),
        "supported device classes are missing"
    );
    for host in &spec.supported_host_classes {
        ensure_resolved_policy(host, "supported host class")?;
    }
    for device in &spec.supported_device_classes {
        ensure_resolved_policy(device, "supported device class")?;
    }
    ensure!(
        spec.target_move_time_ms > 0
            && spec.max_serialized_bytes > 0
            && spec.max_peak_memory_bytes > 0
            && spec.max_load_time_ms > 0
            && spec.max_per_move_latency_ms > 0,
        "all production numerical ceilings must be positive"
    );
    ensure!(
        spec.max_per_move_latency_ms >= spec.target_move_time_ms,
        "per-move latency ceiling cannot be below the target move time"
    );
    Ok(())
}

fn ensure_resolved_policy(value: &str, label: &str) -> Result<()> {
    let value = value.trim();
    ensure!(!value.is_empty(), "{label} is missing");
    ensure!(
        !value.to_ascii_uppercase().contains("PENDING"),
        "{label} still contains a pending product decision"
    );
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    serde_json::from_slice(
        &fs::read(path).with_context(|| format!("failed to read {}", path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", path.display()))
}

fn ensure_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        bail!("{label} must be 64 lowercase hexadecimal characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_manifests_reject_cross_stage_fields() {
        let raw = r#"{
            "schema":"haitaka-combined-generation-v1",
            "executable":{"path":"x","bytes":1,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "config_source_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "config_canonical_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "trajectory_evaluator":{"kind":"handcrafted","model":null,"search":"depth=1"},
            "label_evaluator":{"kind":"handcrafted","model":null,"search":"nodes=50000"},
            "label_target_semantics":"root-backed-up-v1",
            "score_transform_version":"raw-score-v1",
            "record_format":"haitaka-packed-training-record-v3-72-byte",
            "seeds":[1],
            "output_shards":[],
            "initialization":"scratch"
        }"#;
        let error = serde_json::from_str::<CombinedGenerationManifestV1>(raw).unwrap_err();
        assert!(error.to_string().contains("unknown field `initialization`"));
    }

    #[test]
    fn registry_rejects_an_empty_preregistration_set() {
        let registry = ExperimentRegistryV1 {
            schema: "haitaka-experiment-registry-v1".to_string(),
            preregistrations: Vec::new(),
            outcomes: vec![ExperimentOutcome {
                preregistration_id: "missing".to_string(),
                recorded_at: "2026-09-01T00:00:00+09:00".to_string(),
                stage_manifests: Vec::new(),
                trajectory_evaluator_identity: "handcrafted/depth1".to_string(),
                label_evaluator_identity: "handcrafted/nodes50000".to_string(),
                initialization_identity: "scratch".to_string(),
                outcome: "unknown".to_string(),
                proves: vec!["nothing".to_string()],
                does_not_prove: vec!["strength".to_string()],
            }],
        };
        assert!(
            validate_registry(&registry, Path::new("."))
                .unwrap_err()
                .to_string()
                .contains("no preregistrations")
        );
    }

    #[test]
    fn registry_rejects_vacuous_outcomes_and_manifest_links() {
        let preregistration = ExperimentPreregistration {
            id: "r0".to_string(),
            registered_at: "2026-09-01T00:00:00+09:00".to_string(),
            hypothesis: "independence".to_string(),
            changed_variable: "training.initial_checkpoint".to_string(),
            controls: vec!["generation config".to_string()],
            config_stage: StageKind::CombinedGeneration,
            config: ArtifactRef {
                path: PathBuf::from("not-read-before-vacuity-check.toml"),
                bytes: 1,
                sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            },
            gate: GateRegistration {
                metric: "mismatches".to_string(),
                direction: MetricDirection::LowerIsBetter,
                baseline: "scratch".to_string(),
                minimum_effect_or_margin: 0.0,
                uncertainty_rule: "exact".to_string(),
                multiplicity_handling: "all shards".to_string(),
                decision_rule: "zero".to_string(),
                cost_ceiling: "CPU".to_string(),
            },
        };
        let empty = ExperimentRegistryV1 {
            schema: "haitaka-experiment-registry-v1".to_string(),
            preregistrations: vec![preregistration.clone()],
            outcomes: Vec::new(),
        };
        assert!(
            validate_registry(&empty, Path::new("."))
                .unwrap_err()
                .to_string()
                .contains("no outcomes")
        );

        let no_links = ExperimentRegistryV1 {
            schema: "haitaka-experiment-registry-v1".to_string(),
            preregistrations: vec![preregistration],
            outcomes: vec![ExperimentOutcome {
                preregistration_id: "r0".to_string(),
                recorded_at: "2026-09-01T00:00:00+09:00".to_string(),
                stage_manifests: Vec::new(),
                trajectory_evaluator_identity: "handcrafted/depth=1".to_string(),
                label_evaluator_identity: "handcrafted/depth=2".to_string(),
                initialization_identity: "scratch versus diagnostic".to_string(),
                outcome: "pass".to_string(),
                proves: vec!["independence".to_string()],
                does_not_prove: vec!["strength".to_string()],
            }],
        };
        assert!(
            validate_registry(&no_links, Path::new("."))
                .unwrap_err()
                .to_string()
                .contains("without stage-manifest links")
        );
    }

    #[test]
    fn production_spec_rejects_unresolved_policy_placeholders() {
        let spec = ProductionExecutionSpecV1 {
            schema: "haitaka-production-execution-v1".to_string(),
            interface: "PENDING_PRODUCT_DECISION".to_string(),
            complete_search_path: "usi-wasm-v1 position+moves".to_string(),
            concurrent_games: 1,
            supported_host_classes: vec!["desktop-browser".to_string()],
            supported_device_classes: vec!["x86_64".to_string()],
            clock_policy: "monotonic deadline".to_string(),
            cold_warm_protocol: "warm model".to_string(),
            target_move_time_ms: 100,
            max_serialized_bytes: 1,
            max_peak_memory_bytes: 1,
            max_load_time_ms: 1,
            max_per_move_latency_ms: 1,
        };
        assert!(
            validate_production_spec(&spec)
                .unwrap_err()
                .to_string()
                .contains("pending product decision")
        );
    }
}

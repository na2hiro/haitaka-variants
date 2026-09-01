use std::collections::BTreeSet;
use std::fs;
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
    pub config_sha256: String,
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
    let mut ids = BTreeSet::new();
    for prereg in &registry.preregistrations {
        ensure!(
            ids.insert(&prereg.id),
            "duplicate preregistration {}",
            prereg.id
        );
        ensure_sha256(&prereg.config_sha256, "preregistration config_sha256")?;
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
    for outcome in &registry.outcomes {
        ensure!(
            ids.contains(&outcome.preregistration_id),
            "outcome references missing preregistration {}",
            outcome.preregistration_id
        );
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
        spec.max_serialized_bytes > 0
            && spec.max_peak_memory_bytes > 0
            && spec.max_load_time_ms > 0
            && spec.max_per_move_latency_ms > 0,
        "all production numerical ceilings must be positive"
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
    fn registry_rejects_outcome_without_preregistration() {
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
                .contains("missing preregistration")
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

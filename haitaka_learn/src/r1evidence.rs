use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(crate) const SOURCE_SCHEMA: &str = "haitaka-r1-source-identity-v2";
const SOURCE_POLICY_SCHEMA: &str = "haitaka-r1-source-identity-policy-v2";
const AUTHORIZATION_ALLOWED_PATHS: &[&str] = &["plans/anhoku-nnue-training-workflow-reboot.md"];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArtifactIdentity {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Exclusion {
    path: String,
    bytes: u64,
    sha256: String,
    reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PrefixExclusion {
    path_prefix: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourcePolicy {
    schema: String,
    workspace_untracked_exclusions: Vec<Exclusion>,
    external_trainer_untracked_exclusion_prefixes: Vec<PrefixExclusion>,
    authorization_allowed_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryIdentity {
    path: String,
    commit: String,
    tree: String,
    tracked_diff_sha256: String,
    staged_diff_sha256: String,
    tracked_changes: Vec<String>,
    relevant_untracked: Vec<ArtifactIdentity>,
    excluded_untracked: Vec<Exclusion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SourceIdentity {
    schema: String,
    schema_version: u32,
    /// Immutable build/evidence anchor. The eventual authorization commit is
    /// deliberately stored in the closeout, not in this artifact, so writing
    /// the closeout cannot create a hash/commit cycle.
    evidence_source_commit: String,
    evidence_source_tree: String,
    /// The source snapshot's authorization parent. This is equal to the
    /// evidence anchor at production time; a later plan-only commit is
    /// represented by `AuthorizationTransition` in the closeout.
    authorization_commit: String,
    authorization_tree: String,
    build_relevant_manifest_sha256: String,
    authorization_allowed_paths: Vec<String>,
    workspace: RepositoryIdentity,
    cargo_lock: ArtifactIdentity,
    submodule_status: String,
    submodule_status_sha256: String,
    external_trainer: RepositoryIdentity,
    policy: ArtifactIdentity,
    producer_executable: ArtifactIdentity,
    rebuild_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AuthorizationTransition {
    pub schema: String,
    pub schema_version: u32,
    pub evidence_source_commit: String,
    pub evidence_source_tree: String,
    pub authorization_commit: String,
    pub authorization_tree: String,
    pub authorization_parent_commit: String,
    pub authorization_parent_tree: String,
    pub build_relevant_manifest_sha256: String,
    pub allowed_workflow_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct StageSpec {
    pub id: &'static str,
    pub schema: &'static str,
    pub gates: &'static [&'static str],
    pub prior_links: &'static [(&'static str, &'static str)],
    pub frozen_inputs: &'static [(&'static str, &'static str)],
}

const R1A_GATES: &[&str] = &[
    "atLeast10000LegalPositions",
    "canonicalPackingRoundTrip",
    "colorSwapScoreTransform",
    "depthOneReference",
    "incrementalEqualsFullRefresh",
    "independentFeatureSignature",
    "requiredCoverage",
    "rustCppExactFeatureIndices",
    "sideToMoveScoreOrientation",
];
const R1B_GATES: &[&str] = &[
    "absoluteQuantizationLimits",
    "checkpointRegenerationByteIdentical",
    "noAccumulatorOverflow",
    "noSerializerWeightClamping",
    "pythonIntegerEqualsRustFullRefresh",
    "repeatExportByteIdentical",
    "repeatExportMetadataByteIdentical",
    "requiredSentinelPatterns",
    "rustFullRefreshEqualsIncremental",
];
const R1C_GATES: &[&str] = &[
    "identityCollisionDiagnostic",
    "pythonIntegerEqualsRustRuntime",
    "pythonLearnabilityOracle",
    "r1aReportPassing",
    "r1bReportPassing",
    "rustFullRefreshEqualsIncremental",
];
const R1D1_GATES: &[&str] = &[
    "adapterSemanticsAgree",
    "combinedNodeAccountingExact",
    "contractAndFixturesFrozen",
    "everyNonterminalMoveLegal",
    "exactFixtureAssertions",
    "partialValuesExcludedFromLabels",
    "priorReportsPassing",
    "qsearchTelemetryComplete",
];
const R1D2_GATES: &[&str] = &[
    "alphaBetaAndQsearchAgree",
    "contractFrozenAndValid",
    "dfpnHistorySemanticsAgree",
    "dfpnInterruptionAndReservation",
    "enteringKingPolicyFrozen",
    "goldenHistoriesExact",
    "priorR1d1ReportPassing",
    "terminationPrecedenceAndDistinction",
    "ttHistoryContextSafe",
    "usiAndInProcessAgree",
];
const R1D3_GATES: &[&str] = &[
    "aaZeroBiasEquivalent",
    "abOrderReversalEquivalent",
    "cleanStrengthSourceIdentity",
    "completePairsAndBinsExact",
    "contractFrozenAndValid",
    "explicitLegalTerminations",
    "oneGameProductionConcurrency",
    "priorR1ReportsPassing",
    "productionNativeEquivalence",
    "productionTimingQualified",
    "zeroMissingMoves",
    "zeroUnsearchedEmergencyFallbacks",
];

pub(crate) const R1A: StageSpec = StageSpec {
    id: "r1a",
    schema: "haitaka-anhoku-r1a-gate",
    gates: R1A_GATES,
    prior_links: &[],
    frozen_inputs: &[("config", "haitaka_learn.anhoku-reboot-r1a.training.toml")],
};
pub(crate) const R1B: StageSpec = StageSpec {
    id: "r1b",
    schema: "haitaka-anhoku-r1b-gate",
    gates: R1B_GATES,
    prior_links: &[("r1aReport", "r1a")],
    frozen_inputs: &[(
        "frozenLimits",
        "r0/anhoku-reboot/r1b-quantization-limits.json",
    )],
};
pub(crate) const R1C: StageSpec = StageSpec {
    id: "r1c",
    schema: "haitaka-anhoku-r1c-gate",
    gates: R1C_GATES,
    prior_links: &[("r1aReport", "r1a"), ("r1bReport", "r1b")],
    frozen_inputs: &[
        (
            "contract",
            "r0/anhoku-reboot/r1c-learnability-contract.json",
        ),
        (
            "frozenQuantizationLimits",
            "r0/anhoku-reboot/r1b-quantization-limits.json",
        ),
    ],
};
pub(crate) const R1D1: StageSpec = StageSpec {
    id: "r1d1",
    schema: "haitaka-anhoku-r1d1-gate",
    gates: R1D1_GATES,
    prior_links: &[
        ("r1aReport", "r1a"),
        ("r1bReport", "r1b"),
        ("r1cReport", "r1c"),
    ],
    frozen_inputs: &[
        ("contract", "r0/anhoku-reboot/r1d1-search-contract.json"),
        (
            "fixtures",
            "r0/anhoku-reboot/r1d1-forced-interruption-fixtures.json",
        ),
    ],
};
pub(crate) const R1D2: StageSpec = StageSpec {
    id: "r1d2",
    schema: "haitaka-anhoku-r1d2-gate",
    gates: R1D2_GATES,
    prior_links: &[("r1d1Report", "r1d1")],
    frozen_inputs: &[("contract", "r0/anhoku-reboot/r1d2-history-contract.json")],
};
pub(crate) const R1D3: StageSpec = StageSpec {
    id: "r1d3",
    schema: "haitaka-anhoku-r1d3-gate",
    gates: R1D3_GATES,
    prior_links: &[
        ("r1aReport", "r1a"),
        ("r1bReport", "r1b"),
        ("r1cReport", "r1c"),
        ("r1d1Report", "r1d1"),
        ("r1d2Report", "r1d2"),
    ],
    frozen_inputs: &[
        ("contract", "r0/anhoku-reboot/r1d3-match-contract.json"),
        ("openings", "r0/anhoku-reboot/r1d3-openings.tsv"),
    ],
};

pub(crate) fn artifact_identity(path: &Path) -> Result<ArtifactIdentity> {
    Ok(ArtifactIdentity {
        path: path.to_string_lossy().into_owned(),
        bytes: fs::metadata(path)
            .with_context(|| format!("failed to stat {}", path.display()))?
            .len(),
        sha256: sha256_file(path)?,
    })
}

pub(crate) fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) fn validate_artifact(identity: &ArtifactIdentity, workspace_root: &Path) -> Result<()> {
    let recorded = Path::new(&identity.path);
    let path = if recorded.is_absolute() {
        recorded.to_path_buf()
    } else {
        workspace_root.join(recorded)
    };
    let actual = artifact_identity(&path)?;
    ensure!(
        actual.bytes == identity.bytes,
        "artifact byte count mismatch for {}",
        path.display()
    );
    ensure!(
        actual.sha256 == identity.sha256,
        "artifact SHA-256 mismatch for {}",
        path.display()
    );
    Ok(())
}

pub(crate) fn write_source_identity(
    workspace_root: &Path,
    external_trainer: &Path,
    policy_path: &Path,
    output_path: &Path,
) -> Result<SourceIdentity> {
    let identity = collect_source_identity(workspace_root, external_trainer, policy_path)?;
    ensure!(
        identity.rebuild_complete,
        "source tree is not rebuild-complete"
    );
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_path, serde_json::to_vec_pretty(&identity)?)?;
    Ok(identity)
}

pub(crate) fn validate_source_identity(
    source_path: &Path,
    workspace_root: &Path,
    expected_executable: &Path,
) -> Result<SourceIdentity> {
    let bytes = fs::read(source_path)?;
    let recorded: SourceIdentity = serde_json::from_slice(&bytes)?;
    ensure!(recorded.schema == SOURCE_SCHEMA && recorded.schema_version == 2);
    let external = PathBuf::from(&recorded.external_trainer.path);
    let policy = PathBuf::from(&recorded.policy.path);
    let policy = if policy.is_absolute() {
        policy
    } else {
        workspace_root.join(policy)
    };
    let current = collect_source_identity(workspace_root, &external, &policy)?;

    // The evidence source is immutable, but the repository may receive the
    // eventual closeout commit. Only the exact workflow document is allowed
    // between those commits. All build-relevant state remains compared by
    // identity below; a source/config/script/model change cannot hide behind a
    // different Git commit.
    validate_source_transition(&recorded, &current, workspace_root)?;

    ensure!(recorded.workspace.path == current.workspace.path);
    ensure!(
        recorded.workspace.excluded_untracked == current.workspace.excluded_untracked
            && recorded.workspace.tracked_diff_sha256 == current.workspace.tracked_diff_sha256
            && recorded.workspace.staged_diff_sha256 == current.workspace.staged_diff_sha256,
        "workspace exclusion or diff identity changed"
    );
    ensure!(recorded.external_trainer == current.external_trainer);
    ensure!(recorded.cargo_lock == current.cargo_lock);
    ensure!(recorded.submodule_status == current.submodule_status);
    ensure!(recorded.submodule_status_sha256 == current.submodule_status_sha256);
    ensure!(recorded.policy == current.policy);
    ensure!(recorded.producer_executable == current.producer_executable);
    ensure!(recorded.authorization_allowed_paths == current.authorization_allowed_paths);
    ensure!(current.rebuild_complete);
    ensure!(
        recorded.workspace.tracked_changes.is_empty()
            && recorded.workspace.relevant_untracked.is_empty()
            && current.workspace.tracked_changes.is_empty()
            && current.workspace.relevant_untracked.is_empty(),
        "source identity requires a clean workspace"
    );
    let expected = artifact_identity(expected_executable)?;
    ensure!(
        recorded.producer_executable.bytes == expected.bytes
            && recorded.producer_executable.sha256 == expected.sha256,
        "source identity was produced by another executable"
    );
    ensure!(recorded.rebuild_complete);
    Ok(recorded)
}

fn validate_source_transition(
    recorded: &SourceIdentity,
    current: &SourceIdentity,
    workspace_root: &Path,
) -> Result<()> {
    ensure!(
        current.workspace.tracked_changes.is_empty()
            && current.workspace.relevant_untracked.is_empty()
            && current.workspace.tracked_diff_sha256 == sha256_bytes(b"")
            && current.workspace.staged_diff_sha256 == sha256_bytes(b""),
        "authorization transition requires a clean workspace"
    );
    ensure!(
        recorded.evidence_source_commit == recorded.workspace.commit
            && recorded.evidence_source_tree == recorded.workspace.tree
            && recorded.authorization_commit == recorded.evidence_source_commit
            && recorded.authorization_tree == recorded.evidence_source_tree,
        "source identity anchor fields are inconsistent"
    );
    ensure!(
        current.evidence_source_commit == current.workspace.commit
            && current.evidence_source_tree == current.workspace.tree
            && current.authorization_commit == current.evidence_source_commit
            && current.authorization_tree == current.evidence_source_tree,
        "current authorization identity anchor fields are inconsistent"
    );
    ensure!(
        recorded.authorization_allowed_paths
            == AUTHORIZATION_ALLOWED_PATHS
                .iter()
                .map(|path| (*path).to_string())
                .collect::<Vec<_>>(),
        "source identity authorization allowlist is not the frozen exact list"
    );
    ensure!(
        current.build_relevant_manifest_sha256 == recorded.build_relevant_manifest_sha256,
        "build-relevant inclusion manifest changed"
    );
    let current_commit = &current.workspace.commit;
    ensure!(
        current.workspace.tree == git_rev_parse(workspace_root, "HEAD^{tree}")?,
        "current workspace tree could not be resolved"
    );
    ensure!(
        git_is_ancestor(
            workspace_root,
            &recorded.evidence_source_commit,
            current_commit
        )?,
        "authorization commit is not a descendant of the evidence source"
    );
    let changed = authorization_diff_paths(
        workspace_root,
        &recorded.evidence_source_commit,
        current_commit,
    )?;
    let allowed = recorded
        .authorization_allowed_paths
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    ensure!(
        changed.iter().all(|path| allowed.contains(path.as_str())),
        "authorization transition changed a non-allowlisted path: {:?}",
        changed
    );
    Ok(())
}

pub(crate) fn current_authorization_transition(
    source: &SourceIdentity,
    workspace_root: &Path,
) -> Result<AuthorizationTransition> {
    let current_commit = git_rev_parse(workspace_root, "HEAD")?;
    let current_tree = git_rev_parse(workspace_root, "HEAD^{tree}")?;
    let parent = git_rev_parse(workspace_root, "HEAD^")?;
    let parent_tree = git_rev_parse(workspace_root, &format!("{parent}^{{tree}}"))?;
    let current_manifest = build_relevant_manifest(workspace_root, &current_commit)?;
    ensure!(
        current_manifest == source.build_relevant_manifest_sha256,
        "authorization build-relevant inclusion manifest changed"
    );
    ensure!(
        git_is_ancestor(
            workspace_root,
            &source.evidence_source_commit,
            &current_commit
        )?,
        "authorization commit is not a descendant of the evidence source"
    );
    let changed = authorization_diff_paths(
        workspace_root,
        &source.evidence_source_commit,
        &current_commit,
    )?;
    let allowed = source
        .authorization_allowed_paths
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    ensure!(
        changed.iter().all(|path| allowed.contains(path.as_str())),
        "authorization transition changed a non-allowlisted path: {:?}",
        changed
    );
    Ok(AuthorizationTransition {
        schema: "haitaka-r1-authorization-transition-v1".to_string(),
        schema_version: 1,
        evidence_source_commit: source.evidence_source_commit.clone(),
        evidence_source_tree: source.evidence_source_tree.clone(),
        authorization_commit: current_commit,
        authorization_tree: current_tree,
        authorization_parent_commit: parent,
        authorization_parent_tree: parent_tree,
        build_relevant_manifest_sha256: current_manifest,
        allowed_workflow_paths: source.authorization_allowed_paths.clone(),
    })
}

pub(crate) fn validate_authorization_transition(
    source: &SourceIdentity,
    transition: &AuthorizationTransition,
    workspace_root: &Path,
) -> Result<()> {
    ensure!(
        transition.schema == "haitaka-r1-authorization-transition-v1"
            && transition.schema_version == 1,
        "authorization transition schema/version mismatch"
    );
    ensure!(
        transition.evidence_source_commit == source.evidence_source_commit
            && transition.evidence_source_tree == source.evidence_source_tree
            && transition.build_relevant_manifest_sha256 == source.build_relevant_manifest_sha256
            && transition.allowed_workflow_paths == source.authorization_allowed_paths,
        "authorization transition does not bind to source identity"
    );
    let current_commit = git_rev_parse(workspace_root, "HEAD")?;
    let current_tree = git_rev_parse(workspace_root, "HEAD^{tree}")?;
    ensure!(transition.authorization_commit == current_commit);
    ensure!(transition.authorization_tree == current_tree);
    let current_parent = git_rev_parse(workspace_root, "HEAD^")?;
    let current_parent_tree = git_rev_parse(workspace_root, &format!("{current_parent}^{{tree}}"))?;
    ensure!(transition.authorization_parent_commit == current_parent);
    ensure!(transition.authorization_parent_tree == current_parent_tree);
    let current_manifest = build_relevant_manifest(workspace_root, &current_commit)?;
    ensure!(current_manifest == source.build_relevant_manifest_sha256);
    ensure!(
        git_is_ancestor(
            workspace_root,
            &source.evidence_source_commit,
            &current_commit
        )?,
        "authorization commit is not a descendant of the evidence source"
    );
    let changed = authorization_diff_paths(
        workspace_root,
        &source.evidence_source_commit,
        &current_commit,
    )?;
    let allowed = transition
        .allowed_workflow_paths
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    ensure!(
        changed.iter().all(|path| allowed.contains(path.as_str())),
        "authorization transition changed a non-allowlisted path: {:?}",
        changed
    );
    Ok(())
}

fn collect_source_identity(
    workspace_root: &Path,
    external_trainer: &Path,
    policy_path: &Path,
) -> Result<SourceIdentity> {
    let policy_path = if policy_path.is_absolute() {
        policy_path.to_path_buf()
    } else {
        workspace_root.join(policy_path)
    };
    let policy: SourcePolicy = serde_json::from_slice(&fs::read(&policy_path)?)?;
    ensure!(policy.schema == SOURCE_POLICY_SCHEMA);
    let expected_allowed_paths = AUTHORIZATION_ALLOWED_PATHS
        .iter()
        .map(|path| (*path).to_string())
        .collect::<Vec<_>>();
    ensure!(policy.authorization_allowed_paths == expected_allowed_paths);
    let workspace =
        repository_identity(workspace_root, &policy.workspace_untracked_exclusions, &[])?;
    let external = repository_identity(
        external_trainer,
        &[],
        &policy.external_trainer_untracked_exclusion_prefixes,
    )?;
    let submodule_status = git_output(workspace_root, &["submodule", "status", "--recursive"])?;
    let cargo_lock = artifact_identity(&workspace_root.join("Cargo.lock"))?;
    let producer_executable = artifact_identity(&std::env::current_exe()?)?;
    let evidence_source_commit = workspace.commit.clone();
    let evidence_source_tree = workspace.tree.clone();
    let build_relevant_manifest_sha256 =
        build_relevant_manifest(workspace_root, &evidence_source_commit)?;
    let rebuild_complete = workspace.tracked_changes.is_empty()
        && workspace.relevant_untracked.is_empty()
        && external.tracked_changes.is_empty()
        && external.relevant_untracked.is_empty()
        && workspace.tracked_diff_sha256 == sha256_bytes(b"")
        && workspace.staged_diff_sha256 == sha256_bytes(b"")
        && external.tracked_diff_sha256 == sha256_bytes(b"")
        && external.staged_diff_sha256 == sha256_bytes(b"");
    Ok(SourceIdentity {
        schema: SOURCE_SCHEMA.to_string(),
        schema_version: 2,
        evidence_source_commit: evidence_source_commit.clone(),
        evidence_source_tree: evidence_source_tree.clone(),
        authorization_commit: evidence_source_commit,
        authorization_tree: evidence_source_tree,
        build_relevant_manifest_sha256,
        authorization_allowed_paths: expected_allowed_paths,
        workspace,
        cargo_lock,
        submodule_status_sha256: sha256_bytes(submodule_status.as_bytes()),
        submodule_status,
        external_trainer: external,
        policy: artifact_identity(&policy_path)?,
        producer_executable,
        rebuild_complete,
    })
}

fn git_rev_parse(repo: &Path, revision: &str) -> Result<String> {
    Ok(git_output(repo, &["rev-parse", revision])?
        .trim()
        .to_string())
}

fn git_is_ancestor(repo: &Path, ancestor: &str, descendant: &str) -> Result<bool> {
    let status = Command::new("git")
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .current_dir(repo)
        .status()?;
    Ok(status.success())
}

fn authorization_diff_paths(
    repo: &Path,
    evidence: &str,
    authorization: &str,
) -> Result<Vec<String>> {
    let range = format!("{evidence}..{authorization}");
    let diff = git_output(repo, &["diff", "--name-status", "--no-renames", &range])?;
    diff.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut fields = line.split('\t');
            let status = fields
                .next()
                .ok_or_else(|| anyhow!("malformed authorization diff line: {line}"))?;
            ensure!(
                status == "M",
                "authorization transition contains non-modification status {status}"
            );
            let path = fields
                .next()
                .ok_or_else(|| anyhow!("authorization diff line lacks path: {line}"))?;
            ensure!(
                fields.next().is_none(),
                "authorization diff line has extra path fields"
            );
            Ok(path.to_string())
        })
        .collect()
}

fn build_relevant_manifest(repo: &Path, revision: &str) -> Result<String> {
    let entries = git_bytes(repo, &["ls-tree", "-r", "-z", revision])?;
    let allowed = AUTHORIZATION_ALLOWED_PATHS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut hasher = Sha256::new();
    for entry in entries
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let tab = entry
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| anyhow!("malformed git tree entry"))?;
        let path = &entry[tab + 1..];
        if allowed.contains(std::str::from_utf8(path)?) {
            continue;
        }
        hasher.update(entry);
        hasher.update([0]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn repository_identity(
    repo: &Path,
    exact: &[Exclusion],
    prefixes: &[PrefixExclusion],
) -> Result<RepositoryIdentity> {
    let status = git_output(repo, &["status", "--porcelain=v1", "--untracked-files=all"])?;
    let mut tracked_changes = Vec::new();
    let mut untracked = Vec::new();
    for line in status.lines().filter(|line| !line.is_empty()) {
        let path = line
            .get(3..)
            .ok_or_else(|| anyhow!("malformed git status line: {line}"))?;
        if line.starts_with("?? ") {
            untracked.push(path.to_string());
        } else {
            tracked_changes.push(line.to_string());
        }
    }
    let mut excluded = Vec::new();
    let mut relevant = Vec::new();
    for relative in untracked {
        let full = repo.join(&relative);
        let id = artifact_identity(&full)?;
        if let Some(rule) = exact.iter().find(|rule| rule.path == relative) {
            ensure!(
                id.bytes == rule.bytes && id.sha256 == rule.sha256,
                "excluded file identity changed: {relative}"
            );
            excluded.push(rule.clone());
        } else if let Some(rule) = prefixes
            .iter()
            .find(|rule| relative.starts_with(&rule.path_prefix))
        {
            excluded.push(Exclusion {
                path: relative,
                bytes: id.bytes,
                sha256: id.sha256,
                reason: rule.reason.clone(),
            });
        } else {
            relevant.push(id);
        }
    }
    for rule in exact {
        ensure!(
            excluded.iter().any(|item| item.path == rule.path),
            "declared exclusion missing: {}",
            rule.path
        );
    }
    tracked_changes.sort();
    relevant.sort_by(|a, b| a.path.cmp(&b.path));
    excluded.sort_by(|a, b| a.path.cmp(&b.path));
    let diff = git_bytes(repo, &["diff", "--binary", "--no-ext-diff"])?;
    let staged = git_bytes(repo, &["diff", "--cached", "--binary", "--no-ext-diff"])?;
    Ok(RepositoryIdentity {
        path: repo.canonicalize()?.to_string_lossy().into_owned(),
        commit: git_output(repo, &["rev-parse", "HEAD"])?.trim().to_string(),
        tree: git_output(repo, &["rev-parse", "HEAD^{tree}"])?
            .trim()
            .to_string(),
        tracked_diff_sha256: sha256_bytes(&diff),
        staged_diff_sha256: sha256_bytes(&staged),
        tracked_changes,
        relevant_untracked: relevant,
        excluded_untracked: excluded,
    })
}

fn git_output(repo: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8(git_bytes(repo, args)?)?)
}

fn git_bytes(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git").args(args).current_dir(repo).output()?;
    ensure!(
        output.status.success(),
        "git {} failed in {}: {}",
        args.join(" "),
        repo.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output.stdout)
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn validate_report_chain(
    reports: &BTreeMap<&str, PathBuf>,
    required: &[StageSpec],
    workspace_root: &Path,
    executable: &Path,
    source_manifest: &Path,
) -> Result<()> {
    let source_id = artifact_identity(source_manifest)?;
    validate_source_identity(source_manifest, workspace_root, executable)?;
    let exe_id = artifact_identity(executable)?;
    let mut values = BTreeMap::new();
    for spec in required {
        let path = reports
            .get(spec.id)
            .with_context(|| format!("missing {} report path", spec.id))?;
        values.insert(spec.id, read_strict_json(path)?);
    }
    ensure!(values.len() == required.len(), "extra stage links supplied");
    for spec in required {
        validate_one_report(spec, &values, reports, workspace_root, &exe_id, &source_id)?;
    }
    Ok(())
}

pub(crate) fn linked_report_path(
    report_path: &Path,
    artifact_name: &str,
    workspace_root: &Path,
) -> Result<PathBuf> {
    let report = read_strict_json(report_path)?;
    let identity: ArtifactIdentity = serde_json::from_value(
        report["artifacts"]
            .get(artifact_name)
            .with_context(|| format!("missing linked artifact {artifact_name}"))?
            .clone(),
    )?;
    validate_artifact(&identity, workspace_root)?;
    let path = PathBuf::from(identity.path);
    Ok(if path.is_absolute() {
        path
    } else {
        workspace_root.join(path)
    })
}

fn validate_one_report(
    spec: &StageSpec,
    values: &BTreeMap<&str, Value>,
    paths: &BTreeMap<&str, PathBuf>,
    workspace_root: &Path,
    exe: &ArtifactIdentity,
    source: &ArtifactIdentity,
) -> Result<()> {
    let report = &values[spec.id];
    ensure!(
        report["schema"] == spec.schema
            && report["schemaVersion"] == 1
            && report["ruleset"] == "anhoku",
        "{} report schema/version/ruleset mismatch",
        spec.id
    );
    let gate_obj = report["gates"]
        .as_object()
        .context("report gates must be an object")?;
    let actual_gates = gate_obj.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected_gates = spec.gates.iter().copied().collect::<BTreeSet<_>>();
    ensure!(
        actual_gates == expected_gates,
        "{} named-gate set mismatch",
        spec.id
    );
    ensure!(
        gate_obj.values().all(|value| value == &Value::Bool(true)),
        "{} contains a failing gate",
        spec.id
    );
    ensure!(
        report["passed"] == true,
        "{} top-level pass disagrees",
        spec.id
    );
    let artifacts = report["artifacts"]
        .as_object()
        .context("report artifacts must be an object")?;
    for (name, value) in artifacts {
        let id: ArtifactIdentity = serde_json::from_value(value.clone())
            .with_context(|| format!("invalid {name} identity"))?;
        validate_artifact(&id, workspace_root)?;
    }
    require_same_identity(artifacts, "gateExecutable", exe, spec.id)?;
    require_same_identity(artifacts, "sourceIdentity", source, spec.id)?;
    for (name, relative) in spec.frozen_inputs {
        let expected = artifact_identity(&workspace_root.join(relative))?;
        require_same_identity(artifacts, name, &expected, spec.id)?;
    }
    let actual_links = artifacts
        .keys()
        .filter(|key| key.ends_with("Report"))
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected_links = spec
        .prior_links
        .iter()
        .map(|(name, _)| *name)
        .collect::<BTreeSet<_>>();
    ensure!(
        actual_links == expected_links,
        "{} prior-stage link set mismatch",
        spec.id
    );
    for (name, prior) in spec.prior_links {
        ensure!(
            values.contains_key(prior),
            "{} links an unavailable prior stage {prior}",
            spec.id
        );
        let prior_path = paths.get(prior).context("missing prior path")?;
        let expected = artifact_identity(prior_path)?;
        require_same_identity(artifacts, name, &expected, spec.id)?;
    }
    Ok(())
}

fn require_same_identity(
    artifacts: &serde_json::Map<String, Value>,
    name: &str,
    expected: &ArtifactIdentity,
    stage: &str,
) -> Result<()> {
    let actual: ArtifactIdentity = serde_json::from_value(
        artifacts
            .get(name)
            .with_context(|| format!("{stage} missing {name}"))?
            .clone(),
    )?;
    ensure!(
        actual.bytes == expected.bytes && actual.sha256 == expected.sha256,
        "{stage} {name} identity mismatch"
    );
    Ok(())
}

pub(crate) fn read_strict_json(path: &Path) -> Result<Value> {
    // serde_json's normal map representation accepts duplicate keys. This small
    // preflight rejects them before typed deserialization.
    let bytes = fs::read(path)?;
    let mut de = serde_json::Deserializer::from_slice(&bytes);
    let value = StrictValue::deserialize(&mut de)?.0;
    de.end()?;
    Ok(value)
}

struct StrictValue(Value);
impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = StrictValue;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("JSON value without duplicate object keys")
            }
            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Bool(v)))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Number(v.into())))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Number(v.into())))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(Value::Number)
                    .map(StrictValue)
                    .ok_or_else(|| E::custom("non-finite JSON number"))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::String(v.to_string())))
            }
            fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::String(v)))
            }
            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_some<D: serde::Deserializer<'de>>(
                self,
                d: D,
            ) -> Result<Self::Value, D::Error> {
                StrictValue::deserialize(d)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(v) = seq.next_element::<StrictValue>()? {
                    out.push(v.0);
                }
                Ok(StrictValue(Value::Array(out)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, StrictValue>()? {
                    if out.insert(key.clone(), value.0).is_some() {
                        return Err(serde::de::Error::custom(format!(
                            "duplicate JSON key {key}"
                        )));
                    }
                }
                Ok(StrictValue(Value::Object(out)))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_GATES: &[&str] = &["complete", "identity"];
    const TEST_PRIOR_LINKS: &[(&str, &str)] = &[("priorReport", "prior")];
    const TEST_INPUTS: &[(&str, &str)] = &[("contract", "contract.json")];
    const TEST_SPEC: StageSpec = StageSpec {
        id: "stage",
        schema: "test-stage-v1",
        gates: TEST_GATES,
        prior_links: TEST_PRIOR_LINKS,
        frozen_inputs: TEST_INPUTS,
    };
    const PRIOR_SPEC: StageSpec = StageSpec {
        id: "prior",
        schema: "test-prior-v1",
        gates: TEST_GATES,
        prior_links: &[],
        frozen_inputs: TEST_INPUTS,
    };

    fn report(schema: &str, artifacts: BTreeMap<String, ArtifactIdentity>) -> Value {
        serde_json::json!({
            "schema": schema, "schemaVersion": 1, "ruleset": "anhoku",
            "artifacts": artifacts, "gates": {"complete": true, "identity": true}, "passed": true
        })
    }

    fn fixture() -> (
        tempfile::TempDir,
        BTreeMap<&'static str, Value>,
        BTreeMap<&'static str, PathBuf>,
        ArtifactIdentity,
        ArtifactIdentity,
    ) {
        let dir = tempfile::tempdir().unwrap();
        for (name, bytes) in [
            ("contract.json", b"contract".as_slice()),
            ("exe", b"executable"),
            ("source", b"source"),
            ("payload", b"payload"),
        ] {
            fs::write(dir.path().join(name), bytes).unwrap();
        }
        let exe = artifact_identity(&dir.path().join("exe")).unwrap();
        let source = artifact_identity(&dir.path().join("source")).unwrap();
        let contract = artifact_identity(&dir.path().join("contract.json")).unwrap();
        let payload = artifact_identity(&dir.path().join("payload")).unwrap();
        let prior_path = dir.path().join("prior.json");
        let prior = report(
            "test-prior-v1",
            BTreeMap::from([
                ("contract".to_string(), contract.clone()),
                ("gateExecutable".to_string(), exe.clone()),
                ("payload".to_string(), payload.clone()),
                ("sourceIdentity".to_string(), source.clone()),
            ]),
        );
        fs::write(&prior_path, serde_json::to_vec(&prior).unwrap()).unwrap();
        let stage_path = dir.path().join("stage.json");
        let stage = report(
            "test-stage-v1",
            BTreeMap::from([
                ("contract".to_string(), contract),
                ("gateExecutable".to_string(), exe.clone()),
                ("payload".to_string(), payload),
                (
                    "priorReport".to_string(),
                    artifact_identity(&prior_path).unwrap(),
                ),
                ("sourceIdentity".to_string(), source.clone()),
            ]),
        );
        fs::write(&stage_path, serde_json::to_vec(&stage).unwrap()).unwrap();
        (
            dir,
            BTreeMap::from([("prior", prior), ("stage", stage)]),
            BTreeMap::from([("prior", prior_path), ("stage", stage_path)]),
            exe,
            source,
        )
    }

    fn validate_fixture(
        values: &BTreeMap<&str, Value>,
        paths: &BTreeMap<&str, PathBuf>,
        root: &Path,
        exe: &ArtifactIdentity,
        source: &ArtifactIdentity,
    ) -> Result<()> {
        validate_one_report(&PRIOR_SPEC, values, paths, root, exe, source)?;
        validate_one_report(&TEST_SPEC, values, paths, root, exe, source)
    }
    #[test]
    fn duplicate_json_keys_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("duplicate.json");
        fs::write(&path, br#"{"provenance":{"sha256":"a","sha256":"b"}}"#).unwrap();
        assert!(read_strict_json(&path).is_err());
    }

    #[test]
    fn unlisted_untracked_file_is_relevant() {
        let dir = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "r1@test.invalid"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "R1 test"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        fs::write(dir.path().join("tracked"), b"x").unwrap();
        Command::new("git")
            .args(["add", "tracked"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-qm", "fixture"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        fs::write(dir.path().join("relevant.js"), b"source").unwrap();
        let identity = repository_identity(dir.path(), &[], &[]).unwrap();
        assert_eq!(identity.relevant_untracked.len(), 1);
        assert!(identity.excluded_untracked.is_empty());
    }

    #[test]
    fn named_gates_and_top_level_boolean_fail_closed() {
        let (dir, values, paths, exe, source) = fixture();
        assert!(validate_fixture(&values, &paths, dir.path(), &exe, &source).is_ok());
        for mutation in ["remove", "add", "gate-false", "top-false"] {
            let mut bad = values.clone();
            let stage = bad.get_mut("stage").unwrap();
            match mutation {
                "remove" => {
                    stage["gates"].as_object_mut().unwrap().remove("identity");
                }
                "add" => {
                    stage["gates"]["unexpected"] = Value::Bool(true);
                }
                "gate-false" => {
                    stage["gates"]["identity"] = Value::Bool(false);
                    stage["passed"] = Value::Bool(true);
                }
                "top-false" => {
                    stage["passed"] = Value::Bool(false);
                }
                _ => unreachable!(),
            }
            assert!(
                validate_fixture(&bad, &paths, dir.path(), &exe, &source).is_err(),
                "mutation {mutation} passed"
            );
        }
    }

    #[test]
    fn artifact_and_prior_substitution_fail_closed() {
        let (dir, values, paths, exe, source) = fixture();
        let payload = dir.path().join("payload");
        fs::write(&payload, b"PAYLOAD").unwrap(); // same byte count, different content
        assert!(validate_fixture(&values, &paths, dir.path(), &exe, &source).is_err());

        fs::write(&payload, b"payload").unwrap();
        let mut bad = values.clone();
        bad.get_mut("stage").unwrap()["artifacts"]["priorReport"]["sha256"] =
            Value::String("0".repeat(64));
        assert!(validate_fixture(&bad, &paths, dir.path(), &exe, &source).is_err());

        let mut extra = values.clone();
        extra.get_mut("stage").unwrap()["artifacts"]["otherReport"] =
            serde_json::to_value(artifact_identity(&paths["prior"]).unwrap()).unwrap();
        assert!(validate_fixture(&extra, &paths, dir.path(), &exe, &source).is_err());
    }

    fn git_test(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} failed", args);
    }

    fn transition_fixture() -> (tempfile::TempDir, SourceIdentity, SourceIdentity) {
        let dir = tempfile::tempdir().unwrap();
        git_test(dir.path(), &["init", "-q"]);
        git_test(
            dir.path(),
            &["config", "user.email", "r1-transition@test.invalid"],
        );
        git_test(dir.path(), &["config", "user.name", "R1 transition"]);
        fs::create_dir_all(dir.path().join("plans")).unwrap();
        fs::write(dir.path().join("engine.rs"), "source\n").unwrap();
        fs::write(
            dir.path()
                .join("plans/anhoku-nnue-training-workflow-reboot.md"),
            "status\n",
        )
        .unwrap();
        git_test(dir.path(), &["add", "."]);
        git_test(dir.path(), &["commit", "-qm", "evidence"]);
        let evidence_commit = git_rev_parse(dir.path(), "HEAD").unwrap();
        let evidence_tree = git_rev_parse(dir.path(), "HEAD^{tree}").unwrap();
        let workspace = repository_identity(dir.path(), &[], &[]).unwrap();
        let manifest = build_relevant_manifest(dir.path(), &evidence_commit).unwrap();
        let mut source = SourceIdentity {
            schema: SOURCE_SCHEMA.to_string(),
            schema_version: 2,
            evidence_source_commit: evidence_commit.clone(),
            evidence_source_tree: evidence_tree.clone(),
            authorization_commit: evidence_commit,
            authorization_tree: evidence_tree,
            build_relevant_manifest_sha256: manifest.clone(),
            authorization_allowed_paths: AUTHORIZATION_ALLOWED_PATHS
                .iter()
                .map(|path| (*path).to_string())
                .collect(),
            workspace,
            cargo_lock: ArtifactIdentity {
                path: "unused".to_string(),
                bytes: 0,
                sha256: String::new(),
            },
            submodule_status: String::new(),
            submodule_status_sha256: sha256_bytes(b""),
            external_trainer: RepositoryIdentity {
                path: "unused".to_string(),
                commit: String::new(),
                tree: String::new(),
                tracked_diff_sha256: sha256_bytes(b""),
                staged_diff_sha256: sha256_bytes(b""),
                tracked_changes: Vec::new(),
                relevant_untracked: Vec::new(),
                excluded_untracked: Vec::new(),
            },
            policy: ArtifactIdentity {
                path: "unused".to_string(),
                bytes: 0,
                sha256: String::new(),
            },
            producer_executable: ArtifactIdentity {
                path: "unused".to_string(),
                bytes: 0,
                sha256: String::new(),
            },
            rebuild_complete: true,
        };
        git_test(
            dir.path(),
            &["commit", "--allow-empty", "-qm", "authorization-parent"],
        );
        // The evidence anchor intentionally remains the first commit; this
        // empty parent is useful for testing that a real authorization commit
        // is checked by ancestry rather than by an editable string.
        source.workspace.commit = source.evidence_source_commit.clone();
        let current = source.clone();
        (dir, source, current)
    }

    #[test]
    fn plan_only_authorization_transition_is_the_only_allowed_change() {
        let (dir, source, _) = transition_fixture();
        fs::write(
            dir.path()
                .join("plans/anhoku-nnue-training-workflow-reboot.md"),
            "authorized\n",
        )
        .unwrap();
        git_test(
            dir.path(),
            &["add", "plans/anhoku-nnue-training-workflow-reboot.md"],
        );
        git_test(dir.path(), &["commit", "-qm", "closeout"]);
        let current_workspace = repository_identity(dir.path(), &[], &[]).unwrap();
        let current_commit = current_workspace.commit.clone();
        let current_tree = current_workspace.tree.clone();
        let current = SourceIdentity {
            workspace: current_workspace,
            evidence_source_commit: current_commit.clone(),
            evidence_source_tree: current_tree.clone(),
            authorization_commit: current_commit,
            authorization_tree: current_tree,
            build_relevant_manifest_sha256: build_relevant_manifest(
                dir.path(),
                &git_rev_parse(dir.path(), "HEAD").unwrap(),
            )
            .unwrap(),
            ..source.clone()
        };
        assert!(validate_source_transition(&source, &current, dir.path()).is_ok());
    }

    #[test]
    fn build_relevant_or_untracked_changes_fail_closed() {
        for path in [
            "engine.rs",
            "engine.js",
            "config.toml",
            "scripts/run.sh",
            "model.nnue",
            "contract.json",
            "openings.tsv",
            "Cargo.lock",
            "plans/another-closeout.md",
        ] {
            let (dir, source, _) = transition_fixture();
            if let Some(parent) = dir.path().join(path).parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(dir.path().join(path), "mutated\n").unwrap();
            git_test(dir.path(), &["add", path]);
            git_test(dir.path(), &["commit", "-qm", "forbidden"]);
            let current_workspace = repository_identity(dir.path(), &[], &[]).unwrap();
            let current_commit = current_workspace.commit.clone();
            let current_tree = current_workspace.tree.clone();
            let current = SourceIdentity {
                workspace: current_workspace,
                evidence_source_commit: current_commit.clone(),
                evidence_source_tree: current_tree.clone(),
                authorization_commit: current_commit,
                authorization_tree: current_tree,
                build_relevant_manifest_sha256: build_relevant_manifest(
                    dir.path(),
                    &git_rev_parse(dir.path(), "HEAD").unwrap(),
                )
                .unwrap(),
                ..source.clone()
            };
            assert!(
                validate_source_transition(&source, &current, dir.path()).is_err(),
                "forbidden authorization path passed: {path}"
            );
        }

        let (dir, source, _) = transition_fixture();
        fs::write(dir.path().join("relevant.js"), "untracked\n").unwrap();
        let current_workspace = repository_identity(dir.path(), &[], &[]).unwrap();
        let current_commit = current_workspace.commit.clone();
        let current_tree = current_workspace.tree.clone();
        let current = SourceIdentity {
            workspace: current_workspace,
            evidence_source_commit: current_commit.clone(),
            evidence_source_tree: current_tree.clone(),
            authorization_commit: current_commit,
            authorization_tree: current_tree,
            ..source.clone()
        };
        assert!(validate_source_transition(&source, &current, dir.path()).is_err());
    }

    #[test]
    fn substituted_or_non_descendant_authorization_fails_closed() {
        let (dir, source, _) = transition_fixture();
        git_test(
            dir.path(),
            &["commit", "--allow-empty", "-qm", "authorized-parent"],
        );
        let current_workspace = repository_identity(dir.path(), &[], &[]).unwrap();
        let current_commit = current_workspace.commit.clone();
        let current_tree = current_workspace.tree.clone();
        let mut current = SourceIdentity {
            workspace: current_workspace,
            evidence_source_commit: current_commit.clone(),
            evidence_source_tree: current_tree.clone(),
            authorization_commit: current_commit,
            authorization_tree: current_tree,
            ..source.clone()
        };
        current.evidence_source_commit = git_rev_parse(dir.path(), "HEAD^").unwrap();
        assert!(validate_source_transition(&source, &current, dir.path()).is_err());

        let other = tempfile::tempdir().unwrap();
        git_test(other.path(), &["init", "-q"]);
        git_test(
            other.path(),
            &["config", "user.email", "other@test.invalid"],
        );
        git_test(other.path(), &["config", "user.name", "Other"]);
        fs::write(other.path().join("engine.rs"), "other\n").unwrap();
        git_test(other.path(), &["add", "."]);
        git_test(other.path(), &["commit", "-qm", "substituted"]);
        let other_workspace = repository_identity(other.path(), &[], &[]).unwrap();
        let other_current = SourceIdentity {
            workspace: other_workspace,
            build_relevant_manifest_sha256: build_relevant_manifest(
                other.path(),
                &git_rev_parse(other.path(), "HEAD").unwrap(),
            )
            .unwrap(),
            ..source
        };
        assert!(validate_source_transition(&other_current, &other_current, dir.path()).is_err());
    }
}

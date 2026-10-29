use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail, ensure};
use haitaka::{Board, Color, GameStatus, Move, Piece};
use haitaka_wasm::{
    NnueModel, SEARCH_MATE_SCORE_THRESHOLD, SEARCH_NODE_COUNTING_VERSION, SearchEvalMode,
    SearchTrainingTrace, SearchWorkspace,
    search_board_impl_handcrafted_with_node_budget_and_training_trace_in_workspace,
    search_board_impl_handcrafted_with_node_budget_in_workspace,
    search_board_impl_with_eval_mode_and_node_budget_in_workspace,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Contract {
    schema: String,
    ruleset: String,
    authorization: Authorization,
    probe: ProbeContract,
    teachers: TeacherContract,
    target_comparison: TargetContract,
    score_calibration: ScoreContract,
    decision_boundaries: Boundaries,
    cost_model: CostContract,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Authorization {
    r2_gpu_gate: PathBuf,
    r2_gpu_gate_sha256: String,
    require_passed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProbeContract {
    schema: String,
    source: PathBuf,
    source_sha256: String,
    supplemental_source: PathBuf,
    supplemental_source_sha256: String,
    selection_version: String,
    selected_id_schedule_sha256: String,
    split_seed: u64,
    tuning_positions: usize,
    decision_positions: usize,
    maximum_unique_boards: usize,
    minimum_per_required_stratum_per_subset: usize,
    required_strata: RequiredStrata,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequiredStrata {
    source: Vec<String>,
    phase: Vec<String>,
    material: Vec<String>,
    proxy_score: Vec<String>,
    position_type: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TeacherContract {
    label_evaluator: String,
    node_counting_version: String,
    maximum_depth: u8,
    candidate_node_budgets: Vec<u64>,
    deep_reference_nodes: u64,
    minimum_deep_to_candidate_ratio: f64,
    move_ranking_total_nodes: u64,
    diagnostic_control: DiagnosticControl,
    external_teacher: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticControl {
    name: String,
    model: PathBuf,
    model_sha256: String,
    nodes: u64,
    eligible_for_selection: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetContract {
    semantics: Vec<String>,
    mate_score_threshold: i32,
    near_zero_score: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScoreContract {
    legacy_logistic_scale: f64,
    calibrated_logistic_quantile: f64,
    calibrated_logistic_quantile_probability: f64,
    minimum_logistic_scale: f64,
    maximum_logistic_scale: f64,
    robust_clip_quantile: f64,
    minimum_robust_clip: f64,
    maximum_robust_clip: f64,
    huber_delta_normalized: f64,
    saturation_low: f64,
    saturation_high: f64,
    maximum_calibrated_saturation_fraction: f64,
    strength_training_lambda: f64,
    result_loss_enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Boundaries {
    minimum_stratum_count_for_worst_case: usize,
    maximum_overall_probability_rmse: f64,
    maximum_worst_stratum_probability_rmse: f64,
    maximum_overall_robust_normalized_mae: f64,
    minimum_top3_agreement: f64,
    maximum_overall_regret_to_clip_ratio: f64,
    maximum_worst_stratum_regret_to_clip_ratio: f64,
    maximum_sign_disagreement_rate: f64,
    maximum_incomplete_label_rate: f64,
    required_node_accounting_rate: f64,
    minimum_c16_handcrafted_disagreements: usize,
    decision_uses_same_boundaries: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CostContract {
    worker_count: usize,
    cpu_core_hour_usd: f64,
    forecast_corpus_sizes: Vec<u64>,
    production_corpus_authorized: bool,
    strength_games_authorized: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorpusRecord {
    id: String,
    sfen: String,
    seed_class: Option<String>,
    label_score_side_to_move: i32,
    #[serde(default)]
    origin: String,
}

#[derive(Debug, Deserialize)]
struct SupplementalManifest {
    games: Vec<SupplementalGame>,
}

#[derive(Debug, Deserialize)]
struct SupplementalGame {
    game_id: String,
    sfen: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbePosition {
    id: String,
    subset: String,
    sfen: String,
    source: String,
    trajectory_policy: String,
    phase: String,
    material: String,
    proxy_score: String,
    position_types: Vec<String>,
    proxy_score_value: i32,
}

impl ProbePosition {
    fn strata(&self, reference_score: i32, c16_disagrees: bool, near_zero: i32) -> Vec<String> {
        let mut result = vec![
            format!("source={}", self.source),
            format!("trajectory={}", self.trajectory_policy),
            format!("phase={}", self.phase),
            format!("material={}", self.material),
            format!("proxy-score={}", self.proxy_score),
            format!(
                "reference-score={}",
                score_bucket(reference_score, near_zero)
            ),
            format!("c16-disagreement={c16_disagrees}"),
        ];
        result.extend(
            self.position_types
                .iter()
                .map(|kind| format!("position-type={kind}")),
        );
        result
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchObservation {
    teacher: String,
    node_limit: u64,
    completed_depth: u8,
    best_move: Option<String>,
    root_score: Option<i32>,
    qsearch_leaf_score_reoriented_to_root: Option<i32>,
    mate_class: String,
    mate_distance: Option<i32>,
    alpha_beta_nodes: u64,
    qsearch_nodes: u64,
    total_nodes: u64,
    node_accounting_exact: bool,
    incomplete_label: bool,
    partial_iteration: bool,
    elapsed_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RankedMove {
    move_: String,
    score: i32,
    child_node_limit: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RawPositionResult {
    probe: ProbePosition,
    deep_reference: SearchObservation,
    handcrafted_candidates: Vec<SearchObservation>,
    c16_diagnostic: SearchObservation,
    reference_move_ranking: Vec<RankedMove>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct MetricSet {
    positions: usize,
    probability_rmse: f64,
    raw_score_mae: f64,
    robust_normalized_mae: f64,
    robust_clipped_huber_loss: f64,
    top1_agreement: f64,
    top3_agreement: f64,
    mean_regret: f64,
    regret_to_clip_ratio: f64,
    sign_disagreement_rate: f64,
    incomplete_label_rate: f64,
    node_accounting_rate: f64,
    mate_rate: f64,
    partial_iteration_rate: f64,
    clipping_rate: f64,
    labels_per_second: f64,
    pearson: Option<f64>,
    spearman: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateReport {
    teacher: String,
    node_budget: u64,
    overall: MetricSet,
    by_stratum: BTreeMap<String, MetricSet>,
    macro_average: MetricSet,
    worst_stratum_probability_rmse: f64,
    worst_stratum_regret_to_clip_ratio: f64,
    passes_boundary: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetSemanticsReport {
    semantics: String,
    robust_normalized_mae: f64,
    robust_clipped_huber_loss: f64,
    probability_rmse: f64,
    mate_class_agreement: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SaturationBucket {
    bucket: String,
    positions: usize,
    below_low: usize,
    above_high: usize,
    mean_gradient_magnitude: f64,
    gradient_contribution_fraction: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransformAudit {
    name: String,
    scale: f64,
    below_low_fraction: f64,
    above_high_fraction: f64,
    mean_gradient_magnitude: f64,
    by_score_bucket: Vec<SaturationBucket>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CostForecast {
    positions: u64,
    projected_wall_seconds_one_worker: f64,
    projected_core_hours: f64,
    projected_cpu_cost_usd: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactIdentity {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct R3Report {
    schema: &'static str,
    schema_version: u32,
    phase: &'static str,
    pub(crate) passed: bool,
    protocol_frozen_before_results: bool,
    probe_schema: String,
    probe_selection_version: String,
    unique_boards: usize,
    tuning_boards: usize,
    decision_boards: usize,
    decision_opened_once: bool,
    artifacts: BTreeMap<String, ArtifactIdentity>,
    pub(crate) selected_teacher: String,
    pub(crate) selected_node_budget: u64,
    node_counting_version: String,
    label_evaluator: String,
    target_semantics: String,
    mate_policy: String,
    score_transform: String,
    score_transform_scale: Option<f64>,
    robust_score_clip: f64,
    primary_loss: String,
    strength_training_lambda: f64,
    result_loss_enabled: bool,
    tuning_candidates: Vec<CandidateReport>,
    decision_candidates: Vec<CandidateReport>,
    c16_diagnostic_tuning: CandidateReport,
    c16_diagnostic_decision: CandidateReport,
    target_semantics_comparison: Vec<TargetSemanticsReport>,
    legacy_transform_audit: TransformAudit,
    calibrated_transform_audit: TransformAudit,
    c16_handcrafted_disagreements: usize,
    throughput_labels_per_second: f64,
    cost_forecast: Vec<CostForecast>,
    gates: BTreeMap<String, bool>,
    pub(crate) r4_authorized: bool,
    production_corpus_generated: bool,
    strength_games_played: bool,
}

pub(crate) struct RunArgs<'a> {
    pub contract_path: &'a Path,
    pub output_dir: &'a Path,
    pub executable_path: &'a Path,
    pub workspace_root: &'a Path,
}

pub(crate) fn run(args: RunArgs<'_>) -> Result<R3Report> {
    let contract: Contract = read_json(args.contract_path)?;
    validate_contract(&contract)?;
    let contract_path = resolve(args.workspace_root, args.contract_path);
    let source_path = resolve(args.workspace_root, &contract.probe.source);
    let supplemental_source_path =
        resolve(args.workspace_root, &contract.probe.supplemental_source);
    let r2_path = resolve(args.workspace_root, &contract.authorization.r2_gpu_gate);
    let model_path = resolve(
        args.workspace_root,
        &contract.teachers.diagnostic_control.model,
    );
    validate_sha256(
        &r2_path,
        &contract.authorization.r2_gpu_gate_sha256,
        "R2 GPU gate",
    )?;
    validate_sha256(
        &source_path,
        &contract.probe.source_sha256,
        "R3 probe source",
    )?;
    validate_sha256(
        &supplemental_source_path,
        &contract.probe.supplemental_source_sha256,
        "R3 supplemental probe source",
    )?;
    validate_sha256(
        &model_path,
        &contract.teachers.diagnostic_control.model_sha256,
        "C/16 diagnostic model",
    )?;
    let r2: serde_json::Value = read_json(&r2_path)?;
    ensure!(
        !contract.authorization.require_passed || r2["passed"] == true,
        "R3 requires the frozen passing R2 CUDA gate"
    );
    if args.output_dir.exists() && fs::read_dir(args.output_dir)?.next().is_some() {
        bail!(
            "R3 gate refuses non-empty output directory {}; the one-shot block may be opened only once",
            args.output_dir.display()
        );
    }
    fs::create_dir_all(args.output_dir)?;

    let mut records = read_corpus(&source_path)?;
    records.extend(read_supplemental_corpus(&supplemental_source_path)?);
    let positions = select_probe(&records, &contract)?;
    ensure!(positions.len() <= contract.probe.maximum_unique_boards);
    write_json(&args.output_dir.join("teacher-probe-v1.json"), &positions)?;

    let model_bytes = fs::read(&model_path)?;
    let c16 = Arc::new(
        NnueModel::from_bytes(&model_bytes)
            .map_err(|err| anyhow::anyhow!("failed to load C/16 diagnostic: {err}"))?,
    );
    let mut raw = Vec::with_capacity(positions.len());
    for (index, probe) in positions.into_iter().enumerate() {
        eprintln!(
            "R3 probe {}/{} {} {}",
            index + 1,
            contract.probe.tuning_positions + contract.probe.decision_positions,
            probe.subset,
            probe.id
        );
        raw.push(run_position(probe, &contract, Arc::clone(&c16))?);
    }
    let raw_path = args.output_dir.join("teacher-probe-results.jsonl");
    write_jsonl(&raw_path, &raw)?;

    let tuning = raw
        .iter()
        .filter(|item| item.probe.subset == "tuning")
        .collect::<Vec<_>>();
    let decision = raw
        .iter()
        .filter(|item| item.probe.subset == "decision")
        .collect::<Vec<_>>();
    let tuning_reference_scores = tuning
        .iter()
        .filter_map(|item| item.deep_reference.root_score)
        .collect::<Vec<_>>();
    ensure!(tuning_reference_scores.len() == tuning.len());
    let logistic_scale = calibrated_scale(&tuning_reference_scores, &contract.score_calibration);
    let robust_clip = robust_clip(&tuning_reference_scores, &contract.score_calibration);

    let tuning_candidates =
        candidate_reports(&tuning, &contract, logistic_scale, robust_clip, false);
    let selected = tuning_candidates
        .iter()
        .find(|candidate| candidate.passes_boundary)
        .context("no affordable handcrafted teacher met the preregistered tuning boundary")?;
    let selected_budget = selected.node_budget;
    let decision_candidates =
        candidate_reports(&decision, &contract, logistic_scale, robust_clip, false);
    let decision_selected = decision_candidates
        .iter()
        .find(|candidate| candidate.node_budget == selected_budget)
        .expect("selected budget was run on decision block");

    let c16_tuning = diagnostic_report(&tuning, &contract, logistic_scale, robust_clip, "c16");
    let c16_decision = diagnostic_report(&decision, &contract, logistic_scale, robust_clip, "c16");
    let disagreements = raw
        .iter()
        .filter(|item| {
            item.c16_diagnostic.best_move != candidate(item, selected_budget).best_move
                || sign(
                    item.c16_diagnostic.root_score,
                    contract.target_comparison.near_zero_score,
                ) != sign(
                    candidate(item, selected_budget).root_score,
                    contract.target_comparison.near_zero_score,
                )
        })
        .count();

    let semantics = target_semantics_reports(
        &tuning,
        selected_budget,
        logistic_scale,
        robust_clip,
        &contract,
    );
    let root = semantics
        .iter()
        .find(|item| item.semantics == "root-backed-up-value")
        .unwrap();
    let bounded = semantics
        .iter()
        .find(|item| item.semantics == "bounded-nonmate-root-plus-explicit-mate-class-distance")
        .unwrap();
    let leaf = semantics
        .iter()
        .find(|item| item.semantics == "qsearch-pv-leaf-value-reoriented-to-root")
        .unwrap();
    let target_semantics = if bounded.robust_normalized_mae
        <= root.robust_normalized_mae * 1.01 + 1e-12
        && bounded.mate_class_agreement >= 0.95
    {
        bounded.semantics.clone()
    } else if leaf.robust_normalized_mae < root.robust_normalized_mae * 0.95 {
        leaf.semantics.clone()
    } else {
        root.semantics.clone()
    };

    let legacy_audit = transform_audit(
        "legacy-score-over-410",
        &tuning_reference_scores,
        contract.score_calibration.legacy_logistic_scale,
        &contract.score_calibration,
    );
    let calibrated_audit = transform_audit(
        "anhoku-deep-reference-p95-logistic-v1",
        &tuning_reference_scores,
        logistic_scale,
        &contract.score_calibration,
    );
    let calibrated_saturation =
        calibrated_audit.below_low_fraction + calibrated_audit.above_high_fraction;
    let use_logistic = calibrated_saturation
        <= contract
            .score_calibration
            .maximum_calibrated_saturation_fraction
        && selected.overall.probability_rmse
            <= contract
                .decision_boundaries
                .maximum_overall_probability_rmse;
    let primary_loss = if use_logistic {
        "calibrated-logistic-mse-v1"
    } else {
        "robust-clipped-huber-v1"
    };
    let transform = if use_logistic {
        "anhoku-deep-reference-p95-logistic-v1"
    } else {
        "identity-with-robust-clip-v1"
    };

    let labels_per_second = selected.overall.labels_per_second;
    let forecasts = contract
        .cost_model
        .forecast_corpus_sizes
        .iter()
        .map(|&positions| {
            let seconds = positions as f64 / labels_per_second;
            let hours = seconds / 3600.0 * contract.cost_model.worker_count as f64;
            CostForecast {
                positions,
                projected_wall_seconds_one_worker: seconds,
                projected_core_hours: hours,
                projected_cpu_cost_usd: hours * contract.cost_model.cpu_core_hour_usd,
            }
        })
        .collect::<Vec<_>>();

    let mut artifacts = BTreeMap::new();
    for (name, path) in [
        ("contract", contract_path.as_path()),
        ("probeSource", source_path.as_path()),
        (
            "supplementalProbeSource",
            supplemental_source_path.as_path(),
        ),
        ("r2GpuGate", r2_path.as_path()),
        ("c16Model", model_path.as_path()),
        ("rawResults", raw_path.as_path()),
        ("executable", args.executable_path),
    ] {
        artifacts.insert(name.to_string(), artifact(path)?);
    }
    let mut gates = BTreeMap::new();
    gates.insert("r2Authorized".to_string(), true);
    gates.insert("protocolFrozenBeforeResults".to_string(), true);
    gates.insert(
        "probeWithin50000BoardCeiling".to_string(),
        raw.len() <= 50_000,
    );
    gates.insert(
        "tuningSelectedAffordableTeacher".to_string(),
        selected.passes_boundary,
    );
    gates.insert(
        "oneShotDecisionBoundary".to_string(),
        decision_selected.passes_boundary,
    );
    gates.insert(
        "c16DisagreementStratumPresent".to_string(),
        disagreements
            >= contract
                .decision_boundaries
                .minimum_c16_handcrafted_disagreements,
    );
    gates.insert(
        "targetSemanticsFrozen".to_string(),
        contract
            .target_comparison
            .semantics
            .contains(&target_semantics),
    );
    gates.insert("scoreObjectiveFrozen".to_string(), true);
    gates.insert(
        "teacherOnlyLossFrozen".to_string(),
        contract.score_calibration.strength_training_lambda == 1.0
            && !contract.score_calibration.result_loss_enabled,
    );
    gates.insert(
        "throughputCostForecastPublished".to_string(),
        labels_per_second.is_finite() && labels_per_second > 0.0 && !forecasts.is_empty(),
    );
    gates.insert(
        "noProductionCorpus".to_string(),
        !contract.cost_model.production_corpus_authorized,
    );
    gates.insert(
        "noStrengthGames".to_string(),
        !contract.cost_model.strength_games_authorized,
    );
    let passed = gates.values().all(|value| *value);

    let report = R3Report {
        schema: "haitaka-anhoku-r3-gate-v1",
        schema_version: 1,
        phase: "R3",
        passed,
        protocol_frozen_before_results: true,
        probe_schema: contract.probe.schema.clone(),
        probe_selection_version: contract.probe.selection_version.clone(),
        unique_boards: raw.len(),
        tuning_boards: tuning.len(),
        decision_boards: decision.len(),
        decision_opened_once: true,
        artifacts,
        selected_teacher: contract.teachers.label_evaluator.clone(),
        selected_node_budget: selected_budget,
        node_counting_version: contract.teachers.node_counting_version.clone(),
        label_evaluator: contract.teachers.label_evaluator.clone(),
        target_semantics,
        mate_policy: "explicit-class-win-loss-none-plus-distance-from-30000-v1".to_string(),
        score_transform: transform.to_string(),
        score_transform_scale: use_logistic.then_some(logistic_scale),
        robust_score_clip: robust_clip,
        primary_loss: primary_loss.to_string(),
        strength_training_lambda: contract.score_calibration.strength_training_lambda,
        result_loss_enabled: contract.score_calibration.result_loss_enabled,
        tuning_candidates,
        decision_candidates,
        c16_diagnostic_tuning: c16_tuning,
        c16_diagnostic_decision: c16_decision,
        target_semantics_comparison: semantics,
        legacy_transform_audit: legacy_audit,
        calibrated_transform_audit: calibrated_audit,
        c16_handcrafted_disagreements: disagreements,
        throughput_labels_per_second: labels_per_second,
        cost_forecast: forecasts,
        gates,
        r4_authorized: passed,
        production_corpus_generated: false,
        strength_games_played: false,
    };
    write_json(&args.output_dir.join("r3-gate-report.json"), &report)?;
    Ok(report)
}

fn validate_contract(contract: &Contract) -> Result<()> {
    ensure!(contract.schema == "haitaka-r3-teacher-probe-contract-v1");
    ensure!(contract.ruleset == "anhoku");
    ensure!(contract.probe.schema == "teacher-probe-v1");
    ensure!(contract.teachers.node_counting_version == SEARCH_NODE_COUNTING_VERSION);
    ensure!(contract.target_comparison.mate_score_threshold == SEARCH_MATE_SCORE_THRESHOLD);
    ensure!(!contract.teachers.candidate_node_budgets.is_empty());
    ensure!(
        contract
            .teachers
            .candidate_node_budgets
            .windows(2)
            .all(|w| w[0] < w[1])
    );
    ensure!(
        contract.teachers.deep_reference_nodes as f64
            / *contract.teachers.candidate_node_budgets.last().unwrap() as f64
            >= contract.teachers.minimum_deep_to_candidate_ratio
    );
    ensure!(contract.teachers.maximum_depth > 0);
    ensure!(!contract.teachers.diagnostic_control.eligible_for_selection);
    ensure!(contract.teachers.external_teacher.is_none());
    ensure!(
        contract.probe.tuning_positions + contract.probe.decision_positions
            <= contract.probe.maximum_unique_boards
    );
    ensure!(contract.decision_boundaries.decision_uses_same_boundaries);
    ensure!(contract.cost_model.worker_count == 1);
    ensure!(contract.score_calibration.huber_delta_normalized > 0.0);
    ensure!(!contract.cost_model.production_corpus_authorized);
    ensure!(!contract.cost_model.strength_games_authorized);
    Ok(())
}

fn read_corpus(path: &Path) -> Result<Vec<CorpusRecord>> {
    fs::read_to_string(path)?
        .lines()
        .enumerate()
        .map(|(index, line)| {
            serde_json::from_str(line)
                .with_context(|| format!("invalid corpus JSON at line {}", index + 1))
        })
        .collect()
}

fn read_supplemental_corpus(path: &Path) -> Result<Vec<CorpusRecord>> {
    let manifest: SupplementalManifest = read_json(path)?;
    manifest
        .games
        .into_iter()
        .map(|game| {
            let board = Board::from_sfen(&game.sfen).map_err(anyhow::Error::msg)?;
            let side = board.side_to_move();
            let proxy = material(&board, side) - material(&board, !side);
            Ok(CorpusRecord {
                id: format!("phase8d-opening-{}", game.game_id),
                sfen: game.sfen,
                seed_class: None,
                label_score_side_to_move: proxy,
                origin: "anhoku-v3-suite".to_string(),
            })
        })
        .collect()
}

fn select_probe(records: &[CorpusRecord], contract: &Contract) -> Result<Vec<ProbePosition>> {
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    for record in records {
        if !seen.insert(record.sfen.clone()) {
            continue;
        }
        let board = Board::from_sfen(&record.sfen)
            .map_err(|err| anyhow::anyhow!("invalid probe board {}: {err}", record.id))?;
        if board.status() != GameStatus::Ongoing
            || !board.has(Color::Black, Piece::King)
            || !board.has(Color::White, Piece::King)
        {
            continue;
        }
        let legal = legal_moves(&board);
        if legal.is_empty() {
            continue;
        }
        let terminal_adjacent = legal.len() <= 1;
        let phase =
            if terminal_adjacent || record.seed_class.as_deref() == Some("terminal-adjacent") {
                "terminal-adjacent"
            } else if board.move_number() <= 8 {
                "early"
            } else if board.move_number() <= 32 {
                "middle"
            } else {
                "late"
            };
        let mut types = Vec::new();
        if board.checkers().is_empty() {
            types.push("quiet".to_string());
        } else {
            types.push("in-check".to_string());
        }
        let has_capture = legal.iter().any(|mv| board.piece_on(mv.to()).is_some());
        if has_capture || !board.checkers().is_empty() {
            types.push("tactical".to_string());
        }
        if legal.iter().any(Move::is_promotion) {
            types.push("promotion".to_string());
        }
        if legal.iter().any(Move::is_drop) {
            types.push("drop".to_string());
        }
        if record.seed_class.as_deref() == Some("donor-receiver") {
            types.push("donor-sensitive".to_string());
        }
        let material = material_bucket(&board);
        let proxy_score = score_bucket(record.label_score_side_to_move, 80);
        let source = if record.origin == "anhoku-v3-suite" {
            "anhoku-v3-suite"
        } else if record.seed_class.is_some() {
            "curated-fixture"
        } else {
            "startpos-tree"
        };
        let policy = if record.origin == "anhoku-v3-suite" {
            "historical-suite-root-v3"
        } else if record.seed_class.is_some() {
            "curated-fixture-v1"
        } else {
            "deterministic-legal-enumeration-v1"
        };
        let subset = if stable_hash(contract.probe.split_seed, &record.id)[0] & 1 == 0 {
            "tuning"
        } else {
            "decision"
        };
        candidates.push(ProbePosition {
            id: record.id.clone(),
            subset: subset.to_string(),
            sfen: record.sfen.clone(),
            source: source.to_string(),
            trajectory_policy: policy.to_string(),
            phase: phase.to_string(),
            material: material.to_string(),
            proxy_score: proxy_score.to_string(),
            position_types: types,
            proxy_score_value: record.label_score_side_to_move,
        });
    }
    let mut selected = Vec::new();
    for (subset, count) in [
        ("tuning", contract.probe.tuning_positions),
        ("decision", contract.probe.decision_positions),
    ] {
        let pool = candidates
            .iter()
            .filter(|item| item.subset == subset)
            .cloned()
            .collect::<Vec<_>>();
        selected.extend(greedy_select(pool, count, subset, contract)?);
    }
    let unique = selected
        .iter()
        .map(|item| item.sfen.as_str())
        .collect::<BTreeSet<_>>();
    ensure!(
        unique.len() == selected.len(),
        "R3 probe contains duplicate boards"
    );
    let schedule = selected
        .iter()
        .map(|position| position.id.as_str())
        .collect::<Vec<_>>()
        .join(",");
    ensure!(
        format!("{:x}", Sha256::digest(schedule.as_bytes()))
            == contract.probe.selected_id_schedule_sha256,
        "R3 selected probe schedule does not match the frozen identity"
    );
    Ok(selected)
}

fn required_keys(contract: &Contract) -> Vec<String> {
    let required = &contract.probe.required_strata;
    required
        .source
        .iter()
        .map(|v| format!("source={v}"))
        .chain(required.phase.iter().map(|v| format!("phase={v}")))
        .chain(required.material.iter().map(|v| format!("material={v}")))
        .chain(
            required
                .proxy_score
                .iter()
                .map(|v| format!("proxy-score={v}")),
        )
        .chain(
            required
                .position_type
                .iter()
                .map(|v| format!("position-type={v}")),
        )
        .collect()
}

fn selection_keys(position: &ProbePosition) -> Vec<String> {
    let mut keys = vec![
        format!("source={}", position.source),
        format!("phase={}", position.phase),
        format!("material={}", position.material),
        format!("proxy-score={}", position.proxy_score),
    ];
    keys.extend(
        position
            .position_types
            .iter()
            .map(|v| format!("position-type={v}")),
    );
    keys
}

fn greedy_select(
    mut pool: Vec<ProbePosition>,
    count: usize,
    subset: &str,
    contract: &Contract,
) -> Result<Vec<ProbePosition>> {
    let required = required_keys(contract);
    let minimum = contract.probe.minimum_per_required_stratum_per_subset;
    let mut counts = BTreeMap::<String, usize>::new();
    let mut selected = Vec::new();
    while selected.len() < count {
        let best = pool
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                let score = |position: &ProbePosition| {
                    selection_keys(position)
                        .iter()
                        .filter(|key| {
                            required.contains(key)
                                && counts.get(*key).copied().unwrap_or(0) < minimum
                        })
                        .count()
                };
                score(a).cmp(&score(b)).then_with(|| {
                    stable_hash(contract.probe.split_seed ^ 0xa53, &b.id)
                        .cmp(&stable_hash(contract.probe.split_seed ^ 0xa53, &a.id))
                })
            })
            .map(|(index, _)| index)
            .context("not enough eligible positions for R3 probe")?;
        let position = pool.swap_remove(best);
        for key in selection_keys(&position) {
            *counts.entry(key).or_default() += 1;
        }
        selected.push(position);
    }
    for key in required {
        ensure!(
            counts.get(&key).copied().unwrap_or(0) >= minimum,
            "R3 {subset} subset does not cover required stratum {key}"
        );
    }
    selected.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(selected)
}

fn run_position(
    probe: ProbePosition,
    contract: &Contract,
    c16: Arc<NnueModel>,
) -> Result<RawPositionResult> {
    let board = Board::from_sfen(&probe.sfen).map_err(anyhow::Error::msg)?;
    let mut workspace = SearchWorkspace::new();
    let deep_reference = handcrafted_observation(
        "handcrafted-deep-reference",
        &board,
        contract.teachers.deep_reference_nodes,
        contract.teachers.maximum_depth,
        &mut workspace,
    )?;
    ensure!(
        !deep_reference.incomplete_label,
        "deep reference incomplete for {}",
        probe.id
    );
    let mut candidates = Vec::new();
    for &nodes in &contract.teachers.candidate_node_budgets {
        candidates.push(handcrafted_observation(
            &format!("handcrafted-{nodes}"),
            &board,
            nodes,
            contract.teachers.maximum_depth,
            &mut workspace,
        )?);
    }
    let summary = search_board_impl_with_eval_mode_and_node_budget_in_workspace(
        &board,
        contract.teachers.diagnostic_control.nodes,
        contract.teachers.maximum_depth,
        c16,
        SearchEvalMode::Incremental,
        &mut workspace,
    )
    .map_err(anyhow::Error::msg)?;
    let c16_diagnostic = observation_from_summary(
        contract.teachers.diagnostic_control.name.clone(),
        summary,
        None,
    );
    let ranking = reference_move_ranking(&board, contract)?;
    Ok(RawPositionResult {
        probe,
        deep_reference,
        handcrafted_candidates: candidates,
        c16_diagnostic,
        reference_move_ranking: ranking,
    })
}

fn handcrafted_observation(
    teacher: &str,
    board: &Board,
    nodes: u64,
    max_depth: u8,
    workspace: &mut SearchWorkspace,
) -> Result<SearchObservation> {
    let (summary, trace) =
        search_board_impl_handcrafted_with_node_budget_and_training_trace_in_workspace(
            board, nodes, max_depth, workspace,
        )
        .map_err(anyhow::Error::msg)?;
    Ok(observation_from_summary(
        teacher.to_string(),
        summary,
        trace,
    ))
}

fn observation_from_summary(
    teacher: String,
    summary: haitaka_wasm::NodeBudgetSearchSummary,
    trace: Option<SearchTrainingTrace>,
) -> SearchObservation {
    let root_score = summary.best_score;
    let leaf = trace.map(|trace| {
        if trace.root_ply_distance % 2 == 0 {
            trace.static_eval
        } else {
            -trace.static_eval
        }
    });
    let (mate_class, mate_distance) = mate_target(root_score);
    SearchObservation {
        teacher,
        node_limit: summary.node_limit,
        completed_depth: summary.completed_depth,
        best_move: summary.best_move,
        root_score,
        qsearch_leaf_score_reoriented_to_root: leaf,
        mate_class,
        mate_distance,
        alpha_beta_nodes: summary.alpha_beta_nodes,
        qsearch_nodes: summary.qsearch_nodes,
        total_nodes: summary.total_nodes,
        node_accounting_exact: summary.alpha_beta_nodes.checked_add(summary.qsearch_nodes)
            == Some(summary.total_nodes)
            && summary.total_nodes <= summary.node_limit,
        incomplete_label: summary.best_score.is_none() || summary.completed_depth == 0,
        partial_iteration: summary.incomplete_iterations > 0,
        elapsed_ms: summary.elapsed_ms,
    }
}

fn reference_move_ranking(board: &Board, contract: &Contract) -> Result<Vec<RankedMove>> {
    let moves = legal_moves(board);
    ensure!(
        !moves.is_empty(),
        "reference move ranking requires a nonterminal root"
    );
    let per_move = (contract.teachers.move_ranking_total_nodes / moves.len() as u64).max(256);
    let mut ranking = Vec::with_capacity(moves.len());
    let mut workspace = SearchWorkspace::new();
    for mv in moves {
        let mut child = board.clone();
        child.play_unchecked(mv);
        let (score, child_node_limit) = match child.status() {
            GameStatus::Won => (Some(29_999), 0),
            GameStatus::Drawn => (Some(0), 0),
            GameStatus::Ongoing => {
                let summary = search_board_impl_handcrafted_with_node_budget_in_workspace(
                    &child,
                    per_move,
                    contract.teachers.maximum_depth,
                    &mut workspace,
                )
                .map_err(anyhow::Error::msg)?;
                (summary.best_score.map(|score| -score), per_move)
            }
        };
        if let Some(score) = score {
            ranking.push(RankedMove {
                move_: mv.to_string(),
                score,
                child_node_limit,
            });
        }
    }
    ranking.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.move_.cmp(&b.move_)));
    ensure!(!ranking.is_empty(), "reference move ranking is empty");
    Ok(ranking)
}

fn candidate(item: &RawPositionResult, nodes: u64) -> &SearchObservation {
    item.handcrafted_candidates
        .iter()
        .find(|candidate| candidate.node_limit == nodes)
        .expect("every candidate budget was run")
}

fn candidate_reports(
    items: &[&RawPositionResult],
    contract: &Contract,
    scale: f64,
    clip: f64,
    _decision_only: bool,
) -> Vec<CandidateReport> {
    contract
        .teachers
        .candidate_node_budgets
        .iter()
        .map(|&nodes| {
            report_for(
                items,
                contract,
                scale,
                clip,
                &format!("handcrafted-{nodes}"),
                |item| candidate(item, nodes),
            )
        })
        .collect()
}

fn diagnostic_report(
    items: &[&RawPositionResult],
    contract: &Contract,
    scale: f64,
    clip: f64,
    name: &str,
) -> CandidateReport {
    report_for(items, contract, scale, clip, name, |item| {
        &item.c16_diagnostic
    })
}

fn report_for<'a>(
    items: &[&'a RawPositionResult],
    contract: &Contract,
    scale: f64,
    clip: f64,
    teacher: &str,
    observation: impl Fn(&'a RawPositionResult) -> &'a SearchObservation,
) -> CandidateReport {
    let overall = compute_metrics(items, scale, clip, contract, &observation);
    let mut grouped = BTreeMap::<String, Vec<&RawPositionResult>>::new();
    for &item in items {
        let c16_disagrees = item.c16_diagnostic.best_move != item.deep_reference.best_move
            || sign(
                item.c16_diagnostic.root_score,
                contract.target_comparison.near_zero_score,
            ) != sign(
                item.deep_reference.root_score,
                contract.target_comparison.near_zero_score,
            );
        for stratum in item.probe.strata(
            item.deep_reference.root_score.unwrap_or(0),
            c16_disagrees,
            contract.target_comparison.near_zero_score,
        ) {
            grouped.entry(stratum).or_default().push(item);
        }
    }
    let by_stratum = grouped
        .into_iter()
        .map(|(key, values)| {
            let metrics = compute_metrics(&values, scale, clip, contract, &observation);
            (key, metrics)
        })
        .collect::<BTreeMap<_, _>>();
    let macro_average = average_metrics(by_stratum.values());
    let eligible = by_stratum
        .values()
        .filter(|metrics| {
            metrics.positions
                >= contract
                    .decision_boundaries
                    .minimum_stratum_count_for_worst_case
        })
        .collect::<Vec<_>>();
    let worst_probability = eligible
        .iter()
        .map(|metrics| metrics.probability_rmse)
        .fold(0.0, f64::max);
    let worst_regret = eligible
        .iter()
        .map(|metrics| metrics.regret_to_clip_ratio)
        .fold(0.0, f64::max);
    let boundary = &contract.decision_boundaries;
    let passes = overall.probability_rmse <= boundary.maximum_overall_probability_rmse
        && worst_probability <= boundary.maximum_worst_stratum_probability_rmse
        && overall.robust_normalized_mae <= boundary.maximum_overall_robust_normalized_mae
        && overall.top3_agreement >= boundary.minimum_top3_agreement
        && overall.regret_to_clip_ratio <= boundary.maximum_overall_regret_to_clip_ratio
        && worst_regret <= boundary.maximum_worst_stratum_regret_to_clip_ratio
        && overall.sign_disagreement_rate <= boundary.maximum_sign_disagreement_rate
        && overall.incomplete_label_rate <= boundary.maximum_incomplete_label_rate
        && overall.node_accounting_rate >= boundary.required_node_accounting_rate;
    CandidateReport {
        teacher: teacher.to_string(),
        node_budget: observation(items[0]).node_limit,
        overall,
        by_stratum,
        macro_average,
        worst_stratum_probability_rmse: worst_probability,
        worst_stratum_regret_to_clip_ratio: worst_regret,
        passes_boundary: passes,
    }
}

fn compute_metrics<'a>(
    items: &[&'a RawPositionResult],
    scale: f64,
    clip: f64,
    contract: &Contract,
    observation: impl Fn(&'a RawPositionResult) -> &'a SearchObservation,
) -> MetricSet {
    let mut probability_squared = 0.0;
    let mut raw_absolute = 0.0;
    let mut robust_absolute = 0.0;
    let mut robust_huber = 0.0;
    let mut top1 = 0usize;
    let mut top3 = 0usize;
    let mut regret = 0.0;
    let mut sign_disagreements = 0usize;
    let mut sign_comparisons = 0usize;
    let mut incomplete = 0usize;
    let mut exact = 0usize;
    let mut mates = 0usize;
    let mut partial = 0usize;
    let mut clipped = 0usize;
    let mut elapsed = 0.0;
    let mut candidates = Vec::new();
    let mut references = Vec::new();
    for &item in items {
        let candidate = observation(item);
        let reference = item.deep_reference.root_score.unwrap_or(0);
        let value = candidate.root_score.unwrap_or(0);
        let delta = f64::from(value - reference);
        probability_squared +=
            (sigmoid(f64::from(value) / scale) - sigmoid(f64::from(reference) / scale)).powi(2);
        raw_absolute += delta.abs();
        robust_absolute += (delta / clip).abs().min(1.0);
        robust_huber += huber(
            (delta / clip).clamp(-1.0, 1.0),
            contract.score_calibration.huber_delta_normalized,
        );
        top1 += usize::from(candidate.best_move == item.deep_reference.best_move);
        let top_moves = item
            .reference_move_ranking
            .iter()
            .take(3)
            .map(|entry| entry.move_.as_str())
            .collect::<Vec<_>>();
        top3 += usize::from(
            candidate
                .best_move
                .as_deref()
                .is_some_and(|mv| top_moves.contains(&mv)),
        );
        let best_ranked = item.reference_move_ranking[0].score;
        let selected_ranked = candidate
            .best_move
            .as_deref()
            .and_then(|mv| {
                item.reference_move_ranking
                    .iter()
                    .find(|entry| entry.move_ == mv)
            })
            .map(|entry| entry.score)
            .unwrap_or(best_ranked - clip as i32);
        regret += f64::from((best_ranked - selected_ranked).max(0));
        let ref_sign = sign(Some(reference), contract.target_comparison.near_zero_score);
        if ref_sign != 0 {
            sign_comparisons += 1;
            sign_disagreements += usize::from(
                sign(
                    candidate.root_score,
                    contract.target_comparison.near_zero_score,
                ) != ref_sign,
            );
        }
        incomplete += usize::from(candidate.incomplete_label);
        exact += usize::from(candidate.node_accounting_exact);
        mates += usize::from(reference.abs() >= contract.target_comparison.mate_score_threshold);
        partial += usize::from(candidate.partial_iteration);
        clipped += usize::from(reference.abs() as f64 > clip);
        elapsed += candidate.elapsed_ms / 1000.0;
        candidates.push(f64::from(value));
        references.push(f64::from(reference));
    }
    let n = items.len() as f64;
    let mean_regret = regret / n;
    MetricSet {
        positions: items.len(),
        probability_rmse: (probability_squared / n).sqrt(),
        raw_score_mae: raw_absolute / n,
        robust_normalized_mae: robust_absolute / n,
        robust_clipped_huber_loss: robust_huber / n,
        top1_agreement: top1 as f64 / n,
        top3_agreement: top3 as f64 / n,
        mean_regret,
        regret_to_clip_ratio: mean_regret / clip,
        sign_disagreement_rate: if sign_comparisons == 0 {
            0.0
        } else {
            sign_disagreements as f64 / sign_comparisons as f64
        },
        incomplete_label_rate: incomplete as f64 / n,
        node_accounting_rate: exact as f64 / n,
        mate_rate: mates as f64 / n,
        partial_iteration_rate: partial as f64 / n,
        clipping_rate: clipped as f64 / n,
        labels_per_second: if elapsed > 0.0 { n / elapsed } else { 0.0 },
        pearson: correlation(&candidates, &references),
        spearman: spearman(&candidates, &references),
    }
}

fn average_metrics<'a>(metrics: impl Iterator<Item = &'a MetricSet>) -> MetricSet {
    let values = metrics.collect::<Vec<_>>();
    let n = values.len() as f64;
    if values.is_empty() {
        return MetricSet::default();
    }
    let avg = |f: fn(&MetricSet) -> f64| values.iter().map(|value| f(value)).sum::<f64>() / n;
    MetricSet {
        positions: values.len(),
        probability_rmse: avg(|v| v.probability_rmse),
        raw_score_mae: avg(|v| v.raw_score_mae),
        robust_normalized_mae: avg(|v| v.robust_normalized_mae),
        robust_clipped_huber_loss: avg(|v| v.robust_clipped_huber_loss),
        top1_agreement: avg(|v| v.top1_agreement),
        top3_agreement: avg(|v| v.top3_agreement),
        mean_regret: avg(|v| v.mean_regret),
        regret_to_clip_ratio: avg(|v| v.regret_to_clip_ratio),
        sign_disagreement_rate: avg(|v| v.sign_disagreement_rate),
        incomplete_label_rate: avg(|v| v.incomplete_label_rate),
        node_accounting_rate: avg(|v| v.node_accounting_rate),
        mate_rate: avg(|v| v.mate_rate),
        partial_iteration_rate: avg(|v| v.partial_iteration_rate),
        clipping_rate: avg(|v| v.clipping_rate),
        labels_per_second: avg(|v| v.labels_per_second),
        pearson: optional_average(values.iter().filter_map(|v| v.pearson)),
        spearman: optional_average(values.iter().filter_map(|v| v.spearman)),
    }
}

fn target_semantics_reports(
    items: &[&RawPositionResult],
    selected_budget: u64,
    scale: f64,
    clip: f64,
    contract: &Contract,
) -> Vec<TargetSemanticsReport> {
    contract
        .target_comparison
        .semantics
        .iter()
        .map(|semantics| {
            let mut robust = 0.0;
            let mut robust_huber = 0.0;
            let mut prob = 0.0;
            let mut mate_agreement = 0usize;
            for item in items {
                let selected = candidate(item, selected_budget);
                let reference_root = item.deep_reference.root_score.unwrap_or(0);
                let candidate_root = selected.root_score.unwrap_or(0);
                let (candidate_score, reference_score) = match semantics.as_str() {
                    "qsearch-pv-leaf-value-reoriented-to-root" => (
                        selected
                            .qsearch_leaf_score_reoriented_to_root
                            .unwrap_or(candidate_root),
                        item.deep_reference
                            .qsearch_leaf_score_reoriented_to_root
                            .unwrap_or(reference_root),
                    ),
                    "bounded-nonmate-root-plus-explicit-mate-class-distance" => (
                        candidate_root.clamp(-(clip as i32), clip as i32),
                        reference_root.clamp(-(clip as i32), clip as i32),
                    ),
                    _ => (candidate_root, reference_root),
                };
                let normalized =
                    (f64::from(candidate_score - reference_score) / clip).clamp(-1.0, 1.0);
                robust += normalized.abs();
                robust_huber += huber(
                    normalized,
                    contract.score_calibration.huber_delta_normalized,
                );
                prob += (sigmoid(f64::from(candidate_score) / scale)
                    - sigmoid(f64::from(reference_score) / scale))
                .powi(2);
                mate_agreement +=
                    usize::from(selected.mate_class == item.deep_reference.mate_class);
            }
            let n = items.len() as f64;
            TargetSemanticsReport {
                semantics: semantics.clone(),
                robust_normalized_mae: robust / n,
                robust_clipped_huber_loss: robust_huber / n,
                probability_rmse: (prob / n).sqrt(),
                mate_class_agreement: mate_agreement as f64 / n,
            }
        })
        .collect()
}

fn transform_audit(
    name: &str,
    scores: &[i32],
    scale: f64,
    contract: &ScoreContract,
) -> TransformAudit {
    let mut grouped = BTreeMap::<String, Vec<i32>>::new();
    for &score in scores {
        grouped
            .entry(score_bucket(score, 100).to_string())
            .or_default()
            .push(score);
    }
    let total_gradient = scores
        .iter()
        .map(|&score| logistic_gradient(score, scale))
        .sum::<f64>();
    let buckets = grouped
        .into_iter()
        .map(|(bucket, values)| {
            let gradients = values
                .iter()
                .map(|&score| logistic_gradient(score, scale))
                .sum::<f64>();
            SaturationBucket {
                bucket,
                positions: values.len(),
                below_low: values
                    .iter()
                    .filter(|&&score| sigmoid(f64::from(score) / scale) < contract.saturation_low)
                    .count(),
                above_high: values
                    .iter()
                    .filter(|&&score| sigmoid(f64::from(score) / scale) > contract.saturation_high)
                    .count(),
                mean_gradient_magnitude: gradients / values.len() as f64,
                gradient_contribution_fraction: if total_gradient > 0.0 {
                    gradients / total_gradient
                } else {
                    0.0
                },
            }
        })
        .collect();
    let n = scores.len() as f64;
    TransformAudit {
        name: name.to_string(),
        scale,
        below_low_fraction: scores
            .iter()
            .filter(|&&score| sigmoid(f64::from(score) / scale) < contract.saturation_low)
            .count() as f64
            / n,
        above_high_fraction: scores
            .iter()
            .filter(|&&score| sigmoid(f64::from(score) / scale) > contract.saturation_high)
            .count() as f64
            / n,
        mean_gradient_magnitude: total_gradient / n,
        by_score_bucket: buckets,
    }
}

fn calibrated_scale(scores: &[i32], contract: &ScoreContract) -> f64 {
    let target = quantile_abs(scores, contract.calibrated_logistic_quantile);
    let logit = (contract.calibrated_logistic_quantile_probability
        / (1.0 - contract.calibrated_logistic_quantile_probability))
        .ln();
    (target / logit).clamp(
        contract.minimum_logistic_scale,
        contract.maximum_logistic_scale,
    )
}

fn robust_clip(scores: &[i32], contract: &ScoreContract) -> f64 {
    quantile_abs(scores, contract.robust_clip_quantile)
        .clamp(contract.minimum_robust_clip, contract.maximum_robust_clip)
}

fn quantile_abs(scores: &[i32], quantile: f64) -> f64 {
    let mut values = scores
        .iter()
        .map(|score| score.unsigned_abs())
        .collect::<Vec<_>>();
    values.sort_unstable();
    let index = ((values.len() - 1) as f64 * quantile).ceil() as usize;
    f64::from(values[index])
}

fn logistic_gradient(score: i32, scale: f64) -> f64 {
    let probability = sigmoid(f64::from(score) / scale);
    probability * (1.0 - probability) / scale
}

fn sigmoid(value: f64) -> f64 {
    if value >= 0.0 {
        1.0 / (1.0 + (-value).exp())
    } else {
        let exp = value.exp();
        exp / (1.0 + exp)
    }
}

fn huber(value: f64, delta: f64) -> f64 {
    let absolute = value.abs();
    if absolute <= delta {
        0.5 * value * value
    } else {
        delta * (absolute - 0.5 * delta)
    }
}

fn mate_target(score: Option<i32>) -> (String, Option<i32>) {
    match score {
        Some(value) if value >= SEARCH_MATE_SCORE_THRESHOLD => {
            ("win".to_string(), Some(30_000 - value.abs()))
        }
        Some(value) if value <= -SEARCH_MATE_SCORE_THRESHOLD => {
            ("loss".to_string(), Some(30_000 - value.abs()))
        }
        _ => ("none".to_string(), None),
    }
}

fn sign(score: Option<i32>, near_zero: i32) -> i8 {
    match score.unwrap_or(0) {
        value if value > near_zero => 1,
        value if value < -near_zero => -1,
        _ => 0,
    }
}

fn score_bucket(score: i32, near_zero: i32) -> &'static str {
    match score.abs() {
        value if value <= near_zero => "near-zero",
        value if value <= 1000 => "moderate",
        _ => "large",
    }
}

fn material_bucket(board: &Board) -> &'static str {
    let difference = (material(board, Color::Black) - material(board, Color::White)).abs();
    if difference <= 200 {
        "balanced"
    } else if difference <= 1000 {
        "imbalanced"
    } else {
        "extreme"
    }
}

fn material(board: &Board, color: Color) -> i32 {
    Piece::ALL
        .iter()
        .map(|&piece| {
            let value = match piece {
                Piece::Pawn => 100,
                Piece::Lance | Piece::Knight => 300,
                Piece::Silver => 400,
                Piece::Gold => 500,
                Piece::Bishop => 700,
                Piece::Rook => 800,
                Piece::King => 0,
                Piece::Tokin | Piece::PLance | Piece::PKnight | Piece::PSilver => 550,
                Piece::PBishop => 900,
                Piece::PRook => 1000,
            };
            value
                * (board.colored_pieces(color, piece).len() as i32
                    + i32::from(board.num_in_hand(color, piece)))
        })
        .sum()
}

fn legal_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|piece_moves| {
        moves.extend(piece_moves);
        false
    });
    moves
}

fn correlation(a: &[f64], b: &[f64]) -> Option<f64> {
    if a.len() < 2 || a.len() != b.len() {
        return None;
    }
    let mean_a = a.iter().sum::<f64>() / a.len() as f64;
    let mean_b = b.iter().sum::<f64>() / b.len() as f64;
    let numerator = a
        .iter()
        .zip(b)
        .map(|(x, y)| (x - mean_a) * (y - mean_b))
        .sum::<f64>();
    let denom_a = a.iter().map(|x| (x - mean_a).powi(2)).sum::<f64>();
    let denom_b = b.iter().map(|x| (x - mean_b).powi(2)).sum::<f64>();
    let denominator = (denom_a * denom_b).sqrt();
    (denominator > 0.0).then_some(numerator / denominator)
}

fn spearman(a: &[f64], b: &[f64]) -> Option<f64> {
    correlation(&ranks(a), &ranks(b))
}

fn ranks(values: &[f64]) -> Vec<f64> {
    let mut order = values.iter().copied().enumerate().collect::<Vec<_>>();
    order.sort_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let mut result = vec![0.0; values.len()];
    let mut begin = 0;
    while begin < order.len() {
        let mut end = begin + 1;
        while end < order.len() && order[end].1 == order[begin].1 {
            end += 1;
        }
        let rank = (begin + end - 1) as f64 / 2.0;
        for &(index, _) in &order[begin..end] {
            result[index] = rank;
        }
        begin = end;
    }
    result
}

fn optional_average(values: impl Iterator<Item = f64>) -> Option<f64> {
    let values = values.collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn stable_hash(seed: u64, value: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(seed.to_le_bytes());
    hasher.update(value.as_bytes());
    hasher.finalize().into()
}

fn resolve(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn validate_sha256(path: &Path, expected: &str, label: &str) -> Result<()> {
    let actual = sha256_file(path)?;
    ensure!(
        actual == expected,
        "{label} hash mismatch for {}: expected {expected}, got {actual}",
        path.display()
    );
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(
            fs::read(path).with_context(|| format!("failed to read {}", path.display()))?
        )
    ))
}

fn artifact(path: &Path) -> Result<ArtifactIdentity> {
    Ok(ArtifactIdentity {
        path: path.display().to_string(),
        bytes: fs::metadata(path)?.len(),
        sha256: sha256_file(path)?,
    })
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    serde_json::from_slice(
        &fs::read(path).with_context(|| format!("failed to read {}", path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", path.display()))
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

fn write_jsonl(path: &Path, values: &[impl Serialize]) -> Result<()> {
    let mut bytes = Vec::new();
    for value in values {
        serde_json::to_writer(&mut bytes, value)?;
        bytes.push(b'\n');
    }
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "anhoku")]
    fn reference_ranking_includes_immediate_mates_without_searching_terminal_children() {
        let board = Board::from_sfen("4k4/9/3LGL3/9/9/9/9/9/4K4 b R 1").unwrap();
        let mut contract = test_contract();
        contract.teachers.maximum_depth = 1;
        contract.teachers.move_ranking_total_nodes = 256;
        let mates = legal_moves(&board)
            .into_iter()
            .filter(|&mv| {
                let mut child = board.clone();
                child.play_unchecked(mv);
                child.status() == GameStatus::Won
            })
            .collect::<Vec<_>>();
        assert!(!mates.is_empty());
        let ranking = reference_move_ranking(&board, &contract).unwrap();
        assert_eq!(ranking[0].score, 29_999);
        for mv in mates {
            let ranked = ranking
                .iter()
                .find(|item| item.move_ == mv.to_string())
                .unwrap();
            assert_eq!(ranked.score, 29_999);
            assert_eq!(ranked.child_node_limit, 0);
            let mut terminal = board.clone();
            terminal.play_unchecked(mv);
            assert!(reference_move_ranking(&terminal, &contract).is_err());
        }
    }

    #[test]
    fn ranks_average_ties() {
        assert_eq!(ranks(&[10.0, 20.0, 20.0, 30.0]), vec![0.0, 1.5, 1.5, 3.0]);
    }

    #[test]
    fn mate_targets_are_explicit() {
        assert_eq!(mate_target(Some(29_997)), ("win".to_string(), Some(3)));
        assert_eq!(mate_target(Some(-29_996)), ("loss".to_string(), Some(4)));
        assert_eq!(mate_target(Some(100)), ("none".to_string(), None));
    }

    #[test]
    fn calibrated_scale_bounds_saturation() {
        let contract = ScoreContract {
            legacy_logistic_scale: 410.0,
            calibrated_logistic_quantile: 0.95,
            calibrated_logistic_quantile_probability: 0.99,
            minimum_logistic_scale: 410.0,
            maximum_logistic_scale: 10_000.0,
            robust_clip_quantile: 0.95,
            minimum_robust_clip: 1000.0,
            maximum_robust_clip: 12_000.0,
            huber_delta_normalized: 0.1,
            saturation_low: 0.01,
            saturation_high: 0.99,
            maximum_calibrated_saturation_fraction: 0.02,
            strength_training_lambda: 1.0,
            result_loss_enabled: false,
        };
        let scale = calibrated_scale(&[-5000, -1000, 0, 1000, 5000], &contract);
        assert!((410.0..=10_000.0).contains(&scale));
    }

    fn test_contract() -> Contract {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        read_json(&root.join("r0/anhoku-reboot/r3-teacher-probe-contract.json")).unwrap()
    }

    #[test]
    fn probe_selection_covers_required_strata_without_generated_artifacts() {
        let contract = test_contract();
        let required = &contract.probe.required_strata;
        for (subset, count) in [
            ("tuning", contract.probe.tuning_positions),
            ("decision", contract.probe.decision_positions),
        ] {
            let pool = (0..96)
                .map(|index| ProbePosition {
                    id: format!("unit-{subset}-{index:03}"),
                    subset: subset.to_string(),
                    sfen: haitaka::SFEN_STARTPOS.to_string(),
                    source: required.source[index % required.source.len()].clone(),
                    trajectory_policy: "unit-fixture".to_string(),
                    phase: required.phase[index % required.phase.len()].clone(),
                    material: required.material[index % required.material.len()].clone(),
                    proxy_score: required.proxy_score[index % required.proxy_score.len()].clone(),
                    position_types: vec![
                        required.position_type[index % required.position_type.len()].clone(),
                    ],
                    proxy_score_value: 0,
                })
                .collect::<Vec<_>>();
            let selected = greedy_select(pool.clone(), count, subset, &contract).unwrap();
            let keys = selected
                .iter()
                .flat_map(selection_keys)
                .collect::<BTreeSet<_>>();
            assert_eq!(selected.len(), count);
            for key in required_keys(&contract) {
                assert!(keys.contains(&key), "missing {subset} {key}");
            }
            let reversed = greedy_select(
                pool.iter().rev().cloned().collect(),
                count,
                subset,
                &contract,
            )
            .unwrap();
            assert_eq!(
                selected.iter().map(|p| &p.id).collect::<Vec<_>>(),
                reversed.iter().map(|p| &p.id).collect::<Vec<_>>()
            );
            let missing = pool
                .into_iter()
                .filter(|p| !p.position_types.contains(&"donor-sensitive".to_string()))
                .collect();
            assert!(greedy_select(missing, count, subset, &contract).is_err());
        }
    }

    #[test]
    fn probe_selection_checks_frozen_schedule_with_local_fixture() {
        let mut contract = test_contract();
        contract.probe.tuning_positions = 1;
        contract.probe.decision_positions = 1;
        contract.probe.maximum_unique_boards = 2;
        contract.probe.required_strata = RequiredStrata {
            source: vec!["startpos-tree".to_string()],
            phase: vec!["early".to_string()],
            material: vec!["balanced".to_string()],
            proxy_score: vec!["near-zero".to_string()],
            position_type: vec!["quiet".to_string()],
        };
        contract.probe.selected_id_schedule_sha256 =
            "327d89171bb64192301550b1f52e462693d66c9b58f71f8f394372002b769031".to_string();
        let startpos = Board::startpos();
        let mut child = startpos.clone();
        child.try_play("7g7f".parse().unwrap()).unwrap();
        let records = [("unit-probe-0", startpos), ("unit-probe-2", child)]
            .into_iter()
            .map(|(id, board)| CorpusRecord {
                id: id.to_string(),
                sfen: board.to_string(),
                seed_class: None,
                label_score_side_to_move: 0,
                origin: String::new(),
            })
            .collect::<Vec<_>>();
        let selected = select_probe(&records, &contract).unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|p| (p.subset.as_str(), p.id.as_str()))
                .collect::<Vec<_>>(),
            [("tuning", "unit-probe-2"), ("decision", "unit-probe-0")]
        );
        contract.probe.selected_id_schedule_sha256 = "invalid".to_string();
        assert!(select_probe(&records, &contract).is_err());
    }

    #[test]
    #[cfg(feature = "anhoku")]
    #[ignore = "requires the frozen R1-A and Phase 8D corpus artifacts under out/"]
    fn frozen_probe_selector_covers_every_required_stratum() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let contract: Contract =
            read_json(&root.join("r0/anhoku-reboot/r3-teacher-probe-contract.json")).unwrap();
        let mut records = read_corpus(&root.join(&contract.probe.source)).unwrap();
        records.extend(
            read_supplemental_corpus(&root.join(&contract.probe.supplemental_source)).unwrap(),
        );
        let selected = select_probe(&records, &contract).unwrap();
        assert_eq!(
            selected.len(),
            contract.probe.tuning_positions + contract.probe.decision_positions
        );
        for subset in ["tuning", "decision"] {
            let keys = selected
                .iter()
                .filter(|position| position.subset == subset)
                .flat_map(selection_keys)
                .collect::<BTreeSet<_>>();
            for required in required_keys(&contract) {
                assert!(keys.contains(&required), "missing {subset} {required}");
            }
        }
    }
}

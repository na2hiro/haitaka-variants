use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, anyhow, ensure};
use haitaka::Board;
use haitaka_wasm::{
    R1d1FixtureObservation, SEARCH_NODE_COUNTING_VERSION, SEARCH_ROOT_RESULT_SCHEMA,
    SearchQsearchStats, SearchRootResult, UsiSession, r1d1_forced_interruption_observations,
    search_impl_handcrafted, search_iterative_deepening,
    search_iterative_deepening_impl_with_dfpn_mode,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
struct Contract {
    schema: String,
    ruleset: String,
    scope: String,
    node_counting: NodeCountingContract,
    result_schema: ResultSchemaContract,
    fixture_driver: FixtureDriverContract,
    adapter_assertions: Vec<String>,
    forbidden_changes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct NodeCountingContract {
    version: String,
    formula: String,
    admission: String,
    exhaustion: String,
}

#[derive(Debug, Deserialize)]
struct ResultSchemaContract {
    schema: String,
    required_fields: Vec<String>,
    label_rule: String,
    play_rule: String,
    terminal_rule: String,
}

#[derive(Debug, Deserialize)]
struct FixtureDriverContract {
    schema: String,
    position: String,
    evaluator: String,
    determinism: String,
    cases: Vec<ContractCase>,
}

#[derive(Debug, Deserialize)]
struct ContractCase {
    id: String,
    trigger: String,
    expected_completed_depth: Option<u8>,
    expected_completed_root_moves: Option<u32>,
    expected_play_move_was_searched: Option<bool>,
    expected_partial_root_state: Option<bool>,
    expected_emergency_fallback_used: Option<bool>,
    expected_completed_value: Option<String>,
    expected_qnodes: Option<u64>,
    expected_qsearch_max_ply: Option<u8>,
    requested_nodes: Option<u64>,
    expected_consumed_nodes: Option<u64>,
    expected_alpha_beta_nodes: Option<u64>,
    expected_qsearch_nodes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct Fixtures {
    schema: String,
    ruleset: String,
    position: String,
    evaluator: String,
    cases: Vec<FixtureCase>,
}

#[derive(Debug, Deserialize)]
struct FixtureCase {
    id: String,
    trigger: String,
    requested_nodes: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RootResultTrace {
    schema: &'static str,
    play_move_best_so_far: Option<String>,
    play_move_was_searched: bool,
    last_completed_iteration_value: Option<i32>,
    completed_iteration_depth: u8,
    completed_root_moves_in_interrupted_iteration: u32,
    partial_root_state: bool,
    interruption_reason: &'static str,
    emergency_fallback_used: bool,
    missing_move: bool,
}

impl From<&SearchRootResult> for RootResultTrace {
    fn from(value: &SearchRootResult) -> Self {
        Self {
            schema: SEARCH_ROOT_RESULT_SCHEMA,
            play_move_best_so_far: value.play_move_best_so_far.clone(),
            play_move_was_searched: value.play_move_was_searched,
            last_completed_iteration_value: value.last_completed_iteration_value,
            completed_iteration_depth: value.completed_iteration_depth,
            completed_root_moves_in_interrupted_iteration: value
                .completed_root_moves_in_interrupted_iteration,
            partial_root_state: value.partial_root_state,
            interruption_reason: value.interruption_reason.as_str(),
            emergency_fallback_used: value.emergency_fallback_used,
            missing_move: value.missing_move,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct QsearchTrace {
    qnodes: u64,
    qsearch_max_ply: u8,
    qsearch_cap_hits: u64,
    qsearch_check_move_tries: u64,
    qsearch_delta_prunes: u64,
}

impl From<SearchQsearchStats> for QsearchTrace {
    fn from(value: SearchQsearchStats) -> Self {
        Self {
            qnodes: value.qnodes,
            qsearch_max_ply: value.qsearch_max_ply,
            qsearch_cap_hits: value.qsearch_cap_hits,
            qsearch_check_move_tries: value.qsearch_check_move_tries,
            qsearch_delta_prunes: value.qsearch_delta_prunes,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservationTrace {
    id: String,
    root_result: RootResultTrace,
    alpha_beta_nodes: u64,
    qsearch: QsearchTrace,
    consumed_nodes: u64,
    requested_nodes: Option<u64>,
    node_budget_cap_hits: u64,
    training_trace_present: bool,
    legal_play_move: bool,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct AdapterParity {
    fixed_depth_equals_iterative: bool,
    usi_node_budget_equals_in_process: bool,
    usi_publishes_complete_schema: bool,
    usi_bestmove_equals_play_move: bool,
    production_aliases_are_exact: bool,
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
pub(crate) struct R1d1Report {
    schema: &'static str,
    schema_version: u32,
    ruleset: &'static str,
    node_counting_version: &'static str,
    result_schema: &'static str,
    observations: Vec<ObservationTrace>,
    adapter_parity: AdapterParity,
    artifacts: BTreeMap<String, ArtifactIdentity>,
    gates: BTreeMap<String, bool>,
    pub(crate) passed: bool,
}

pub(crate) struct RunArgs<'a> {
    pub r1a_dir: &'a Path,
    pub r1b_dir: &'a Path,
    pub r1c_dir: &'a Path,
    pub output_dir: &'a Path,
    pub contract_path: &'a Path,
    pub fixtures_path: &'a Path,
    pub workspace_root: &'a Path,
}

pub(crate) fn run(args: RunArgs<'_>) -> Result<R1d1Report> {
    let contract: Contract = read_json(args.contract_path)?;
    let fixtures: Fixtures = read_json(args.fixtures_path)?;
    validate_contract(&contract, &fixtures)?;
    for (path, phase) in [
        (args.r1a_dir.join("r1a-gate-report.json"), "R1-A"),
        (args.r1b_dir.join("r1b-gate-report.json"), "R1-B"),
        (args.r1c_dir.join("r1c-gate-report.json"), "R1-C"),
    ] {
        ensure_passing_report(&path, phase)?;
    }

    fs::create_dir_all(args.output_dir)
        .with_context(|| format!("failed to create {}", args.output_dir.display()))?;
    let board = Board::from_sfen(haitaka::SFEN_STARTPOS)
        .map_err(|err| anyhow!("failed to parse startpos: {err}"))?;
    let legal_moves = legal_move_strings(&board);
    let depth_one =
        search_impl_handcrafted(haitaka::SFEN_STARTPOS, 1).map_err(anyhow::Error::msg)?;
    let raw = r1d1_forced_interruption_observations().map_err(anyhow::Error::msg)?;
    let raw_by_id = raw
        .iter()
        .map(|observation| (observation.id, observation))
        .collect::<BTreeMap<_, _>>();
    ensure!(raw_by_id.len() == fixtures.cases.len());

    let mut observations = Vec::new();
    let mut exact_assertions = true;
    let mut all_nonterminal_moves_legal = true;
    let mut node_accounting_exact = true;
    let mut qsearch_telemetry_complete = true;
    let mut partial_values_excluded = true;
    for expected in &contract.fixture_driver.cases {
        let observation = raw_by_id
            .get(expected.id.as_str())
            .with_context(|| format!("missing R1-D1 observation {}", expected.id))?;
        let legal = observation
            .root_result
            .play_move_best_so_far
            .as_ref()
            .is_some_and(|mv| legal_moves.contains(mv));
        all_nonterminal_moves_legal &= legal && !observation.root_result.missing_move;
        exact_assertions &= case_matches(expected, observation, depth_one.best_score);
        exact_assertions &= if observation.root_result.emergency_fallback_used {
            !observation.root_result.play_move_was_searched
                && observation
                    .root_result
                    .completed_root_moves_in_interrupted_iteration
                    == 0
        } else {
            observation
                .root_result
                .completed_root_moves_in_interrupted_iteration
                == 0
                || observation.root_result.play_move_was_searched
        };
        node_accounting_exact &= observation.consumed_nodes
            == observation
                .alpha_beta_nodes
                .saturating_add(observation.qsearch_stats.qnodes)
            && observation
                .requested_nodes
                .is_none_or(|requested| observation.consumed_nodes <= requested);
        qsearch_telemetry_complete &=
            observation.qsearch_stats.qnodes == 0 || observation.qsearch_stats.qsearch_max_ply <= 8;
        partial_values_excluded &= observation.root_result.completed_iteration_depth > 0
            || (observation
                .root_result
                .last_completed_iteration_value
                .is_none()
                && !observation.training_trace_present);
        observations.push(observation_trace(observation, legal));
    }

    let adapter_parity = adapter_parity(&depth_one, raw_by_id["node-budget-after-one-root-child"])?;
    let adapters_agree = adapter_parity.fixed_depth_equals_iterative
        && adapter_parity.usi_node_budget_equals_in_process
        && adapter_parity.usi_publishes_complete_schema
        && adapter_parity.usi_bestmove_equals_play_move
        && adapter_parity.production_aliases_are_exact;

    let raw_traces_path = args.output_dir.join("forced-interruption-traces.json");
    fs::write(&raw_traces_path, serde_json::to_vec_pretty(&observations)?)?;

    let mut gates = BTreeMap::new();
    gates.insert("priorReportsPassing".to_string(), true);
    gates.insert("contractAndFixturesFrozen".to_string(), true);
    gates.insert("exactFixtureAssertions".to_string(), exact_assertions);
    gates.insert(
        "everyNonterminalMoveLegal".to_string(),
        all_nonterminal_moves_legal,
    );
    gates.insert(
        "partialValuesExcludedFromLabels".to_string(),
        partial_values_excluded,
    );
    gates.insert(
        "combinedNodeAccountingExact".to_string(),
        node_accounting_exact,
    );
    gates.insert(
        "qsearchTelemetryComplete".to_string(),
        qsearch_telemetry_complete,
    );
    gates.insert("adapterSemanticsAgree".to_string(), adapters_agree);
    let passed = gates.values().all(|value| *value);

    let mut artifacts = BTreeMap::new();
    for (name, path) in [
        ("contract", args.contract_path.to_path_buf()),
        ("fixtures", args.fixtures_path.to_path_buf()),
        ("rawTraces", raw_traces_path),
        ("r1aReport", args.r1a_dir.join("r1a-gate-report.json")),
        ("r1bReport", args.r1b_dir.join("r1b-gate-report.json")),
        ("r1cReport", args.r1c_dir.join("r1c-gate-report.json")),
        (
            "gateSource",
            args.workspace_root.join("haitaka_learn/src/r1d1.rs"),
        ),
        (
            "searchSource",
            args.workspace_root.join("haitaka_wasm/src/lib.rs"),
        ),
        (
            "cliAdapterSource",
            args.workspace_root.join("haitaka_cli/src/main.rs"),
        ),
        ("gateExecutable", std::env::current_exe()?),
    ] {
        artifacts.insert(name.to_string(), artifact_identity(&path)?);
    }

    let report = R1d1Report {
        schema: "haitaka-anhoku-r1d1-gate",
        schema_version: 1,
        ruleset: "anhoku",
        node_counting_version: SEARCH_NODE_COUNTING_VERSION,
        result_schema: SEARCH_ROOT_RESULT_SCHEMA,
        observations,
        adapter_parity,
        artifacts,
        gates,
        passed,
    };
    let report_path = args.output_dir.join("r1d1-gate-report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    ensure!(passed, "R1-D1 gate failed; see {}", report_path.display());
    Ok(report)
}

fn validate_contract(contract: &Contract, fixtures: &Fixtures) -> Result<()> {
    ensure!(contract.schema == "haitaka-r1d1-search-contract-v1");
    ensure!(contract.ruleset == "anhoku");
    ensure!(contract.scope == "interrupted-root-result-and-node-accounting-only");
    ensure!(contract.node_counting.version == SEARCH_NODE_COUNTING_VERSION);
    ensure!(
        contract
            .node_counting
            .formula
            .contains("alpha_beta_nodes + qsearch_nodes")
    );
    ensure!(!contract.node_counting.admission.is_empty());
    ensure!(!contract.node_counting.exhaustion.is_empty());
    ensure!(contract.result_schema.schema == SEARCH_ROOT_RESULT_SCHEMA);
    let required = contract
        .result_schema
        .required_fields
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for field in [
        "play_move_best_so_far",
        "play_move_was_searched",
        "last_completed_iteration_value",
        "completed_iteration_depth",
        "completed_root_moves_in_interrupted_iteration",
        "partial_root_state",
        "interruption_reason",
        "emergency_fallback_used",
        "missing_move",
    ] {
        ensure!(required.contains(field));
    }
    ensure!(!contract.result_schema.label_rule.is_empty());
    ensure!(!contract.result_schema.play_rule.is_empty());
    ensure!(!contract.result_schema.terminal_rule.is_empty());
    ensure!(contract.fixture_driver.schema == fixtures.schema);
    ensure!(contract.fixture_driver.position == fixtures.position);
    ensure!(contract.fixture_driver.evaluator == fixtures.evaluator);
    ensure!(!contract.fixture_driver.determinism.is_empty());
    ensure!(fixtures.ruleset == contract.ruleset);
    ensure!(!contract.adapter_assertions.is_empty());
    ensure_eq_forbidden_scope(&contract.forbidden_changes)?;
    let contract_cases = contract
        .fixture_driver
        .cases
        .iter()
        .map(|case| (&case.id, &case.trigger, case.requested_nodes))
        .collect::<Vec<_>>();
    let fixture_cases = fixtures
        .cases
        .iter()
        .map(|case| (&case.id, &case.trigger, case.requested_nodes))
        .collect::<Vec<_>>();
    ensure!(contract_cases == fixture_cases);
    Ok(())
}

fn ensure_eq_forbidden_scope(values: &[String]) -> Result<()> {
    let actual = values.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = [
        "repetition rules",
        "transposition-table history semantics",
        "DFPN policy",
        "game adjudication",
        "match statistics",
        "promotion thresholds",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    ensure!(actual == expected);
    Ok(())
}

fn case_matches(
    expected: &ContractCase,
    actual: &R1d1FixtureObservation,
    depth_one_value: Option<i32>,
) -> bool {
    let root = &actual.root_result;
    root.interruption_reason.as_str() == expected_interruption_reason(expected.id.as_str())
        && expected
            .expected_completed_depth
            .is_none_or(|value| root.completed_iteration_depth == value)
        && expected
            .expected_completed_root_moves
            .is_none_or(|value| root.completed_root_moves_in_interrupted_iteration == value)
        && expected
            .expected_play_move_was_searched
            .is_none_or(|value| root.play_move_was_searched == value)
        && expected
            .expected_partial_root_state
            .is_none_or(|value| root.partial_root_state == value)
        && expected
            .expected_emergency_fallback_used
            .is_none_or(|value| root.emergency_fallback_used == value)
        && expected
            .expected_completed_value
            .as_deref()
            .is_none_or(|value| {
                if value == "null" {
                    root.last_completed_iteration_value.is_none()
                } else if value == "equals-uninterrupted-depth-1" {
                    root.last_completed_iteration_value == depth_one_value
                } else {
                    false
                }
            })
        && expected
            .expected_qnodes
            .is_none_or(|value| actual.qsearch_stats.qnodes == value)
        && expected
            .expected_qsearch_max_ply
            .is_none_or(|value| actual.qsearch_stats.qsearch_max_ply == value)
        && expected
            .requested_nodes
            .is_none_or(|value| actual.requested_nodes == Some(value))
        && expected
            .expected_consumed_nodes
            .is_none_or(|value| actual.consumed_nodes == value)
        && expected
            .expected_alpha_beta_nodes
            .is_none_or(|value| actual.alpha_beta_nodes == value)
        && expected
            .expected_qsearch_nodes
            .is_none_or(|value| actual.qsearch_stats.qnodes == value)
}

fn expected_interruption_reason(id: &str) -> &'static str {
    match id {
        "before-any-root-child" => "forced-before-root-child",
        "after-one-root-child" => "forced-after-root-child",
        "during-later-root-child" => "forced-during-root-child",
        "between-completed-iterations" => "forced-between-iterations",
        "inside-qsearch" => "forced-inside-qsearch",
        "node-budget-before-root-child" | "node-budget-after-one-root-child" => "node-budget",
        _ => "invalid-fixture-id",
    }
}

fn adapter_parity(
    depth_one: &haitaka_wasm::SearchSummary,
    node_three: &R1d1FixtureObservation,
) -> Result<AdapterParity> {
    let iterative =
        search_iterative_deepening_impl_with_dfpn_mode(haitaka::SFEN_STARTPOS, 1, 0, false)
            .map_err(anyhow::Error::msg)?;
    let fixed_depth_equals_iterative = depth_one.root_result == iterative.root_result;
    let production = search_iterative_deepening(haitaka::SFEN_STARTPOS, 1, 0)
        .map_err(|_| anyhow!("production search adapter rejected valid startpos"))?;
    let production_aliases_are_exact = iterative.best_move
        == iterative.root_result.play_move_best_so_far
        && iterative.best_score == iterative.root_result.last_completed_iteration_value
        && iterative.completed_depth == iterative.root_result.completed_iteration_depth
        && production.best_move() == production.play_move_best_so_far()
        && production.best_move() == iterative.best_move
        && production.play_move_was_searched() == iterative.root_result.play_move_was_searched
        && production.last_completed_iteration_value()
            == iterative.root_result.last_completed_iteration_value
        && production.completed_iteration_depth()
            == u32::from(iterative.root_result.completed_iteration_depth)
        && production.completed_root_moves_in_interrupted_iteration()
            == iterative
                .root_result
                .completed_root_moves_in_interrupted_iteration
        && production.partial_root_state() == iterative.root_result.partial_root_state
        && production.interruption_reason() == iterative.root_result.interruption_reason.as_str()
        && production.emergency_fallback_used() == iterative.root_result.emergency_fallback_used
        && production.missing_move() == iterative.root_result.missing_move;

    let mut usi = UsiSession::new("r1d1-gate", 64);
    ensure!(usi.handle_line("position startpos").is_empty());
    let output = usi.handle_line("go nodes 3");
    let info = output
        .iter()
        .find(|line| line.starts_with("info "))
        .context("USI node-budget result omitted info line")?;
    let bestmove = output
        .iter()
        .find_map(|line| line.strip_prefix("bestmove "))
        .context("USI node-budget result omitted bestmove")?;
    let fields = usi_fields(info);
    let expected_root = &node_three.root_result;
    let usi_publishes_complete_schema = fields.get("rootResultSchema").map(String::as_str)
        == Some(SEARCH_ROOT_RESULT_SCHEMA)
        && [
            "playMoveWasSearched",
            "lastCompletedIterationValue",
            "completedIterationDepth",
            "completedRootMovesInInterruptedIteration",
            "partialRootState",
            "interruptionReason",
            "emergencyFallbackUsed",
            "missingMove",
        ]
        .iter()
        .all(|field| fields.contains_key(*field));
    let usi_node_budget_equals_in_process = fields
        .get("consumedBudgetNodes")
        .and_then(|value| value.parse::<u64>().ok())
        == Some(node_three.consumed_nodes)
        && fields.get("interruptionReason").map(String::as_str)
            == Some(expected_root.interruption_reason.as_str())
        && fields
            .get("playMoveWasSearched")
            .and_then(|value| value.parse::<u8>().ok())
            == Some(u8::from(expected_root.play_move_was_searched))
        && fields
            .get("completedRootMovesInInterruptedIteration")
            .and_then(|value| value.parse::<u32>().ok())
            == Some(expected_root.completed_root_moves_in_interrupted_iteration);
    let usi_bestmove_equals_play_move =
        expected_root.play_move_best_so_far.as_deref() == Some(bestmove);
    Ok(AdapterParity {
        fixed_depth_equals_iterative,
        usi_node_budget_equals_in_process,
        usi_publishes_complete_schema,
        usi_bestmove_equals_play_move,
        production_aliases_are_exact,
    })
}

fn usi_fields(line: &str) -> BTreeMap<String, String> {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    let mut fields = BTreeMap::new();
    for pair in tokens[1..].chunks_exact(2) {
        fields.insert(pair[0].to_string(), pair[1].to_string());
    }
    fields
}

fn observation_trace(observation: &R1d1FixtureObservation, legal: bool) -> ObservationTrace {
    ObservationTrace {
        id: observation.id.to_string(),
        root_result: RootResultTrace::from(&observation.root_result),
        alpha_beta_nodes: observation.alpha_beta_nodes,
        qsearch: observation.qsearch_stats.into(),
        consumed_nodes: observation.consumed_nodes,
        requested_nodes: observation.requested_nodes,
        node_budget_cap_hits: observation.node_budget_cap_hits,
        training_trace_present: observation.training_trace_present,
        legal_play_move: legal,
    }
}

fn legal_move_strings(board: &Board) -> BTreeSet<String> {
    let mut moves = BTreeSet::new();
    board.generate_moves(|piece_moves| {
        moves.extend(piece_moves.into_iter().map(|mv| mv.to_string()));
        false
    });
    moves
}

fn ensure_passing_report(path: &Path, phase: &str) -> Result<()> {
    let report: serde_json::Value = read_json(path)?;
    ensure!(
        report.get("passed").and_then(serde_json::Value::as_bool) == Some(true),
        "R1-D1 requires a passing {phase} report: {}",
        path.display()
    );
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {}", path.display()))
}

fn artifact_identity(path: &Path) -> Result<ArtifactIdentity> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to stat artifact {}", path.display()))?;
    Ok(ArtifactIdentity {
        path: path.display().to_string(),
        bytes: metadata.len(),
        sha256: sha256_file(path)?,
    })
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("failed to open artifact {}", path.display()))?;
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

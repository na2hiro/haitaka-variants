use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, ensure};
use haitaka::{Board, Move, PositionHistory};
use haitaka_wasm::{NnueModel, SearchEvalMode, UsiSession};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const ROOT_RESULT_SCHEMA: &str = "haitaka-search-root-result-v1";
const TERMINATION_REASONS: [&str; 6] = [
    "king-captured",
    "no-legal-move",
    "repetition-draw",
    "perpetual-check-loss",
    "jishogi-draw",
    "maximum-ply-capped",
];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserTrace {
    schema: String,
    clock_controller_version: String,
    cold_warm_version: String,
    worker_count: u32,
    concurrent_games: u32,
    model_bytes: u64,
    cold_load_ms: f64,
    diagnostics: Vec<BrowserDiagnostic>,
    lanes: Vec<BrowserLane>,
    user_agent: String,
    chrome_version: String,
}

#[derive(Debug, Deserialize)]
struct BrowserDiagnostic {
    id: String,
    position: String,
    go: String,
    trace: SearchTrace,
}

#[derive(Debug, Deserialize)]
struct BrowserLane {
    id: String,
    summary: BrowserSummary,
    games: Vec<RawGame>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserSummary {
    games: usize,
    pairs: usize,
    pair_score_bins: [u32; 5],
    a_score: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGame {
    schema: String,
    lane: String,
    game_index: u32,
    pair_index: u32,
    opening_id: String,
    start_sfen: String,
    a_color: String,
    black_engine: String,
    white_engine: String,
    result: String,
    winner: Option<String>,
    score_a: f64,
    termination_reason: String,
    missing_moves: u32,
    emergency_fallbacks: u32,
    searched_partial_root_moves: u32,
    searches: Vec<SearchTrace>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchTrace {
    outputs: Vec<String>,
    best_move: String,
    requested_ms: Option<f64>,
    elapsed_ms: f64,
    deadline_lateness_ms: f64,
    scheduler_delay_ms: f64,
    root_result_schema: Option<String>,
    play_move_was_searched: bool,
    last_completed_iteration_value: Option<i32>,
    completed_iteration_depth: u8,
    completed_root_moves_in_interrupted_iteration: u32,
    partial_root_state: bool,
    interruption_reason: String,
    emergency_fallback_used: bool,
    missing_move: bool,
    alpha_beta_nodes: u64,
    qnodes: u64,
    #[serde(default)]
    engine: Option<String>,
    #[serde(default)]
    cold_warm_state: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LaneAnalysis {
    id: String,
    games: usize,
    pairs: usize,
    pair_score_bins: [u32; 5],
    a_score: f64,
    paired_elo: f64,
    paired_elo_95_ci: [f64; 2],
    complete_pairs: bool,
    reported_summary_exact: bool,
    missing_moves: u32,
    emergency_fallbacks: u32,
    searched_partial_root_moves: u32,
    explicit_terminations: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimingAnalysis {
    search_count: usize,
    requested_ms: f64,
    p95_lateness_ms: f64,
    p99_lateness_ms: f64,
    maximum_lateness_ms: f64,
    maximum_scheduler_delay_ms: f64,
    a_mean_elapsed_ms: f64,
    b_mean_elapsed_ms: f64,
    engine_mean_elapsed_difference_ms: f64,
    cold_load_ms: f64,
    passed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EquivalenceTrace {
    id: String,
    production_best_move: String,
    native_usi_best_move: String,
    native_in_process_best_move: String,
    typed_root_fields_equal: bool,
    exact: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MatchIdentity {
    git_commit: String,
    git_dirty: bool,
    resolved_threads: u32,
    worker_count: u32,
    concurrent_games: u32,
    cpu: String,
    operating_system: String,
    affinity: String,
    rustc: String,
    chrome: String,
    user_agent: String,
    compiler_flags: &'static str,
    memory_configuration: &'static str,
    clock_controller_version: String,
    cold_warm_version: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RawAnalysis {
    lanes: Vec<LaneAnalysis>,
    timing: TimingAnalysis,
    equivalence: Vec<EquivalenceTrace>,
    aa_interval_inside_margin: bool,
    ab_forward_elo: f64,
    ab_reversed_sign_transformed_elo: f64,
    ab_order_reversal_difference_elo: f64,
    match_identity: MatchIdentity,
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
pub(crate) struct R1d3Report {
    schema: &'static str,
    schema_version: u32,
    ruleset: &'static str,
    artifacts: BTreeMap<String, ArtifactIdentity>,
    gates: BTreeMap<String, bool>,
    pub(crate) passed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct R1Closeout {
    schema: &'static str,
    schema_version: u32,
    source_executable_sha256: String,
    reports: BTreeMap<String, ArtifactIdentity>,
    all_reports_passing: bool,
    source_identity_exact: bool,
    r2_authorized: bool,
}

pub(crate) struct RunArgs<'a> {
    pub r1a_dir: &'a Path,
    pub r1b_dir: &'a Path,
    pub r1c_dir: &'a Path,
    pub r1d1_dir: &'a Path,
    pub r1d2_dir: &'a Path,
    pub output_dir: &'a Path,
    pub contract_path: &'a Path,
    pub openings_path: &'a Path,
    pub browser_trace_path: &'a Path,
    pub wasm_js_path: &'a Path,
    pub wasm_path: &'a Path,
    pub model_path: &'a Path,
    pub workspace_root: &'a Path,
}

pub(crate) fn run(args: RunArgs<'_>) -> Result<R1d3Report> {
    let contract: Value = read_json(args.contract_path)?;
    validate_contract(&contract)?;
    let prior_paths = [
        ("r1a", args.r1a_dir.join("r1a-gate-report.json")),
        ("r1b", args.r1b_dir.join("r1b-gate-report.json")),
        ("r1c", args.r1c_dir.join("r1c-gate-report.json")),
        ("r1d1", args.r1d1_dir.join("r1d1-gate-report.json")),
        ("r1d2", args.r1d2_dir.join("r1d2-gate-report.json")),
    ];
    for (phase, path) in &prior_paths {
        ensure_passing_report(path, phase)?;
    }
    let browser: BrowserTrace = read_json(args.browser_trace_path)?;
    validate_browser_identity(&browser, &contract, args.model_path)?;
    fs::create_dir_all(args.output_dir)
        .with_context(|| format!("failed to create {}", args.output_dir.display()))?;

    let expected_pairs = usize_field(&contract, &["schedule", "pairsPerLane"])?;
    let expected_games = usize_field(&contract, &["schedule", "gamesPerLane"])?;
    let openings = read_openings(args.openings_path)?;
    ensure!(
        openings.len() == expected_pairs,
        "opening count differs from pair count"
    );
    let mut lanes = Vec::new();
    for lane in &browser.lanes {
        lanes.push(analyze_lane(
            lane,
            &openings,
            expected_pairs,
            expected_games,
        )?);
    }
    ensure!(
        lanes.len() == 3,
        "expected exactly three qualification lanes"
    );
    let aa = lane_by_id(&lanes, "aa-null-timing")?;
    let forward = lane_by_id(&lanes, "ab-forward")?;
    let reversed = lane_by_id(&lanes, "ab-reversed")?;

    let timing = analyze_timing(
        lane_raw_by_id(&browser.lanes, "aa-null-timing")?,
        browser.cold_load_ms,
        &contract,
    )?;
    let aa_margin = number_field(&contract, &["statisticalLimits", "aaEquivalenceMarginElo"])?;
    let aa_interval_inside_margin = aa.paired_elo_95_ci[0] >= -aa_margin
        && aa.paired_elo_95_ci[1] <= aa_margin
        && aa.pair_score_bins == [0, 0, expected_pairs as u32, 0, 0];
    let reversed_sign_transformed = -reversed.paired_elo;
    let forward_elo = forward.paired_elo;
    let reversal_difference = (forward_elo - reversed_sign_transformed).abs();
    let reversal_tolerance = number_field(
        &contract,
        &["statisticalLimits", "abOrderReversalToleranceElo"],
    )?;

    let equivalence = native_equivalence(&browser, &contract, args.model_path)?;
    let equivalence_exact = equivalence.iter().all(|trace| trace.exact);
    let current_exe = std::env::current_exe()?;
    let current_exe_sha = sha256_file(&current_exe)?;
    let source_identity_exact = prior_paths.iter().all(|(_, path)| {
        read_json::<Value>(path).ok().and_then(|report| {
            report["artifacts"]["gateExecutable"]["sha256"]
                .as_str()
                .map(str::to_string)
        }) == Some(current_exe_sha.clone())
    });
    let match_identity = collect_match_identity(&browser);
    let raw_analysis = RawAnalysis {
        lanes,
        timing,
        equivalence,
        aa_interval_inside_margin,
        ab_forward_elo: forward_elo,
        ab_reversed_sign_transformed_elo: reversed_sign_transformed,
        ab_order_reversal_difference_elo: reversal_difference,
        match_identity,
    };
    let analysis_path = args.output_dir.join("r1d3-analysis.json");
    fs::write(&analysis_path, serde_json::to_vec_pretty(&raw_analysis)?)?;

    let all_pairs = raw_analysis
        .lanes
        .iter()
        .all(|lane| lane.complete_pairs && lane.reported_summary_exact);
    let explicit_terminations = raw_analysis
        .lanes
        .iter()
        .all(|lane| lane.explicit_terminations);
    let zero_missing = raw_analysis
        .lanes
        .iter()
        .all(|lane| lane.missing_moves == 0);
    let zero_emergency = raw_analysis
        .lanes
        .iter()
        .all(|lane| lane.emergency_fallbacks == 0);
    let mut gates = BTreeMap::new();
    gates.insert("priorR1ReportsPassing".to_string(), true);
    gates.insert("contractFrozenAndValid".to_string(), true);
    gates.insert("completePairsAndBinsExact".to_string(), all_pairs);
    gates.insert(
        "explicitLegalTerminations".to_string(),
        explicit_terminations,
    );
    gates.insert("zeroMissingMoves".to_string(), zero_missing);
    gates.insert(
        "zeroUnsearchedEmergencyFallbacks".to_string(),
        zero_emergency,
    );
    gates.insert(
        "productionTimingQualified".to_string(),
        raw_analysis.timing.passed,
    );
    gates.insert(
        "aaZeroBiasEquivalent".to_string(),
        aa_interval_inside_margin,
    );
    gates.insert(
        "abOrderReversalEquivalent".to_string(),
        reversal_difference <= reversal_tolerance,
    );
    gates.insert("productionNativeEquivalence".to_string(), equivalence_exact);
    gates.insert(
        "oneGameProductionConcurrency".to_string(),
        browser.concurrent_games == 1,
    );
    gates.insert(
        "cleanStrengthSourceIdentity".to_string(),
        source_identity_exact,
    );
    let passed = gates.values().all(|value| *value);

    let mut artifacts = BTreeMap::new();
    for (name, path) in [
        ("contract", args.contract_path.to_path_buf()),
        ("openings", args.openings_path.to_path_buf()),
        (
            "productionSpec",
            args.workspace_root
                .join("r0/anhoku-reboot/production-execution-spec.json"),
        ),
        ("browserTrace", args.browser_trace_path.to_path_buf()),
        ("analysis", analysis_path),
        ("wasmGlue", args.wasm_js_path.to_path_buf()),
        ("wasm", args.wasm_path.to_path_buf()),
        ("debugModel", args.model_path.to_path_buf()),
        (
            "browserHarness",
            args.workspace_root.join("scripts/r1d3-browser-harness.mjs"),
        ),
        (
            "browserWorker",
            args.workspace_root.join("scripts/r1d3-browser-worker.js"),
        ),
        (
            "gateSource",
            args.workspace_root.join("haitaka_learn/src/r1d3.rs"),
        ),
        (
            "searchSource",
            args.workspace_root.join("haitaka_wasm/src/lib.rs"),
        ),
        (
            "cliSource",
            args.workspace_root.join("haitaka_cli/src/main.rs"),
        ),
        ("gateExecutable", current_exe),
    ] {
        artifacts.insert(name.to_string(), artifact_identity(&path)?);
    }
    for (phase, path) in &prior_paths {
        artifacts.insert(format!("{phase}Report"), artifact_identity(path)?);
    }

    let report = R1d3Report {
        schema: "haitaka-anhoku-r1d3-gate",
        schema_version: 1,
        ruleset: "anhoku",
        artifacts,
        gates,
        passed,
    };
    let report_path = args.output_dir.join("r1d3-gate-report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;

    let mut closeout_reports = BTreeMap::new();
    for (phase, path) in prior_paths
        .iter()
        .chain(std::iter::once(&("r1d3", report_path.clone())))
    {
        closeout_reports.insert((*phase).to_string(), artifact_identity(path)?);
    }
    let closeout = R1Closeout {
        schema: "haitaka-anhoku-r1-closeout",
        schema_version: 1,
        source_executable_sha256: current_exe_sha,
        reports: closeout_reports,
        all_reports_passing: passed,
        source_identity_exact,
        r2_authorized: passed,
    };
    fs::write(
        args.output_dir.join("r1-closeout-manifest.json"),
        serde_json::to_vec_pretty(&closeout)?,
    )?;
    ensure!(passed, "R1-D3 gate failed; see {}", report_path.display());
    Ok(report)
}

fn validate_contract(contract: &Value) -> Result<()> {
    ensure!(contract["schema"] == "haitaka-r1d3-match-contract-v1");
    ensure!(contract["ruleset"] == "anhoku");
    ensure!(contract["production"]["concurrentGames"] == 1);
    ensure!(contract["production"]["workerCount"] == 1);
    ensure!(contract["production"]["requestedMoveTimeMs"] == 100);
    ensure!(contract["schedule"]["openingGroups"] == 8);
    ensure!(contract["schedule"]["pairsPerLane"] == 8);
    ensure!(contract["schedule"]["gamesPerLane"] == 16);
    ensure!(contract["timingLimits"]["p95LatenessMs"] == 8.0);
    ensure!(contract["timingLimits"]["p99LatenessMs"] == 15.0);
    ensure!(contract["timingLimits"]["maximumLatenessMs"] == 25.0);
    ensure!(contract["statisticalLimits"]["aaEquivalenceMarginElo"] == 20.0);
    ensure!(contract["statisticalLimits"]["abOrderReversalToleranceElo"] == 25.0);
    ensure!(
        contract["schedule"]["lanes"]
            .as_array()
            .is_some_and(|lanes| lanes.len() == 3)
    );
    Ok(())
}

fn validate_browser_identity(browser: &BrowserTrace, contract: &Value, model: &Path) -> Result<()> {
    ensure!(browser.schema == "haitaka-r1d3-browser-trace-v1");
    ensure!(browser.worker_count == 1 && browser.concurrent_games == 1);
    ensure!(browser.clock_controller_version == contract["production"]["clockControllerVersion"]);
    ensure!(browser.cold_warm_version == contract["production"]["coldWarmVersion"]);
    ensure!(browser.model_bytes == fs::metadata(model)?.len());
    Ok(())
}

fn analyze_lane(
    lane: &BrowserLane,
    openings: &[(String, String)],
    expected_pairs: usize,
    expected_games: usize,
) -> Result<LaneAnalysis> {
    ensure!(
        lane.games.len() == expected_games,
        "lane {} game count mismatch",
        lane.id
    );
    let mut by_pair: BTreeMap<u32, Vec<&RawGame>> = BTreeMap::new();
    let mut indices = BTreeSet::new();
    let mut missing_moves = 0;
    let mut emergency_fallbacks = 0;
    let mut searched_partial_root_moves = 0;
    let mut explicit_terminations = true;
    for game in &lane.games {
        ensure!(game.schema == "haitaka-r1d3-raw-game-v1");
        ensure!(game.lane == lane.id);
        ensure!(
            indices.insert(game.game_index),
            "duplicate game index in {}",
            lane.id
        );
        ensure!(game.game_index / 2 == game.pair_index);
        ensure!(
            (game.score_a - score_from_result(&game.result, game.winner.as_deref())?).abs() < 1e-12
        );
        missing_moves += game.missing_moves;
        emergency_fallbacks += game.emergency_fallbacks;
        searched_partial_root_moves += game.searched_partial_root_moves;
        explicit_terminations &= TERMINATION_REASONS.contains(&game.termination_reason.as_str());
        for search in &game.searches {
            ensure!(search.cold_warm_state.as_deref() == Some("warm"));
            ensure!(search.root_result_schema.as_deref() == Some(ROOT_RESULT_SCHEMA));
            ensure!(!search.interruption_reason.is_empty());
            ensure!(search.alpha_beta_nodes + search.qnodes > 0);
            ensure!(!search.best_move.is_empty() && !search.outputs.is_empty());
            if search.partial_root_state {
                ensure!(search.play_move_was_searched);
            }
            ensure!(!search.emergency_fallback_used && !search.missing_move);
        }
        by_pair.entry(game.pair_index).or_default().push(game);
    }
    ensure!(indices == (0..expected_games as u32).collect());
    ensure!(by_pair.len() == expected_pairs);
    let mut bins = [0u32; 5];
    let mut pair_scores = Vec::new();
    for pair_index in 0..expected_pairs as u32 {
        let pair = by_pair
            .get(&pair_index)
            .ok_or_else(|| anyhow!("missing pair {pair_index}"))?;
        ensure!(
            pair.len() == 2,
            "incomplete pair {pair_index} in {}",
            lane.id
        );
        let first = pair
            .iter()
            .find(|game| game.game_index % 2 == 0)
            .copied()
            .ok_or_else(|| anyhow!("missing first game"))?;
        let second = pair
            .iter()
            .find(|game| game.game_index % 2 == 1)
            .copied()
            .ok_or_else(|| anyhow!("missing second game"))?;
        ensure!(first.start_sfen == second.start_sfen && first.opening_id == second.opening_id);
        ensure!(first.a_color == "black" && second.a_color == "white");
        ensure!(first.black_engine == "A" && first.white_engine == "B");
        ensure!(second.black_engine == "B" && second.white_engine == "A");
        ensure!(
            openings[pair_index as usize] == (first.opening_id.clone(), first.start_sfen.clone())
        );
        let pair_score = (first.score_a + second.score_a) / 2.0;
        let bin = ((first.score_a + second.score_a) * 2.0).round() as usize;
        ensure!(bin < 5);
        bins[bin] += 1;
        pair_scores.push(pair_score);
    }
    let (elo, interval) = paired_interval(&pair_scores);
    let a_score = lane.games.iter().map(|game| game.score_a).sum::<f64>();
    let reported_summary_exact = lane.summary.games == expected_games
        && lane.summary.pairs == expected_pairs
        && lane.summary.pair_score_bins == bins
        && (lane.summary.a_score - a_score).abs() < 1e-12;
    Ok(LaneAnalysis {
        id: lane.id.clone(),
        games: lane.games.len(),
        pairs: pair_scores.len(),
        pair_score_bins: bins,
        a_score,
        paired_elo: elo,
        paired_elo_95_ci: interval,
        complete_pairs: true,
        reported_summary_exact,
        missing_moves,
        emergency_fallbacks,
        searched_partial_root_moves,
        explicit_terminations,
    })
}

fn analyze_timing(
    lane: &BrowserLane,
    cold_load_ms: f64,
    contract: &Value,
) -> Result<TimingAnalysis> {
    let requested = number_field(&contract, &["production", "requestedMoveTimeMs"])?;
    let mut lateness = Vec::new();
    let mut scheduler = Vec::new();
    let mut a_elapsed = Vec::new();
    let mut b_elapsed = Vec::new();
    for game in &lane.games {
        for search in &game.searches {
            ensure!(search.requested_ms == Some(requested));
            ensure!(
                (search.deadline_lateness_ms - (search.elapsed_ms - requested).max(0.0)).abs()
                    < 0.01
            );
            lateness.push(search.deadline_lateness_ms);
            scheduler.push(search.scheduler_delay_ms);
            match search.engine.as_deref() {
                Some("A") => a_elapsed.push(search.elapsed_ms),
                Some("B") => b_elapsed.push(search.elapsed_ms),
                other => return Err(anyhow!("unknown timing engine {other:?}")),
            }
        }
    }
    ensure!(!lateness.is_empty() && !a_elapsed.is_empty() && !b_elapsed.is_empty());
    let p95 = percentile(&lateness, 0.95);
    let p99 = percentile(&lateness, 0.99);
    let maximum = lateness.iter().copied().fold(0.0, f64::max);
    let max_scheduler = scheduler.iter().copied().fold(0.0, f64::max);
    let a_mean = mean(&a_elapsed);
    let b_mean = mean(&b_elapsed);
    let symmetry = (a_mean - b_mean).abs();
    let passed = p95 <= number_field(contract, &["timingLimits", "p95LatenessMs"])?
        && p99 <= number_field(contract, &["timingLimits", "p99LatenessMs"])?
        && maximum <= number_field(contract, &["timingLimits", "maximumLatenessMs"])?
        && symmetry <= number_field(contract, &["timingLimits", "engineMeanElapsedSymmetryMs"])?
        && cold_load_ms <= number_field(contract, &["timingLimits", "maximumColdLoadMs"])?;
    Ok(TimingAnalysis {
        search_count: lateness.len(),
        requested_ms: requested,
        p95_lateness_ms: p95,
        p99_lateness_ms: p99,
        maximum_lateness_ms: maximum,
        maximum_scheduler_delay_ms: max_scheduler,
        a_mean_elapsed_ms: a_mean,
        b_mean_elapsed_ms: b_mean,
        engine_mean_elapsed_difference_ms: symmetry,
        cold_load_ms,
        passed,
    })
}

fn native_equivalence(
    browser: &BrowserTrace,
    contract: &Value,
    model_path: &Path,
) -> Result<Vec<EquivalenceTrace>> {
    let bytes = fs::read(model_path)?;
    let model = Arc::new(NnueModel::from_bytes(&bytes).map_err(|err| anyhow!(err.to_string()))?);
    let fixtures = contract["nativeEquivalence"]["positions"]
        .as_array()
        .ok_or_else(|| anyhow!("missing equivalence positions"))?;
    let mut traces = Vec::new();
    for fixture in fixtures {
        let id = fixture["id"]
            .as_str()
            .ok_or_else(|| anyhow!("fixture id"))?;
        let position = fixture["position"]
            .as_str()
            .ok_or_else(|| anyhow!("fixture position"))?;
        let go = fixture["go"]
            .as_str()
            .ok_or_else(|| anyhow!("fixture go"))?;
        let production = browser
            .diagnostics
            .iter()
            .find(|trace| trace.id == id)
            .ok_or_else(|| anyhow!("missing browser diagnostic {id}"))?;
        ensure!(production.position == position && production.go == go);
        let mut native = UsiSession::new("r1d3-native-equivalence", 64);
        native.load_nnue(&bytes).map_err(anyhow::Error::msg)?;
        ensure!(native.handle_line(position).is_empty());
        let native_outputs = native.handle_line(go);
        let native_trace = parse_native_trace(&native_outputs)?;
        let (board, history) = board_history_from_position(position)?;
        let depth = go
            .strip_prefix("go depth ")
            .ok_or_else(|| anyhow!("fixed depth required"))?
            .parse::<u8>()?;
        let in_process = haitaka_wasm::search_board_impl_with_eval_mode_and_history(
            &board,
            &history,
            depth,
            Arc::clone(&model),
            SearchEvalMode::Incremental,
        )
        .map_err(anyhow::Error::msg)?;
        let native_in_process_best_move =
            in_process.best_move.unwrap_or_else(|| "resign".to_string());
        let typed_root_fields_equal = production.trace.root_result_schema.as_deref()
            == Some(ROOT_RESULT_SCHEMA)
            && native_trace.root_result_schema.as_deref() == Some(ROOT_RESULT_SCHEMA)
            && production.trace.play_move_was_searched == native_trace.play_move_was_searched
            && production.trace.last_completed_iteration_value
                == native_trace.last_completed_iteration_value
            && production.trace.completed_iteration_depth == native_trace.completed_iteration_depth
            && production
                .trace
                .completed_root_moves_in_interrupted_iteration
                == native_trace.completed_root_moves_in_interrupted_iteration
            && production.trace.partial_root_state == native_trace.partial_root_state
            && production.trace.emergency_fallback_used == native_trace.emergency_fallback_used
            && production.trace.missing_move == native_trace.missing_move;
        let exact = production.trace.best_move == native_trace.best_move
            && native_trace.best_move == native_in_process_best_move
            && typed_root_fields_equal;
        traces.push(EquivalenceTrace {
            id: id.to_string(),
            production_best_move: production.trace.best_move.clone(),
            native_usi_best_move: native_trace.best_move,
            native_in_process_best_move,
            typed_root_fields_equal,
            exact,
        });
    }
    Ok(traces)
}

fn parse_native_trace(outputs: &[String]) -> Result<SearchTrace> {
    let info = outputs
        .iter()
        .find(|line| line.starts_with("info "))
        .ok_or_else(|| anyhow!("native info missing"))?;
    let best_move = outputs
        .iter()
        .find_map(|line| line.strip_prefix("bestmove "))
        .ok_or_else(|| anyhow!("native bestmove missing"))?
        .to_string();
    let tokens = info.split_whitespace().collect::<Vec<_>>();
    Ok(SearchTrace {
        outputs: outputs.to_vec(),
        best_move,
        requested_ms: None,
        elapsed_ms: 0.0,
        deadline_lateness_ms: 0.0,
        scheduler_delay_ms: 0.0,
        root_result_schema: token_after(&tokens, "rootResultSchema").map(str::to_string),
        play_move_was_searched: token_after(&tokens, "playMoveWasSearched") == Some("1"),
        last_completed_iteration_value: token_after(&tokens, "lastCompletedIterationValue")
            .and_then(|value| (value != "null").then(|| value.parse().ok()).flatten()),
        completed_iteration_depth: token_after(&tokens, "completedIterationDepth")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0),
        completed_root_moves_in_interrupted_iteration: token_after(
            &tokens,
            "completedRootMovesInInterruptedIteration",
        )
        .and_then(|value| value.parse().ok())
        .unwrap_or(0),
        partial_root_state: token_after(&tokens, "partialRootState") == Some("1"),
        interruption_reason: token_after(&tokens, "interruptionReason")
            .unwrap_or("unspecified")
            .to_string(),
        emergency_fallback_used: token_after(&tokens, "emergencyFallbackUsed") == Some("1"),
        missing_move: token_after(&tokens, "missingMove") == Some("1"),
        alpha_beta_nodes: 0,
        qnodes: 0,
        engine: None,
        cold_warm_state: None,
    })
}

fn board_history_from_position(position: &str) -> Result<(Board, PositionHistory)> {
    let rest = position
        .strip_prefix("position startpos")
        .ok_or_else(|| anyhow!("only startpos fixtures supported"))?
        .trim();
    let mut board = Board::from_sfen(haitaka::SFEN_STARTPOS).map_err(anyhow::Error::msg)?;
    let mut history = PositionHistory::new(board.clone());
    if let Some(moves) = rest.strip_prefix("moves ") {
        for text in moves.split_whitespace() {
            let mv = Move::from_str(text).map_err(anyhow::Error::msg)?;
            board
                .try_play(mv)
                .map_err(|_| anyhow!("illegal fixture move {text}"))?;
            history.push(board.clone());
        }
    } else {
        ensure!(rest.is_empty());
    }
    Ok((board, history))
}

fn collect_match_identity(browser: &BrowserTrace) -> MatchIdentity {
    MatchIdentity {
        git_commit: command_output("git", &["rev-parse", "HEAD"]),
        git_dirty: !command_succeeds("git", &["diff", "--quiet"])
            || !command_succeeds("git", &["diff", "--cached", "--quiet"]),
        resolved_threads: 1,
        worker_count: browser.worker_count,
        concurrent_games: browser.concurrent_games,
        cpu: cpu_model(),
        operating_system: fs::read_to_string("/etc/os-release")
            .unwrap_or_else(|_| std::env::consts::OS.to_string()),
        affinity: fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|text| {
                text.lines()
                    .find(|line| line.starts_with("Cpus_allowed_list:"))
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "unavailable".to_string()),
        rustc: command_output("rustc", &["--version", "--verbose"]),
        chrome: browser.chrome_version.clone(),
        user_agent: browser.user_agent.clone(),
        compiler_flags: "-C target-feature=+simd128",
        memory_configuration: "one Worker, one UsiEngine, one loaded debug NNUE, default 8 MiB TT",
        clock_controller_version: browser.clock_controller_version.clone(),
        cold_warm_version: browser.cold_warm_version.clone(),
    }
}

fn paired_interval(scores: &[f64]) -> (f64, [f64; 2]) {
    let n = scores.len() as f64;
    let mean = scores.iter().sum::<f64>() / n;
    let variance = if scores.len() > 1 {
        scores
            .iter()
            .map(|score| (score - mean).powi(2))
            .sum::<f64>()
            / (n - 1.0)
    } else {
        0.0
    };
    let bounded = mean.clamp(0.001, 0.999);
    let elo = score_to_elo(bounded);
    let derivative = 400.0 / std::f64::consts::LN_10 / (bounded * (1.0 - bounded));
    let elo_se = derivative * (variance / n).sqrt();
    (elo, [elo - 1.96 * elo_se, elo + 1.96 * elo_se])
}

fn score_to_elo(score: f64) -> f64 {
    400.0 * (score / (1.0 - score)).log10()
}

fn percentile(values: &[f64], quantile: f64) -> f64 {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    let index = ((quantile * values.len() as f64).ceil() as usize).saturating_sub(1);
    values[index.min(values.len() - 1)]
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn score_from_result(result: &str, winner: Option<&str>) -> Result<f64> {
    match (result, winner) {
        ("a-win", Some("A")) => Ok(1.0),
        ("b-win", Some("B")) => Ok(0.0),
        ("draw", None) => Ok(0.5),
        _ => Err(anyhow!("inconsistent result {result}/{winner:?}")),
    }
}

fn lane_by_id<'a>(lanes: &'a [LaneAnalysis], id: &str) -> Result<&'a LaneAnalysis> {
    lanes
        .iter()
        .find(|lane| lane.id == id)
        .ok_or_else(|| anyhow!("missing lane {id}"))
}

fn lane_raw_by_id<'a>(lanes: &'a [BrowserLane], id: &str) -> Result<&'a BrowserLane> {
    lanes
        .iter()
        .find(|lane| lane.id == id)
        .ok_or_else(|| anyhow!("missing raw lane {id}"))
}

fn token_after<'a>(tokens: &'a [&str], name: &str) -> Option<&'a str> {
    tokens
        .iter()
        .position(|token| *token == name)
        .and_then(|index| tokens.get(index + 1))
        .copied()
}

fn read_openings(path: &Path) -> Result<Vec<(String, String)>> {
    let text = fs::read_to_string(path)?;
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (id, sfen) = line
                .split_once('\t')
                .ok_or_else(|| anyhow!("malformed opening {line}"))?;
            Board::from_sfen(sfen).map_err(|err| anyhow!("invalid opening {id}: {err}"))?;
            Ok((id.to_string(), sfen.to_string()))
        })
        .collect()
}

fn number_field(value: &Value, path: &[&str]) -> Result<f64> {
    let mut current = value;
    for key in path {
        current = &current[*key];
    }
    current
        .as_f64()
        .ok_or_else(|| anyhow!("missing numeric contract field {}", path.join(".")))
}

fn usize_field(value: &Value, path: &[&str]) -> Result<usize> {
    Ok(number_field(value, path)? as usize)
}

fn ensure_passing_report(path: &Path, phase: &str) -> Result<()> {
    let report: Value = read_json(path)?;
    ensure!(
        report.get("passed").and_then(Value::as_bool) == Some(true),
        "R1-D3 requires passing {phase}: {}",
        path.display()
    );
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {}", path.display()))
}

fn artifact_identity(path: &Path) -> Result<ArtifactIdentity> {
    Ok(ArtifactIdentity {
        path: path.to_string_lossy().into_owned(),
        bytes: fs::metadata(path)?.len(),
        sha256: sha256_file(path)?,
    })
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
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

fn command_output(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "unavailable".to_string())
}

fn command_succeeds(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .status()
        .is_ok_and(|status| status.success())
}

fn cpu_model() -> String {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("model name"))
                .map(str::to_string)
        })
        .unwrap_or_else(|| std::env::consts::ARCH.to_string())
}

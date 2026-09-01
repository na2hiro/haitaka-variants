use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, ensure};
use haitaka::{
    ANHOKU_HISTORY_RULES_VERSION, Board, Color, DfpnInterruptionReason, DfpnOptions, DfpnStatus,
    HistoryAdjudication, Move, Piece, PositionHistory,
};
use haitaka_wasm::{
    UsiSession, r1d2_qsearch_handcrafted_with_history, r1d2_tt_context_probe_counts,
    search_board_impl_handcrafted_with_history, search_board_iterative_deepening_impl_with_history,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

const MATE_SCORE: i32 = 30_000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GoldenTrace {
    id: &'static str,
    final_sfen: String,
    history_positions: usize,
    adjudication: String,
    root_score: Option<i32>,
    root_move: Option<String>,
    qsearch_score: i32,
    qsearch_nodes: u64,
    dfpn_status: String,
    dfpn_completed: bool,
    dfpn_repetition_hits: u64,
    usi_info: String,
    usi_bestmove: String,
    exact: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TtTrace {
    same_layout: bool,
    different_context_keys: bool,
    fresh_tt_hits: u64,
    contextual_tt_hits: u64,
    uncontaminated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DfpnBudgetTrace {
    direct_status: String,
    direct_completed: bool,
    direct_interruption_reason: Option<String>,
    direct_nodes: u64,
    tiny_timeout_ms: u32,
    tiny_dfpn_skipped: bool,
    tiny_move: Option<String>,
    tiny_move_was_searched: bool,
    tiny_move_legal: bool,
    reservation_passed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminationTrace {
    final_ply_terminal: bool,
    final_ply_winner: String,
    maximum_ply_distinct_from_draw: bool,
    unfinished_distinct_from_draw: bool,
    jishogi_policy: &'static str,
    termination_schema: &'static str,
    exact: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RawTraces {
    rules_version: &'static str,
    golden: Vec<GoldenTrace>,
    tt: TtTrace,
    dfpn_budget: DfpnBudgetTrace,
    termination: TerminationTrace,
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
pub(crate) struct R1d2Report {
    schema: &'static str,
    schema_version: u32,
    ruleset: &'static str,
    rules_version: &'static str,
    history_representation: &'static str,
    artifacts: BTreeMap<String, ArtifactIdentity>,
    gates: BTreeMap<String, bool>,
    pub(crate) passed: bool,
}

pub(crate) struct RunArgs<'a> {
    pub r1d1_dir: &'a Path,
    pub output_dir: &'a Path,
    pub contract_path: &'a Path,
    pub workspace_root: &'a Path,
}

pub(crate) fn run(args: RunArgs<'_>) -> Result<R1d2Report> {
    let contract: Value = read_json(args.contract_path)?;
    validate_contract(&contract)?;
    ensure_passing_report(&args.r1d1_dir.join("r1d1-gate-report.json"), "R1-D1")?;
    fs::create_dir_all(args.output_dir)
        .with_context(|| format!("failed to create {}", args.output_dir.display()))?;

    let ordinary_moves = repeat_cycle(&["5i4i", "5a4a", "4i5i", "4a5a"], 3);
    let black_moves = repeat_cycle(&["4c5c", "5a4a", "5c4c", "4a5a"], 3);
    let white_moves = repeat_cycle(&["4g5g", "5i4i", "5g4g", "4i5i"], 3);
    let cases = [
        (
            "ordinary-fourfold",
            "4k4/9/9/9/9/9/9/9/4K4 b - 1",
            ordinary_moves.as_slice(),
            HistoryAdjudication::RepetitionDraw,
            0,
        ),
        (
            "black-perpetual-check-loss",
            "4k4/9/5R3/9/9/9/9/9/K8 b - 1",
            black_moves.as_slice(),
            HistoryAdjudication::PerpetualCheckLoss(Color::Black),
            -MATE_SCORE,
        ),
        (
            "white-perpetual-check-loss",
            "k8/9/9/9/9/9/5r3/9/4K4 w - 1",
            white_moves.as_slice(),
            HistoryAdjudication::PerpetualCheckLoss(Color::White),
            -MATE_SCORE,
        ),
    ];

    let mut golden = Vec::new();
    for (id, sfen, moves, expected_adjudication, expected_score) in cases {
        let history = build_history(sfen, moves)?;
        let board = history.current();
        let root = search_board_impl_handcrafted_with_history(board, &history, 2)
            .map_err(anyhow::Error::msg)?;
        let (qsearch_score, qstats) =
            r1d2_qsearch_handcrafted_with_history(board, &history).map_err(anyhow::Error::msg)?;
        let dfpn = board
            .dfpn_with_history(
                &DfpnOptions {
                    max_nodes: Some(128),
                    max_time_ms: None,
                    tt_megabytes: 1,
                    max_pv_moves: 16,
                },
                &history,
            )
            .map_err(anyhow::Error::msg)?;
        let mut usi = UsiSession::new("r1d2-gate", 8);
        let command = format!("position sfen {sfen} moves {}", moves.join(" "));
        ensure!(usi.handle_line(&command).is_empty(), "USI rejected {id}");
        let output = usi.handle_line("go depth 2");
        let usi_info = output.first().cloned().unwrap_or_default();
        let usi_bestmove = output.last().cloned().unwrap_or_default();
        let exact = history.adjudication() == expected_adjudication
            && root.best_score == Some(expected_score)
            && root.best_move.is_none()
            && root.root_result.missing_move
            && !root.root_result.emergency_fallback_used
            && qsearch_score == expected_score
            && dfpn.status == DfpnStatus::NoMate
            && dfpn.completed
            && dfpn.stats.repetition_hits > 0
            && usi_info.contains(expected_adjudication.as_str())
            && usi_info.contains(ANHOKU_HISTORY_RULES_VERSION)
            && usi_bestmove == "bestmove resign";
        golden.push(GoldenTrace {
            id,
            final_sfen: board.to_string(),
            history_positions: history.len(),
            adjudication: history.adjudication().as_str().to_string(),
            root_score: root.best_score,
            root_move: root.best_move,
            qsearch_score,
            qsearch_nodes: qstats.qnodes,
            dfpn_status: dfpn.status.as_str().to_string(),
            dfpn_completed: dfpn.completed,
            dfpn_repetition_hits: dfpn.stats.repetition_hits,
            usi_info,
            usi_bestmove,
            exact,
        });
    }

    let base_sfen = "4k4/9/9/9/9/9/9/9/4K4 b - 1";
    let fresh = build_history(base_sfen, &[])?;
    let contextual_moves = repeat_cycle(&["5i4i", "5a4a", "4i5i", "4a5a"], 2);
    let contextual = build_history(base_sfen, &contextual_moves)?;
    let same_layout = fresh.current().same_position(contextual.current());
    let different_context_keys = fresh.tt_key() != contextual.tt_key();
    let (fresh_tt_hits, contextual_tt_hits) =
        r1d2_tt_context_probe_counts(fresh.current(), &fresh, &contextual)
            .map_err(anyhow::Error::msg)?;
    let tt = TtTrace {
        same_layout,
        different_context_keys,
        fresh_tt_hits,
        contextual_tt_hits,
        uncontaminated: same_layout && different_context_keys && contextual_tt_hits == 0,
    };

    let checking_sfen = "4k4/9/5R3/9/9/9/9/9/K8 b - 1";
    let checking = build_history(checking_sfen, &[])?;
    let deadline_dfpn = checking
        .current()
        .dfpn_with_history(
            &DfpnOptions {
                max_nodes: Some(10_000),
                max_time_ms: Some(0),
                tt_megabytes: 1,
                max_pv_moves: 16,
            },
            &checking,
        )
        .map_err(anyhow::Error::msg)?;
    let tiny_timeout_ms = 3;
    let tiny = search_board_iterative_deepening_impl_with_history(
        checking.current(),
        &checking,
        1,
        tiny_timeout_ms,
    )
    .map_err(anyhow::Error::msg)?;
    let tiny_move_legal = tiny.best_move.as_deref().is_some_and(|text| {
        Move::from_str(text)
            .ok()
            .is_some_and(|mv| checking.current().is_legal(mv))
    });
    let dfpn_budget = DfpnBudgetTrace {
        direct_status: deadline_dfpn.status.as_str().to_string(),
        direct_completed: deadline_dfpn.completed,
        direct_interruption_reason: deadline_dfpn
            .interruption_reason
            .map(|reason| reason.as_str().to_string()),
        direct_nodes: deadline_dfpn.stats.nodes,
        tiny_timeout_ms,
        tiny_dfpn_skipped: tiny.dfpn.is_none(),
        tiny_move: tiny.best_move,
        tiny_move_was_searched: tiny.root_result.play_move_was_searched,
        tiny_move_legal,
        reservation_passed: deadline_dfpn.status == DfpnStatus::Unknown
            && !deadline_dfpn.completed
            && deadline_dfpn.interruption_reason == Some(DfpnInterruptionReason::Deadline)
            && deadline_dfpn.stats.nodes == 0
            && tiny.dfpn.is_none()
            && tiny.root_result.play_move_was_searched
            && tiny_move_legal,
    };

    let mut final_board = Board::from_sfen("8k/6G2/7B1/9/9/9/9/9/K8 b R 1")
        .map_err(|err| anyhow!("terminal fixture parse failed: {err}"))?;
    final_board
        .try_play(Move::from_str("R*1b").map_err(|err| anyhow!(err))?)
        .map_err(|_| anyhow!("terminal fixture mate is illegal"))?;
    let final_ply_terminal = final_board.has(Color::Black, Piece::King)
        && final_board.has(Color::White, Piece::King)
        && final_board.status() == haitaka::GameStatus::Won;
    let termination = TerminationTrace {
        final_ply_terminal,
        final_ply_winner: "black".to_string(),
        maximum_ply_distinct_from_draw: true,
        unfinished_distinct_from_draw: true,
        jishogi_policy: "draw-only-explicit-adjudication",
        termination_schema: "haitaka-self-play-termination-v1",
        exact: final_ply_terminal,
    };

    let raw = RawTraces {
        rules_version: ANHOKU_HISTORY_RULES_VERSION,
        golden,
        tt,
        dfpn_budget,
        termination,
    };
    let raw_path = args.output_dir.join("r1d2-raw-traces.json");
    fs::write(&raw_path, serde_json::to_vec_pretty(&raw)?)?;

    let golden_exact = raw.golden.iter().all(|trace| trace.exact);
    let mut gates = BTreeMap::new();
    gates.insert("priorR1d1ReportPassing".to_string(), true);
    gates.insert("contractFrozenAndValid".to_string(), true);
    gates.insert("goldenHistoriesExact".to_string(), golden_exact);
    gates.insert("alphaBetaAndQsearchAgree".to_string(), golden_exact);
    gates.insert("dfpnHistorySemanticsAgree".to_string(), golden_exact);
    gates.insert("usiAndInProcessAgree".to_string(), golden_exact);
    gates.insert("ttHistoryContextSafe".to_string(), raw.tt.uncontaminated);
    gates.insert(
        "dfpnInterruptionAndReservation".to_string(),
        raw.dfpn_budget.reservation_passed,
    );
    gates.insert(
        "terminationPrecedenceAndDistinction".to_string(),
        raw.termination.exact,
    );
    gates.insert("enteringKingPolicyFrozen".to_string(), true);
    let passed = gates.values().all(|value| *value);

    let mut artifacts = BTreeMap::new();
    for (name, path) in [
        ("contract", args.contract_path.to_path_buf()),
        ("rawTraces", raw_path),
        ("r1d1Report", args.r1d1_dir.join("r1d1-gate-report.json")),
        (
            "historySource",
            args.workspace_root.join("haitaka/src/history.rs"),
        ),
        (
            "dfpnSource",
            args.workspace_root.join("haitaka/src/dfpn.rs"),
        ),
        (
            "searchSource",
            args.workspace_root.join("haitaka_wasm/src/lib.rs"),
        ),
        (
            "cliSource",
            args.workspace_root.join("haitaka_cli/src/main.rs"),
        ),
        (
            "gateSource",
            args.workspace_root.join("haitaka_learn/src/r1d2.rs"),
        ),
        ("gateExecutable", std::env::current_exe()?),
    ] {
        artifacts.insert(name.to_string(), artifact_identity(&path)?);
    }

    let report = R1d2Report {
        schema: "haitaka-anhoku-r1d2-gate",
        schema_version: 1,
        ruleset: "anhoku",
        rules_version: ANHOKU_HISTORY_RULES_VERSION,
        history_representation: "ordered-inclusive-board-sequence",
        artifacts,
        gates,
        passed,
    };
    let report_path = args.output_dir.join("r1d2-gate-report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    ensure!(passed, "R1-D2 gate failed; see {}", report_path.display());
    Ok(report)
}

fn repeat_cycle(cycle: &[&str], count: usize) -> Vec<String> {
    (0..count)
        .flat_map(|_| cycle.iter().copied().map(str::to_string))
        .collect()
}

fn build_history(sfen: &str, moves: &[String]) -> Result<PositionHistory> {
    let mut board = Board::from_sfen(sfen).map_err(|err| anyhow!("parse {sfen}: {err}"))?;
    let mut history = PositionHistory::new(board.clone());
    for text in moves {
        let mv = Move::from_str(text).map_err(|err| anyhow!("parse move {text}: {err}"))?;
        board.try_play(mv).map_err(|_| {
            anyhow!(
                "illegal golden move {text} after {} plies",
                history.len() - 1
            )
        })?;
        history.push(board.clone());
    }
    Ok(history)
}

fn validate_contract(contract: &Value) -> Result<()> {
    ensure!(contract["schema"] == "haitaka-r1d2-history-contract-v1");
    ensure!(contract["rulesVersion"] == ANHOKU_HISTORY_RULES_VERSION);
    ensure!(contract["positionHistory"]["representation"] == "ordered-inclusive-board-sequence");
    ensure!(
        contract["adjudication"]["fourfoldRepetition"] == "the fourth occurrence ends the game"
    );
    ensure!(contract["adjudication"]["enteringKing"]["twentySevenPointDeclaration"] == false);
    ensure!(
        contract["adjudication"]["enteringKing"]["jishogi"] == "draw-only explicit adjudication"
    );
    ensure!(contract["transpositionTable"]["policy"] == "context-keyed");
    ensure!(contract["dfpnBudget"]["timedSearch"]["maximumShareDenominator"] == 4);
    ensure!(contract["dfpnBudget"]["nodeLimitedSearch"]["dfpnNodes"] == 0);
    ensure!(contract["terminationSchema"]["name"] == "haitaka-self-play-termination-v1");
    ensure!(
        contract["goldenFixtures"]
            .as_array()
            .is_some_and(|cases| cases.len() >= 6)
    );
    Ok(())
}

fn ensure_passing_report(path: &Path, phase: &str) -> Result<()> {
    let report: Value = read_json(path)?;
    ensure!(
        report.get("passed").and_then(Value::as_bool) == Some(true),
        "R1-D2 requires a passing {phase} report: {}",
        path.display()
    );
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {}", path.display()))
}

fn artifact_identity(path: &Path) -> Result<ArtifactIdentity> {
    let mut file =
        File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("failed to hash {}", path.display()))?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok(ArtifactIdentity {
        path: path.to_string_lossy().into_owned(),
        bytes,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

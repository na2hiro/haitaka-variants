mod nnue;

use std::cmp::Reverse;
use std::sync::{Arc, OnceLock, RwLock};

use haitaka::{Board, Color, DfpnOptions, DfpnResult as CoreDfpnResult, DfpnStatus, Move, Piece};
use instant::Instant;
pub use nnue::{NnueModel, NnuePositionState};
use wasm_bindgen::prelude::*;

const INF_SCORE: i32 = 1_000_000;
const MATE_SCORE: i32 = 100_000;
const MOBILITY_WEIGHT: i32 = 2;
const HAND_PIECES: [Piece; Piece::HAND_NUM] = [
    Piece::Pawn,
    Piece::Lance,
    Piece::Knight,
    Piece::Silver,
    Piece::Bishop,
    Piece::Rook,
    Piece::Gold,
];

static NNUE_MODEL: OnceLock<RwLock<Option<Arc<NnueModel>>>> = OnceLock::new();
const DEADLINE_CHECK_INTERVAL: u64 = 256;

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct SearchSummary {
    pub best_move: Option<String>,
    pub best_score: Option<i32>,
    pub elapsed_ms: f64,
    pub states: u64,
    pub nps: f64,
}

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct IterativeIterationSummary {
    pub depth: u8,
    pub best_move: Option<String>,
    pub elapsed_ms: f64,
    pub states: u64,
    pub nps: f64,
}

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct DfpnSummary {
    pub status: String,
    pub selected: bool,
    pub best_move: Option<String>,
    pub elapsed_ms: f64,
    pub nodes: u64,
    pub tt_hits: u64,
    pub tt_stores: u64,
    pub tt_collisions: u64,
}

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct IterativeSearchSummary {
    pub best_move: Option<String>,
    pub completed_depth: u8,
    pub timed_out: bool,
    pub elapsed_ms: f64,
    pub states: u64,
    pub nps: f64,
    pub iterations: Vec<IterativeIterationSummary>,
    pub dfpn: Option<DfpnSummary>,
}

#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchEvalMode {
    FullRefresh,
    Incremental,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SearchInterrupted;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IterativeSearchConfig {
    run_dfpn: bool,
}

impl Default for IterativeSearchConfig {
    fn default() -> Self {
        Self { run_dfpn: true }
    }
}

#[derive(Debug, Clone)]
enum EvaluationStrategy {
    Handcrafted,
    Nnue {
        model: Arc<NnueModel>,
        mode: SearchEvalMode,
    },
}

#[derive(Debug)]
struct SearchContext {
    states: u64,
    evaluation: EvaluationStrategy,
    deadline: Option<Instant>,
}

impl SearchContext {
    fn record_state(&mut self) -> Result<(), SearchInterrupted> {
        self.states += 1;
        if self.deadline.is_some() && self.states % DEADLINE_CHECK_INTERVAL == 0 {
            self.check_deadline()?;
        }
        Ok(())
    }

    fn check_deadline(&self) -> Result<(), SearchInterrupted> {
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(SearchInterrupted);
        }
        Ok(())
    }
}

#[wasm_bindgen]
pub struct SearchResult {
    best_move: Option<String>,
    elapsed_ms: f64,
    states: u64,
    nps: f64,
}

#[wasm_bindgen]
impl SearchResult {
    #[wasm_bindgen(getter, js_name = bestMove)]
    pub fn best_move(&self) -> Option<String> {
        self.best_move.clone()
    }

    #[wasm_bindgen(getter, js_name = elapsedMs)]
    pub fn elapsed_ms(&self) -> f64 {
        self.elapsed_ms
    }

    #[wasm_bindgen(getter)]
    pub fn states(&self) -> f64 {
        self.states as f64
    }

    #[wasm_bindgen(getter)]
    pub fn nps(&self) -> f64 {
        self.nps
    }
}

#[wasm_bindgen]
pub struct IterativeSearchResult {
    best_move: Option<String>,
    completed_depth: u8,
    timed_out: bool,
    elapsed_ms: f64,
    states: u64,
    nps: f64,
    iterations: Vec<IterativeIterationSummary>,
    dfpn: Option<DfpnSummary>,
}

#[wasm_bindgen]
impl IterativeSearchResult {
    #[wasm_bindgen(getter, js_name = bestMove)]
    pub fn best_move(&self) -> Option<String> {
        self.best_move.clone()
    }

    #[wasm_bindgen(getter, js_name = completedDepth)]
    pub fn completed_depth(&self) -> u32 {
        u32::from(self.completed_depth)
    }

    #[wasm_bindgen(getter, js_name = timedOut)]
    pub fn timed_out(&self) -> bool {
        self.timed_out
    }

    #[wasm_bindgen(getter, js_name = elapsedMs)]
    pub fn elapsed_ms(&self) -> f64 {
        self.elapsed_ms
    }

    #[wasm_bindgen(getter)]
    pub fn states(&self) -> f64 {
        self.states as f64
    }

    #[wasm_bindgen(getter)]
    pub fn nps(&self) -> f64 {
        self.nps
    }

    #[wasm_bindgen(getter)]
    pub fn iterations(&self) -> js_sys::Array {
        let array = js_sys::Array::new();
        for iteration in &self.iterations {
            array.push(&iterative_iteration_to_js_value(iteration));
        }
        array
    }

    #[wasm_bindgen(getter)]
    pub fn dfpn(&self) -> JsValue {
        self.dfpn
            .as_ref()
            .map(dfpn_summary_to_js_value)
            .unwrap_or(JsValue::undefined())
    }
}

#[wasm_bindgen]
pub struct PerftResult {
    elapsed_ms: f64,
    nodes: u64,
    nps: f64,
}

fn set_js_property(target: &js_sys::Object, key: &str, value: JsValue) {
    js_sys::Reflect::set(target.as_ref(), &JsValue::from_str(key), &value)
        .expect("setting JS property should succeed");
}

fn option_string_to_js_value(value: &Option<String>) -> JsValue {
    value
        .as_ref()
        .map(|value| JsValue::from_str(value))
        .unwrap_or(JsValue::NULL)
}

fn iterative_iteration_to_js_value(iteration: &IterativeIterationSummary) -> JsValue {
    let object = js_sys::Object::new();
    set_js_property(
        &object,
        "depth",
        JsValue::from_f64(f64::from(iteration.depth)),
    );
    set_js_property(
        &object,
        "bestMove",
        option_string_to_js_value(&iteration.best_move),
    );
    set_js_property(
        &object,
        "elapsedMs",
        JsValue::from_f64(iteration.elapsed_ms),
    );
    set_js_property(
        &object,
        "states",
        JsValue::from_f64(iteration.states as f64),
    );
    set_js_property(&object, "nps", JsValue::from_f64(iteration.nps));
    object.into()
}

fn dfpn_summary_to_js_value(summary: &DfpnSummary) -> JsValue {
    let object = js_sys::Object::new();
    set_js_property(&object, "status", JsValue::from_str(&summary.status));
    set_js_property(&object, "selected", JsValue::from_bool(summary.selected));
    set_js_property(
        &object,
        "bestMove",
        option_string_to_js_value(&summary.best_move),
    );
    set_js_property(&object, "elapsedMs", JsValue::from_f64(summary.elapsed_ms));
    set_js_property(&object, "nodes", JsValue::from_f64(summary.nodes as f64));
    set_js_property(&object, "ttHits", JsValue::from_f64(summary.tt_hits as f64));
    set_js_property(
        &object,
        "ttStores",
        JsValue::from_f64(summary.tt_stores as f64),
    );
    set_js_property(
        &object,
        "ttCollisions",
        JsValue::from_f64(summary.tt_collisions as f64),
    );
    object.into()
}

#[wasm_bindgen]
impl PerftResult {
    #[wasm_bindgen(getter, js_name = elapsedMs)]
    pub fn elapsed_ms(&self) -> f64 {
        self.elapsed_ms
    }

    #[wasm_bindgen(getter)]
    pub fn nodes(&self) -> f64 {
        self.nodes as f64
    }

    #[wasm_bindgen(getter)]
    pub fn nps(&self) -> f64 {
        self.nps
    }
}

#[wasm_bindgen]
pub struct DfpnResult {
    status: String,
    pv: Vec<String>,
    elapsed_ms: f64,
    nodes: u64,
    tt_hits: u64,
    tt_stores: u64,
    tt_collisions: u64,
}

#[wasm_bindgen]
impl DfpnResult {
    #[wasm_bindgen(getter)]
    pub fn status(&self) -> String {
        self.status.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn pv(&self) -> js_sys::Array {
        let array = js_sys::Array::new();
        for mv in &self.pv {
            array.push(&JsValue::from_str(mv));
        }
        array
    }

    #[wasm_bindgen(getter, js_name = elapsedMs)]
    pub fn elapsed_ms(&self) -> f64 {
        self.elapsed_ms
    }

    #[wasm_bindgen(getter)]
    pub fn nodes(&self) -> f64 {
        self.nodes as f64
    }

    #[wasm_bindgen(getter, js_name = ttHits)]
    pub fn tt_hits(&self) -> f64 {
        self.tt_hits as f64
    }

    #[wasm_bindgen(getter, js_name = ttStores)]
    pub fn tt_stores(&self) -> f64 {
        self.tt_stores as f64
    }

    #[wasm_bindgen(getter, js_name = ttCollisions)]
    pub fn tt_collisions(&self) -> f64 {
        self.tt_collisions as f64
    }
}

fn elapsed_ms_since(started_at: Instant) -> f64 {
    started_at.elapsed().as_secs_f64() * 1_000.0
}

fn current_evaluation_strategy() -> EvaluationStrategy {
    match current_nnue_model() {
        Some(model) => EvaluationStrategy::Nnue {
            model,
            mode: SearchEvalMode::Incremental,
        },
        None => EvaluationStrategy::Handcrafted,
    }
}

fn best_move_impl(sfen: &str, depth: u8) -> Result<Option<String>, String> {
    Ok(search_impl(sfen, depth)?.best_move)
}

fn load_nnue_impl(bytes: &[u8]) -> Result<String, String> {
    #[cfg(any(feature = "annan", feature = "anhoku", feature = "antouzai"))]
    {
        let _ = bytes;
        Err("NNUE is currently only supported for standard shogi rules.".to_string())
    }

    #[cfg(not(any(feature = "annan", feature = "anhoku", feature = "antouzai")))]
    {
        let model =
            NnueModel::from_bytes(bytes).map_err(|err| format!("failed to load NNUE: {err}"))?;
        let description = model.description().to_string();
        *nnue_model_slot().write().unwrap() = Some(Arc::new(model));
        Ok(description)
    }
}

fn search_impl(sfen: &str, depth: u8) -> Result<SearchSummary, String> {
    search_impl_with_strategy(sfen, depth, current_evaluation_strategy())
}

fn search_impl_with_strategy(
    sfen: &str,
    depth: u8,
    evaluation: EvaluationStrategy,
) -> Result<SearchSummary, String> {
    let board = Board::from_sfen(sfen).map_err(|err| format!("failed to parse SFEN: {err}"))?;
    search_board_with_strategy(&board, depth.max(1), evaluation, None)
        .map_err(|_| "search timed out unexpectedly".to_string())
}

fn search_board_with_strategy(
    board: &Board,
    depth: u8,
    evaluation: EvaluationStrategy,
    deadline: Option<Instant>,
) -> Result<SearchSummary, SearchInterrupted> {
    let started_at = Instant::now();
    let root_state = match &evaluation {
        EvaluationStrategy::Nnue {
            model,
            mode: SearchEvalMode::Incremental,
        } if has_both_kings(board) => Some(model.build_position_state_full(board)),
        _ => None,
    };
    let mut ctx = SearchContext {
        states: 0,
        evaluation,
        deadline,
    };
    let (best_move, best_score) = search_best_move(board, depth, &mut ctx, root_state)?
        .map(|(mv, score)| (Some(mv.to_string()), Some(score)))
        .unwrap_or((None, None));
    let elapsed_ms = elapsed_ms_since(started_at).max(0.0);
    let nps = if elapsed_ms > 0.0 {
        ctx.states as f64 / (elapsed_ms / 1_000.0)
    } else {
        0.0
    };

    Ok(SearchSummary {
        best_move,
        best_score,
        elapsed_ms,
        states: ctx.states,
        nps,
    })
}

fn root_dfpn_options(timeout_ms: u32) -> DfpnOptions {
    let max_time_ms = if timeout_ms == 0 {
        None
    } else {
        Some(u64::from((timeout_ms / 4).min(25).max(1)))
    };

    DfpnOptions {
        max_nodes: Some(10_000),
        max_time_ms,
        tt_megabytes: 4,
        max_pv_moves: 64,
    }
}

fn to_dfpn_summary(core: CoreDfpnResult) -> DfpnSummary {
    let best_move = core.pv.first().map(ToString::to_string);
    let selected = core.status == DfpnStatus::Mate && best_move.is_some();
    DfpnSummary {
        status: core.status.as_str().to_string(),
        selected,
        best_move,
        elapsed_ms: core.stats.elapsed_ms,
        nodes: core.stats.nodes,
        tt_hits: core.stats.tt_hits,
        tt_stores: core.stats.tt_stores,
        tt_collisions: core.stats.tt_collisions,
    }
}

fn has_checking_move(board: &Board) -> bool {
    board.generate_checks(|_| true)
}

fn strict_parse_error(err: impl std::fmt::Display) -> String {
    format!("failed to parse SFEN: {err}")
}

fn search_iterative_deepening_with_strategy(
    sfen: &str,
    max_depth: u8,
    timeout_ms: u32,
    evaluation: EvaluationStrategy,
    config: IterativeSearchConfig,
) -> Result<IterativeSearchSummary, String> {
    let deadline = if timeout_ms == 0 {
        None
    } else {
        Some(Instant::now() + std::time::Duration::from_millis(u64::from(timeout_ms)))
    };
    search_iterative_deepening_with_strategy_and_deadline(
        sfen, max_depth, timeout_ms, evaluation, config, deadline,
    )
}

fn search_iterative_deepening_with_strategy_and_deadline(
    sfen: &str,
    max_depth: u8,
    timeout_ms: u32,
    evaluation: EvaluationStrategy,
    config: IterativeSearchConfig,
    deadline: Option<Instant>,
) -> Result<IterativeSearchSummary, String> {
    let max_depth = max_depth.max(1);
    let started_at = Instant::now();
    let mut dfpn = None;

    let board = match Board::from_sfen(sfen) {
        Ok(board) => board,
        Err(strict_err) => {
            if config.run_dfpn {
                let options = root_dfpn_options(timeout_ms);
                let root_dfpn = dfpn_impl(
                    sfen,
                    options.max_nodes,
                    options.max_time_ms,
                    options.tt_megabytes,
                    options.max_pv_moves,
                )?;
                let dfpn_summary = to_dfpn_summary(root_dfpn);
                if dfpn_summary.selected {
                    let elapsed_ms = elapsed_ms_since(started_at).max(0.0);
                    let best_move = dfpn_summary.best_move.clone();
                    return Ok(IterativeSearchSummary {
                        best_move,
                        completed_depth: 0,
                        timed_out: false,
                        elapsed_ms,
                        states: 0,
                        nps: 0.0,
                        iterations: Vec::new(),
                        dfpn: Some(dfpn_summary),
                    });
                }
            }
            return Err(strict_parse_error(strict_err));
        }
    };

    if config.run_dfpn && has_checking_move(&board) {
        let dfpn_summary = to_dfpn_summary(board.dfpn(&root_dfpn_options(timeout_ms)));
        if dfpn_summary.selected {
            let elapsed_ms = elapsed_ms_since(started_at).max(0.0);
            let best_move = dfpn_summary.best_move.clone();
            return Ok(IterativeSearchSummary {
                best_move,
                completed_depth: 0,
                timed_out: false,
                elapsed_ms,
                states: 0,
                nps: 0.0,
                iterations: Vec::new(),
                dfpn: Some(dfpn_summary),
            });
        }
        dfpn = Some(dfpn_summary);
    }

    let mut iterations = Vec::with_capacity(max_depth as usize);
    let mut completed_depth = 0;
    let mut total_states = 0;
    let mut latest_best_move = None;
    let mut timed_out = false;

    for depth in 1..=max_depth {
        if deadline.is_some_and(|limit| Instant::now() >= limit) {
            timed_out = true;
            break;
        }

        match search_board_with_strategy(&board, depth, evaluation.clone(), deadline) {
            Ok(summary) => {
                total_states += summary.states;
                completed_depth = depth;
                latest_best_move = summary.best_move.clone();
                iterations.push(IterativeIterationSummary {
                    depth,
                    best_move: summary.best_move,
                    elapsed_ms: summary.elapsed_ms,
                    states: summary.states,
                    nps: summary.nps,
                });
            }
            Err(SearchInterrupted) => {
                timed_out = true;
                break;
            }
        }
    }

    let elapsed_ms = elapsed_ms_since(started_at).max(0.0);
    let nps = if elapsed_ms > 0.0 {
        total_states as f64 / (elapsed_ms / 1_000.0)
    } else {
        0.0
    };

    Ok(IterativeSearchSummary {
        best_move: latest_best_move,
        completed_depth,
        timed_out,
        elapsed_ms,
        states: total_states,
        nps,
        iterations,
        dfpn,
    })
}

#[doc(hidden)]
pub fn search_iterative_deepening_impl(
    sfen: &str,
    max_depth: u8,
    timeout_ms: u32,
) -> Result<IterativeSearchSummary, String> {
    search_iterative_deepening_with_strategy(
        sfen,
        max_depth,
        timeout_ms,
        current_evaluation_strategy(),
        IterativeSearchConfig::default(),
    )
}

#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub fn search_iterative_deepening_impl_with_dfpn_mode(
    sfen: &str,
    max_depth: u8,
    timeout_ms: u32,
    run_dfpn: bool,
) -> Result<IterativeSearchSummary, String> {
    search_iterative_deepening_with_strategy(
        sfen,
        max_depth,
        timeout_ms,
        current_evaluation_strategy(),
        IterativeSearchConfig { run_dfpn },
    )
}

#[cfg(test)]
fn search_iterative_deepening_impl_with_deadline(
    sfen: &str,
    max_depth: u8,
    timeout_ms: u32,
    deadline: Instant,
) -> Result<IterativeSearchSummary, String> {
    search_iterative_deepening_with_strategy_and_deadline(
        sfen,
        max_depth,
        timeout_ms,
        current_evaluation_strategy(),
        IterativeSearchConfig::default(),
        Some(deadline),
    )
}

#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub fn search_impl_with_eval_mode(
    sfen: &str,
    depth: u8,
    model: Arc<NnueModel>,
    mode: SearchEvalMode,
) -> Result<SearchSummary, String> {
    search_impl_with_strategy(sfen, depth, EvaluationStrategy::Nnue { model, mode })
}

#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub fn search_board_impl_with_eval_mode(
    board: &Board,
    depth: u8,
    model: Arc<NnueModel>,
    mode: SearchEvalMode,
) -> Result<SearchSummary, String> {
    search_board_with_strategy(
        board,
        depth.max(1),
        EvaluationStrategy::Nnue { model, mode },
        None,
    )
    .map_err(|_| "search timed out unexpectedly".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub fn search_impl_handcrafted(sfen: &str, depth: u8) -> Result<SearchSummary, String> {
    search_impl_with_strategy(sfen, depth, EvaluationStrategy::Handcrafted)
}

#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub fn search_board_impl_handcrafted(board: &Board, depth: u8) -> Result<SearchSummary, String> {
    search_board_with_strategy(board, depth.max(1), EvaluationStrategy::Handcrafted, None)
        .map_err(|_| "search timed out unexpectedly".to_string())
}

fn perft_impl(sfen: &str, depth: u8) -> Result<PerftResult, String> {
    let board = Board::from_sfen(sfen).map_err(|err| format!("failed to parse SFEN: {err}"))?;
    let started_at = Instant::now();
    let nodes = perft_bulk(&board, depth);
    let elapsed_ms = elapsed_ms_since(started_at).max(0.0);
    let nps = if elapsed_ms > 0.0 {
        nodes as f64 / (elapsed_ms / 1_000.0)
    } else {
        0.0
    };

    Ok(PerftResult {
        elapsed_ms,
        nodes,
        nps,
    })
}

fn parse_dfpn_board(sfen: &str) -> Result<Board, String> {
    Board::from_sfen(sfen)
        .or_else(|_| Board::tsume(sfen))
        .map_err(|err| format!("failed to parse SFEN: {err}"))
}

#[doc(hidden)]
pub fn dfpn_impl(
    sfen: &str,
    max_nodes: Option<u64>,
    max_time_ms: Option<u64>,
    tt_megabytes: usize,
    max_pv_moves: usize,
) -> Result<CoreDfpnResult, String> {
    let board = parse_dfpn_board(sfen)?;
    let options = DfpnOptions {
        max_nodes,
        max_time_ms,
        tt_megabytes,
        max_pv_moves,
    };
    Ok(board.dfpn(&options))
}

fn optional_u64_from_f64(name: &str, value: Option<f64>) -> Result<Option<u64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 {
        return Err(format!("{name} must be a non-negative integer"));
    }
    Ok(Some(value as u64))
}

#[wasm_bindgen(js_name = best_move)]
pub fn best_move(sfen: &str, depth: u8) -> Result<Option<String>, JsValue> {
    best_move_impl(sfen, depth).map_err(|err| JsValue::from_str(&err))
}

#[wasm_bindgen(js_name = load_nnue)]
pub fn load_nnue(bytes: &[u8]) -> Result<String, JsValue> {
    load_nnue_impl(bytes).map_err(|err| JsValue::from_str(&err))
}

#[wasm_bindgen]
pub fn search(sfen: &str, depth: u8) -> Result<SearchResult, JsValue> {
    let summary = search_impl(sfen, depth).map_err(|err| JsValue::from_str(&err))?;
    Ok(SearchResult {
        best_move: summary.best_move,
        elapsed_ms: summary.elapsed_ms,
        states: summary.states,
        nps: summary.nps,
    })
}

#[wasm_bindgen(js_name = search_iterative_deepening)]
pub fn search_iterative_deepening(
    sfen: &str,
    max_depth: u8,
    timeout_ms: u32,
) -> Result<IterativeSearchResult, JsValue> {
    let summary = search_iterative_deepening_impl(sfen, max_depth, timeout_ms)
        .map_err(|err| JsValue::from_str(&err))?;
    Ok(IterativeSearchResult {
        best_move: summary.best_move,
        completed_depth: summary.completed_depth,
        timed_out: summary.timed_out,
        elapsed_ms: summary.elapsed_ms,
        states: summary.states,
        nps: summary.nps,
        iterations: summary.iterations,
        dfpn: summary.dfpn,
    })
}

#[wasm_bindgen]
pub fn perft(sfen: &str, depth: u8) -> Result<PerftResult, JsValue> {
    perft_impl(sfen, depth).map_err(|err| JsValue::from_str(&err))
}

#[wasm_bindgen]
pub fn dfpn(
    sfen: &str,
    max_nodes: Option<f64>,
    max_time_ms: Option<f64>,
    tt_megabytes: Option<u32>,
    max_pv_moves: Option<u32>,
) -> Result<DfpnResult, JsValue> {
    let max_nodes =
        optional_u64_from_f64("max_nodes", max_nodes).map_err(|err| JsValue::from_str(&err))?;
    let max_time_ms =
        optional_u64_from_f64("max_time_ms", max_time_ms).map_err(|err| JsValue::from_str(&err))?;
    let core = dfpn_impl(
        sfen,
        max_nodes,
        max_time_ms,
        tt_megabytes.map(|value| value as usize).unwrap_or(16),
        max_pv_moves.map(|value| value as usize).unwrap_or(256),
    )
    .map_err(|err| JsValue::from_str(&err))?;

    Ok(DfpnResult {
        status: core.status.as_str().to_string(),
        pv: core.pv.iter().map(ToString::to_string).collect(),
        elapsed_ms: core.stats.elapsed_ms,
        nodes: core.stats.nodes,
        tt_hits: core.stats.tt_hits,
        tt_stores: core.stats.tt_stores,
        tt_collisions: core.stats.tt_collisions,
    })
}

fn search_best_move(
    board: &Board,
    depth: u8,
    ctx: &mut SearchContext,
    nnue_state: Option<NnuePositionState>,
) -> Result<Option<(Move, i32)>, SearchInterrupted> {
    ctx.record_state()?;
    ctx.check_deadline()?;
    if terminal_score_for_side_to_move(board, 0).is_some() {
        return Ok(None);
    }
    let moves = legal_moves(board);
    if moves.is_empty() {
        return Ok(None);
    }

    let mut alpha = -INF_SCORE;
    let beta = INF_SCORE;
    let mut best_score = -INF_SCORE;
    let mut best_move = None;

    for mv in moves {
        ctx.check_deadline()?;
        let mut child = board.clone();
        child.play_unchecked(mv);
        let score = if let Some(terminal) = terminal_score_for_side_to_move(&child, 1) {
            -terminal
        } else {
            let child_state = child_nnue_state(ctx, board, &child, nnue_state.as_ref(), mv);
            -negamax(
                &child,
                depth.saturating_sub(1),
                -beta,
                -alpha,
                1,
                ctx,
                child_state,
            )?
        };
        if score > best_score {
            best_score = score;
            best_move = Some(mv);
        }
        alpha = alpha.max(score);
    }

    Ok(best_move.map(|mv| (mv, best_score)))
}

fn negamax(
    board: &Board,
    depth: u8,
    mut alpha: i32,
    beta: i32,
    ply: i32,
    ctx: &mut SearchContext,
    nnue_state: Option<NnuePositionState>,
) -> Result<i32, SearchInterrupted> {
    ctx.record_state()?;
    ctx.check_deadline()?;
    if let Some(terminal) = terminal_score_for_side_to_move(board, ply) {
        return Ok(terminal);
    }
    if depth == 0 {
        return Ok(evaluate_or_mate(board, ply, ctx, nnue_state.as_ref()));
    }

    let moves = legal_moves(board);
    if moves.is_empty() {
        return Ok(-MATE_SCORE + ply);
    }

    let mut best_score = -INF_SCORE;
    for mv in moves {
        ctx.check_deadline()?;
        let mut child = board.clone();
        child.play_unchecked(mv);
        let score = if let Some(terminal) = terminal_score_for_side_to_move(&child, ply + 1) {
            -terminal
        } else {
            let child_state = child_nnue_state(ctx, board, &child, nnue_state.as_ref(), mv);
            -negamax(&child, depth - 1, -beta, -alpha, ply + 1, ctx, child_state)?
        };
        if score > best_score {
            best_score = score;
        }
        if score > alpha {
            alpha = score;
        }
        if alpha >= beta {
            break;
        }
    }

    Ok(best_score)
}

fn child_nnue_state(
    ctx: &SearchContext,
    parent_board: &Board,
    child_board: &Board,
    parent_state: Option<&NnuePositionState>,
    mv: Move,
) -> Option<NnuePositionState> {
    if !has_both_kings(child_board) {
        return None;
    }
    match &ctx.evaluation {
        EvaluationStrategy::Nnue {
            model,
            mode: SearchEvalMode::Incremental,
        } => Some(model.apply_move(
            parent_board,
            child_board,
            parent_state.expect("incremental search should have NNUE state"),
            mv,
        )),
        _ => None,
    }
}

fn evaluate_or_mate(
    board: &Board,
    ply: i32,
    ctx: &SearchContext,
    nnue_state: Option<&NnuePositionState>,
) -> i32 {
    if let Some(terminal) = terminal_score_for_side_to_move(board, ply) {
        return terminal;
    }
    let us = board.side_to_move();
    let our_mobility = count_legal_moves(board) as i32;
    if our_mobility == 0 {
        return -MATE_SCORE + ply;
    }

    match &ctx.evaluation {
        EvaluationStrategy::Handcrafted => {
            let them = !us;
            material_score(board, us) - material_score(board, them)
                + MOBILITY_WEIGHT * (our_mobility - opponent_mobility(board) as i32)
        }
        EvaluationStrategy::Nnue {
            model,
            mode: SearchEvalMode::FullRefresh,
        } => model.evaluate_full_refresh(board),
        EvaluationStrategy::Nnue {
            model,
            mode: SearchEvalMode::Incremental,
        } => model.evaluate_from_state(
            board,
            nnue_state.expect("incremental evaluation should receive NNUE state"),
        ),
    }
}

fn has_both_kings(board: &Board) -> bool {
    board.has(Color::Black, Piece::King) && board.has(Color::White, Piece::King)
}

fn terminal_score_for_side_to_move(board: &Board, ply: i32) -> Option<i32> {
    let us = board.side_to_move();
    if !board.has(us, Piece::King) {
        Some(-MATE_SCORE + ply)
    } else if !board.has(!us, Piece::King) {
        Some(MATE_SCORE - ply)
    } else {
        None
    }
}

fn material_score(board: &Board, color: Color) -> i32 {
    let mut score = 0;

    for &piece in &Piece::ALL {
        score += board.colored_pieces(color, piece).len() as i32 * piece_value(piece);
    }

    for &piece in &HAND_PIECES {
        score += i32::from(board.num_in_hand(color, piece)) * piece_value(piece);
    }

    score
}

fn opponent_mobility(board: &Board) -> usize {
    board
        .null_move()
        .map(|opponent_board| count_legal_moves(&opponent_board))
        .unwrap_or(0)
}

fn count_legal_moves(board: &Board) -> usize {
    let mut count = 0;
    board.generate_moves(|moves| {
        count += moves.len();
        false
    });
    count
}

fn legal_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|piece_moves| {
        moves.extend(piece_moves);
        false
    });
    if moves.len() > 1 {
        moves.sort_unstable_by_key(|mv| move_order_key(board, *mv));
    }
    moves
}

fn move_order_key(board: &Board, mv: Move) -> (Reverse<i32>, Reverse<u8>, u8, u8, u8, u8) {
    (
        Reverse(capture_value(board, mv)),
        Reverse(u8::from(mv.is_promotion())),
        u8::from(mv.is_drop()),
        move_to_index(mv),
        move_from_or_piece_index(mv),
        move_piece_kind_index(mv),
    )
}

fn move_to_index(mv: Move) -> u8 {
    mv.to() as u8
}

fn move_from_or_piece_index(mv: Move) -> u8 {
    match mv {
        Move::BoardMove { from, .. } => from as u8,
        Move::Drop { piece, .. } => piece as u8,
    }
}

fn move_piece_kind_index(mv: Move) -> u8 {
    match mv {
        Move::Drop { piece, .. } => piece as u8,
        Move::BoardMove { .. } => u8::MAX,
    }
}

fn capture_value(board: &Board, mv: Move) -> i32 {
    match mv {
        Move::BoardMove { to, .. } => board
            .color_on(to)
            .filter(|color| *color != board.side_to_move())
            .and_then(|_| board.piece_on(to))
            .map(piece_value)
            .unwrap_or(0),
        Move::Drop { .. } => 0,
    }
}

fn perft_bulk(board: &Board, depth: u8) -> u64 {
    let mut nodes = 0;
    match depth {
        0 => nodes += 1,
        1 => {
            board.generate_board_moves(|moves| {
                nodes += moves.into_iter().len() as u64;
                false
            });
            board.generate_drops(|moves| {
                nodes += moves.into_iter().len() as u64;
                false
            });
        }
        _ => {
            board.generate_board_moves(|moves| {
                for mv in moves {
                    let mut child = board.clone();
                    child.play_unchecked(mv);
                    nodes += perft_bulk(&child, depth - 1);
                }
                false
            });
            board.generate_drops(|moves| {
                for mv in moves {
                    let mut child = board.clone();
                    child.play_unchecked(mv);
                    nodes += perft_bulk(&child, depth - 1);
                }
                false
            });
        }
    }
    nodes
}

fn piece_value(piece: Piece) -> i32 {
    match piece {
        Piece::Pawn => 100,
        Piece::Lance => 300,
        Piece::Knight => 300,
        Piece::Silver => 400,
        Piece::Gold => 500,
        Piece::Bishop => 700,
        Piece::Rook => 800,
        Piece::King => 0,
        Piece::Tokin | Piece::PLance | Piece::PKnight | Piece::PSilver => 550,
        Piece::PBishop => 900,
        Piece::PRook => 1000,
    }
}

fn nnue_model_slot() -> &'static RwLock<Option<Arc<NnueModel>>> {
    NNUE_MODEL.get_or_init(|| RwLock::new(None))
}

fn current_nnue_model() -> Option<Arc<NnueModel>> {
    nnue_model_slot().read().unwrap().clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(any(feature = "annan", feature = "anhoku", feature = "antouzai")))]
    use std::path::PathBuf;
    #[cfg(not(any(feature = "annan", feature = "anhoku", feature = "antouzai")))]
    use std::sync::Arc;

    const DFPN_MATE_SFEN: &str = "8k/6G2/7B1/9/9/9/9/9/K8 b R 1";
    const DFPN_NO_MATE_SFEN: &str = "4k4/9/9/9/9/9/9/9/4K4 b - 1";
    #[cfg(feature = "annan")]
    const DFPN_ANNAN_PROBLEM_SFEN: &str = "7p1/8k/5+R3/6P2/7G1/9/9/9/9 b N 1";

    #[cfg(feature = "annan")]
    fn parse_dfpn_test_board(sfen: &str) -> Board {
        Board::from_sfen(sfen)
            .or_else(|_| Board::tsume(sfen))
            .unwrap()
    }

    #[cfg(not(any(feature = "annan", feature = "anhoku", feature = "antouzai")))]
    fn load_test_nnue() -> Option<NnueModel> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shogi-878ca61334a7.nnue");
        let bytes = std::fs::read(path).ok()?;
        NnueModel::from_bytes(&bytes).ok()
    }

    fn first_checking_child(board: &Board) -> Board {
        let mut checking_move = None;
        board.generate_checks(|moves| {
            checking_move = moves.into_iter().next();
            checking_move.is_some()
        });
        let mv = checking_move.expect("expected at least one checking move");
        let mut child = board.clone();
        child.play_unchecked(mv);
        child
    }

    #[cfg(any(
        feature = "annan",
        not(any(feature = "annan", feature = "anhoku", feature = "antouzai"))
    ))]
    fn assert_legal_best_move(sfen: &str, depth: u8) {
        let board = Board::from_sfen(sfen).unwrap();
        let best = search_impl(sfen, depth)
            .unwrap()
            .best_move
            .expect("expected a legal move");
        let mv: Move = best.parse().unwrap();
        assert!(
            board.is_legal(mv),
            "best move {best} should be legal for {sfen}"
        );
    }

    #[test]
    #[cfg(not(any(feature = "annan", feature = "anhoku", feature = "antouzai")))]
    fn returns_legal_move_for_start_position() {
        assert_legal_best_move(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            2,
        );
    }

    #[test]
    #[cfg(not(any(feature = "annan", feature = "anhoku", feature = "antouzai")))]
    fn returns_legal_move_for_handicap_position() {
        assert_legal_best_move(
            "2sgkgs2/9/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL w - 2",
            2,
        );
    }

    #[test]
    #[cfg(feature = "annan")]
    fn returns_legal_move_for_annan_start_position() {
        assert_legal_best_move(haitaka::SFEN_STARTPOS, 2);
    }

    #[test]
    #[cfg(not(any(feature = "annan", feature = "anhoku", feature = "antouzai")))]
    fn returns_none_when_side_to_move_has_no_legal_move() {
        let sfen = "lns4+Rl/1r1g5/p1p1pSp1p/1p1p1p3/8k/7NG/PPPPPPP1P/1B7/LNSGKGSNL w B2p 26";
        assert_eq!(best_move_impl(sfen, 2).unwrap(), None);
    }

    #[test]
    fn prefers_capturing_a_hanging_rook_in_a_simple_tactical_position() {
        let sfen = "9/9/k8/9/4Rr3/9/9/9/4K4 b - 1";
        assert_eq!(best_move_impl(sfen, 1).unwrap().as_deref(), Some("5e4e"));
    }

    #[test]
    fn reports_search_statistics() {
        let summary = search_impl(haitaka::SFEN_STARTPOS, 1).unwrap();
        assert!(summary.states > 0);
        assert!(summary.elapsed_ms >= 0.0);
        assert!(summary.nps >= 0.0);
        assert!(summary.best_move.is_some());
    }

    #[test]
    fn board_native_handcrafted_search_handles_live_check_positions() {
        let board = Board::from_sfen(DFPN_MATE_SFEN).unwrap();
        let checking_child = first_checking_child(&board);
        let strict_sfen = checking_child.to_string();

        let summary = search_board_impl_handcrafted(&checking_child, 1).unwrap();
        assert!(summary.states > 0);
        if let Ok(roundtripped) = search_impl_handcrafted(&strict_sfen, 1) {
            assert_eq!(summary.best_move, roundtripped.best_move);
            assert_eq!(summary.best_score, roundtripped.best_score);
        }
    }

    #[test]
    fn iterative_search_reaches_requested_depth_when_time_allows() {
        let summary = search_iterative_deepening_impl(haitaka::SFEN_STARTPOS, 3, 5_000).unwrap();
        assert_eq!(summary.completed_depth, 3);
        assert!(!summary.timed_out);
        assert_eq!(summary.iterations.len(), 3);
        assert_eq!(
            summary.best_move,
            summary
                .iterations
                .last()
                .and_then(|it| it.best_move.clone())
        );
        assert!(summary.states > 0);
        assert!(summary.nps >= 0.0);
        assert!(summary.dfpn.is_none());
    }

    #[test]
    fn iterative_search_times_out_before_any_completed_iteration() {
        let summary = search_iterative_deepening_impl_with_deadline(
            haitaka::SFEN_STARTPOS,
            3,
            1,
            Instant::now(),
        )
        .unwrap();

        assert_eq!(summary.completed_depth, 0);
        assert!(summary.timed_out);
        assert!(summary.best_move.is_none());
        assert!(summary.iterations.is_empty());
        assert_eq!(summary.states, 0);
        assert_eq!(summary.nps, 0.0);
    }

    #[test]
    fn iterative_search_uses_dfpn_for_standard_mate() {
        let summary =
            search_iterative_deepening_impl_with_dfpn_mode(DFPN_MATE_SFEN, 4, 0, true).unwrap();

        assert_eq!(summary.completed_depth, 0);
        assert!(!summary.timed_out);
        assert!(summary.iterations.is_empty());
        assert_eq!(summary.best_move.as_deref(), Some("R*1b"));
        let dfpn = summary.dfpn.expect("expected DFPN telemetry");
        assert_eq!(dfpn.status, "mate");
        assert!(dfpn.selected);
        assert_eq!(dfpn.best_move.as_deref(), Some("R*1b"));
    }

    #[test]
    fn iterative_search_can_disable_dfpn_short_circuiting() {
        let summary =
            search_iterative_deepening_impl_with_dfpn_mode(DFPN_MATE_SFEN, 2, 5_000, false)
                .unwrap();

        assert!(summary.completed_depth > 0);
        assert!(!summary.timed_out);
        assert!(summary.dfpn.is_none());
    }

    #[test]
    #[cfg(not(any(feature = "annan", feature = "anhoku", feature = "antouzai")))]
    fn iterative_search_uses_dfpn_tsume_fallback_for_invalid_strict_sfen() {
        let summary =
            search_iterative_deepening_impl("8k/6G2/7B1/9/9/9/9/9/9 b R 1", 4, 5_000).unwrap();

        assert_eq!(summary.completed_depth, 0);
        assert_eq!(
            summary.dfpn.as_ref().map(|dfpn| dfpn.status.as_str()),
            Some("mate")
        );
        assert!(summary.dfpn.as_ref().is_some_and(|dfpn| dfpn.selected));
        assert_eq!(summary.best_move.as_deref(), Some("R*1b"));
    }

    #[test]
    fn iterative_search_preserves_parse_error_when_dfpn_cannot_help() {
        let err = search_iterative_deepening_impl("invalid", 4, 5_000).unwrap_err();
        assert!(err.contains("failed to parse SFEN"));
    }

    #[test]
    fn dfpn_matches_core_mate_result() {
        let board = Board::from_sfen(DFPN_MATE_SFEN).unwrap();
        let expected = board.dfpn(&DfpnOptions::default());
        let actual = dfpn_impl(DFPN_MATE_SFEN, None, None, 16, 256).unwrap();
        assert_eq!(actual.status, expected.status);
        assert_eq!(actual.pv.first(), expected.pv.first());
    }

    #[test]
    fn dfpn_matches_core_no_mate_result() {
        let board = Board::from_sfen(DFPN_NO_MATE_SFEN).unwrap();
        let expected = board.dfpn(&DfpnOptions::default());
        let actual = dfpn_impl(DFPN_NO_MATE_SFEN, None, None, 16, 256).unwrap();
        assert_eq!(actual.status, expected.status);
        assert_eq!(actual.pv, expected.pv);
    }

    #[test]
    #[cfg(not(any(feature = "annan", feature = "anhoku", feature = "antouzai")))]
    fn dfpn_parses_tsume_sfens() {
        let result = dfpn_impl(
            "lpg6/3s2R2/1kpppp3/p8/9/P8/2N6/9/9 b BGN 1",
            None,
            None,
            16,
            256,
        )
        .unwrap();
        assert_eq!(result.status.as_str(), "mate");
    }

    #[test]
    fn dfpn_rejects_invalid_sfen() {
        let err = dfpn_impl("invalid", None, None, 16, 256).unwrap_err();
        assert!(err.contains("failed to parse SFEN"));
    }

    #[test]
    #[cfg(feature = "annan")]
    fn dfpn_matches_core_on_specific_annan_problem() {
        let board = parse_dfpn_test_board(DFPN_ANNAN_PROBLEM_SFEN);
        let expected = board.dfpn(&DfpnOptions::default());
        let actual = dfpn_impl(DFPN_ANNAN_PROBLEM_SFEN, None, None, 16, 256).unwrap();
        assert_eq!(actual.status, expected.status);
        assert_eq!(actual.pv.first(), expected.pv.first());
    }

    #[test]
    #[cfg(feature = "annan")]
    fn iterative_search_uses_dfpn_for_annan_mate() {
        let summary = search_iterative_deepening_impl(DFPN_ANNAN_PROBLEM_SFEN, 4, 0).unwrap();

        assert_eq!(summary.completed_depth, 0);
        assert!(!summary.timed_out);
        assert_eq!(summary.best_move.as_deref(), Some("4c1c"));
        assert_eq!(
            summary.dfpn.as_ref().map(|dfpn| dfpn.status.as_str()),
            Some("mate")
        );
        assert!(summary.dfpn.as_ref().is_some_and(|dfpn| dfpn.selected));
    }

    #[test]
    #[cfg(feature = "annan")]
    fn iterative_search_does_not_false_mate_shared_backer_double_check_position() {
        let summary = search_iterative_deepening_impl(
            "1nsgkgs+Bl/1r5b1/2pp2p1p/1p5P1/2n6/1P1Pl4/2P2PP1P/5K3/1+lS+rpGSN+l w N4Pgp 1",
            6,
            5_000,
        )
        .unwrap();

        assert!(
            !summary.dfpn.as_ref().is_some_and(|dfpn| dfpn.selected),
            "DFPN should not short-circuit this position as mate"
        );
    }

    #[test]
    #[cfg(feature = "annan")]
    fn iterative_search_does_not_choose_illegal_pawn_drop_mate() {
        let summary = search_iterative_deepening_impl(
            "1nsg1gb1+B/1r1k5/2pp2p1p/1p4gp1/2n3n2/1P1PP2P1/2P1LKP1P/2+r6/2+l2GS+l1 w SL3Psn2p 1",
            6,
            5_000,
        )
        .unwrap();

        assert_ne!(summary.best_move.as_deref(), Some("P*4f"));
        assert!(
            !summary
                .dfpn
                .as_ref()
                .is_some_and(|dfpn| dfpn.best_move.as_deref() == Some("P*4f")),
            "DFPN should not treat the illegal pawn-drop mate as a candidate"
        );
    }

    #[cfg(not(any(feature = "annan", feature = "anhoku", feature = "antouzai")))]
    fn assert_search_modes_match(sfen: &str, depth: u8) {
        let Some(model) = load_test_nnue() else {
            return;
        };
        let model = Arc::new(model);
        let full_refresh =
            search_impl_with_eval_mode(sfen, depth, model.clone(), SearchEvalMode::FullRefresh)
                .unwrap();
        let incremental =
            search_impl_with_eval_mode(sfen, depth, model, SearchEvalMode::Incremental).unwrap();

        assert_eq!(incremental.best_move, full_refresh.best_move);
        assert_eq!(incremental.best_score, full_refresh.best_score);
    }

    #[test]
    #[cfg(not(any(feature = "annan", feature = "anhoku", feature = "antouzai")))]
    fn nnue_search_modes_match_on_start_position() {
        assert_search_modes_match(haitaka::SFEN_STARTPOS, 3);
    }

    #[test]
    #[cfg(not(any(feature = "annan", feature = "anhoku", feature = "antouzai")))]
    fn nnue_search_modes_match_on_handicap_position() {
        assert_search_modes_match(haitaka::SFEN_6PIECE_HANDICAP, 3);
    }

    #[test]
    #[cfg(not(any(feature = "annan", feature = "anhoku", feature = "antouzai")))]
    fn nnue_search_modes_match_on_tactical_position() {
        assert_search_modes_match("9/9/k8/9/4Rr3/9/9/9/4K4 b - 1", 3);
    }

    #[test]
    #[cfg(not(any(feature = "annan", feature = "anhoku", feature = "antouzai")))]
    fn loads_test_nnue_when_available() {
        let Some(model) = load_test_nnue() else {
            return;
        };
        assert!(!model.description().is_empty());
        let score = model.evaluate(&Board::startpos());
        assert!(score.abs() < INF_SCORE);
    }

    #[test]
    #[cfg(not(any(feature = "annan", feature = "anhoku", feature = "antouzai")))]
    fn board_native_nnue_search_handles_live_check_positions() {
        let Some(model) = load_test_nnue() else {
            return;
        };
        let model = Arc::new(model);

        let board = Board::from_sfen(DFPN_MATE_SFEN).unwrap();
        let checking_child = first_checking_child(&board);
        let strict_sfen = checking_child.to_string();

        let summary = search_board_impl_with_eval_mode(
            &checking_child,
            1,
            model.clone(),
            SearchEvalMode::Incremental,
        )
        .unwrap();
        assert!(summary.states > 0);
        if let Ok(roundtripped) =
            search_impl_with_eval_mode(&strict_sfen, 1, model, SearchEvalMode::Incremental)
        {
            assert_eq!(summary.best_move, roundtripped.best_move);
            assert_eq!(summary.best_score, roundtripped.best_score);
        }
    }

    #[test]
    #[cfg(feature = "annan")]
    fn perft_matches_annan_start_position_depth_four() {
        assert_eq!(perft_bulk(&Board::startpos(), 4), 605_424);
    }
}

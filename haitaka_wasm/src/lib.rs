mod movepick;
mod nnue;
mod tt;

use std::str::FromStr;
use std::sync::{Arc, OnceLock, RwLock};

use haitaka::{Board, Color, DfpnOptions, DfpnResult as CoreDfpnResult, DfpnStatus, Move, Piece};
use instant::Instant;
use movepick::{MovePicker, MoveSource, QsearchMovePicker, SearchOrdering, SearchOrderingStats};
pub use nnue::{NnueModel, NnuePositionState};
use tt::{Bound, SearchTtStats, TranspositionTable};
use wasm_bindgen::prelude::*;

const INF_SCORE: i32 = 32_000;
const MATE_SCORE: i32 = 30_000;
const MATE_TT_THRESHOLD: i32 = MATE_SCORE - 1024;
const MOBILITY_WEIGHT: i32 = 2;
const ENGINE_NAME: &str = "Haitaka Variants";
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
static SEARCH_TT: OnceLock<RwLock<TranspositionTable>> = OnceLock::new();
const DEADLINE_CHECK_INTERVAL: u64 = 256;
const DEFAULT_QSEARCH_LIMITS: QsearchLimits = QsearchLimits {
    max_ply: 8,
    check_budget: 1,
    node_limit: 1_000_000,
    delta_margin: 300,
    delta_min_qply: 1,
};
const NEKO_QSEARCH_LIMITS: QsearchLimits = QsearchLimits {
    max_ply: 6,
    check_budget: 0,
    node_limit: 250_000,
    delta_margin: 500,
    delta_min_qply: u8::MAX,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QsearchLimits {
    max_ply: u8,
    check_budget: u8,
    node_limit: u64,
    delta_margin: i32,
    delta_min_qply: u8,
}

fn qsearch_limits() -> QsearchLimits {
    if cfg!(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )) {
        NEKO_QSEARCH_LIMITS
    } else {
        DEFAULT_QSEARCH_LIMITS
    }
}

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct SearchSummary {
    pub best_move: Option<String>,
    pub best_score: Option<i32>,
    pub elapsed_ms: f64,
    pub states: u64,
    pub nps: f64,
    pub tt_stats: SearchTtStats,
    pub ordering_stats: SearchOrderingStats,
    pub qsearch_stats: SearchQsearchStats,
}

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct IterativeIterationSummary {
    pub depth: u8,
    pub best_move: Option<String>,
    pub elapsed_ms: f64,
    pub states: u64,
    pub nps: f64,
    pub tt_stats: SearchTtStats,
    pub ordering_stats: SearchOrderingStats,
    pub qsearch_stats: SearchQsearchStats,
}

#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchQsearchStats {
    pub qnodes: u64,
    pub qsearch_max_ply: u8,
    pub qsearch_cap_hits: u64,
    pub qsearch_check_move_tries: u64,
    pub qsearch_delta_prunes: u64,
}

impl SearchQsearchStats {
    pub fn add_iteration(&mut self, iteration: Self) {
        self.qnodes += iteration.qnodes;
        self.qsearch_max_ply = self.qsearch_max_ply.max(iteration.qsearch_max_ply);
        self.qsearch_cap_hits += iteration.qsearch_cap_hits;
        self.qsearch_check_move_tries += iteration.qsearch_check_move_tries;
        self.qsearch_delta_prunes += iteration.qsearch_delta_prunes;
    }
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
    pub tt_stats: SearchTtStats,
    pub ordering_stats: SearchOrderingStats,
    pub qsearch_stats: SearchQsearchStats,
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

struct SearchContext<'a> {
    states: u64,
    evaluation: EvaluationStrategy,
    deadline: Option<Instant>,
    tt: &'a mut TranspositionTable,
    tt_stats: SearchTtStats,
    ordering: &'a mut SearchOrdering,
    ordering_stats: SearchOrderingStats,
    qsearch_stats: SearchQsearchStats,
    qsearch_limits: QsearchLimits,
}

impl SearchContext<'_> {
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

    fn record_qnode(&mut self, qply: u8) -> Result<bool, SearchInterrupted> {
        self.qsearch_stats.qnodes += 1;
        self.qsearch_stats.qsearch_max_ply = self.qsearch_stats.qsearch_max_ply.max(qply);
        if self.deadline.is_some() && self.qsearch_stats.qnodes % DEADLINE_CHECK_INTERVAL == 0 {
            self.check_deadline()?;
        }
        if self.qsearch_stats.qnodes > self.qsearch_limits.node_limit {
            self.qsearch_stats.qsearch_cap_hits += 1;
            return Ok(false);
        }
        Ok(true)
    }
}

#[wasm_bindgen]
pub struct SearchResult {
    best_move: Option<String>,
    elapsed_ms: f64,
    states: u64,
    nps: f64,
    tt_stats: SearchTtStats,
    ordering_stats: SearchOrderingStats,
    qsearch_stats: SearchQsearchStats,
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

    #[wasm_bindgen(getter, js_name = ttProbes)]
    pub fn tt_probes(&self) -> f64 {
        self.tt_stats.tt_probes as f64
    }

    #[wasm_bindgen(getter, js_name = ttHits)]
    pub fn tt_hits(&self) -> f64 {
        self.tt_stats.tt_hits as f64
    }

    #[wasm_bindgen(getter, js_name = ttCutoffs)]
    pub fn tt_cutoffs(&self) -> f64 {
        self.tt_stats.tt_cutoffs as f64
    }

    #[wasm_bindgen(getter, js_name = ttStores)]
    pub fn tt_stores(&self) -> f64 {
        self.tt_stats.tt_stores as f64
    }

    #[wasm_bindgen(getter, js_name = ttCollisions)]
    pub fn tt_collisions(&self) -> f64 {
        self.tt_stats.tt_collisions as f64
    }

    #[wasm_bindgen(getter, js_name = ttHashfull)]
    pub fn tt_hashfull(&self) -> f64 {
        self.tt_stats.tt_hashfull as f64
    }

    #[wasm_bindgen(getter, js_name = betaCutoffs)]
    pub fn beta_cutoffs(&self) -> f64 {
        self.ordering_stats.beta_cutoffs as f64
    }

    #[wasm_bindgen(getter, js_name = firstMoveCutoffs)]
    pub fn first_move_cutoffs(&self) -> f64 {
        self.ordering_stats.first_move_cutoffs as f64
    }

    #[wasm_bindgen(getter, js_name = hashMoveTries)]
    pub fn hash_move_tries(&self) -> f64 {
        self.ordering_stats.hash_move_tries as f64
    }

    #[wasm_bindgen(getter, js_name = hashMoveCutoffs)]
    pub fn hash_move_cutoffs(&self) -> f64 {
        self.ordering_stats.hash_move_cutoffs as f64
    }

    #[wasm_bindgen(getter, js_name = killerMoveTries)]
    pub fn killer_move_tries(&self) -> f64 {
        self.ordering_stats.killer_move_tries as f64
    }

    #[wasm_bindgen(getter, js_name = killerMoveCutoffs)]
    pub fn killer_move_cutoffs(&self) -> f64 {
        self.ordering_stats.killer_move_cutoffs as f64
    }

    #[wasm_bindgen(getter, js_name = historyMoveTries)]
    pub fn history_move_tries(&self) -> f64 {
        self.ordering_stats.history_move_tries as f64
    }

    #[wasm_bindgen(getter, js_name = historyMoveCutoffs)]
    pub fn history_move_cutoffs(&self) -> f64 {
        self.ordering_stats.history_move_cutoffs as f64
    }

    #[wasm_bindgen(getter)]
    pub fn qnodes(&self) -> f64 {
        self.qsearch_stats.qnodes as f64
    }

    #[wasm_bindgen(getter, js_name = qsearchMaxPly)]
    pub fn qsearch_max_ply(&self) -> u32 {
        u32::from(self.qsearch_stats.qsearch_max_ply)
    }

    #[wasm_bindgen(getter, js_name = qsearchCapHits)]
    pub fn qsearch_cap_hits(&self) -> f64 {
        self.qsearch_stats.qsearch_cap_hits as f64
    }

    #[wasm_bindgen(getter, js_name = qsearchCheckMoveTries)]
    pub fn qsearch_check_move_tries(&self) -> f64 {
        self.qsearch_stats.qsearch_check_move_tries as f64
    }

    #[wasm_bindgen(getter, js_name = qsearchDeltaPrunes)]
    pub fn qsearch_delta_prunes(&self) -> f64 {
        self.qsearch_stats.qsearch_delta_prunes as f64
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
    tt_stats: SearchTtStats,
    ordering_stats: SearchOrderingStats,
    qsearch_stats: SearchQsearchStats,
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

    #[wasm_bindgen(getter, js_name = ttProbes)]
    pub fn tt_probes(&self) -> f64 {
        self.tt_stats.tt_probes as f64
    }

    #[wasm_bindgen(getter, js_name = ttHits)]
    pub fn tt_hits(&self) -> f64 {
        self.tt_stats.tt_hits as f64
    }

    #[wasm_bindgen(getter, js_name = ttCutoffs)]
    pub fn tt_cutoffs(&self) -> f64 {
        self.tt_stats.tt_cutoffs as f64
    }

    #[wasm_bindgen(getter, js_name = ttStores)]
    pub fn tt_stores(&self) -> f64 {
        self.tt_stats.tt_stores as f64
    }

    #[wasm_bindgen(getter, js_name = ttCollisions)]
    pub fn tt_collisions(&self) -> f64 {
        self.tt_stats.tt_collisions as f64
    }

    #[wasm_bindgen(getter, js_name = ttHashfull)]
    pub fn tt_hashfull(&self) -> f64 {
        self.tt_stats.tt_hashfull as f64
    }

    #[wasm_bindgen(getter, js_name = betaCutoffs)]
    pub fn beta_cutoffs(&self) -> f64 {
        self.ordering_stats.beta_cutoffs as f64
    }

    #[wasm_bindgen(getter, js_name = firstMoveCutoffs)]
    pub fn first_move_cutoffs(&self) -> f64 {
        self.ordering_stats.first_move_cutoffs as f64
    }

    #[wasm_bindgen(getter, js_name = hashMoveTries)]
    pub fn hash_move_tries(&self) -> f64 {
        self.ordering_stats.hash_move_tries as f64
    }

    #[wasm_bindgen(getter, js_name = hashMoveCutoffs)]
    pub fn hash_move_cutoffs(&self) -> f64 {
        self.ordering_stats.hash_move_cutoffs as f64
    }

    #[wasm_bindgen(getter, js_name = killerMoveTries)]
    pub fn killer_move_tries(&self) -> f64 {
        self.ordering_stats.killer_move_tries as f64
    }

    #[wasm_bindgen(getter, js_name = killerMoveCutoffs)]
    pub fn killer_move_cutoffs(&self) -> f64 {
        self.ordering_stats.killer_move_cutoffs as f64
    }

    #[wasm_bindgen(getter, js_name = historyMoveTries)]
    pub fn history_move_tries(&self) -> f64 {
        self.ordering_stats.history_move_tries as f64
    }

    #[wasm_bindgen(getter, js_name = historyMoveCutoffs)]
    pub fn history_move_cutoffs(&self) -> f64 {
        self.ordering_stats.history_move_cutoffs as f64
    }

    #[wasm_bindgen(getter)]
    pub fn qnodes(&self) -> f64 {
        self.qsearch_stats.qnodes as f64
    }

    #[wasm_bindgen(getter, js_name = qsearchMaxPly)]
    pub fn qsearch_max_ply(&self) -> u32 {
        u32::from(self.qsearch_stats.qsearch_max_ply)
    }

    #[wasm_bindgen(getter, js_name = qsearchCapHits)]
    pub fn qsearch_cap_hits(&self) -> f64 {
        self.qsearch_stats.qsearch_cap_hits as f64
    }

    #[wasm_bindgen(getter, js_name = qsearchCheckMoveTries)]
    pub fn qsearch_check_move_tries(&self) -> f64 {
        self.qsearch_stats.qsearch_check_move_tries as f64
    }

    #[wasm_bindgen(getter, js_name = qsearchDeltaPrunes)]
    pub fn qsearch_delta_prunes(&self) -> f64 {
        self.qsearch_stats.qsearch_delta_prunes as f64
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
    set_js_property(
        &object,
        "ttProbes",
        JsValue::from_f64(iteration.tt_stats.tt_probes as f64),
    );
    set_js_property(
        &object,
        "ttHits",
        JsValue::from_f64(iteration.tt_stats.tt_hits as f64),
    );
    set_js_property(
        &object,
        "ttCutoffs",
        JsValue::from_f64(iteration.tt_stats.tt_cutoffs as f64),
    );
    set_js_property(
        &object,
        "ttStores",
        JsValue::from_f64(iteration.tt_stats.tt_stores as f64),
    );
    set_js_property(
        &object,
        "ttCollisions",
        JsValue::from_f64(iteration.tt_stats.tt_collisions as f64),
    );
    set_js_property(
        &object,
        "ttHashfull",
        JsValue::from_f64(iteration.tt_stats.tt_hashfull as f64),
    );
    set_js_property(
        &object,
        "betaCutoffs",
        JsValue::from_f64(iteration.ordering_stats.beta_cutoffs as f64),
    );
    set_js_property(
        &object,
        "firstMoveCutoffs",
        JsValue::from_f64(iteration.ordering_stats.first_move_cutoffs as f64),
    );
    set_js_property(
        &object,
        "hashMoveTries",
        JsValue::from_f64(iteration.ordering_stats.hash_move_tries as f64),
    );
    set_js_property(
        &object,
        "hashMoveCutoffs",
        JsValue::from_f64(iteration.ordering_stats.hash_move_cutoffs as f64),
    );
    set_js_property(
        &object,
        "killerMoveTries",
        JsValue::from_f64(iteration.ordering_stats.killer_move_tries as f64),
    );
    set_js_property(
        &object,
        "killerMoveCutoffs",
        JsValue::from_f64(iteration.ordering_stats.killer_move_cutoffs as f64),
    );
    set_js_property(
        &object,
        "historyMoveTries",
        JsValue::from_f64(iteration.ordering_stats.history_move_tries as f64),
    );
    set_js_property(
        &object,
        "historyMoveCutoffs",
        JsValue::from_f64(iteration.ordering_stats.history_move_cutoffs as f64),
    );
    set_js_property(
        &object,
        "qnodes",
        JsValue::from_f64(iteration.qsearch_stats.qnodes as f64),
    );
    set_js_property(
        &object,
        "qsearchMaxPly",
        JsValue::from_f64(f64::from(iteration.qsearch_stats.qsearch_max_ply)),
    );
    set_js_property(
        &object,
        "qsearchCapHits",
        JsValue::from_f64(iteration.qsearch_stats.qsearch_cap_hits as f64),
    );
    set_js_property(
        &object,
        "qsearchCheckMoveTries",
        JsValue::from_f64(iteration.qsearch_stats.qsearch_check_move_tries as f64),
    );
    set_js_property(
        &object,
        "qsearchDeltaPrunes",
        JsValue::from_f64(iteration.qsearch_stats.qsearch_delta_prunes as f64),
    );
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
    let model =
        NnueModel::from_bytes(bytes).map_err(|err| format!("failed to load NNUE: {err}"))?;
    let description = model.description().to_string();
    *nnue_model_slot().write().unwrap() = Some(Arc::new(model));
    search_tt_slot().write().unwrap().clear();
    Ok(description)
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
    let mut tt = search_tt_slot().write().unwrap();
    tt.clear();
    search_board_with_strategy_and_tt(board, depth, evaluation, deadline, &mut tt)
}

fn search_board_with_strategy_and_tt(
    board: &Board,
    depth: u8,
    evaluation: EvaluationStrategy,
    deadline: Option<Instant>,
    tt: &mut TranspositionTable,
) -> Result<SearchSummary, SearchInterrupted> {
    let mut ordering = SearchOrdering::default();
    search_board_with_strategy_tt_and_ordering(
        board,
        depth,
        evaluation,
        deadline,
        tt,
        &mut ordering,
    )
}

fn search_board_with_strategy_tt_and_ordering(
    board: &Board,
    depth: u8,
    evaluation: EvaluationStrategy,
    deadline: Option<Instant>,
    tt: &mut TranspositionTable,
    ordering: &mut SearchOrdering,
) -> Result<SearchSummary, SearchInterrupted> {
    search_board_with_strategy_tt_ordering_and_qsearch_limits(
        board,
        depth,
        evaluation,
        deadline,
        tt,
        ordering,
        qsearch_limits(),
    )
}

fn search_board_with_strategy_tt_ordering_and_qsearch_limits(
    board: &Board,
    depth: u8,
    evaluation: EvaluationStrategy,
    deadline: Option<Instant>,
    tt: &mut TranspositionTable,
    ordering: &mut SearchOrdering,
    qsearch_limits: QsearchLimits,
) -> Result<SearchSummary, SearchInterrupted> {
    let started_at = Instant::now();
    tt.new_search();
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
        tt,
        tt_stats: SearchTtStats::default(),
        ordering,
        ordering_stats: SearchOrderingStats::default(),
        qsearch_stats: SearchQsearchStats::default(),
        qsearch_limits,
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

    ctx.tt_stats.tt_hashfull = ctx.tt.hashfull(0);

    Ok(SearchSummary {
        best_move,
        best_score,
        elapsed_ms,
        states: ctx.states,
        nps,
        tt_stats: ctx.tt_stats,
        ordering_stats: ctx.ordering_stats,
        qsearch_stats: ctx.qsearch_stats,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UsiSearchBudget {
    Depth(u8),
    Movetime { max_depth: u8, millis: u32 },
}

pub struct UsiSession {
    engine_name: String,
    board: Board,
    nnue_model: Option<Arc<NnueModel>>,
    movetime_max_depth: u8,
    tt: TranspositionTable,
}

impl Default for UsiSession {
    fn default() -> Self {
        Self::new(ENGINE_NAME, 64)
    }
}

fn usi_info_line(
    depth: u8,
    elapsed_ms: f64,
    nodes: u64,
    nps: f64,
    hashfull: u32,
    qsearch_stats: SearchQsearchStats,
) -> String {
    format!(
        "info depth {} time {:.0} nodes {} nps {:.0} hashfull {} qnodes {} qsearchMaxPly {} qsearchCapHits {} qsearchCheckMoveTries {} qsearchDeltaPrunes {}",
        depth,
        elapsed_ms.max(0.0),
        nodes,
        nps.max(0.0),
        hashfull,
        qsearch_stats.qnodes,
        qsearch_stats.qsearch_max_ply,
        qsearch_stats.qsearch_cap_hits,
        qsearch_stats.qsearch_check_move_tries,
        qsearch_stats.qsearch_delta_prunes,
    )
}

impl UsiSession {
    pub fn new(engine_name: impl Into<String>, movetime_max_depth: u8) -> Self {
        Self {
            engine_name: engine_name.into(),
            board: Board::from_sfen(haitaka::SFEN_STARTPOS).expect("startpos should parse"),
            nnue_model: None,
            movetime_max_depth: movetime_max_depth.max(1),
            tt: TranspositionTable::default(),
        }
    }

    pub fn load_nnue(&mut self, bytes: &[u8]) -> Result<String, String> {
        let model =
            NnueModel::from_bytes(bytes).map_err(|err| format!("failed to load NNUE: {err}"))?;
        let description = model.description().to_string();
        self.nnue_model = Some(Arc::new(model));
        self.tt.clear();
        Ok(description)
    }

    pub fn handle_line(&mut self, line: &str) -> Vec<String> {
        let command = line.trim();
        if command.is_empty() {
            return Vec::new();
        }

        match command {
            "usi" => vec![
                format!("id name {}", self.engine_name),
                format!(
                    "option name Hash type spin default {} min {} max {}",
                    tt::DEFAULT_HASH_MB,
                    tt::MIN_HASH_MB,
                    tt::MAX_HASH_MB
                ),
                "usiok".to_string(),
            ],
            "isready" => {
                self.tt.clear();
                vec!["readyok".to_string()]
            }
            "usinewgame" => {
                self.tt.clear();
                Vec::new()
            }
            "quit" => Vec::new(),
            command if command.starts_with("setoption ") => match self.apply_setoption(command) {
                Ok(()) => Vec::new(),
                Err(err) => vec![format!("info string invalid setoption command: {err}")],
            },
            command if command.starts_with("position ") => {
                if let Err(err) = self.apply_position(command) {
                    vec![format!("info string invalid position command: {err}")]
                } else {
                    Vec::new()
                }
            }
            command if command.starts_with("go") => {
                match parse_usi_go(command, self.movetime_max_depth) {
                    Ok(budget) => self.search_outputs_for_budget(budget),
                    Err(err) => vec![format!("info string invalid go command: {err}")],
                }
            }
            other => vec![format!("info string unsupported command: {other}")],
        }
    }

    pub fn board_sfen(&self) -> String {
        self.board.to_string()
    }

    fn apply_position(&mut self, command: &str) -> Result<(), String> {
        self.board = parse_usi_position(command)?;
        Ok(())
    }

    fn apply_setoption(&mut self, command: &str) -> Result<(), String> {
        let rest = command
            .strip_prefix("setoption ")
            .ok_or_else(|| "expected setoption command".to_string())?;
        let tokens = rest.split_whitespace().collect::<Vec<_>>();
        if tokens.len() < 4 || tokens[0] != "name" {
            return Err("expected setoption name Hash value N".to_string());
        }

        let value_index = tokens
            .iter()
            .position(|token| *token == "value")
            .ok_or_else(|| "missing value".to_string())?;
        if value_index <= 1 {
            return Err("missing option name".to_string());
        }

        let name = tokens[1..value_index].join(" ");
        if name != "Hash" {
            return Err(format!("unsupported option {name}"));
        }

        let value = tokens
            .get(value_index + 1)
            .ok_or_else(|| "missing Hash value".to_string())?
            .parse::<u32>()
            .map_err(|_| "invalid Hash value".to_string())?;
        let size = tt::validate_hash_size_mb(value)?;
        self.tt.resize(size);
        Ok(())
    }

    fn search_outputs_for_budget(&mut self, budget: UsiSearchBudget) -> Vec<String> {
        if self.board.status() != haitaka::GameStatus::Ongoing {
            return vec!["bestmove resign".to_string()];
        }

        match budget {
            UsiSearchBudget::Depth(depth) => search_board_with_strategy_and_tt(
                &self.board,
                depth,
                self.evaluation_strategy(),
                None,
                &mut self.tt,
            )
            .map_err(|_| "search timed out unexpectedly".to_string())
            .map(|summary| {
                let best_move = summary
                    .best_move
                    .clone()
                    .unwrap_or_else(|| self.fallback_bestmove());
                vec![
                    usi_info_line(
                        depth,
                        summary.elapsed_ms,
                        summary.states,
                        summary.nps,
                        summary.tt_stats.tt_hashfull,
                        summary.qsearch_stats,
                    ),
                    format!("bestmove {best_move}"),
                ]
            }),
            UsiSearchBudget::Movetime { max_depth, millis } => {
                search_iterative_deepening_with_strategy_and_deadline_and_tt(
                    &self.board.to_string(),
                    max_depth,
                    millis,
                    self.evaluation_strategy(),
                    IterativeSearchConfig::default(),
                    if millis == 0 {
                        None
                    } else {
                        Some(Instant::now() + std::time::Duration::from_millis(u64::from(millis)))
                    },
                    &mut self.tt,
                )
                .map(|summary| {
                    let best_move = summary
                        .best_move
                        .clone()
                        .unwrap_or_else(|| self.fallback_bestmove());
                    vec![
                        usi_info_line(
                            summary.completed_depth,
                            summary.elapsed_ms,
                            summary.states,
                            summary.nps,
                            summary.tt_stats.tt_hashfull,
                            summary.qsearch_stats,
                        ),
                        format!("bestmove {best_move}"),
                    ]
                })
            }
        }
        .unwrap_or_else(|err| {
            let _ = err;
            vec![format!("bestmove {}", self.fallback_bestmove())]
        })
    }

    fn fallback_bestmove(&self) -> String {
        let ordering = SearchOrdering::default();
        let mut picker = MovePicker::new(&self.board, None, &ordering, 0);
        picker
            .next()
            .map(|picked| picked.mv.to_string())
            .unwrap_or_else(|| "resign".to_string())
    }

    fn evaluation_strategy(&self) -> EvaluationStrategy {
        match self.nnue_model.as_ref() {
            Some(model) => EvaluationStrategy::Nnue {
                model: Arc::clone(model),
                mode: SearchEvalMode::Incremental,
            },
            None => EvaluationStrategy::Handcrafted,
        }
    }
}

fn parse_usi_position(command: &str) -> Result<Board, String> {
    let rest = command
        .strip_prefix("position ")
        .ok_or_else(|| "expected position command".to_string())?;
    let tokens = rest.split_whitespace().collect::<Vec<_>>();
    if tokens.is_empty() {
        return Err("missing position body".to_string());
    }

    let mut index;
    let mut board = match tokens[0] {
        "startpos" => {
            index = 1;
            Board::from_sfen(haitaka::SFEN_STARTPOS).expect("startpos should parse")
        }
        "sfen" => {
            index = 1;
            let sfen_start = index;
            while index < tokens.len() && tokens[index] != "moves" {
                index += 1;
            }
            if sfen_start == index {
                return Err("missing SFEN after position sfen".to_string());
            }
            Board::from_sfen(&tokens[sfen_start..index].join(" "))
                .map_err(|err| format!("failed to parse SFEN: {err}"))?
        }
        other => return Err(format!("unsupported position source {other}")),
    };

    if index < tokens.len() {
        if tokens[index] != "moves" {
            return Err(format!("unexpected token {}", tokens[index]));
        }
        index += 1;
        for move_text in &tokens[index..] {
            let mv = Move::from_str(move_text)
                .map_err(|err| format!("invalid move {move_text}: {err}"))?;
            board
                .try_play(mv)
                .map_err(|_| format!("illegal move {move_text}"))?;
        }
    }

    Ok(board)
}

fn parse_usi_go(command: &str, movetime_max_depth: u8) -> Result<UsiSearchBudget, String> {
    let rest = command
        .strip_prefix("go")
        .ok_or_else(|| "expected go command".to_string())?;
    let tokens = rest.split_whitespace().collect::<Vec<_>>();
    let mut index = 0;
    let mut depth = None;
    let mut movetime = None;

    while index < tokens.len() {
        match tokens[index] {
            "depth" => {
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| "missing depth value".to_string())?;
                depth = Some(
                    value
                        .parse::<u8>()
                        .map_err(|_| format!("invalid depth {value}"))?,
                );
            }
            "movetime" => {
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| "missing movetime value".to_string())?;
                movetime = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| format!("invalid movetime {value}"))?,
                );
            }
            _ => {}
        }
        index += 1;
    }

    if let Some(millis) = movetime {
        return Ok(UsiSearchBudget::Movetime {
            max_depth: depth.unwrap_or(movetime_max_depth).max(1),
            millis,
        });
    }
    if let Some(depth) = depth {
        return Ok(UsiSearchBudget::Depth(depth.max(1)));
    }
    Err("only go depth N, go movetime N, and go movetime N depth D are supported".to_string())
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
    let mut tt = search_tt_slot().write().unwrap();
    tt.clear();
    search_iterative_deepening_with_strategy_and_deadline_and_tt(
        sfen, max_depth, timeout_ms, evaluation, config, deadline, &mut tt,
    )
}

fn search_iterative_deepening_with_strategy_and_deadline_and_tt(
    sfen: &str,
    max_depth: u8,
    timeout_ms: u32,
    evaluation: EvaluationStrategy,
    config: IterativeSearchConfig,
    deadline: Option<Instant>,
    tt: &mut TranspositionTable,
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
                        tt_stats: SearchTtStats::default(),
                        ordering_stats: SearchOrderingStats::default(),
                        qsearch_stats: SearchQsearchStats::default(),
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
                tt_stats: SearchTtStats::default(),
                ordering_stats: SearchOrderingStats::default(),
                qsearch_stats: SearchQsearchStats::default(),
                iterations: Vec::new(),
                dfpn: Some(dfpn_summary),
            });
        }
        dfpn = Some(dfpn_summary);
    }

    let mut iterations = Vec::with_capacity(max_depth as usize);
    let mut completed_depth = 0;
    let mut total_states = 0;
    let mut tt_stats = SearchTtStats::default();
    let mut ordering_stats = SearchOrderingStats::default();
    let mut qsearch_stats = SearchQsearchStats::default();
    let mut ordering = SearchOrdering::default();
    let mut latest_best_move = None;
    let mut timed_out = false;

    for depth in 1..=max_depth {
        if deadline.is_some_and(|limit| Instant::now() >= limit) {
            timed_out = true;
            break;
        }

        match search_board_with_strategy_tt_and_ordering(
            &board,
            depth,
            evaluation.clone(),
            deadline,
            tt,
            &mut ordering,
        ) {
            Ok(summary) => {
                total_states += summary.states;
                completed_depth = depth;
                latest_best_move = summary.best_move.clone();
                tt_stats.add_iteration(summary.tt_stats);
                ordering_stats.add_iteration(summary.ordering_stats);
                qsearch_stats.add_iteration(summary.qsearch_stats);
                iterations.push(IterativeIterationSummary {
                    depth,
                    best_move: summary.best_move,
                    elapsed_ms: summary.elapsed_ms,
                    states: summary.states,
                    nps: summary.nps,
                    tt_stats: summary.tt_stats,
                    ordering_stats: summary.ordering_stats,
                    qsearch_stats: summary.qsearch_stats,
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
        tt_stats,
        ordering_stats,
        qsearch_stats,
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
pub fn search_iterative_deepening_impl_with_eval_mode(
    sfen: &str,
    max_depth: u8,
    timeout_ms: u32,
    model: Arc<NnueModel>,
    mode: SearchEvalMode,
) -> Result<IterativeSearchSummary, String> {
    search_iterative_deepening_with_strategy(
        sfen,
        max_depth,
        timeout_ms,
        EvaluationStrategy::Nnue { model, mode },
        IterativeSearchConfig::default(),
    )
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

#[wasm_bindgen(js_name = set_hash_size_mb)]
pub fn set_hash_size_mb(size_mb: u32) -> Result<(), JsValue> {
    let size = tt::validate_hash_size_mb(size_mb).map_err(|err| JsValue::from_str(&err))?;
    search_tt_slot().write().unwrap().resize(size);
    Ok(())
}

#[wasm_bindgen(js_name = clear_hash)]
pub fn clear_hash() {
    search_tt_slot().write().unwrap().clear();
}

#[wasm_bindgen]
pub struct UsiEngine {
    session: UsiSession,
}

#[wasm_bindgen]
impl UsiEngine {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            session: UsiSession::default(),
        }
    }

    #[wasm_bindgen(js_name = load_nnue)]
    pub fn load_nnue(&mut self, bytes: &[u8]) -> Result<String, JsValue> {
        self.session
            .load_nnue(bytes)
            .map_err(|err| JsValue::from_str(&err))
    }

    pub fn send(&mut self, line: &str) -> js_sys::Array {
        self.session
            .handle_line(line)
            .into_iter()
            .map(JsValue::from)
            .collect()
    }
}

impl Default for UsiEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
pub fn search(sfen: &str, depth: u8) -> Result<SearchResult, JsValue> {
    let summary = search_impl(sfen, depth).map_err(|err| JsValue::from_str(&err))?;
    Ok(SearchResult {
        best_move: summary.best_move,
        elapsed_ms: summary.elapsed_ms,
        states: summary.states,
        nps: summary.nps,
        tt_stats: summary.tt_stats,
        ordering_stats: summary.ordering_stats,
        qsearch_stats: summary.qsearch_stats,
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
        tt_stats: summary.tt_stats,
        ordering_stats: summary.ordering_stats,
        qsearch_stats: summary.qsearch_stats,
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
    ctx: &mut SearchContext<'_>,
    nnue_state: Option<NnuePositionState>,
) -> Result<Option<(Move, i32)>, SearchInterrupted> {
    ctx.record_state()?;
    ctx.check_deadline()?;
    if terminal_score_for_side_to_move(board, 0).is_some() {
        return Ok(None);
    }
    let key = board.hash();
    ctx.tt_stats.tt_probes += 1;
    let probe = ctx.tt.probe(key, board.side_to_move());
    let tt_move = probe.data.and_then(|data| data.best_move);
    if probe.found {
        ctx.tt_stats.tt_hits += 1;
    }

    let mut move_picker = MovePicker::new(board, tt_move, &ctx.ordering, 0);
    if move_picker.is_empty() {
        return Ok(None);
    }

    let original_alpha = -INF_SCORE;
    let mut alpha = original_alpha;
    let beta = INF_SCORE;
    let mut best_score = -INF_SCORE;
    let mut best_move = None;

    while let Some(picked) = move_picker.next() {
        record_move_try(ctx, picked.source);
        let mv = picked.mv;
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

    store_tt_search_result(
        ctx,
        probe,
        key,
        depth,
        0,
        best_score,
        original_alpha,
        beta,
        best_move,
        0,
        true,
    );

    Ok(best_move.map(|mv| (mv, best_score)))
}

fn negamax(
    board: &Board,
    depth: u8,
    mut alpha: i32,
    beta: i32,
    ply: i32,
    ctx: &mut SearchContext<'_>,
    nnue_state: Option<NnuePositionState>,
) -> Result<i32, SearchInterrupted> {
    ctx.record_state()?;
    ctx.check_deadline()?;
    if let Some(terminal) = terminal_score_for_side_to_move(board, ply) {
        return Ok(terminal);
    }
    if depth == 0 {
        return quiescence(
            board,
            alpha,
            beta,
            ply,
            0,
            ctx.qsearch_limits.check_budget,
            ctx,
            nnue_state,
        );
    }

    let key = board.hash();
    let original_alpha = alpha;
    ctx.tt_stats.tt_probes += 1;
    let probe = ctx.tt.probe(key, board.side_to_move());
    let mut tt_move = None;
    if let Some(data) = probe.data {
        ctx.tt_stats.tt_hits += 1;
        let tt_score = score_from_tt(data.score, ply);
        tt_move = data.best_move;
        if data.depth >= depth && tt_bound_can_cutoff(data.bound, tt_score, alpha, beta) {
            ctx.tt_stats.tt_cutoffs += 1;
            return Ok(tt_score);
        }
    }

    let ply_index = usize::try_from(ply).unwrap_or(usize::MAX);
    let mut move_picker = MovePicker::new(board, tt_move, &ctx.ordering, ply_index);
    if move_picker.is_empty() {
        return Ok(-MATE_SCORE + ply);
    }

    let mut best_score = -INF_SCORE;
    let mut best_move = None;
    let mut move_count = 0u64;
    while let Some(picked) = move_picker.next() {
        move_count += 1;
        record_move_try(ctx, picked.source);
        let mv = picked.mv;
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
            best_move = Some(mv);
        }
        if score > alpha {
            alpha = score;
        }
        if alpha >= beta {
            ctx.ordering_stats.beta_cutoffs += 1;
            if move_count == 1 {
                ctx.ordering_stats.first_move_cutoffs += 1;
            }
            record_move_cutoff(ctx, picked.source);
            if !matches!(picked.source, MoveSource::Hash | MoveSource::Tactical) {
                ctx.ordering
                    .record_beta_cutoff(board.side_to_move(), mv, depth, ply_index);
            }
            break;
        }
    }

    store_tt_search_result(
        ctx,
        probe,
        key,
        depth,
        ply,
        best_score,
        original_alpha,
        beta,
        best_move,
        0,
        false,
    );

    Ok(best_score)
}

#[allow(clippy::too_many_arguments)]
fn quiescence(
    board: &Board,
    mut alpha: i32,
    beta: i32,
    ply: i32,
    qply: u8,
    check_budget: u8,
    ctx: &mut SearchContext<'_>,
    nnue_state: Option<NnuePositionState>,
) -> Result<i32, SearchInterrupted> {
    ctx.check_deadline()?;
    if let Some(terminal) = terminal_score_for_side_to_move(board, ply) {
        return Ok(terminal);
    }
    if !ctx.record_qnode(qply)? || qply >= ctx.qsearch_limits.max_ply {
        ctx.qsearch_stats.qsearch_cap_hits += u64::from(qply >= ctx.qsearch_limits.max_ply);
        return Ok(evaluate_or_mate(board, ply, ctx, nnue_state.as_ref()));
    }

    let in_check = !board.checkers().is_empty();
    let mut stand_pat_for_delta = None;
    if !in_check {
        let stand_pat = evaluate_or_mate(board, ply, ctx, nnue_state.as_ref());
        if stand_pat >= beta {
            return Ok(stand_pat);
        }
        alpha = alpha.max(stand_pat);
        stand_pat_for_delta = Some(stand_pat);
    }

    let mut searched_move = false;
    let mut tactical_picker = if in_check {
        QsearchMovePicker::new_evasions(board)
    } else {
        QsearchMovePicker::new_tactical(board)
    };

    while let Some(picked) = tactical_picker.next() {
        let mv = picked.mv;
        if let Some(stand_pat) = stand_pat_for_delta {
            if qply >= ctx.qsearch_limits.delta_min_qply
                && stand_pat
                    .saturating_add(picked.tactical_score.optimistic_delta)
                    .saturating_add(ctx.qsearch_limits.delta_margin)
                    <= alpha
            {
                ctx.qsearch_stats.qsearch_delta_prunes += 1;
                continue;
            }
        }
        searched_move = true;
        ctx.check_deadline()?;
        let mut child = board.clone();
        child.play_unchecked(mv);
        let score = if let Some(terminal) = terminal_score_for_side_to_move(&child, ply + 1) {
            -terminal
        } else {
            let child_state = child_nnue_state(ctx, board, &child, nnue_state.as_ref(), mv);
            -quiescence(
                &child,
                -beta,
                -alpha,
                ply + 1,
                qply + 1,
                check_budget,
                ctx,
                child_state,
            )?
        };

        if score >= beta {
            return Ok(score);
        }
        alpha = alpha.max(score);
    }

    if in_check {
        if !searched_move {
            return Ok(-MATE_SCORE + ply);
        }
        return Ok(alpha);
    }

    if check_budget > 0 && qply == 0 {
        let mut check_picker = QsearchMovePicker::new_quiet_checks(board);
        while let Some(picked) = check_picker.next() {
            let mv = picked.mv;
            ctx.qsearch_stats.qsearch_check_move_tries += 1;
            ctx.check_deadline()?;
            let mut child = board.clone();
            child.play_unchecked(mv);
            let score = if let Some(terminal) = terminal_score_for_side_to_move(&child, ply + 1) {
                -terminal
            } else {
                let child_state = child_nnue_state(ctx, board, &child, nnue_state.as_ref(), mv);
                -quiescence(
                    &child,
                    -beta,
                    -alpha,
                    ply + 1,
                    qply + 1,
                    check_budget - 1,
                    ctx,
                    child_state,
                )?
            };

            if score >= beta {
                return Ok(score);
            }
            alpha = alpha.max(score);
        }
    }

    Ok(alpha)
}

#[allow(clippy::too_many_arguments)]
fn store_tt_search_result(
    ctx: &mut SearchContext<'_>,
    probe: tt::TtProbe,
    key: u64,
    depth: u8,
    ply: i32,
    score: i32,
    original_alpha: i32,
    beta: i32,
    best_move: Option<Move>,
    eval: i32,
    is_pv: bool,
) {
    let bound = if score >= beta {
        Bound::Lower
    } else if score > original_alpha {
        Bound::Exact
    } else {
        Bound::Upper
    };
    let stored_score = score_to_tt(score, ply);
    if i16::try_from(stored_score).is_err() || i16::try_from(eval).is_err() {
        return;
    }
    let (stored, collision) = ctx.tt.write(
        probe,
        key,
        stored_score,
        is_pv,
        bound,
        depth,
        best_move,
        eval,
    );
    if stored {
        ctx.tt_stats.tt_stores += 1;
    }
    if collision {
        ctx.tt_stats.tt_collisions += 1;
    }
}

fn tt_bound_can_cutoff(bound: Bound, score: i32, alpha: i32, beta: i32) -> bool {
    match bound {
        Bound::Exact => true,
        Bound::Lower => score >= beta,
        Bound::Upper => score <= alpha,
        Bound::None => false,
    }
}

fn score_to_tt(score: i32, ply: i32) -> i32 {
    if score >= MATE_TT_THRESHOLD {
        score + ply
    } else if score <= -MATE_TT_THRESHOLD {
        score - ply
    } else {
        score
    }
}

fn score_from_tt(score: i32, ply: i32) -> i32 {
    if score >= MATE_TT_THRESHOLD {
        score - ply
    } else if score <= -MATE_TT_THRESHOLD {
        score + ply
    } else {
        score
    }
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

fn record_move_try(ctx: &mut SearchContext<'_>, source: MoveSource) {
    match source {
        MoveSource::Hash => ctx.ordering_stats.hash_move_tries += 1,
        MoveSource::Killer => ctx.ordering_stats.killer_move_tries += 1,
        MoveSource::History => ctx.ordering_stats.history_move_tries += 1,
        MoveSource::Tactical => {}
    }
}

fn record_move_cutoff(ctx: &mut SearchContext<'_>, source: MoveSource) {
    match source {
        MoveSource::Hash => ctx.ordering_stats.hash_move_cutoffs += 1,
        MoveSource::Killer => ctx.ordering_stats.killer_move_cutoffs += 1,
        MoveSource::History => ctx.ordering_stats.history_move_cutoffs += 1,
        MoveSource::Tactical => {}
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

fn search_tt_slot() -> &'static RwLock<TranspositionTable> {
    SEARCH_TT.get_or_init(|| RwLock::new(TranspositionTable::default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
    use std::cmp::Reverse;
    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen"
    )))]
    use std::path::PathBuf;
    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen"
    )))]
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

    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen"
    )))]
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

    fn handcrafted_context<'a>(
        tt: &'a mut TranspositionTable,
        ordering: &'a mut SearchOrdering,
        qsearch_limits: QsearchLimits,
    ) -> SearchContext<'a> {
        SearchContext {
            states: 0,
            evaluation: EvaluationStrategy::Handcrafted,
            deadline: None,
            tt,
            tt_stats: SearchTtStats::default(),
            ordering,
            ordering_stats: SearchOrderingStats::default(),
            qsearch_stats: SearchQsearchStats::default(),
            qsearch_limits,
        }
    }

    fn static_handcrafted_eval(board: &Board) -> i32 {
        let mut tt = TranspositionTable::default();
        let mut ordering = SearchOrdering::default();
        let ctx = handcrafted_context(&mut tt, &mut ordering, qsearch_limits());
        evaluate_or_mate(board, 0, &ctx, None)
    }

    fn qsearch_handcrafted(
        board: &Board,
        qply: u8,
        check_budget: u8,
        qsearch_node_limit: u64,
    ) -> (i32, SearchQsearchStats) {
        qsearch_handcrafted_window(
            board,
            -INF_SCORE,
            INF_SCORE,
            qply,
            check_budget,
            qsearch_node_limit,
        )
    }

    fn qsearch_handcrafted_window(
        board: &Board,
        alpha: i32,
        beta: i32,
        qply: u8,
        check_budget: u8,
        qsearch_node_limit: u64,
    ) -> (i32, SearchQsearchStats) {
        let mut tt = TranspositionTable::default();
        let mut ordering = SearchOrdering::default();
        let mut ctx = handcrafted_context(
            &mut tt,
            &mut ordering,
            QsearchLimits {
                max_ply: qsearch_limits().max_ply,
                check_budget,
                node_limit: qsearch_node_limit,
                delta_margin: qsearch_limits().delta_margin,
                delta_min_qply: qsearch_limits().delta_min_qply,
            },
        );
        let score = quiescence(board, alpha, beta, 0, qply, check_budget, &mut ctx, None).unwrap();
        (score, ctx.qsearch_stats)
    }

    #[cfg(not(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
    fn qsearch_limits_without_delta_pruning() -> QsearchLimits {
        QsearchLimits {
            delta_min_qply: u8::MAX,
            ..qsearch_limits()
        }
    }

    #[cfg(not(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
    fn search_board_impl_handcrafted_with_qsearch_limits(
        board: &Board,
        depth: u8,
        qsearch_limits: QsearchLimits,
    ) -> Result<SearchSummary, String> {
        let mut tt = TranspositionTable::default();
        let mut ordering = SearchOrdering::default();
        search_board_with_strategy_tt_ordering_and_qsearch_limits(
            board,
            depth.max(1),
            EvaluationStrategy::Handcrafted,
            None,
            &mut tt,
            &mut ordering,
            qsearch_limits,
        )
        .map_err(|_| "search timed out unexpectedly".to_string())
    }

    #[test]
    fn usi_session_reports_id_and_ready() {
        let mut session = UsiSession::default();
        assert_eq!(
            session.handle_line("usi"),
            vec![
                "id name Haitaka Variants".to_string(),
                "option name Hash type spin default 16 min 1 max 1024".to_string(),
                "usiok".to_string()
            ]
        );
        assert_eq!(session.handle_line("isready"), vec!["readyok".to_string()]);
    }

    #[test]
    fn usi_session_applies_startpos_moves() {
        let mut session = UsiSession::default();
        let output = session.handle_line("position startpos moves 7g7f");
        assert!(output.is_empty(), "unexpected output: {output:?}");

        let mut expected = Board::from_sfen(haitaka::SFEN_STARTPOS).expect("startpos should parse");
        expected.try_play(Move::from_str("7g7f").unwrap()).unwrap();
        assert_eq!(session.board_sfen(), expected.to_string());
    }

    #[test]
    fn usi_session_rejects_bad_position_without_corrupting_board() {
        let mut session = UsiSession::default();
        let before = session.board_sfen();
        let output = session.handle_line("position startpos moves 1a1b");

        assert_eq!(session.board_sfen(), before);
        assert_eq!(output.len(), 1);
        assert!(output[0].contains("invalid position command"));
        assert!(output[0].contains("illegal move"));
    }

    #[test]
    fn usi_session_returns_legal_bestmove_for_depth_search() {
        let mut session = UsiSession::default();
        assert!(session.handle_line("position startpos").is_empty());
        let output = session.handle_line("go depth 1");

        assert_eq!(output.len(), 2);
        assert!(output[0].starts_with("info "));
        assert!(output[0].contains(" qnodes "));
        assert!(output[0].contains(" qsearchMaxPly "));
        assert!(output[0].contains(" qsearchCapHits "));
        assert!(output[0].contains(" qsearchCheckMoveTries "));
        assert!(output[0].contains(" qsearchDeltaPrunes "));
        let best_move = output[1]
            .strip_prefix("bestmove ")
            .expect("expected bestmove output");
        assert_ne!(best_move, "resign");
        let board = Board::from_sfen(haitaka::SFEN_STARTPOS).expect("startpos should parse");
        let mv = Move::from_str(best_move).expect("bestmove should parse");
        assert!(board.is_legal(mv), "{best_move} should be legal");
    }

    #[test]
    fn usi_session_tiny_movetime_falls_back_to_legal_move() {
        let mut session = UsiSession::default();
        assert!(session.handle_line("position startpos").is_empty());
        let output = session.handle_line("go movetime 1");

        assert!(matches!(output.len(), 1 | 2));
        let bestmove_output = output.last().expect("expected at least bestmove output");
        let best_move = bestmove_output
            .strip_prefix("bestmove ")
            .expect("expected bestmove output");
        assert_ne!(best_move, "resign");
        let board = Board::from_sfen(haitaka::SFEN_STARTPOS).expect("startpos should parse");
        let mv = Move::from_str(best_move).expect("bestmove should parse");
        assert!(board.is_legal(mv), "{best_move} should be legal");
    }

    #[test]
    fn usi_go_movetime_depth_uses_depth_as_cap() {
        assert_eq!(
            parse_usi_go("go movetime 100 depth 5", 64).expect("go should parse"),
            UsiSearchBudget::Movetime {
                max_depth: 5,
                millis: 100
            }
        );
        assert_eq!(
            parse_usi_go("go movetime 100", 64).expect("go should parse"),
            UsiSearchBudget::Movetime {
                max_depth: 64,
                millis: 100
            }
        );
    }

    #[test]
    fn usi_session_accepts_hash_option() {
        let mut session = UsiSession::default();
        let output = session.handle_line("setoption name Hash value 16");

        assert!(output.is_empty(), "unexpected output: {output:?}");
    }

    #[test]
    fn usi_session_reports_unsupported_setoption() {
        let mut session = UsiSession::default();
        let output = session.handle_line("setoption name Unknown value 16");

        assert_eq!(output.len(), 1);
        assert!(output[0].contains("unsupported option"));
    }

    #[cfg(any(
        feature = "annan",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen"
        ))
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
    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen"
    )))]
    fn returns_legal_move_for_start_position() {
        assert_legal_best_move(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            2,
        );
    }

    #[test]
    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen"
    )))]
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
    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
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
        assert!(summary.tt_stats.tt_probes > 0);
        assert!(summary.tt_stats.tt_stores > 0);
        assert!(summary.tt_stats.tt_hashfull <= 1000);
        assert!(summary.ordering_stats.history_move_tries > 0);
        assert!(summary.qsearch_stats.qnodes > 0);
    }

    #[test]
    fn qsearch_limits_match_variant_family() {
        let limits = qsearch_limits();
        if cfg!(any(
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko"
        )) {
            assert_eq!(limits.max_ply, 6);
            assert_eq!(limits.check_budget, 0);
            assert_eq!(limits.node_limit, 250_000);
            assert_eq!(limits.delta_margin, 500);
            assert_eq!(limits.delta_min_qply, u8::MAX);
        } else {
            assert_eq!(limits, DEFAULT_QSEARCH_LIMITS);
            assert_eq!(limits.delta_margin, 300);
            assert_eq!(limits.delta_min_qply, 1);
        }
    }

    #[test]
    fn qsearch_quiet_leaf_matches_static_eval() {
        let board = Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b - 1").unwrap();
        let static_eval = static_handcrafted_eval(&board);
        let limits = qsearch_limits();
        let (qscore, stats) =
            qsearch_handcrafted(&board, 0, limits.check_budget, limits.node_limit);

        assert_eq!(qscore, static_eval);
        assert_eq!(stats.qnodes, 1);
        assert_eq!(stats.qsearch_max_ply, 0);
    }

    #[test]
    fn qsearch_expands_tactical_captures() {
        let board = Board::from_sfen("9/9/k8/9/4Rr3/9/9/9/4K4 b - 1").unwrap();
        let static_eval = static_handcrafted_eval(&board);
        let limits = qsearch_limits();
        let (qscore, stats) =
            qsearch_handcrafted(&board, 0, limits.check_budget, limits.node_limit);

        assert!(stats.qnodes > 1, "expected qsearch to search capture nodes");
        assert!(
            qscore > static_eval,
            "expected capture qscore {qscore} to improve on static eval {static_eval}"
        );
    }

    #[test]
    fn qsearch_in_check_searches_evasions_without_stand_pat() {
        let board = Board::from_sfen("4r4/9/9/9/9/9/9/9/4K3k b - 1").unwrap();
        assert!(!board.checkers().is_empty());

        let limits = qsearch_limits();
        let (_, stats) = qsearch_handcrafted(&board, 0, limits.check_budget, limits.node_limit);
        assert!(stats.qnodes > 1, "expected qsearch to search evasions");
    }

    #[test]
    fn qsearch_check_budget_controls_quiet_check_search() {
        let board = Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b R 1").unwrap();

        let limits = qsearch_limits();
        let (_, no_checks) = qsearch_handcrafted(&board, 0, 0, limits.node_limit);
        let (_, with_checks) = qsearch_handcrafted(&board, 0, 1, limits.node_limit);

        assert_eq!(no_checks.qsearch_check_move_tries, 0);
        assert!(
            with_checks.qsearch_check_move_tries > 0,
            "expected qsearch to try quiet checking moves"
        );
    }

    #[test]
    fn qsearch_caps_are_reported() {
        let board = Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b - 1").unwrap();
        let static_eval = static_handcrafted_eval(&board);

        let limits = qsearch_limits();
        let (node_capped, node_stats) = qsearch_handcrafted(&board, 0, limits.check_budget, 0);
        assert_eq!(node_capped, static_eval);
        assert_eq!(node_stats.qsearch_cap_hits, 1);

        let (ply_capped, ply_stats) = qsearch_handcrafted(
            &board,
            limits.max_ply,
            limits.check_budget,
            limits.node_limit,
        );
        assert_eq!(ply_capped, static_eval);
        assert_eq!(ply_stats.qsearch_cap_hits, 1);
    }

    #[test]
    fn qsearch_delta_pruning_is_reported_for_narrow_windows() {
        let board = Board::from_sfen("9/9/k8/9/4Rr3/9/9/9/4K4 b - 1").unwrap();
        let limits = qsearch_limits();
        if limits.delta_min_qply == u8::MAX {
            let (_, stats) = qsearch_handcrafted_window(
                &board,
                INF_SCORE - 1,
                INF_SCORE,
                2,
                limits.check_budget,
                limits.node_limit,
            );
            assert_eq!(stats.qsearch_delta_prunes, 0);
            return;
        }

        let (_, stats) = qsearch_handcrafted_window(
            &board,
            INF_SCORE - 1,
            INF_SCORE,
            limits.delta_min_qply,
            limits.check_budget,
            limits.node_limit,
        );

        assert!(
            stats.qsearch_delta_prunes > 0,
            "expected narrow-window qsearch to delta-prune at min qply"
        );
    }

    #[test]
    fn qsearch_root_qply_never_delta_prunes() {
        let board = Board::from_sfen("9/9/k8/9/4Rr3/9/9/9/4K4 b - 1").unwrap();
        let limits = qsearch_limits();

        let (_, stats) =
            qsearch_handcrafted_window(&board, INF_SCORE - 1, INF_SCORE, 0, limits.check_budget, 1);

        assert_eq!(stats.qsearch_delta_prunes, 0);
    }

    #[test]
    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
    fn qsearch_tactical_fixture_suite_pins_best_moves_and_scores() {
        struct TacticalFixture {
            name: &'static str,
            sfen: &'static str,
            expected_best_move: &'static str,
            expected_score: i32,
            expected_qscore: i32,
            min_qnodes: u64,
        }

        let fixtures = [
            TacticalFixture {
                name: "hanging_rook_capture",
                sfen: "9/9/k8/9/4Rr3/9/9/9/4K4 b - 1",
                expected_best_move: "5e4e",
                expected_score: 1788,
                expected_qscore: 1788,
                min_qnodes: 2,
            },
            TacticalFixture {
                name: "silver_promotion",
                sfen: "4k4/9/4S4/9/9/9/9/9/4K4 b - 1",
                expected_best_move: "5c4d+",
                expected_score: 562,
                expected_qscore: 562,
                min_qnodes: 2,
            },
            TacticalFixture {
                name: "bishop_promotion",
                sfen: "4k4/9/4B4/9/9/9/9/9/4K4 b - 1",
                expected_best_move: "5c4d+",
                expected_score: 938,
                expected_qscore: 938,
                min_qnodes: 2,
            },
        ];

        let limits = qsearch_limits();
        for fixture in fixtures {
            let board = Board::from_sfen(fixture.sfen).unwrap();
            let summary = search_board_impl_handcrafted(&board, 1).unwrap();
            assert_eq!(
                summary.best_move.as_deref(),
                Some(fixture.expected_best_move),
                "{} best move changed",
                fixture.name
            );
            assert_eq!(
                summary.best_score,
                Some(fixture.expected_score),
                "{} root score changed",
                fixture.name
            );
            assert!(
                summary.qsearch_stats.qnodes >= fixture.min_qnodes,
                "{} should exercise qsearch, got {:?}",
                fixture.name,
                summary.qsearch_stats
            );

            let (qscore, qstats) =
                qsearch_handcrafted(&board, 0, limits.check_budget, limits.node_limit);
            assert_eq!(
                qscore, fixture.expected_qscore,
                "{} direct qsearch score changed",
                fixture.name
            );
            assert!(
                qstats.qnodes >= fixture.min_qnodes,
                "{} direct qsearch should search tactical leaves, got {:?}",
                fixture.name,
                qstats
            );
        }
    }

    #[test]
    fn tt_score_conversion_round_trips() {
        for ply in [0, 1, 17, 63] {
            for score in [
                0,
                123,
                -456,
                MATE_SCORE - 3,
                -MATE_SCORE + 5,
                MATE_TT_THRESHOLD,
                -MATE_TT_THRESHOLD,
            ] {
                assert_eq!(score_from_tt(score_to_tt(score, ply), ply), score);
            }
        }
    }

    #[test]
    fn tiny_hash_search_returns_legal_move() {
        let board = Board::from_sfen(haitaka::SFEN_STARTPOS).unwrap();
        let mut tt = TranspositionTable::new(1);
        let summary = search_board_with_strategy_and_tt(
            &board,
            2,
            EvaluationStrategy::Handcrafted,
            None,
            &mut tt,
        )
        .unwrap();
        let best = summary.best_move.expect("expected best move");
        let mv = Move::from_str(&best).expect("best move should parse");
        assert!(board.is_legal(mv), "{best} should be legal");
    }

    #[test]
    #[cfg(not(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
    fn iterative_search_reuses_tt_between_depths() {
        let summary =
            search_iterative_deepening_impl_with_dfpn_mode(haitaka::SFEN_STARTPOS, 2, 0, false)
                .unwrap();
        assert_eq!(summary.completed_depth, 2);
        assert!(
            summary.tt_stats.tt_hits > 0,
            "expected TT hits from previous iteration, got {:?}",
            summary.tt_stats
        );
        assert!(
            summary.ordering_stats.hash_move_tries > 0,
            "expected hash move tries from previous iteration, got {:?}",
            summary.ordering_stats
        );
    }

    #[cfg(not(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
    fn reference_fixed_depth_score(
        board: &Board,
        depth: u8,
        qsearch_limits: QsearchLimits,
    ) -> Option<i32> {
        if terminal_score_for_side_to_move(board, 0).is_some() {
            return None;
        }

        let moves = reference_ordered_moves(board);
        if moves.is_empty() {
            return None;
        }

        let mut alpha = -INF_SCORE;
        let beta = INF_SCORE;
        let mut best_score = -INF_SCORE;
        for mv in moves {
            let mut child = board.clone();
            child.play_unchecked(mv);
            let score = if let Some(terminal) = terminal_score_for_side_to_move(&child, 1) {
                -terminal
            } else {
                -reference_negamax(
                    &child,
                    depth.saturating_sub(1),
                    -beta,
                    -alpha,
                    1,
                    qsearch_limits,
                )
            };
            best_score = best_score.max(score);
            alpha = alpha.max(score);
        }
        Some(best_score)
    }

    #[cfg(not(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
    fn reference_negamax(
        board: &Board,
        depth: u8,
        mut alpha: i32,
        beta: i32,
        ply: i32,
        qsearch_limits: QsearchLimits,
    ) -> i32 {
        if let Some(terminal) = terminal_score_for_side_to_move(board, ply) {
            return terminal;
        }
        if depth == 0 {
            return reference_quiescence(board, alpha, beta, ply, 0, qsearch_limits);
        }

        let moves = reference_ordered_moves(board);
        if moves.is_empty() {
            return -MATE_SCORE + ply;
        }

        let mut best_score = -INF_SCORE;
        for mv in moves {
            let mut child = board.clone();
            child.play_unchecked(mv);
            let score = if let Some(terminal) = terminal_score_for_side_to_move(&child, ply + 1) {
                -terminal
            } else {
                -reference_negamax(&child, depth - 1, -beta, -alpha, ply + 1, qsearch_limits)
            };
            best_score = best_score.max(score);
            alpha = alpha.max(score);
            if alpha >= beta {
                break;
            }
        }
        best_score
    }

    #[cfg(not(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
    fn reference_quiescence(
        board: &Board,
        mut alpha: i32,
        beta: i32,
        ply: i32,
        qply: u8,
        qsearch_limits: QsearchLimits,
    ) -> i32 {
        if let Some(terminal) = terminal_score_for_side_to_move(board, ply) {
            return terminal;
        }
        if qply >= qsearch_limits.max_ply {
            return reference_handcrafted_eval(board, ply);
        }

        let in_check = !board.checkers().is_empty();
        let mut stand_pat_for_delta = None;
        if !in_check {
            let stand_pat = reference_handcrafted_eval(board, ply);
            if stand_pat >= beta {
                return stand_pat;
            }
            alpha = alpha.max(stand_pat);
            stand_pat_for_delta = Some(stand_pat);
        }

        let mut searched_move = false;
        let mut tactical_picker = if in_check {
            QsearchMovePicker::new_evasions(board)
        } else {
            QsearchMovePicker::new_tactical(board)
        };
        while let Some(picked) = tactical_picker.next() {
            let mv = picked.mv;
            if let Some(stand_pat) = stand_pat_for_delta {
                if qply >= qsearch_limits.delta_min_qply
                    && stand_pat
                        .saturating_add(picked.tactical_score.optimistic_delta)
                        .saturating_add(qsearch_limits.delta_margin)
                        <= alpha
                {
                    continue;
                }
            }
            searched_move = true;
            let mut child = board.clone();
            child.play_unchecked(mv);
            let score = if let Some(terminal) = terminal_score_for_side_to_move(&child, ply + 1) {
                -terminal
            } else {
                -reference_quiescence(&child, -beta, -alpha, ply + 1, qply + 1, qsearch_limits)
            };
            if score >= beta {
                return score;
            }
            alpha = alpha.max(score);
        }

        if in_check {
            return if searched_move {
                alpha
            } else {
                -MATE_SCORE + ply
            };
        }

        if qsearch_limits.check_budget > 0 && qply == 0 {
            let mut check_picker = QsearchMovePicker::new_quiet_checks(board);
            while let Some(picked) = check_picker.next() {
                let mv = picked.mv;
                let mut child = board.clone();
                child.play_unchecked(mv);
                let score = if let Some(terminal) = terminal_score_for_side_to_move(&child, ply + 1)
                {
                    -terminal
                } else {
                    -reference_quiescence(
                        &child,
                        -beta,
                        -alpha,
                        ply + 1,
                        qply + 1,
                        QsearchLimits {
                            check_budget: qsearch_limits.check_budget - 1,
                            ..qsearch_limits
                        },
                    )
                };
                if score >= beta {
                    return score;
                }
                alpha = alpha.max(score);
            }
        }

        alpha
    }

    #[cfg(not(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
    fn reference_handcrafted_eval(board: &Board, ply: i32) -> i32 {
        if let Some(terminal) = terminal_score_for_side_to_move(board, ply) {
            return terminal;
        }
        let us = board.side_to_move();
        let our_mobility = count_legal_moves(board) as i32;
        if our_mobility == 0 {
            return -MATE_SCORE + ply;
        }
        let them = !us;
        material_score(board, us) - material_score(board, them)
            + MOBILITY_WEIGHT * (our_mobility - opponent_mobility(board) as i32)
    }

    #[cfg(not(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
    fn reference_ordered_moves(board: &Board) -> Vec<Move> {
        let mut moves = Vec::new();
        board.generate_moves(|piece_moves| {
            moves.extend(piece_moves);
            false
        });
        moves.sort_unstable_by_key(|mv| reference_move_order_key(board, *mv));
        moves
    }

    #[cfg(not(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
    fn reference_move_order_key(
        board: &Board,
        mv: Move,
    ) -> (Reverse<i32>, Reverse<u8>, u8, u8, u8) {
        (
            Reverse(reference_capture_value(board, mv)),
            Reverse(u8::from(mv.is_promotion())),
            u8::from(mv.is_drop()),
            mv.to() as u8,
            reference_from_or_piece_index(mv),
        )
    }

    #[cfg(not(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
    fn reference_capture_value(board: &Board, mv: Move) -> i32 {
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

    #[cfg(not(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
    const fn reference_from_or_piece_index(mv: Move) -> u8 {
        match mv {
            Move::BoardMove { from, .. } => from as u8,
            Move::Drop { piece, .. } => piece as u8,
        }
    }

    #[test]
    #[cfg(not(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
    fn fixed_depth_ordering_matches_reference_scores_on_representative_openings() {
        let openings = [
            "lnsg1gsnl/1r2k2b1/1ppppp+Bpp/p8/9/2P6/PP1PPPPPP/7R1/LNSGKGSNL b P 5",
            "lnsgkgsnl/2r4b1/ppppp1ppp/5p3/7P1/9/PPPPPPP1P/1B5R1/LNSGKGSNL b - 5",
            "lnsgk1snl/1r4gb1/pp1pppppp/2p6/9/9/PPPPPPPPP/1B2GK1R1/LNSG2SNL b - 5",
            "lns1k1snl/1r1g1g1b1/ppppppppp/9/9/9/PPPPPPPPP/1B1RK4/LNSG1GSNL b - 5",
        ];
        let depths: &[u8] = if cfg!(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen"
        )) {
            &[3]
        } else {
            &[4, 5]
        };

        for sfen in openings {
            let board = Board::from_sfen(sfen).unwrap();
            let qsearch_limits = qsearch_limits_without_delta_pruning();
            for &depth in depths {
                let summary = search_board_impl_handcrafted_with_qsearch_limits(
                    &board,
                    depth,
                    qsearch_limits,
                )
                .unwrap();
                let reference = reference_fixed_depth_score(&board, depth, qsearch_limits);
                assert_eq!(
                    summary.best_score, reference,
                    "fixed-depth score diverged at depth {depth} for {sfen}; current best move {:?}",
                    summary.best_move
                );
            }
        }
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

    // The neko run-reflection variants verify king safety by cloning and
    // recomputing for every candidate move, which is correct but far slower, so a
    // fixed 5s budget is not enough to complete depth 3 from the start position.
    // (Performance optimization is deferred; see docs/supported-rules.md.)
    #[test]
    #[cfg(not(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
    fn iterative_search_reaches_requested_depth_when_time_allows() {
        let summary =
            search_iterative_deepening_impl_with_dfpn_mode(haitaka::SFEN_STARTPOS, 3, 0, false)
                .unwrap();
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
    // In the enemy-donor variants (taimen/haimen) an enemy piece adjacent in the
    // donor axis changes the defender's effective movement, so this standard-shogi
    // mate has a different (still valid) solution and the asserted line no longer
    // matches. The any-color neko variants (nekoneko/yokonekoneko) likewise dissolve
    // the mate via run re-segmentation. The mate-solving itself is exercised by the
    // variant tests elsewhere.
    #[cfg(not(any(
        feature = "taimen",
        feature = "haimen",
        feature = "nekoneko",
        feature = "yokonekoneko"
    )))]
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
    #[cfg(not(any(
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
    fn iterative_search_can_disable_dfpn_short_circuiting() {
        let summary =
            search_iterative_deepening_impl_with_dfpn_mode(DFPN_MATE_SFEN, 1, 5_000, false)
                .unwrap();

        assert!(summary.completed_depth > 0);
        assert!(!summary.timed_out);
        assert!(summary.dfpn.is_none());
    }

    #[test]
    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
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
    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko"
    )))]
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

    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen"
    )))]
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
    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen"
    )))]
    fn nnue_search_modes_match_on_start_position() {
        assert_search_modes_match(haitaka::SFEN_STARTPOS, 3);
    }

    #[test]
    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen"
    )))]
    fn nnue_search_modes_match_on_handicap_position() {
        assert_search_modes_match(haitaka::SFEN_6PIECE_HANDICAP, 3);
    }

    #[test]
    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen"
    )))]
    fn nnue_search_modes_match_on_tactical_position() {
        assert_search_modes_match("9/9/k8/9/4Rr3/9/9/9/4K4 b - 1", 3);
    }

    #[test]
    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen"
    )))]
    fn loads_test_nnue_when_available() {
        let Some(model) = load_test_nnue() else {
            return;
        };
        assert!(!model.description().is_empty());
        let score = model.evaluate(&Board::startpos());
        assert!(score.abs() < INF_SCORE);
    }

    #[test]
    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen"
    )))]
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

mod nnue;

use std::cmp::Reverse;
use std::sync::{Arc, OnceLock, RwLock};

use haitaka::{Board, Color, Move, Piece};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchEvalMode {
    FullRefresh,
    Incremental,
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
}

impl SearchContext {
    fn record_state(&mut self) {
        self.states += 1;
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
pub struct PerftResult {
    elapsed_ms: f64,
    nodes: u64,
    nps: f64,
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

fn now_ms() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now()
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};

        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_secs_f64()
            * 1_000.0
    }
}

fn best_move_impl(sfen: &str, depth: u8) -> Result<Option<String>, String> {
    Ok(search_impl(sfen, depth)?.best_move)
}

fn load_nnue_impl(bytes: &[u8]) -> Result<String, String> {
    #[cfg(feature = "annan")]
    {
        let _ = bytes;
        Err("NNUE is currently only supported for standard shogi rules.".to_string())
    }

    #[cfg(not(feature = "annan"))]
    {
        let model =
            NnueModel::from_bytes(bytes).map_err(|err| format!("failed to load NNUE: {err}"))?;
        let description = model.description().to_string();
        *nnue_model_slot().write().unwrap() = Some(Arc::new(model));
        Ok(description)
    }
}

fn search_impl(sfen: &str, depth: u8) -> Result<SearchSummary, String> {
    let evaluation = match current_nnue_model() {
        Some(model) => EvaluationStrategy::Nnue {
            model,
            mode: SearchEvalMode::Incremental,
        },
        None => EvaluationStrategy::Handcrafted,
    };
    search_impl_with_strategy(sfen, depth, evaluation)
}

fn search_impl_with_strategy(
    sfen: &str,
    depth: u8,
    evaluation: EvaluationStrategy,
) -> Result<SearchSummary, String> {
    let board = Board::from_sfen(sfen)
        .map_err(|err| format!("failed to parse SFEN: {err}"))?;
    let depth = depth.max(1);
    let started_at_ms = now_ms();
    let root_state = match &evaluation {
        EvaluationStrategy::Nnue {
            model,
            mode: SearchEvalMode::Incremental,
        } => Some(model.build_position_state_full(&board)),
        _ => None,
    };
    let mut ctx = SearchContext {
        states: 0,
        evaluation,
    };
    let (best_move, best_score) = search_best_move(&board, depth, &mut ctx, root_state)
        .map(|(mv, score)| (Some(mv.to_string()), Some(score)))
        .unwrap_or((None, None));
    let elapsed_ms = (now_ms() - started_at_ms).max(0.0);
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

fn perft_impl(sfen: &str, depth: u8) -> Result<PerftResult, String> {
    let board = Board::from_sfen(sfen)
        .map_err(|err| format!("failed to parse SFEN: {err}"))?;
    let started_at_ms = now_ms();
    let nodes = perft_bulk(&board, depth);
    let elapsed_ms = (now_ms() - started_at_ms).max(0.0);
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

#[wasm_bindgen]
pub fn perft(sfen: &str, depth: u8) -> Result<PerftResult, JsValue> {
    perft_impl(sfen, depth).map_err(|err| JsValue::from_str(&err))
}

fn search_best_move(
    board: &Board,
    depth: u8,
    ctx: &mut SearchContext,
    nnue_state: Option<NnuePositionState>,
) -> Option<(Move, i32)> {
    ctx.record_state();
    let moves = legal_moves(board);
    if moves.is_empty() {
        return None;
    }

    let mut alpha = -INF_SCORE;
    let beta = INF_SCORE;
    let mut best_score = -INF_SCORE;
    let mut best_move = None;

    for mv in moves {
        let mut child = board.clone();
        child.play_unchecked(mv);
        let child_state = child_nnue_state(ctx, board, &child, nnue_state.as_ref(), mv);
        let score = -negamax(
            &child,
            depth.saturating_sub(1),
            -beta,
            -alpha,
            1,
            ctx,
            child_state,
        );
        if score > best_score {
            best_score = score;
            best_move = Some(mv);
        }
        alpha = alpha.max(score);
    }

    best_move.map(|mv| (mv, best_score))
}

fn negamax(
    board: &Board,
    depth: u8,
    mut alpha: i32,
    beta: i32,
    ply: i32,
    ctx: &mut SearchContext,
    nnue_state: Option<NnuePositionState>,
) -> i32 {
    ctx.record_state();
    if depth == 0 {
        return evaluate_or_mate(board, ply, ctx, nnue_state.as_ref());
    }

    let moves = legal_moves(board);
    if moves.is_empty() {
        return -MATE_SCORE + ply;
    }

    let mut best_score = -INF_SCORE;
    for mv in moves {
        let mut child = board.clone();
        child.play_unchecked(mv);
        let child_state = child_nnue_state(ctx, board, &child, nnue_state.as_ref(), mv);
        let score = -negamax(&child, depth - 1, -beta, -alpha, ply + 1, ctx, child_state);
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

    best_score
}

fn child_nnue_state(
    ctx: &SearchContext,
    parent_board: &Board,
    child_board: &Board,
    parent_state: Option<&NnuePositionState>,
    mv: Move,
) -> Option<NnuePositionState> {
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
    let us = board.side_to_move();
    let our_mobility = count_legal_moves(board) as i32;
    if our_mobility == 0 {
        return -MATE_SCORE + ply;
    }

    match &ctx.evaluation {
        EvaluationStrategy::Handcrafted => {
            let them = !us;
            material_score(board, us)
                - material_score(board, them)
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
    board.null_move()
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
    #[cfg(not(feature = "annan"))]
    use std::path::PathBuf;
    #[cfg(not(feature = "annan"))]
    use std::sync::Arc;

    #[cfg(not(feature = "annan"))]
    fn load_test_nnue() -> Option<NnueModel> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shogi-878ca61334a7.nnue");
        let bytes = std::fs::read(path).ok()?;
        NnueModel::from_bytes(&bytes).ok()
    }

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
    #[cfg(not(feature = "annan"))]
    fn returns_legal_move_for_start_position() {
        assert_legal_best_move(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            2,
        );
    }

    #[test]
    #[cfg(not(feature = "annan"))]
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

    #[cfg(not(feature = "annan"))]
    fn assert_search_modes_match(sfen: &str, depth: u8) {
        let Some(model) = load_test_nnue() else {
            return;
        };
        let model = Arc::new(model);
        let full_refresh = search_impl_with_eval_mode(
            sfen,
            depth,
            model.clone(),
            SearchEvalMode::FullRefresh,
        )
        .unwrap();
        let incremental = search_impl_with_eval_mode(
            sfen,
            depth,
            model,
            SearchEvalMode::Incremental,
        )
        .unwrap();

        assert_eq!(incremental.best_move, full_refresh.best_move);
        assert_eq!(incremental.best_score, full_refresh.best_score);
    }

    #[test]
    #[cfg(not(feature = "annan"))]
    fn nnue_search_modes_match_on_start_position() {
        assert_search_modes_match(haitaka::SFEN_STARTPOS, 3);
    }

    #[test]
    #[cfg(not(feature = "annan"))]
    fn nnue_search_modes_match_on_handicap_position() {
        assert_search_modes_match(haitaka::SFEN_6PIECE_HANDICAP, 3);
    }

    #[test]
    #[cfg(not(feature = "annan"))]
    fn nnue_search_modes_match_on_tactical_position() {
        assert_search_modes_match("9/9/k8/9/4Rr3/9/9/9/4K4 b - 1", 3);
    }

    #[test]
    #[cfg(not(feature = "annan"))]
    fn loads_test_nnue_when_available() {
        let Some(model) = load_test_nnue() else {
            return;
        };
        assert!(!model.description().is_empty());
        let score = model.evaluate(&Board::startpos());
        assert!(score.abs() < INF_SCORE);
    }

    #[test]
    #[cfg(feature = "annan")]
    fn perft_matches_annan_start_position_depth_four() {
        assert_eq!(perft_bulk(&Board::startpos(), 4), 605_424);
    }
}

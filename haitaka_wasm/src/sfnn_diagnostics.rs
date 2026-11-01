//! Optional P2 verification exports. Not enabled in production packages.
use super::*;

fn trace(board: &Board, model: &SfnnModel, state: &SfnnPositionState) -> String {
    let result = model.evaluate_from_state(board, state);
    let exact = *state == model.build_position_state_full(board)
        && result == model.evaluate_full_refresh(board);
    format!(
        r#"{{"progress":{{"sumsQ16":{{"black":"{}","white":"{}"}},"selectedSumQ16":"{}","bucket":{}}},"evalCp":{},"fullRefreshMatches":{}}}"#,
        result.progress_sums_q16[0],
        result.progress_sums_q16[1],
        result.selected_sum_q16,
        result.bucket,
        result.eval_cp,
        exact
    )
}

#[wasm_bindgen(js_name = sfnn_trace)]
pub fn sfnn_trace(sfen: &str, moves: &str) -> Result<String, JsValue> {
    trace_impl(sfen, moves).map_err(|e| JsValue::from_str(&e))
}
fn trace_impl(sfen: &str, moves: &str) -> Result<String, String> {
    let EvaluationStrategy::Sfnn { model, .. } = current_evaluation_strategy() else {
        return Err("SFNN diagnostic requires loaded model".into());
    };
    let mut board = Board::from_sfen(sfen).map_err(|e| e.to_string())?;
    if !has_both_kings(&board) {
        return Err("both kings required".into());
    }
    let mut state = model.build_position_state_full(&board);
    let mut output = vec![trace(&board, &model, &state)];
    for token in moves.split_whitespace() {
        let mv: Move = token.parse().map_err(|_| format!("invalid move {token}"))?;
        if !board.is_legal(mv) {
            return Err(format!("illegal move {token}"));
        }
        let mut child = board.clone();
        child.play(mv);
        if !has_both_kings(&child) {
            return Err("terminal king capture has no static evaluation".into());
        }
        state = model.apply_move(&board, &child, &state, mv);
        board = child;
        output.push(trace(&board, &model, &state));
    }
    Ok(format!("[{}]", output.join(",")))
}

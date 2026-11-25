//! Extended builds export the explicit JSON protocol. Ordinary Rust APIs
//! remain available to legacy native consumers, but are not exposed to JS.
use haitaka::extended::compiled as core;
#[cfg(feature = "chushogi")]
use std::sync::{Arc, RwLock};
use wasm_bindgen::prelude::*;

#[cfg(feature = "chushogi")]
static MINASE_NNUE: RwLock<Option<Arc<core::MinaseNnue>>> = RwLock::new(None);

fn js(result: Result<String, String>) -> Result<String, JsValue> {
    result.map_err(|e| JsValue::from_str(&e))
}
#[wasm_bindgen]
pub fn start_position_json() -> String {
    core::start_position_json()
}
#[wasm_bindgen]
pub fn validate_position_json(position: &str) -> Result<String, JsValue> {
    js(core::validate_position_json(position))
}
#[wasm_bindgen]
pub fn inspect_json(position: &str) -> Result<String, JsValue> {
    js(core::inspect_json(position))
}
#[wasm_bindgen]
pub fn legal_moves_json(position: &str) -> Result<String, JsValue> {
    js(core::legal_moves_json(position))
}
#[wasm_bindgen]
pub fn apply_move_json(position: &str, mv: &str) -> Result<String, JsValue> {
    js(core::apply_move_json(position, mv))
}
#[wasm_bindgen]
pub fn perft_json(position: &str, depth: u8) -> Result<String, JsValue> {
    js(core::perft_json(position, depth))
}
#[wasm_bindgen]
pub fn search_json(
    position: &str,
    max_depth: u8,
    timeout_ms: u32,
    node_budget: u32,
) -> Result<String, JsValue> {
    #[cfg(feature = "chushogi")]
    let model = MINASE_NNUE.read().unwrap().clone();
    #[cfg(feature = "chushogi")]
    if let Some(model) = model {
        return js(core::search_json_with_nnue(
            position,
            max_depth,
            timeout_ms,
            node_budget,
            &model,
        ));
    }
    js(core::search_json(
        position,
        max_depth,
        timeout_ms,
        node_budget,
    ))
}
#[cfg(feature = "chushogi")]
#[wasm_bindgen]
pub fn load_nnue(bytes: &[u8]) -> Result<String, JsValue> {
    let model = core::MinaseNnue::from_bytes(bytes).map_err(|e| JsValue::from_str(&e))?;
    let format = model.format();
    *MINASE_NNUE.write().unwrap() = Some(Arc::new(model));
    Ok(format.into())
}

#[cfg(not(feature = "chushogi"))]
#[wasm_bindgen(js_name=load_nnue)]
pub fn reject_nnue(_bytes: &[u8]) -> Result<String, JsValue> {
    Err(JsValue::from_str(
        "NNUE_UNSUPPORTED_RULESET: extended rules require handcrafted evaluation",
    ))
}
#[wasm_bindgen(js_name=load_sfnn)]
pub fn reject_sfnn(_nn: &[u8], _progress: &[u8], _arch: &str) -> Result<String, JsValue> {
    Err(JsValue::from_str(
        "SFNN_UNSUPPORTED_RULESET: extended rules require handcrafted evaluation",
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_loaders_reject_standard_models_before_reading_them() {
        assert!(
            crate::nnue::NnueModel::from_bytes(&[])
                .err()
                .unwrap()
                .to_string()
                .contains("NNUE_UNSUPPORTED_RULESET")
        );
        assert!(
            crate::sfnn::SfnnModel::from_bytes(&[], &[], "mini")
                .err()
                .unwrap()
                .to_string()
                .contains("SFNN_UNSUPPORTED_RULESET")
        );
    }
}

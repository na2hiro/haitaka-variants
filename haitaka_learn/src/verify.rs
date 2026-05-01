use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use haitaka::Board;
use haitaka_wasm::{NnueModel, SearchEvalMode, search_impl_with_eval_mode};
use serde::Serialize;

use crate::config::LoadedConfig;

#[derive(Debug, Serialize)]
pub struct VerificationReport {
    pub report_path: PathBuf,
    pub network_path: String,
    pub description: String,
    pub positions: Vec<VerifiedPosition>,
    pub search_smoke: Option<SearchSmoke>,
}

#[derive(Debug, Serialize)]
pub struct VerifiedPosition {
    pub name: String,
    pub sfen: String,
    pub score: i32,
}

#[derive(Debug, Serialize)]
pub struct SearchSmoke {
    pub sfen: String,
    pub depth: u8,
    pub best_move: Option<String>,
    pub best_score: Option<i32>,
    pub states: u64,
}

pub fn verify(loaded: &LoadedConfig) -> Result<VerificationReport> {
    let artifacts = loaded.artifact_paths();
    let bytes = fs::read(&artifacts.exported_nnue)
        .with_context(|| format!("failed to read {}", artifacts.exported_nnue.display()))?;
    let model = Arc::new(
        NnueModel::from_bytes(&bytes)
            .map_err(|err| anyhow!("failed to parse exported NNUE: {err}"))?,
    );

    let mut positions = Vec::new();
    for fixture in loaded.verification_fixtures() {
        let name = fixture.name;
        let sfen = fixture.sfen;
        let board = Board::from_sfen(sfen)
            .map_err(|err| anyhow!("failed to parse verification SFEN `{name}`: {err}"))?;
        positions.push(VerifiedPosition {
            name: name.to_string(),
            sfen: sfen.to_string(),
            score: model.evaluate(&board),
        });
    }

    let search_smoke = if loaded.config.verify.run_search_smoke {
        match loaded.opening_sfen() {
            Ok(sfen) => {
                let summary = search_impl_with_eval_mode(
                    &sfen,
                    loaded.config.verify.search_depth,
                    model.clone(),
                    SearchEvalMode::Incremental,
                )
                .map_err(|err| anyhow!("search smoke failed: {err}"))?;
                Some(SearchSmoke {
                    sfen,
                    depth: loaded.config.verify.search_depth,
                    best_move: summary.best_move,
                    best_score: summary.best_score,
                    states: summary.states,
                })
            }
            Err(_) => None,
        }
    } else {
        None
    };

    let report = VerificationReport {
        report_path: artifacts.verify_report.clone(),
        network_path: artifacts.exported_nnue.display().to_string(),
        description: model.description().to_string(),
        positions,
        search_smoke,
    };
    fs::write(
        &artifacts.verify_report,
        serde_json::to_vec_pretty(&report)?,
    )
    .with_context(|| format!("failed to write {}", artifacts.verify_report.display()))?;
    Ok(report)
}

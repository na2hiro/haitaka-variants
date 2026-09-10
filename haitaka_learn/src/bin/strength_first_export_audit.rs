//! CPU-only static feature/export audit. Enumerates legal children but never searches.
use anyhow::{Context, Result, anyhow, ensure};
use clap::Parser;
use haitaka::{Board, Color, GameStatus, Move};
use haitaka_wasm::{NnueModel, NnuePositionState, r1_donor_single_active_feature_indices};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    model: PathBuf,
    #[arg(long)]
    output_dir: PathBuf,
}
fn hash(path: &Path) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
}
fn record(
    parent: &Value,
    board: &Board,
    model: &NnueModel,
    state: &NnuePositionState,
    move_: Option<Move>,
) -> Result<Value> {
    let black = r1_donor_single_active_feature_indices(board, Color::Black);
    let white = r1_donor_single_active_feature_indices(board, Color::White);
    let trace = model.r1_inference_trace_from_state(board, state);
    let full = model.evaluate_full_refresh(board);
    ensure!(
        trace.score == full && model.evaluate_from_state(board, state) == full,
        "native full/incremental/trace mismatch"
    );
    let parent_id = parent["id"].as_str().context("parent id")?;
    let move_text = move_.map(|m| m.to_string());
    Ok(json!({
        "schema": "haitaka-strength-first-export-audit-position-v1",
        "id": format!("{}:{}", parent_id, move_text.as_deref().unwrap_or("parent")),
        "parentId": parent_id, "move": move_text, "sfen": board.to_string(),
        "phase": parent["phase"], "sourceOpeningGroupId": parent["sourceOpeningGroupId"],
        "terminal": board.status() != GameStatus::Ongoing, "outputBucket": trace.bucket,
        "black": {"base": black.base, "donor": black.donor},
        "white": {"base": white.base, "donor": white.donor},
        "nativeScore": full, "transformed": trace.transformed.to_vec(),
        "hidden1": trace.hidden1_relu, "hidden2": trace.hidden2_relu,
        "output": trace.output, "psqt": trace.psqt,
        "transformerZeroCount": trace.transformed.iter().filter(|&&v| v == 0).count(),
        "transformerSaturatedCount": trace.transformed.iter().filter(|&&v| v == 127).count(),
        "inCheck": !board.checkers().is_empty(),
    }))
}
fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(!args.output_dir.exists(), "immutable output exists");
    let model =
        NnueModel::from_bytes(&fs::read(&args.model)?).map_err(|e| anyhow!(e.to_string()))?;
    let parents = BufReader::new(File::open(&args.input)?)
        .lines()
        .map(|line| -> Result<Value> { Ok(serde_json::from_str(&line?)?) })
        .collect::<Result<Vec<_>>>()?;
    ensure!(!parents.is_empty() && parents.len() <= 128);
    fs::create_dir_all(&args.output_dir)?;
    let output = args.output_dir.join("positions.jsonl");
    let mut writer = BufWriter::new(File::create(&output)?);
    let mut count = 0;
    for parent in &parents {
        let board =
            Board::from_sfen(parent["sfen"].as_str().context("SFEN")?).map_err(|e| anyhow!(e))?;
        let state = model.build_position_state_full(&board);
        serde_json::to_writer(&mut writer, &record(parent, &board, &model, &state, None)?)?;
        writeln!(writer)?;
        count += 1;
        let mut moves = Vec::new();
        board.generate_moves(|batch| {
            moves.extend(batch.into_iter().filter(|m| board.is_legal(*m)));
            false
        });
        moves.sort_by_key(ToString::to_string);
        moves.dedup_by_key(|m| m.to_string());
        for mv in moves {
            let mut child = board.clone();
            child.play_unchecked(mv);
            let incremental = model.apply_move(&board, &child, &state, mv);
            serde_json::to_writer(
                &mut writer,
                &record(parent, &child, &model, &incremental, Some(mv))?,
            )?;
            writeln!(writer)?;
            count += 1;
        }
    }
    writer.flush()?;
    fs::write(
        args.output_dir.join("receipt.json"),
        serde_json::to_vec_pretty(
            &json!({"passed": true, "parents": parents.len(), "positions": count, "inputSha256": hash(&args.input)?, "modelSha256": hash(&args.model)?, "outputSha256": hash(&output)?, "searchNodes": 0}),
        )?,
    )?;
    Ok(())
}

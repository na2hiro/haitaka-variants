use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, ensure};
use clap::Parser;
use haitaka::Board;
use haitaka_wasm::NnueModel;
use serde::{Deserialize, Serialize};

#[derive(Debug, Parser)]
#[command(about = "Evaluate a candidate NNUE on the frozen R1-A parity corpus")]
struct Args {
    #[arg(long)]
    network: PathBuf,
    #[arg(long)]
    corpus: PathBuf,
    #[arg(long, default_value_t = 1024)]
    count: usize,
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorpusRow {
    id: String,
    sfen: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScoreRow {
    id: String,
    sfen: String,
    score: i32,
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(args.count > 0, "--count must be positive");
    let network_bytes =
        fs::read(&args.network).with_context(|| format!("read {}", args.network.display()))?;
    let model =
        NnueModel::from_bytes(&network_bytes).map_err(|error| anyhow!(error.to_string()))?;
    let corpus = fs::read_to_string(&args.corpus)
        .with_context(|| format!("read {}", args.corpus.display()))?;
    let mut rows = Vec::with_capacity(args.count);
    for (index, line) in corpus.lines().take(args.count).enumerate() {
        let row: CorpusRow =
            serde_json::from_str(line).with_context(|| format!("parse corpus row {index}"))?;
        ensure!(
            row.id == format!("r1a-{index:05}"),
            "unstable corpus order at {index}"
        );
        let board = Board::from_sfen(&row.sfen)
            .map_err(|error| anyhow!("invalid SFEN at {}: {error}", row.id))?;
        let full = model.evaluate_full_refresh(&board);
        let state = model.build_position_state_full(&board);
        let incremental = model.evaluate_from_state(&board, &state);
        ensure!(
            full == incremental,
            "native full/incremental mismatch at {}",
            row.id
        );
        rows.push(ScoreRow {
            id: row.id,
            sfen: row.sfen,
            score: full,
        });
    }
    ensure!(
        rows.len() == args.count,
        "corpus has fewer than {} rows",
        args.count
    );
    let output = serde_json::json!({
        "schema": "haitaka-anhoku-strength-first-native-parity-v1",
        "networkDescription": model.description(),
        "positions": rows,
        "fullRefreshEqualsIncremental": true,
        "status": "passed"
    });
    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&args.output, serde_json::to_vec_pretty(&output)?)?;
    println!(
        "wrote {} native parity scores to {}",
        args.count,
        args.output.display()
    );
    Ok(())
}

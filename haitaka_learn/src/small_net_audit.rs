//! Phase 0 of `plans/anhoku-nnue-small-net-plan.md`: CPU-only coverage audit.
//!
//! Measures how sparsely the production `HalfKAv2^+DonorSingleEff` rows are
//! trained by an existing corpus, and compares that with the king-free
//! projection (the same piece/hand row without the oriented-king offset) that a
//! small A+Donor network would use. It reads packed 72-byte training records
//! only; it never trains, searches, or plays games.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use haitaka::Color;
use haitaka_wasm::r1_donor_single_active_feature_indices;
use serde_json::{Value, json};

use crate::dataset::{PACKED_SFEN_BYTES, unpack_board_from_training};

const RECORD_BYTES: usize = 72;
const SQUARES: usize = 81;
// Mirrors haitaka_wasm::nnue geometry (checked at runtime against observed indices).
const PIECE_INDICES: usize = 1863;
const HALFKA_ROWS: usize = SQUARES * PIECE_INDICES;
const DONOR_ROWS: usize = 1620;
const COUNT_THRESHOLDS: [u64; 5] = [1, 10, 100, 1_000, 10_000];

pub struct SmallNetAuditArgs {
    pub train: Vec<PathBuf>,
    pub eval: Vec<PathBuf>,
    pub output: PathBuf,
}

struct Decoded {
    records: u64,
    decode_failures: u64,
    distinct_boards: u64,
    max_multiplicity: u64,
    boards_seen_more_than_once: u64,
    ply_resets: u64,
    plies: Vec<u16>,
    abs_scores: Vec<u16>,
    results: [u64; 3],
    own_king: [u64; SQUARES],
    halfka: Vec<u64>,
    king_free: Vec<u64>,
    donor: Vec<u64>,
    activations: u64,
}

impl Decoded {
    fn new() -> Self {
        Self {
            records: 0,
            decode_failures: 0,
            distinct_boards: 0,
            max_multiplicity: 0,
            boards_seen_more_than_once: 0,
            ply_resets: 0,
            plies: Vec::new(),
            abs_scores: Vec::new(),
            results: [0; 3],
            own_king: [0; SQUARES],
            halfka: vec![0; HALFKA_ROWS],
            king_free: vec![0; PIECE_INDICES],
            donor: vec![0; DONOR_ROWS],
            activations: 0,
        }
    }
}

fn decode_files(paths: &[PathBuf]) -> Result<Decoded> {
    let mut out = Decoded::new();
    let mut multiplicity: HashMap<u64, u64> = HashMap::new();
    for path in paths {
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        ensure!(
            bytes.len() % RECORD_BYTES == 0,
            "{} is not a whole number of {RECORD_BYTES}-byte records",
            path.display()
        );
        let mut previous_ply: Option<u16> = None;
        for record in bytes.chunks_exact(RECORD_BYTES) {
            out.records += 1;
            let packed: &[u8; PACKED_SFEN_BYTES] = record[..PACKED_SFEN_BYTES].try_into()?;
            let score = i16::from_le_bytes([record[64], record[65]]);
            let ply = u16::from_le_bytes([record[68], record[69]]);
            let result = record[70] as i8;
            if let Some(prev) = previous_ply {
                if ply <= prev {
                    out.ply_resets += 1;
                }
            }
            previous_ply = Some(ply);
            out.plies.push(ply);
            out.abs_scores.push(score.unsigned_abs());
            out.results[(result.clamp(-1, 1) + 1) as usize] += 1;
            let board = match unpack_board_from_training(packed) {
                Ok(board) => board,
                Err(_) => {
                    out.decode_failures += 1;
                    continue;
                }
            };
            *multiplicity.entry(board.hash()).or_default() += 1;
            for perspective in [Color::Black, Color::White] {
                let features = r1_donor_single_active_feature_indices(&board, perspective);
                let mut king_bucket = None;
                for &index in &features.base {
                    ensure!(index < HALFKA_ROWS, "HalfKA index {index} out of range");
                    let bucket = index / PIECE_INDICES;
                    ensure!(
                        king_bucket.is_none_or(|b| b == bucket),
                        "inconsistent king bucket within one perspective"
                    );
                    king_bucket = Some(bucket);
                    out.halfka[index] += 1;
                    out.king_free[index % PIECE_INDICES] += 1;
                    out.activations += 1;
                }
                if let Some(bucket) = king_bucket {
                    out.own_king[bucket] += 1;
                }
                for &index in &features.donor {
                    let local = index - HALFKA_ROWS;
                    ensure!(local < DONOR_ROWS, "donor index {index} out of range");
                    out.donor[local] += 1;
                }
            }
        }
    }
    out.distinct_boards = multiplicity.len() as u64;
    out.max_multiplicity = multiplicity.values().copied().max().unwrap_or(0);
    out.boards_seen_more_than_once = multiplicity.values().filter(|&&c| c > 1).count() as u64;
    Ok(out)
}

fn quantiles<T: Copy + Ord + Into<u64>>(values: &mut [T]) -> Value {
    values.sort_unstable();
    let at = |q: f64| -> u64 {
        if values.is_empty() {
            return 0;
        }
        let i = ((values.len() - 1) as f64 * q).round() as usize;
        values[i].into()
    };
    json!({"p05": at(0.05), "p25": at(0.25), "p50": at(0.5), "p75": at(0.75), "p95": at(0.95), "max": at(1.0)})
}

/// Row-level coverage summary of one row family.
fn row_summary(counts: &[u64]) -> Value {
    let observed = counts.iter().filter(|&&c| c > 0).count();
    let mut observed_counts: Vec<u64> = counts.iter().copied().filter(|&c| c > 0).collect();
    let mut below = serde_json::Map::new();
    for t in COUNT_THRESHOLDS {
        let n = counts.iter().filter(|&&c| c > 0 && c < t).count();
        below.insert(format!("observedRowsBelow{t}"), json!(n));
    }
    json!({
        "rows": counts.len(),
        "observedRows": observed,
        "observedFraction": observed as f64 / counts.len() as f64,
        "countQuantilesAmongObservedRows": quantiles(&mut observed_counts),
        "observedRowsBelowThreshold": below,
    })
}

/// For every activation in `eval`, how many times was that row seen in `train`?
/// Reported as the activation-weighted share of eval activations whose row had
/// fewer than each threshold of training occurrences.
fn eval_exposure(train: &[u64], eval: &[u64]) -> Value {
    let total: u64 = eval.iter().sum();
    let mut shares = serde_json::Map::new();
    for t in COUNT_THRESHOLDS.iter().copied().chain([100_000]) {
        let hit: u64 = eval
            .iter()
            .zip(train)
            .filter(|(_, tr)| **tr < t)
            .map(|(e, _)| *e)
            .sum();
        shares.insert(
            format!("trainCountBelow{t}"),
            json!(hit as f64 / total.max(1) as f64),
        );
    }
    // Activation-weighted median training count.
    let mut weighted: Vec<(u64, u64)> = eval
        .iter()
        .zip(train)
        .filter(|(e, _)| **e > 0)
        .map(|(e, tr)| (*tr, *e))
        .collect();
    weighted.sort_unstable();
    let mut acc = 0;
    let mut median = 0;
    for (tr, e) in &weighted {
        acc += e;
        if acc * 2 >= total {
            median = *tr;
            break;
        }
    }
    json!({
        "evalActivations": total,
        "activationShareWithTrainCount": shares,
        "activationWeightedMedianTrainCount": median,
    })
}

fn king_summary(own_king: &[u64; SQUARES]) -> Value {
    let total: u64 = own_king.iter().sum();
    let mut sorted: Vec<(usize, u64)> = own_king.iter().copied().enumerate().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    let mut cumulative = 0;
    let mut squares_for = serde_json::Map::new();
    let mut targets = vec![0.5, 0.8, 0.9, 0.95, 0.99];
    for (rank, (_, c)) in sorted.iter().enumerate() {
        cumulative += c;
        while let Some(&t) = targets.first() {
            if cumulative as f64 >= t * total as f64 {
                squares_for.insert(format!("{}", (t * 100.0) as u32), json!(rank + 1));
                targets.remove(0);
            } else {
                break;
            }
        }
    }
    let top: Vec<Value> = sorted
        .iter()
        .take(12)
        .map(|(sq, c)| json!({"orientedSquare": sq, "share": *c as f64 / total as f64}))
        .collect();
    json!({
        "perspectiveSamples": total,
        "squaresWithAnySample": own_king.iter().filter(|&&c| c > 0).count(),
        "squaresWithAtLeast1000Samples": own_king.iter().filter(|&&c| c >= 1000).count(),
        "squaresCoveringPercentOfSamples": squares_for,
        "top12": top,
        "histogram": own_king.to_vec(),
    })
}

fn corpus_summary(name: &str, paths: &[PathBuf], d: &mut Decoded) -> Value {
    json!({
        "name": name,
        "files": paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "records": d.records,
        "decodeFailures": d.decode_failures,
        "distinctBoards": d.distinct_boards,
        "boardsSeenMoreThanOnce": d.boards_seen_more_than_once,
        "maxBoardMultiplicity": d.max_multiplicity,
        "plyResetsInStoredOrder": d.ply_resets,
        "gamePlyQuantiles": quantiles(&mut d.plies),
        "absScoreQuantiles": quantiles(&mut d.abs_scores),
        "storedResult": {"loss": d.results[0], "drawOrUnknown": d.results[1], "win": d.results[2]},
        "halfkaActivationsPerPerspective": d.activations as f64 / (2 * d.records.max(1)) as f64,
        "ownKingSquares": king_summary(&d.own_king),
        "halfkaRows": row_summary(&d.halfka),
        "kingFreeRows": row_summary(&d.king_free),
        "donorRows": row_summary(&d.donor),
    })
}

pub fn run(args: SmallNetAuditArgs) -> Result<Value> {
    let mut train = decode_files(&args.train)?;
    let mut report = json!({
        "schema": "haitaka-small-net-phase0-coverage-audit-v1",
        "geometry": {
            "halfkaRows": HALFKA_ROWS,
            "kingFreeRows": PIECE_INDICES,
            "donorRows": DONOR_ROWS,
            "note": "kingFreeRows is the HalfKAv2^ piece/hand row with the oriented-king offset removed, i.e. the input a king-free A+Donor network would use"
        },
    });
    let mut evals = Vec::new();
    for path in &args.eval {
        let mut e = decode_files(std::slice::from_ref(path))?;
        let exposure = json!({
            "halfka": eval_exposure(&train.halfka, &e.halfka),
            "kingFree": eval_exposure(&train.king_free, &e.king_free),
            "donor": eval_exposure(&train.donor, &e.donor),
        });
        let mut summary = corpus_summary("eval", std::slice::from_ref(path), &mut e);
        summary["exposureToTrainRows"] = exposure;
        evals.push(summary);
    }
    report["train"] = corpus_summary("train", &args.train, &mut train);
    report["eval"] = Value::Array(evals);
    write_json(&args.output, &report)?;
    Ok(report)
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

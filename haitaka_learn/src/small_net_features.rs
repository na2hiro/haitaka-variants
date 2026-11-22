//! Feature export and runtime evaluation for the CPU small-net trainer
//! (`scripts/small_net_train.py`, plan phase 1/2).
//!
//! The exporter turns packed 72-byte training records into fixed-size rows of
//! `KingFreeA+DonorSingleEff` feature indices computed by the production
//! runtime, so the Python trainer never reimplements Anhoku donor geometry.
//!
//! Row layout (little endian, `ROW_BYTES` bytes):
//! `score:i16 ply:u16 stm:u8 bucket:u8 n_black:u8 n_white:u8 result:i8 pad:[u8;3]`
//! followed by `MAX_FEATURES` u16 black-perspective indices and the same for
//! white. Unused slots hold `u16::MAX`.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use haitaka::Color;
use haitaka_wasm::{
    KING_FREE_A_DONOR_SINGLE_FEATURES, NnueModel, R1_HALFKAV2_BASE_FEATURES,
    king_free_donor_single_active_feature_indices, r1_donor_single_active_feature_indices,
};

use crate::dataset::{PACKED_SFEN_BYTES, unpack_board_from_training};

const RECORD_BYTES: usize = 72;
pub const MAX_FEATURES: usize = 96;
pub const HEADER_BYTES: usize = 12;
pub const ROW_BYTES: usize = HEADER_BYTES + 2 * 2 * MAX_FEATURES;
/// HalfKA rows (real + factorizer virtual rows) are u32 with a wider slot count.
pub const HALFKA_MAX_FEATURES: usize = 128;
const HALFKA_PIECE_INDICES: usize = 1863;
/// Virtual (king-free) factorizer rows follow every real HalfKAv2+Donor row.
pub const HALFKA_VIRTUAL_OFFSET: usize = R1_HALFKAV2_BASE_FEATURES + 1620;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ExportFamily {
    KingFree,
    Halfka,
}

fn halfka_rows(board: &haitaka::Board, perspective: Color) -> Vec<usize> {
    let active = r1_donor_single_active_feature_indices(board, perspective);
    let mut rows: Vec<usize> = active.base.iter().chain(&active.donor).copied().collect();
    rows.extend(
        active
            .base
            .iter()
            .map(|&i| HALFKA_VIRTUAL_OFFSET + i % HALFKA_PIECE_INDICES),
    );
    rows
}
const PSQT_BUCKETS: usize = 8;
const MAX_PIECES: usize = 40;

/// Must equal `haitaka_wasm::nnue::bucket_for_board`.
fn bucket(board: &haitaka::Board) -> u8 {
    let piece_count = (board.occupied().len() as usize).max(1);
    (((piece_count - 1) * PSQT_BUCKETS) / MAX_PIECES).min(PSQT_BUCKETS - 1) as u8
}

fn records(path: &Path) -> Result<Vec<u8>> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    ensure!(
        bytes.len() % RECORD_BYTES == 0,
        "{} is not a whole number of records",
        path.display()
    );
    Ok(bytes)
}

pub fn export(input: &Path, output: &Path, family: ExportFamily) -> Result<u64> {
    let (max_features, wide) = match family {
        ExportFamily::KingFree => (MAX_FEATURES, false),
        ExportFamily::Halfka => (HALFKA_MAX_FEATURES, true),
    };
    ensure!(
        KING_FREE_A_DONOR_SINGLE_FEATURES < u16::MAX as usize,
        "feature rows must fit u16"
    );
    let bytes = records(input)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut writer = BufWriter::new(File::create(output)?);
    let mut rows = 0u64;
    let mut max_seen = 0usize;
    for record in bytes.chunks_exact(RECORD_BYTES) {
        let packed: &[u8; PACKED_SFEN_BYTES] = record[..PACKED_SFEN_BYTES].try_into()?;
        let board = unpack_board_from_training(packed)?;
        let score = i16::from_le_bytes([record[64], record[65]]);
        let ply = u16::from_le_bytes([record[68], record[69]]);
        let result = record[70] as i8;
        let (black, white) = match family {
            ExportFamily::KingFree => (
                king_free_donor_single_active_feature_indices(&board, Color::Black),
                king_free_donor_single_active_feature_indices(&board, Color::White),
            ),
            ExportFamily::Halfka => (
                halfka_rows(&board, Color::Black),
                halfka_rows(&board, Color::White),
            ),
        };
        for list in [&black, &white] {
            if list.len() > max_features {
                bail!("{} active features exceed MAX_FEATURES", list.len());
            }
            max_seen = max_seen.max(list.len());
        }
        let mut row = Vec::with_capacity(HEADER_BYTES + 2 * 4 * max_features);
        row.extend_from_slice(&score.to_le_bytes());
        row.extend_from_slice(&ply.to_le_bytes());
        row.push(match board.side_to_move() {
            Color::Black => 0,
            Color::White => 1,
        });
        row.push(bucket(&board));
        row.push(black.len() as u8);
        row.push(white.len() as u8);
        row.push(result as u8);
        row.extend_from_slice(&[0; 3]);
        for list in [&black, &white] {
            for slot in 0..max_features {
                if wide {
                    let value = list.get(slot).map(|&i| i as u32).unwrap_or(u32::MAX);
                    row.extend_from_slice(&value.to_le_bytes());
                } else {
                    let value = list.get(slot).map(|&i| i as u16).unwrap_or(u16::MAX);
                    row.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        writer.write_all(&row)?;
        rows += 1;
    }
    writer.flush()?;
    eprintln!(
        "exported {rows} {family:?} rows ({} bytes each, max active features {max_seen}) to {}",
        HEADER_BYTES + 2 * if wide { 4 } else { 2 } * max_features,
        output.display()
    );
    Ok(rows)
}

/// Writes the production static evaluation (side to move, full refresh) of the
/// first `limit` records as little-endian i32, for float/integer parity checks.
pub fn evaluate(nnue: &Path, input: &Path, limit: usize, output: &Path) -> Result<u64> {
    let model = NnueModel::from_bytes(&fs::read(nnue)?)
        .map_err(|e| anyhow::anyhow!("load {}: {e}", nnue.display()))?;
    let bytes = records(input)?;
    let mut out = Vec::new();
    let mut count = 0u64;
    for record in bytes.chunks_exact(RECORD_BYTES).take(limit) {
        let packed: &[u8; PACKED_SFEN_BYTES] = record[..PACKED_SFEN_BYTES].try_into()?;
        let board = unpack_board_from_training(packed)?;
        out.extend_from_slice(&model.evaluate_full_refresh(&board).to_le_bytes());
        count += 1;
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, out)?;
    eprintln!(
        "evaluated {count} records with {} ({})",
        nnue.display(),
        model.feature_family_name()
    );
    Ok(count)
}

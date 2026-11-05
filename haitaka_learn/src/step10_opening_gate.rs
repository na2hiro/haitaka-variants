//! Independent, model-blind Step 10 opening/corpus-family validation.
//!
//! This command is deliberately separate from the opening generator.  It
//! re-reads the generated TSV, decodes every packed corpus record through the
//! repository reader, and rejects any canonical color-swap family overlap.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, ensure};
use haitaka::{Board, Color, Move, Piece};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::dataset::{ENTRY_BYTES, PACKED_SFEN_BYTES, unpack_board_from_training};
use crate::openings::color_swap_anhoku_training_sfen;

#[derive(Debug, Deserialize)]
struct GateSpec {
    suite: Artifact,
    generation_report: Artifact,
    excludes: Vec<Artifact>,
    corpora: Vec<CorpusArtifact>,
    expected: Expected,
}

#[derive(Debug, Deserialize, Clone)]
struct Artifact {
    path: PathBuf,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Deserialize, Clone)]
struct CorpusArtifact {
    role: String,
    path: PathBuf,
    records: u64,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct Expected {
    schema: String,
    seed: u64,
    random_plies: u16,
    positions: usize,
    id_prefix: String,
}

pub(crate) fn validate(
    spec_path: &Path,
    corpus_output: &Path,
    validation_output: &Path,
) -> Result<()> {
    let spec: GateSpec = serde_json::from_slice(
        &fs::read(spec_path).with_context(|| format!("failed to read {}", spec_path.display()))?,
    )?;
    ensure!(
        spec.expected.schema == "haitaka-anhoku-strength-first-step10-opening-gate-spec-v1",
        "wrong Step 10 opening gate spec schema"
    );
    ensure!(
        spec.expected.positions == 128,
        "Step 10 requires exactly 128 positions"
    );
    ensure!(
        spec.expected.random_plies == 16,
        "Step 10 requires exactly 16 random plies"
    );

    verify_artifact(&spec.generation_report)?;
    let generation: Value = serde_json::from_slice(&fs::read(&spec.generation_report.path)?)?;
    ensure!(
        generation["schema"] == "haitaka-anhoku-strength-first-opening-generation-v1",
        "wrong generation receipt schema"
    );
    ensure!(
        generation["seed"] == spec.expected.seed,
        "generation seed differs from gate spec"
    );
    ensure!(
        generation["randomPlies"] == spec.expected.random_plies,
        "generation random plies differ from gate spec"
    );
    ensure!(
        generation["positions"] == spec.expected.positions,
        "generation position count differs from gate spec"
    );
    ensure!(
        generation["rng"] == "rand-std-rng-0.9-lockfile-v1",
        "generation RNG identity differs"
    );

    let mut excluded = HashSet::new();
    let mut exclusion_receipts = Vec::new();
    for item in &spec.excludes {
        verify_artifact(item)?;
        let bytes = fs::read(&item.path)?;
        let mut count = 0u64;
        for (line_number, raw) in String::from_utf8(bytes)?.lines().enumerate() {
            let line = raw.split_once('#').map_or(raw, |(value, _)| value).trim();
            if line.is_empty() {
                continue;
            }
            let (_, sfen) = line.split_once('\t').ok_or_else(|| {
                anyhow!(
                    "invalid exclusion row {}:{}",
                    item.path.display(),
                    line_number + 1
                )
            })?;
            let board = Board::from_sfen(sfen.trim())
                .map_err(|error| anyhow!("invalid exclusion SFEN: {error:?}"))?;
            ensure!(has_both_kings(&board), "exclusion row has no two kings");
            excluded.insert(canonical_family(&board)?);
            count += 1;
        }
        exclusion_receipts.push(
            json!({"path": item.path, "bytes": item.bytes, "sha256": item.sha256, "rows": count}),
        );
    }

    let suite_bytes = fs::read(&spec.suite.path)?;
    let suite_text = String::from_utf8(suite_bytes.clone())?;
    let mut rows = Vec::with_capacity(spec.expected.positions);
    let mut families = HashSet::new();
    let mut ids = HashSet::new();
    for (line_number, raw) in suite_text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        ensure!(
            line.matches('\t').count() == 1,
            "suite row {} must contain exactly one tab",
            line_number + 1
        );
        let (id, sfen) = line.split_once('\t').unwrap();
        ensure!(
            id == format!("{}-{:03}", spec.expected.id_prefix, rows.len() + 1),
            "suite ID/order mismatch at row {}",
            line_number + 1
        );
        ensure!(ids.insert(id.to_string()), "duplicate suite ID {id}");
        let board =
            Board::from_sfen(sfen).map_err(|error| anyhow!("invalid suite SFEN: {error:?}"))?;
        ensure!(has_both_kings(&board), "suite row {id} has no two kings");
        ensure!(
            !legal_moves(&board).is_empty(),
            "suite row {id} has no legal move"
        );
        let family = canonical_family(&board)?;
        ensure!(
            !excluded.contains(&family),
            "suite row {id} overlaps a prior opening suite"
        );
        ensure!(
            families.insert(family.clone()),
            "suite row {id} duplicates a canonical family"
        );
        rows.push(json!({"rowIndex": rows.len(), "openingId": id, "sfen": sfen, "canonicalFamily": family}));
    }
    ensure!(
        rows.len() == spec.expected.positions,
        "suite has {} rows, expected {}",
        rows.len(),
        spec.expected.positions
    );

    let mut corpus_receipts = Vec::new();
    let mut corpus_overlaps = Vec::new();
    for corpus in &spec.corpora {
        verify_corpus(corpus, &families, &mut corpus_overlaps)?;
        corpus_receipts.push(json!({"role": corpus.role, "path": corpus.path, "records": corpus.records, "bytes": corpus.bytes, "sha256": corpus.sha256}));
    }
    ensure!(
        corpus_overlaps.is_empty(),
        "generated opening suite overlaps corpus canonical families"
    );

    let corpus_receipt = json!({
        "schema": "haitaka-anhoku-strength-first-opening-corpus-family-exclusion-v1",
        "status": "passed",
        "familyDefinition": "repository-packed-board-reader-plus-canonical-color-swap-v1",
        "suite": {"path": spec.suite.path, "bytes": spec.suite.bytes, "sha256": spec.suite.sha256, "rows": rows.len()},
        "corpora": corpus_receipts,
        "overlaps": corpus_overlaps,
        "exclusions": exclusion_receipts,
        "modelBlind": true,
    });
    write_json(corpus_output, &corpus_receipt)?;
    let validation_receipt = json!({
        "schema": "haitaka-anhoku-strength-first-step10-opening-validation-v1",
        "status": "passed",
        "validator": "haitaka_learn.step10_opening_gate",
        "suite": {"path": spec.suite.path, "bytes": spec.suite.bytes, "sha256": spec.suite.sha256},
        "rows": rows,
        "priorSuiteExclusions": exclusion_receipts,
        "corpusFamilyExclusion": {"path": corpus_output, "sha256": file_sha256(corpus_output)?},
        "legalRows": spec.expected.positions,
        "uniqueIds": true,
        "uniqueCanonicalFamilies": true,
        "originalRowOrderPreserved": true,
        "modelBlind": true,
    });
    write_json(validation_output, &validation_receipt)
}

fn verify_corpus(
    corpus: &CorpusArtifact,
    families: &HashSet<String>,
    overlaps: &mut Vec<Value>,
) -> Result<()> {
    ensure!(
        corpus.bytes == corpus.records * ENTRY_BYTES as u64,
        "{} byte count is not records*72",
        corpus.path.display()
    );
    verify_artifact(&Artifact {
        path: corpus.path.clone(),
        bytes: corpus.bytes,
        sha256: corpus.sha256.clone(),
    })?;
    let mut reader = BufReader::new(File::open(&corpus.path)?);
    let mut record = [0u8; ENTRY_BYTES];
    for index in 0..corpus.records {
        reader
            .read_exact(&mut record)
            .with_context(|| format!("short record {} in {}", index, corpus.path.display()))?;
        let packed: [u8; PACKED_SFEN_BYTES] = record[..PACKED_SFEN_BYTES].try_into().unwrap();
        let board = unpack_board_from_training(&packed)?;
        let family = canonical_family(&board)?;
        if families.contains(&family) {
            overlaps.push(
                json!({"corpus": corpus.role, "recordIndex": index, "canonicalFamily": family}),
            );
        }
    }
    Ok(())
}

fn canonical_family(board: &Board) -> Result<String> {
    let direct = board.to_string();
    let swapped = color_swap_anhoku_training_sfen(&direct)?;
    Ok(if direct <= swapped { direct } else { swapped })
}

fn legal_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|generated| {
        moves.extend(generated);
        false
    });
    moves
}

fn has_both_kings(board: &Board) -> bool {
    board.has(Color::Black, Piece::King) && board.has(Color::White, Piece::King)
}

fn verify_artifact(item: &Artifact) -> Result<()> {
    ensure!(
        item.path.is_file(),
        "missing artifact {}",
        item.path.display()
    );
    ensure!(
        fs::metadata(&item.path)?.len() == item.bytes,
        "byte count mismatch for {}",
        item.path.display()
    );
    ensure!(
        file_sha256(&item.path)? == item.sha256,
        "SHA-256 mismatch for {}",
        item.path.display()
    );
    Ok(())
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::pack_board_for_training;

    #[test]
    fn color_swap_boards_share_one_canonical_family() {
        let board = Board::startpos();
        let swapped =
            Board::from_sfen(&color_swap_anhoku_training_sfen(&board.to_string()).unwrap())
                .unwrap();
        assert_eq!(
            canonical_family(&board).unwrap(),
            canonical_family(&swapped).unwrap()
        );
    }

    #[test]
    fn packed_board_round_trip_uses_the_same_family_definition() {
        let board = Board::startpos();
        let packed = pack_board_for_training(&board).unwrap();
        let decoded = unpack_board_from_training(&packed).unwrap();
        assert_eq!(
            canonical_family(&board).unwrap(),
            canonical_family(&decoded).unwrap()
        );
    }

    #[test]
    fn family_fixture_rejects_duplicate_and_overlap_conceptually() {
        let board = Board::startpos();
        let family = canonical_family(&board).unwrap();
        let mut seen = HashSet::new();
        assert!(seen.insert(family.clone()));
        assert!(!seen.insert(family.clone()));
        assert!(seen.contains(&family));
    }
}

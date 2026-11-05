//! Preparation-only family inventory for the Step 12 exclusion allowlist.
//!
//! This command reads only the explicitly named input files supplied by the
//! Step 12 adapter.  Packed records are decoded by the repository training
//! reader and all board families use the Step 11 canonical color-swap
//! implementation.  It never discovers files recursively and rejects the
//! permanently protected abandoned-result path.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use haitaka::{Board, Color, Piece};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::dataset::{ENTRY_BYTES, PACKED_SFEN_BYTES, unpack_board_from_training};
use crate::step11_opening_gate::canonical_family;

const PROTECTED_TOKEN: &str =
    "step9-independent-confirmation-20260913-abandoned-1000ms-partial-unopened";
const FAMILY_DEFINITION: &str = "repository-packed-board-reader-plus-canonical-color-swap-v1";

pub(crate) fn inventory(inputs: &[PathBuf], output: &Path, inventory_id: &str) -> Result<Value> {
    ensure!(!inputs.is_empty(), "at least one --input is required");
    ensure!(
        !inventory_id.trim().is_empty(),
        "--inventory-id must not be empty"
    );
    reject_protected(output)?;

    let mut families = BTreeSet::<String>::new();
    let mut input_reports = Vec::with_capacity(inputs.len());
    let mut total_rows = 0u64;

    for input in inputs {
        reject_protected(input)?;
        ensure!(
            input.is_file(),
            "missing inventory input {}",
            input.display()
        );
        let before = families.len();
        let suffix = input
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match suffix.as_str() {
            "bin" => inventory_packed(input, &mut families)?,
            "sfen" | "tsv" => inventory_text(input, &mut families)?,
            _ => bail!(
                "unsupported family-inventory input {}; use an explicit .bin, .sfen or .tsv file",
                input.display()
            ),
        }
        let rows = rows_for_input(input, &suffix)?;
        total_rows = total_rows
            .checked_add(rows)
            .ok_or_else(|| anyhow!("family-inventory row count overflow"))?;
        input_reports.push(json!({
            "path": input,
            "bytes": fs::metadata(input)?.len(),
            "sha256": file_sha256(input)?,
            "format": suffix,
            "rows": rows,
            "newCanonicalFamilies": families.len() - before,
        }));
    }

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut writer = BufWriter::new(File::create(output)?);
    let metadata = json!({
        "kind": "metadata",
        "schema": "haitaka-anhoku-strength-first-step12-family-inventory-v1",
        "inventoryId": inventory_id,
        "familyDefinition": FAMILY_DEFINITION,
        "canonicalizationSource": {
            "path": "haitaka_learn/src/step11_opening_gate.rs",
            "sha256": file_sha256(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src/step11_opening_gate.rs"))?,
        },
        "inputs": input_reports,
        "rows": total_rows,
        "uniqueCanonicalFamilies": families.len(),
        "ordering": "UTF-8 lexicographic canonical-family string",
    });
    serde_json::to_writer(&mut writer, &metadata)?;
    writer.write_all(b"\n")?;
    for family in &families {
        serde_json::to_writer(
            &mut writer,
            &json!({
                "kind": "family",
                "canonicalFamily": family,
            }),
        )?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;

    let binary = std::env::current_exe().context("failed to resolve inventory producer binary")?;
    Ok(json!({
        "schema": "haitaka-anhoku-strength-first-step12-family-inventory-receipt-v1",
        "status": "passed",
        "inventoryId": inventory_id,
        "output": {
            "path": output,
            "bytes": fs::metadata(output)?.len(),
            "sha256": file_sha256(output)?,
        },
        "producer": {
            "command": "haitaka_learn step12-exclusion-inventory",
            "binary": {
                "path": binary,
                "bytes": fs::metadata(&binary)?.len(),
                "sha256": file_sha256(&binary)?,
            },
            "canonicalizationSource": {
                "path": "haitaka_learn/src/step11_opening_gate.rs",
                "sha256": file_sha256(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src/step11_opening_gate.rs"))?,
            },
        },
        "familyDefinition": FAMILY_DEFINITION,
        "rows": total_rows,
        "uniqueCanonicalFamilies": families.len(),
        "inputCount": inputs.len(),
    }))
}

fn inventory_packed(path: &Path, families: &mut BTreeSet<String>) -> Result<()> {
    let bytes = fs::metadata(path)?.len();
    ensure!(
        bytes % ENTRY_BYTES as u64 == 0,
        "packed inventory input {} has {} bytes, not a multiple of {}",
        path.display(),
        bytes,
        ENTRY_BYTES
    );
    let records = bytes / ENTRY_BYTES as u64;
    let mut reader = BufReader::new(File::open(path)?);
    let mut record = [0u8; ENTRY_BYTES];
    for index in 0..records {
        reader
            .read_exact(&mut record)
            .with_context(|| format!("short packed record {} in {}", index, path.display()))?;
        let packed: [u8; PACKED_SFEN_BYTES] = record[..PACKED_SFEN_BYTES]
            .try_into()
            .expect("fixed packed-board prefix");
        let board = unpack_board_from_training(&packed)
            .with_context(|| format!("invalid packed board {} in {}", index, path.display()))?;
        ensure_both_kings(&board, path, index)?;
        families.insert(canonical_family(&board)?);
    }
    Ok(())
}

fn inventory_text(path: &Path, families: &mut BTreeSet<String>) -> Result<()> {
    let reader = BufReader::new(File::open(path)?);
    for (line_number, raw) in reader.lines().enumerate() {
        let raw = raw?;
        let line = raw
            .split_once('#')
            .map_or(raw.as_str(), |(value, _)| value)
            .trim();
        if line.is_empty() {
            continue;
        }
        let sfen = line
            .split_once('\t')
            .map_or(line, |(_, value)| value)
            .trim();
        let board = Board::from_sfen(sfen).map_err(|error| {
            anyhow!(
                "invalid SFEN in {}:{}: {error:?}",
                path.display(),
                line_number + 1
            )
        })?;
        ensure_both_kings(&board, path, line_number as u64 + 1)?;
        families.insert(canonical_family(&board)?);
    }
    Ok(())
}

fn rows_for_input(path: &Path, suffix: &str) -> Result<u64> {
    if suffix == "bin" {
        return Ok(fs::metadata(path)?.len() / ENTRY_BYTES as u64);
    }
    let reader = BufReader::new(File::open(path)?);
    let mut rows = 0u64;
    for raw in reader.lines() {
        let raw = raw?;
        let line = raw
            .split_once('#')
            .map_or(raw.as_str(), |(value, _)| value)
            .trim();
        if !line.is_empty() {
            rows = rows
                .checked_add(1)
                .ok_or_else(|| anyhow!("family-inventory row count overflow"))?;
        }
    }
    Ok(rows)
}

fn ensure_both_kings(board: &Board, path: &Path, index: u64) -> Result<()> {
    ensure!(
        board.has(Color::Black, Piece::King) && board.has(Color::White, Piece::King),
        "family-inventory input {} row/record {} does not contain both kings",
        path.display(),
        index
    );
    Ok(())
}

fn reject_protected(path: &Path) -> Result<()> {
    ensure!(
        !path.to_string_lossy().contains(PROTECTED_TOKEN),
        "protected historical path is forbidden: {}",
        PROTECTED_TOKEN
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::pack_board_for_training;
    use tempfile::tempdir;

    #[test]
    fn text_inventory_deduplicates_color_swapped_families() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("suite.tsv");
        let output = dir.path().join("families.jsonl");
        let start = Board::startpos();
        let swapped = crate::openings::color_swap_anhoku_training_sfen(&start.to_string()).unwrap();
        fs::write(&input, format!("a\t{}\nb\t{}\n", start, swapped)).unwrap();
        let receipt = inventory(&[input], &output, "fixture").unwrap();
        assert_eq!(receipt["uniqueCanonicalFamilies"], 1);
        assert_eq!(receipt["rows"], 2);
    }

    #[test]
    fn packed_inventory_reads_the_repository_record_shape() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("records.bin");
        let output = dir.path().join("families.jsonl");
        let packed = pack_board_for_training(&Board::startpos()).unwrap();
        let mut record = [0u8; ENTRY_BYTES];
        record[..PACKED_SFEN_BYTES].copy_from_slice(&packed);
        fs::write(&input, record).unwrap();
        let receipt = inventory(&[input], &output, "packed-fixture").unwrap();
        assert_eq!(receipt["rows"], 1);
        assert_eq!(receipt["uniqueCanonicalFamilies"], 1);
    }

    #[test]
    fn malformed_and_truncated_inputs_fail_closed() {
        let dir = tempdir().unwrap();
        let malformed = dir.path().join("malformed.tsv");
        let malformed_output = dir.path().join("malformed.jsonl");
        fs::write(&malformed, b"id\tnot-a-valid-sfen\n").unwrap();
        assert!(inventory(&[malformed], &malformed_output, "malformed").is_err());

        let truncated = dir.path().join("truncated.bin");
        let truncated_output = dir.path().join("truncated.jsonl");
        fs::write(&truncated, [0u8; ENTRY_BYTES - 1]).unwrap();
        assert!(inventory(&[truncated], &truncated_output, "truncated").is_err());
    }

    #[test]
    fn inventory_output_is_deterministic_and_protected_paths_are_rejected() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("suite.sfen");
        let output_a = dir.path().join("families-a.jsonl");
        let output_b = dir.path().join("families-b.jsonl");
        fs::write(&input, format!("{}\n", Board::startpos())).unwrap();
        inventory(std::slice::from_ref(&input), &output_a, "repeat").unwrap();
        inventory(std::slice::from_ref(&input), &output_b, "repeat").unwrap();
        assert_eq!(fs::read(&output_a).unwrap(), fs::read(&output_b).unwrap());

        let forbidden = PathBuf::from(PROTECTED_TOKEN).join("anything.bin");
        assert!(
            inventory(
                &[forbidden],
                &dir.path().join("forbidden.jsonl"),
                "forbidden"
            )
            .is_err()
        );
    }
}

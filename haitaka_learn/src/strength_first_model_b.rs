use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, ensure};
use haitaka::{Board, Color, Move, Piece};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::dataset::{ENTRY_BYTES, PACKED_SFEN_BYTES, unpack_board_from_training};
use crate::openings::color_swap_anhoku_sfen;

#[derive(Clone)]
struct RankedRecord {
    rank: [u8; 32],
    source_index: u64,
    family: String,
    bytes: [u8; ENTRY_BYTES],
}

pub(crate) fn generate_opening_suite(
    output: &Path,
    report: &Path,
    seed: u64,
    positions: usize,
    random_plies: u16,
    id_prefix: &str,
    exclude_suites: &[PathBuf],
) -> Result<()> {
    ensure!(positions > 0, "--positions must be positive");
    ensure!(random_plies > 0, "--random-plies must be positive");
    ensure!(
        !id_prefix.trim().is_empty(),
        "--id-prefix must not be empty"
    );

    let mut excluded = HashSet::new();
    let mut exclusion_receipts = Vec::new();
    for path in exclude_suites {
        let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        for raw in String::from_utf8(bytes.clone())?.lines() {
            let line = raw.split_once('#').map_or(raw, |(value, _)| value).trim();
            if line.is_empty() {
                continue;
            }
            let (_, sfen) = line
                .split_once('\t')
                .with_context(|| format!("invalid opening-suite line in {}", path.display()))?;
            let board = Board::from_sfen(sfen.trim())
                .map_err(|error| anyhow!("invalid SFEN in {}: {error}", path.display()))?;
            excluded.insert(canonical_family(&board)?);
        }
        exclusion_receipts.push(file_receipt(path, Some(bytes.len() as u64))?);
    }

    let mut rng = StdRng::seed_from_u64(seed);
    let mut selected = Vec::with_capacity(positions);
    let mut selected_families = HashSet::with_capacity(positions);
    let mut attempts = 0u64;
    let maximum_attempts = (positions as u64).saturating_mul(10_000).max(10_000);
    while selected.len() < positions && attempts < maximum_attempts {
        attempts += 1;
        let mut board = Board::startpos();
        let mut complete = true;
        for _ in 0..random_plies {
            if !has_both_kings(&board) {
                complete = false;
                break;
            }
            let legal = legal_moves(&board);
            if legal.is_empty() {
                complete = false;
                break;
            }
            board.play_unchecked(legal[rng.random_range(0..legal.len())]);
        }
        if !complete || !has_both_kings(&board) || legal_moves(&board).is_empty() {
            continue;
        }
        let family = canonical_family(&board)?;
        if excluded.contains(&family) || !selected_families.insert(family) {
            continue;
        }
        selected.push(board.to_string());
    }
    ensure!(
        selected.len() == positions,
        "opening generation exhausted {maximum_attempts} attempts after selecting {}/{} positions",
        selected.len(),
        positions
    );

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut writer = BufWriter::new(File::create(output)?);
    writeln!(writer, "# Haitaka Anhoku frozen confirmation opening suite")?;
    writeln!(
        writer,
        "# Uniform legal {random_plies}-ply source games; seed {seed}."
    )?;
    writeln!(writer, "# Format: <stable-opening-id><TAB><SFEN>")?;
    for (index, sfen) in selected.iter().enumerate() {
        writeln!(writer, "{id_prefix}-{:03}\t{sfen}", index + 1)?;
    }
    writer.flush()?;

    let result = json!({
        "schema": "haitaka-anhoku-strength-first-opening-generation-v1",
        "seed": seed,
        "randomPlies": random_plies,
        "rng": "rand-std-rng-0.9-lockfile-v1",
        "attempts": attempts,
        "positions": positions,
        "colorSwapUnique": true,
        "excludedSuites": exclusion_receipts,
        "output": file_receipt(output, None)?,
    });
    write_json(report, &result)
}

pub(crate) fn freeze_corpus(
    model_a: &[PathBuf],
    source_train: &Path,
    source_validation: &Path,
    output_dir: &Path,
    train_records: usize,
    validation_records: usize,
    selection_seed: u64,
) -> Result<PathBuf> {
    ensure!(
        !model_a.is_empty(),
        "at least one --model-a input is required"
    );
    ensure!(train_records > 0, "--train-records must be positive");
    ensure!(
        validation_records > 0,
        "--validation-records must be positive"
    );

    let mut excluded = HashSet::new();
    let mut model_a_receipts = Vec::new();
    for path in model_a {
        let (records, inserted) = load_exclusion_families(path, &mut excluded)?;
        let mut receipt = file_receipt(path, None)?;
        receipt["records"] = json!(records);
        receipt["newCanonicalFamilies"] = json!(inserted);
        model_a_receipts.push(receipt);
    }

    let (train_candidates, train_stats) =
        load_ranked_unique(source_train, "training", selection_seed, &excluded)?;
    ensure!(
        train_candidates.len() >= train_records,
        "model-B training shortfall after exclusions: eligible {} < required {train_records}",
        train_candidates.len()
    );
    let selected_train = train_candidates
        .into_iter()
        .take(train_records)
        .collect::<Vec<_>>();
    let selected_train_families = selected_train
        .iter()
        .map(|record| record.family.clone())
        .collect::<HashSet<_>>();

    let mut validation_exclusions = excluded.clone();
    validation_exclusions.extend(selected_train_families.iter().cloned());
    let (validation_candidates, validation_stats) = load_ranked_unique(
        source_validation,
        "validation",
        selection_seed,
        &validation_exclusions,
    )?;
    ensure!(
        validation_candidates.len() >= validation_records,
        "model-B validation shortfall after exclusions: eligible {} < required {validation_records}",
        validation_candidates.len()
    );
    let selected_validation = validation_candidates
        .into_iter()
        .take(validation_records)
        .collect::<Vec<_>>();

    fs::create_dir_all(output_dir)?;
    let train_path = output_dir.join("training.bin");
    let validation_path = output_dir.join("validation.bin");
    write_records(&train_path, &selected_train)?;
    write_records(&validation_path, &selected_validation)?;

    let train_receipt = file_receipt(&train_path, None)?;
    let validation_receipt = file_receipt(&validation_path, None)?;
    let train_manifest_path = output_dir.join("training.json");
    let validation_manifest_path = output_dir.join("validation.json");
    write_json(
        &train_manifest_path,
        &split_manifest(
            "training",
            train_records,
            &train_receipt,
            source_train,
            &train_stats,
        )?,
    )?;
    write_json(
        &validation_manifest_path,
        &split_manifest(
            "validation",
            validation_records,
            &validation_receipt,
            source_validation,
            &validation_stats,
        )?,
    )?;

    let manifest_path = output_dir.join("model-b-corpus.json");
    let manifest = json!({
        "schema": "haitaka-anhoku-strength-first-model-b-corpus-v1",
        "selectionSeed": selection_seed,
        "selectionPolicy": "sha256-rank-within-split-v1",
        "deduplicationPolicy": "exclude-model-a-and-deduplicate-model-b-by-anhoku-color-swap-board-family-v1",
        "modelA": model_a_receipts,
        "modelACanonicalFamilies": excluded.len(),
        "training": {
            "records": train_records,
            "binary": train_receipt,
            "manifest": file_receipt(&train_manifest_path, None)?,
            "sourceStats": train_stats,
        },
        "validation": {
            "records": validation_records,
            "binary": validation_receipt,
            "manifest": file_receipt(&validation_manifest_path, None)?,
            "sourceStats": validation_stats,
            "excludedSelectedTrainingFamilies": selected_train_families.len(),
        },
        "passed": true,
    });
    write_json(&manifest_path, &manifest)?;
    Ok(manifest_path)
}

fn load_exclusion_families(path: &Path, families: &mut HashSet<String>) -> Result<(u64, u64)> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut record = [0u8; ENTRY_BYTES];
    let mut records = 0u64;
    let initial = families.len();
    loop {
        match reader.read_exact(&mut record) {
            Ok(()) => {
                records += 1;
                families.insert(record_family(&record)?);
            }
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err.into()),
        }
    }
    let bytes = fs::metadata(path)?.len();
    ensure!(
        bytes == records * ENTRY_BYTES as u64,
        "{} is truncated",
        path.display()
    );
    Ok((records, (families.len() - initial) as u64))
}

fn load_ranked_unique(
    path: &Path,
    split: &str,
    seed: u64,
    exclusions: &HashSet<String>,
) -> Result<(Vec<RankedRecord>, Value)> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut record = [0u8; ENTRY_BYTES];
    let mut by_family = HashMap::<String, RankedRecord>::new();
    let mut source_records = 0u64;
    let mut excluded_records = 0u64;
    let mut duplicate_records = 0u64;
    loop {
        match reader.read_exact(&mut record) {
            Ok(()) => {
                let source_index = source_records;
                source_records += 1;
                let family = record_family(&record)?;
                if exclusions.contains(&family) {
                    excluded_records += 1;
                    continue;
                }
                let rank = selection_rank(seed, split, source_index, &record);
                let candidate = RankedRecord {
                    rank,
                    source_index,
                    family: family.clone(),
                    bytes: record,
                };
                match by_family.entry(family) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(candidate);
                    }
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        duplicate_records += 1;
                        if (candidate.rank, candidate.source_index)
                            < (entry.get().rank, entry.get().source_index)
                        {
                            entry.insert(candidate);
                        }
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err.into()),
        }
    }
    ensure!(
        fs::metadata(path)?.len() == source_records * ENTRY_BYTES as u64,
        "{} is truncated",
        path.display()
    );
    let mut selected = by_family.into_values().collect::<Vec<_>>();
    selected.sort_unstable_by_key(|record| (record.rank, record.source_index));
    let stats = json!({
        "source": file_receipt(path, None)?,
        "sourceRecords": source_records,
        "excludedRecords": excluded_records,
        "duplicateCanonicalFamilyRecords": duplicate_records,
        "eligibleCanonicalFamilies": selected.len(),
    });
    Ok((selected, stats))
}

fn record_family(record: &[u8; ENTRY_BYTES]) -> Result<String> {
    let packed: [u8; PACKED_SFEN_BYTES] = record[..PACKED_SFEN_BYTES]
        .try_into()
        .expect("fixed packed-board prefix");
    canonical_family(&unpack_board_from_training(&packed)?)
}

fn canonical_family(board: &Board) -> Result<String> {
    let direct = board.to_string();
    let swapped = color_swap_anhoku_sfen(&direct)?;
    Ok(if direct <= swapped { direct } else { swapped })
}

fn selection_rank(seed: u64, split: &str, source_index: u64, record: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"haitaka-strength-first-model-b-selection-v1\0");
    hasher.update(seed.to_le_bytes());
    hasher.update(split.as_bytes());
    hasher.update([0]);
    hasher.update(source_index.to_le_bytes());
    hasher.update(record);
    hasher.finalize().into()
}

fn write_records(path: &Path, records: &[RankedRecord]) -> Result<()> {
    let mut writer = BufWriter::new(File::create(path)?);
    for record in records {
        writer.write_all(&record.bytes)?;
    }
    writer.flush()?;
    Ok(())
}

fn split_manifest(
    split: &str,
    records: usize,
    binary: &Value,
    source: &Path,
    stats: &Value,
) -> Result<Value> {
    Ok(json!({
        "schema": "haitaka-strength-first-frozen-dataset-manifest-v1",
        "dataset": split,
        "records": records,
        "entryBytes": ENTRY_BYTES,
        "binary": binary,
        "source": file_receipt(source, None)?,
        "sourceStats": stats,
        "labelEvaluator": "haitaka-native-handcrafted-search-v1",
        "labelBudget": 10000,
        "targetSemantics": "bounded-nonmate-root-plus-explicit-mate-class-distance",
        "scoreTransform": "identity-with-robust-clip-v1",
    }))
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

fn file_receipt(path: &Path, known_bytes: Option<u64>) -> Result<Value> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    let mut bytes = 0u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes += read as u64;
    }
    if let Some(expected) = known_bytes {
        ensure!(
            bytes == expected,
            "{} changed while hashing",
            path.display()
        );
    }
    Ok(json!({
        "path": path.display().to_string(),
        "bytes": bytes,
        "sha256": format!("{:x}", hasher.finalize()),
    }))
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::pack_board_for_training;
    use tempfile::tempdir;

    fn boards(count: usize, seed: u64) -> Vec<Board> {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut result = Vec::new();
        let mut families = HashSet::new();
        while result.len() < count {
            let mut board = Board::startpos();
            for _ in 0..16 {
                let moves = legal_moves(&board);
                board.play_unchecked(moves[rng.random_range(0..moves.len())]);
            }
            if has_both_kings(&board) && families.insert(canonical_family(&board).unwrap()) {
                result.push(board);
            }
        }
        result
    }

    fn record(board: &Board, score: i16) -> [u8; ENTRY_BYTES] {
        let mut result = [0u8; ENTRY_BYTES];
        result[..PACKED_SFEN_BYTES].copy_from_slice(&pack_board_for_training(board).unwrap());
        result[PACKED_SFEN_BYTES..PACKED_SFEN_BYTES + 2].copy_from_slice(&score.to_le_bytes());
        result
    }

    fn write_test_records(path: &Path, records: &[[u8; ENTRY_BYTES]]) {
        let mut writer = BufWriter::new(File::create(path).unwrap());
        for record in records {
            writer.write_all(record).unwrap();
        }
        writer.flush().unwrap();
    }

    fn read_families(path: &Path) -> HashSet<String> {
        let mut reader = BufReader::new(File::open(path).unwrap());
        let mut record = [0u8; ENTRY_BYTES];
        let mut result = HashSet::new();
        while reader.read_exact(&mut record).is_ok() {
            assert!(result.insert(record_family(&record).unwrap()));
        }
        result
    }

    #[test]
    #[cfg(feature = "anhoku")]
    fn freezes_exact_disjoint_corpus() {
        let temp = tempdir().unwrap();
        let boards = boards(12, 44);
        let model_a = temp.path().join("model-a.bin");
        let source_train = temp.path().join("source-train.bin");
        let source_validation = temp.path().join("source-validation.bin");
        write_test_records(&model_a, &[record(&boards[0], 1)]);
        write_test_records(
            &source_train,
            &[
                record(&boards[0], 2),
                record(&boards[1], 3),
                record(&boards[1], 4),
                record(&boards[2], 5),
                record(&boards[3], 6),
                record(&boards[4], 7),
                record(&boards[5], 8),
            ],
        );
        write_test_records(
            &source_validation,
            &boards[1..]
                .iter()
                .enumerate()
                .map(|(index, board)| record(board, index as i16))
                .collect::<Vec<_>>(),
        );
        let output = temp.path().join("frozen");
        freeze_corpus(
            std::slice::from_ref(&model_a),
            &source_train,
            &source_validation,
            &output,
            4,
            4,
            99,
        )
        .unwrap();

        assert_eq!(
            fs::metadata(output.join("training.bin")).unwrap().len(),
            4 * 72
        );
        assert_eq!(
            fs::metadata(output.join("validation.bin")).unwrap().len(),
            4 * 72
        );
        let model_a_families = read_families(&model_a);
        let train_families = read_families(&output.join("training.bin"));
        let validation_families = read_families(&output.join("validation.bin"));
        assert!(model_a_families.is_disjoint(&train_families));
        assert!(model_a_families.is_disjoint(&validation_families));
        assert!(train_families.is_disjoint(&validation_families));
        let manifest: Value =
            serde_json::from_slice(&fs::read(output.join("model-b-corpus.json")).unwrap()).unwrap();
        assert_eq!(manifest["passed"], true);
    }

    #[test]
    #[cfg(feature = "anhoku")]
    fn generates_unique_openings_outside_existing_suite() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("confirmation.tsv");
        let report = temp.path().join("confirmation.json");
        let existing = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("openings")
            .join("anhoku-v3.tsv");
        generate_opening_suite(&output, &report, 1234, 8, 16, "confirmation", &[existing]).unwrap();
        let text = fs::read_to_string(output).unwrap();
        assert_eq!(
            text.lines().filter(|line| !line.starts_with('#')).count(),
            8
        );
        let receipt: Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
        assert_eq!(receipt["positions"], 8);
        assert_eq!(receipt["colorSwapUnique"], true);
    }
}

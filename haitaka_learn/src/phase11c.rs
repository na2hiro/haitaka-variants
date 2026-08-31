use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, ensure};
use haitaka::{Board, Color, Move};
use haitaka_wasm::{
    NnueModel, SearchEvalMode, collapse_donor_receiver_pair_v2, donor_receiver_pair_v2_active_rows,
    donor_receiver_pair_v2_quantized_rows, search_board_impl_with_eval_mode,
    search_impl_with_eval_mode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::config::LoadedConfig;
use crate::dataset::{
    ENTRY_BYTES, PACKED_SFEN_BYTES, pack_board_for_training, unpack_board_from_training,
};
use crate::trainer::PreparedTrainer;

const ROWS: usize = 16_200;
const GROUPS: usize = 1_620;
const NATIVE_SLICES: usize = 10;
const FT_DIMS: usize = 512;
const PSQT_DIMS: usize = 8;
const FULL_DIMS: usize = FT_DIMS + PSQT_DIMS;
const SEARCH_POSITIONS: usize = 1_024;
const EXPECTED_SOURCE_ANCESTOR: &str = "8898e0297f5bbc0f57c32623a8d365f69d193d7b";
const EXPECTED_TRAINER_REVISION: &str = "61666d9e3653e4df9881b14c23f8fdcc4bf7779b";
const EXPECTED_V1_CKPT: &str = "442e2030620b21a6f3fdf2add33eae6039f1fd865d466f20a8c3ffe0e0360a39";
const EXPECTED_V2_CKPT: &str = "9d7997027791298b2d4de0a3e61acc571c48ec4c1895c222f0dc2fe292fc373b";
const EXPECTED_V1_NNUE: &str = "f7111caf885db66e528c56f23ffe9446609daf1f9a1b3a13cc1c2043b1a66632";
const EXPECTED_V2_NNUE: &str = "7e94100c24c495265fed01c06c4f9359f44aa52182c8481b46bf936f63c63a31";
const EXPECTED_TRAIN: &str = "aa2fc9decbb767d170c10a523ccefb9bb01ef3a39dc7d2e36606a34fb5e85599";
const EXPECTED_OOD: &str = "36e1360e75c81af311efca4497bc611e99fd6bb01fbad8cb2be8bac605bdb2e6";
const EXPECTED_TACTICAL: &str = "d0343f3583d16d996b5d3ef83eb5113a3cafebdfde0cff01d71d6ed09f41ab9d";
const EXPECTED_REVIEWED_PATCH: &str =
    "79603cc66250e335ba242477137366f0aa8a2e530ffa36f3abfb582fafaf802f";
const EXPECTED_APPLIED_DIFF: &str =
    "87f5a9a446bb929854dbf01b38db16980e4faee73a2f86044ae725f98ee0bc4b";
const EXPECTED_RESULTS_ARCHIVE: &str =
    "7a9c87571dc465f03e9146717b54a190d24dd3a2d0bbf5106418e49e4f43f3ba";
const EXPECTED_CLOSEOUT_ARCHIVE: &str =
    "547754e0ae0bce54fa1ea0296db4bc09c1a250d7de5ab5ff22106c6270298281";
const FP_MAGIC: &[u8; 16] = b"HTK11C-FP-V1\0\0\0\0";

pub struct Phase11cArgs {
    pub trainer_config: PathBuf,
    pub trainer_checkout: PathBuf,
    pub python: PathBuf,
    pub helper: PathBuf,
    pub reviewed_patch: PathBuf,
    pub applied_diff: PathBuf,
    pub v1_checkpoint: PathBuf,
    pub v2_checkpoint: PathBuf,
    pub v1_nnue: PathBuf,
    pub v2_nnue: PathBuf,
    pub train: PathBuf,
    pub ood: PathBuf,
    pub tactical_suite: PathBuf,
    pub batch_1024_games: PathBuf,
    pub batch_1024_report: PathBuf,
    pub batch_3072_games: PathBuf,
    pub batch_3072_report: PathBuf,
    pub results_archive: PathBuf,
    pub closeout_archive: PathBuf,
    pub output_dir: PathBuf,
}

pub struct Phase11cResult {
    pub classification: &'static str,
    pub report_path: PathBuf,
}

#[derive(Default)]
struct CoverageCounts {
    records: Vec<u64>,
    distinct: Vec<u64>,
    record_activations: u64,
    distinct_activations: u64,
}

struct SplitData {
    records: u64,
    boards: BTreeSet<[u8; PACKED_SFEN_BYTES]>,
    coverage: CoverageCounts,
}

struct FullPrecision {
    path_sha256: String,
    relation: Vec<f32>,
    original_ft: Vec<i16>,
    original_psqt: Vec<i32>,
    collapsed_ft: Vec<i16>,
    collapsed_psqt: Vec<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplayGame {
    schema: String,
    schema_version: u32,
    game_index: u32,
    pair_index: u32,
    start_sfen: String,
    moves: Vec<String>,
    failure_state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TacticalSuite {
    schema: String,
    schema_version: u32,
    ruleset: String,
    fixtures: Vec<TacticalFixture>,
}

#[derive(Debug, Deserialize)]
struct TacticalFixture {
    id: String,
    sfen: String,
    depth: u8,
    expected_bestmove: String,
    purpose: String,
}

pub fn run(args: Phase11cArgs) -> Result<Phase11cResult> {
    verify_source_ancestry()?;
    let identities = verify_frozen_inputs(&args)?;
    let audit_source_files = audit_source_identities()?;
    let artifacts = args.output_dir.join("artifacts");
    fs::create_dir_all(&artifacts).with_context(|| format!("create {}", artifacts.display()))?;

    let intermediate_path = artifacts.join("v2-full-precision-relation-v1.bin");
    run_checkpoint_helper(&args, &intermediate_path)?;
    let full = read_full_precision(&intermediate_path)?;

    let v1_bytes = fs::read(&args.v1_nnue)?;
    let v2_bytes = fs::read(&args.v2_nnue)?;
    let quantized = donor_receiver_pair_v2_quantized_rows(&v2_bytes)
        .map_err(|err| anyhow!("read V2 relation rows: {err}"))?;
    ensure!(
        quantized.transformer_dimensions == FT_DIMS,
        "V2 transformer dimensions changed"
    );
    ensure!(
        quantized.psqt_dimensions == PSQT_DIMS,
        "V2 PSQT dimensions changed"
    );
    ensure!(
        quantized.transformer == full.original_ft,
        "checkpoint/export transformer indexing or quantization mismatch"
    );
    ensure!(
        quantized.psqt == full.original_psqt,
        "checkpoint/export PSQT indexing or quantization mismatch"
    );

    let collapsed_bytes =
        collapse_donor_receiver_pair_v2(&v2_bytes, &full.collapsed_ft, &full.collapsed_psqt)
            .map_err(|err| anyhow!("collapse V2 network: {err}"))?;
    let collapsed_path = artifacts.join("collapsed-v2-audit-only.nnue");
    fs::write(&collapsed_path, &collapsed_bytes)?;
    let collapsed_hash = sha256_bytes(&collapsed_bytes);
    let collapsed_model = Arc::new(
        NnueModel::from_bytes(&collapsed_bytes)
            .map_err(|err| anyhow!("reload collapsed V2: {err}"))?,
    );
    ensure!(
        collapsed_model.feature_family_name() == "HalfKAv2^+DonorReceiverPairV2",
        "collapsed network family changed"
    );
    let v1_model =
        Arc::new(NnueModel::from_bytes(&v1_bytes).map_err(|err| anyhow!("load V1: {err}"))?);
    let v2_model =
        Arc::new(NnueModel::from_bytes(&v2_bytes).map_err(|err| anyhow!("load V2: {err}"))?);

    let train = scan_split(&args.train, 279_627, EXPECTED_TRAIN)?;
    let ood = scan_split(&args.ood, 3_218, EXPECTED_OOD)?;
    ensure!(
        train.boards.len() == 276_949,
        "train distinct-board count changed"
    );
    ensure!(
        ood.boards.len() == 3_215,
        "OOD distinct-board count changed"
    );
    let coverage = coverage_report(&train, &ood);
    let dispersion = dispersion_report(
        &full,
        &quantized.transformer,
        &quantized.psqt,
        &train.coverage.records,
    )?;
    let any_quantized_slice_difference = dispersion["survival"]["anyQuantizedSliceDifference"]
        .as_bool()
        .unwrap_or(false);

    let train_scores = compare_boards(&train.boards, &v1_model, &v2_model, &collapsed_model)?;
    let ood_scores = compare_boards(&ood.boards, &v1_model, &v2_model, &collapsed_model)?;

    let (replay_boards, replay_batches) = replay_batches(&args)?;
    let replay_scores =
        compare_replay_boards(&replay_boards, &v1_model, &v2_model, &collapsed_model);
    let selection = select_search_positions(&replay_boards)?;
    let selection_path = artifacts.join("replay-selection-1024.json");
    fs::write(&selection_path, serde_json::to_vec_pretty(&selection.0)?)?;
    let mut search = search_sensitivity(&selection.1, &v2_model, &collapsed_model)?;
    search["selectionHash"] = selection.0["selectionHash"].clone();
    search["selectionArtifact"] = json!(selection_path);
    let tactical = tactical_report(&args.tactical_suite, &v2_model, &collapsed_model)?;

    let any_score_difference = ["originalV2MinusCollapsedV2"].into_iter().any(|key| {
        train_scores[key]["zeroDeltaRate"].as_f64().unwrap_or(1.0) < 1.0
            || ood_scores[key]["zeroDeltaRate"].as_f64().unwrap_or(1.0) < 1.0
            || replay_scores[key]["zeroDeltaRate"].as_f64().unwrap_or(1.0) < 1.0
    });
    let any_search_difference = search["bestMoveDivergences"].as_u64().unwrap_or(0) != 0
        || search["bestScoreDivergences"].as_u64().unwrap_or(0) != 0;
    let classification = classify(
        true,
        any_quantized_slice_difference,
        any_score_difference,
        any_search_difference,
    );
    let next_route = match classification {
        "EXPRESSED_NOT_RETAINED" => {
            "Phase 12-A evaluation-error attribution launch gate is authorized; do not execute it in this assignment."
        }
        "QUANTIZATION_ERASED" => {
            "A new written learnability/quantization review is required; no later phase is authorized."
        }
        _ => "Stop for review; no later phase is authorized.",
    };

    let report = json!({
        "schema": "haitaka-anhoku-phase11c-audit",
        "schemaVersion": 1,
        "determinism": {"timingExcluded": true, "canonicalJson": "serde-json-pretty-v1"},
        "implementation": {"requiredAncestor": EXPECTED_SOURCE_ANCESTOR, "baseCommit": git_output(&["rev-parse", "HEAD"])?, "auditSourceFiles": audit_source_files},
        "inputs": identities,
        "fullPrecisionIntermediate": {"path": intermediate_path, "sha256": full.path_sha256, "rows": ROWS, "dimensionsPerRow": FULL_DIMS},
        "collapsedV2": {"path": collapsed_path, "sha256": collapsed_hash, "bytes": collapsed_bytes.len(), "auditOnly": true, "loadedFeatureFamily": collapsed_model.feature_family_name(), "otherParametersByteIdenticalByConstruction": true},
        "coverage": coverage,
        "sliceDispersion": dispersion,
        "evaluationDeltas": {"train": train_scores, "oodV2": ood_scores, "replay": replay_scores},
        "replay": replay_batches,
        "searchSensitivity": search,
        "tacticalSuite": tactical,
        "classificationEvidence": {
            "checkpointExportIndexingProven": true,
            "anyQuantizedSliceDifference": any_quantized_slice_difference,
            "anyOriginalCollapsedScoreDifference": any_score_difference,
            "anySearchDifference": any_search_difference
        },
        "classification": classification,
        "nextRoute": next_route
    });
    let report_path = artifacts.join("phase11c-audit.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    Ok(Phase11cResult {
        classification,
        report_path,
    })
}

fn audit_source_identities() -> Result<Value> {
    let root = PathBuf::from(git_output(&["rev-parse", "--show-toplevel"])?);
    let relative_paths = [
        "haitaka/src/board/parse.rs",
        "haitaka_learn/src/dataset.rs",
        "haitaka_learn/src/main.rs",
        "haitaka_learn/src/phase11c.rs",
        "haitaka_learn/src/trainer.rs",
        "haitaka_wasm/src/lib.rs",
        "haitaka_wasm/src/nnue.rs",
        "scripts/phase11c-extract-checkpoint.py",
    ];
    let mut files = serde_json::Map::new();
    for relative in relative_paths {
        files.insert(relative.to_string(), artifact_json(&root.join(relative))?);
    }
    Ok(Value::Object(files))
}

fn verify_source_ancestry() -> Result<()> {
    let status = Command::new("git")
        .args([
            "merge-base",
            "--is-ancestor",
            EXPECTED_SOURCE_ANCESTOR,
            "HEAD",
        ])
        .status()?;
    ensure!(
        status.success(),
        "current source does not contain frozen Phase 11-B implementation {EXPECTED_SOURCE_ANCESTOR}"
    );
    Ok(())
}

fn verify_frozen_inputs(args: &Phase11cArgs) -> Result<Value> {
    let files = [
        ("v1Checkpoint", &args.v1_checkpoint, EXPECTED_V1_CKPT),
        ("v2Checkpoint", &args.v2_checkpoint, EXPECTED_V2_CKPT),
        ("v1Nnue", &args.v1_nnue, EXPECTED_V1_NNUE),
        ("v2Nnue", &args.v2_nnue, EXPECTED_V2_NNUE),
        ("trainCorpus", &args.train, EXPECTED_TRAIN),
        ("oodV2Corpus", &args.ood, EXPECTED_OOD),
        ("tacticalSuite", &args.tactical_suite, EXPECTED_TACTICAL),
        (
            "reviewedTrainerPatch",
            &args.reviewed_patch,
            EXPECTED_REVIEWED_PATCH,
        ),
        (
            "appliedTrainerDiff",
            &args.applied_diff,
            EXPECTED_APPLIED_DIFF,
        ),
        (
            "trainingArchive",
            &args.results_archive,
            EXPECTED_RESULTS_ARCHIVE,
        ),
        (
            "closeoutArchive",
            &args.closeout_archive,
            EXPECTED_CLOSEOUT_ARCHIVE,
        ),
    ];
    let mut output = serde_json::Map::new();
    for (name, path, expected) in files {
        let actual = sha256_file(path)?;
        ensure!(
            actual == expected,
            "{name} hash mismatch: expected {expected}, got {actual}"
        );
        output.insert(
            name.to_string(),
            json!({"path": path, "sha256": actual, "bytes": fs::metadata(path)?.len()}),
        );
    }
    output.insert("trainerRevision".into(), json!(EXPECTED_TRAINER_REVISION));
    output.insert(
        "batch1024Games".into(),
        artifact_json(&args.batch_1024_games)?,
    );
    output.insert(
        "batch1024Report".into(),
        artifact_json(&args.batch_1024_report)?,
    );
    output.insert(
        "batch3072Games".into(),
        artifact_json(&args.batch_3072_games)?,
    );
    output.insert(
        "batch3072Report".into(),
        artifact_json(&args.batch_3072_report)?,
    );
    Ok(Value::Object(output))
}

fn run_checkpoint_helper(args: &Phase11cArgs, output: &Path) -> Result<()> {
    let mut loaded = LoadedConfig::from_path(&args.trainer_config)?;
    loaded.config.paths.trainer_checkout = Some(args.trainer_checkout.clone());
    loaded.config.paths.python = args.python.display().to_string();
    let _guard = PreparedTrainer::new_without_build(&loaded, &args.trainer_checkout)?;
    let status = Command::new(&args.python)
        .arg(&args.helper)
        .arg("--checkpoint")
        .arg(&args.v2_checkpoint)
        .arg("--trainer-checkout")
        .arg(&args.trainer_checkout)
        .arg("--reviewed-patch")
        .arg(&args.reviewed_patch)
        .arg("--applied-diff")
        .arg(&args.applied_diff)
        .arg("--output")
        .arg(output)
        .current_dir(&args.trainer_checkout)
        .status()
        .context("run Phase 11-C checkpoint helper")?;
    ensure!(status.success(), "Phase 11-C checkpoint helper failed");
    Ok(())
}

fn read_full_precision(path: &Path) -> Result<FullPrecision> {
    let bytes = fs::read(path)?;
    let mut cursor = 0usize;
    let take = |cursor: &mut usize, count: usize| -> Result<&[u8]> {
        let end = cursor
            .checked_add(count)
            .ok_or_else(|| anyhow!("intermediate length overflow"))?;
        ensure!(end <= bytes.len(), "truncated full-precision intermediate");
        let value = &bytes[*cursor..end];
        *cursor = end;
        Ok(value)
    };
    ensure!(
        take(&mut cursor, 16)? == FP_MAGIC,
        "wrong full-precision intermediate magic"
    );
    for expected in [ROWS, FT_DIMS, PSQT_DIMS, NATIVE_SLICES] {
        let raw = take(&mut cursor, 4)?;
        ensure!(
            u32::from_le_bytes(raw.try_into().unwrap()) as usize == expected,
            "full-precision intermediate geometry mismatch"
        );
    }
    ensure!(
        take(&mut cursor, 32)? == hex_bytes(EXPECTED_V2_CKPT)?.as_slice(),
        "full-precision checkpoint identity mismatch"
    );
    let relation = take(&mut cursor, ROWS * FULL_DIMS * 4)?
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect();
    let original_ft = take(&mut cursor, ROWS * FT_DIMS * 2)?
        .chunks_exact(2)
        .map(|v| i16::from_le_bytes(v.try_into().unwrap()))
        .collect();
    let original_psqt = take(&mut cursor, ROWS * PSQT_DIMS * 4)?
        .chunks_exact(4)
        .map(|v| i32::from_le_bytes(v.try_into().unwrap()))
        .collect();
    let collapsed_ft = take(&mut cursor, GROUPS * FT_DIMS * 2)?
        .chunks_exact(2)
        .map(|v| i16::from_le_bytes(v.try_into().unwrap()))
        .collect();
    let collapsed_psqt = take(&mut cursor, GROUPS * PSQT_DIMS * 4)?
        .chunks_exact(4)
        .map(|v| i32::from_le_bytes(v.try_into().unwrap()))
        .collect();
    ensure!(
        cursor == bytes.len(),
        "overlong full-precision intermediate"
    );
    Ok(FullPrecision {
        path_sha256: sha256_bytes(&bytes),
        relation,
        original_ft,
        original_psqt,
        collapsed_ft,
        collapsed_psqt,
    })
}

fn scan_split(path: &Path, expected_records: u64, expected_hash: &str) -> Result<SplitData> {
    ensure!(
        sha256_file(path)? == expected_hash,
        "dataset changed during audit"
    );
    ensure!(
        fs::metadata(path)?.len() == expected_records * ENTRY_BYTES as u64,
        "dataset length mismatch"
    );
    let mut reader = BufReader::new(File::open(path)?);
    let mut record = [0u8; ENTRY_BYTES];
    let mut boards = BTreeSet::new();
    let mut coverage = CoverageCounts {
        records: vec![0; ROWS],
        distinct: vec![0; ROWS],
        ..CoverageCounts::default()
    };
    for _ in 0..expected_records {
        reader.read_exact(&mut record)?;
        let packed: [u8; PACKED_SFEN_BYTES] = record[..PACKED_SFEN_BYTES].try_into().unwrap();
        let board = unpack_board_from_training(&packed)?;
        add_coverage(
            &board,
            &mut coverage.records,
            &mut coverage.record_activations,
        );
        if boards.insert(packed) {
            add_coverage(
                &board,
                &mut coverage.distinct,
                &mut coverage.distinct_activations,
            );
        }
    }
    let mut trailing = [0u8; 1];
    ensure!(
        reader.read(&mut trailing)? == 0,
        "dataset grew during audit"
    );
    Ok(SplitData {
        records: expected_records,
        boards,
        coverage,
    })
}

fn add_coverage(board: &Board, counts: &mut [u64], activations: &mut u64) {
    for perspective in [Color::Black, Color::White] {
        for row in donor_receiver_pair_v2_active_rows(board, perspective) {
            counts[row.index] += 1;
            *activations += 1;
        }
    }
}

fn coverage_report(train: &SplitData, ood: &SplitData) -> Value {
    let train_set: BTreeSet<_> = train
        .coverage
        .records
        .iter()
        .enumerate()
        .filter_map(|(i, &n)| (n != 0).then_some(i))
        .collect();
    let ood_set: BTreeSet<_> = ood
        .coverage
        .records
        .iter()
        .enumerate()
        .filter_map(|(i, &n)| (n != 0).then_some(i))
        .collect();
    let impossible: BTreeSet<_> = (0..ROWS)
        .filter(|&row| structurally_unreachable(row))
        .collect();
    let reachable_unseen_train = (0..ROWS)
        .filter(|row| !impossible.contains(row) && !train_set.contains(row))
        .count();
    let train_rare8: u64 = train
        .coverage
        .records
        .iter()
        .filter(|&&count| count < 8)
        .sum();
    let train_rare32: u64 = train
        .coverage
        .records
        .iter()
        .filter(|&&count| count < 32)
        .sum();
    let combined_rare8: u64 = train
        .coverage
        .records
        .iter()
        .zip(&ood.coverage.records)
        .filter(|(train_count, _)| **train_count < 8)
        .map(|(train_count, ood_count)| *train_count + *ood_count)
        .sum();
    let combined_rare32: u64 = train
        .coverage
        .records
        .iter()
        .zip(&ood.coverage.records)
        .filter(|(train_count, _)| **train_count < 32)
        .map(|(train_count, ood_count)| *train_count + *ood_count)
        .sum();
    let combined_activations = train.coverage.record_activations + ood.coverage.record_activations;
    json!({
        "relationRows": ROWS,
        "structurallyUnreachableRows": impossible.len(),
        "structurallyUnreachableBreakdown": {
            "boundaryOnly": 1_782,
            "sameColorKingReceiverAndKingDonorOnly": 144,
            "boundaryAndKingKingOverlap": 18,
            "definition": "relative-color 0 cannot receive on oriented rank 8; relative-color 1 cannot receive on oriented rank 0; one color cannot occupy both receiver and donor squares with its sole king"
        },
        "reachableRows": ROWS - impossible.len(),
        "reachableButUnseenTrainRows": reachable_unseen_train,
        "train": split_coverage_json(train),
        "oodV2": split_coverage_json(ood),
        "rowsUniqueToTrain": train_set.difference(&ood_set).count(),
        "rowsUniqueToOodV2": ood_set.difference(&train_set).count(),
        "rowsObservedInBoth": train_set.intersection(&ood_set).count(),
        "observedActivationsInRowsWithFewerThan8TrainOccurrences": {
            "trainCount": train_rare8,
            "trainPercent": percent(train_rare8, train.coverage.record_activations),
            "allSplitCount": combined_rare8,
            "allSplitPercent": percent(combined_rare8, combined_activations)
        },
        "observedActivationsInRowsWithFewerThan32TrainOccurrences": {
            "trainCount": train_rare32,
            "trainPercent": percent(train_rare32, train.coverage.record_activations),
            "allSplitCount": combined_rare32,
            "allSplitPercent": percent(combined_rare32, combined_activations)
        }
    })
}

fn split_coverage_json(split: &SplitData) -> Value {
    json!({
        "records": split.records,
        "distinctPackedBoards": split.boards.len(),
        "recordOccurrence": coverage_domain_json(&split.coverage.records, split.coverage.record_activations),
        "distinctBoardOccurrence": coverage_domain_json(&split.coverage.distinct, split.coverage.distinct_activations),
        "groupedRecordOccurrence": grouped_coverage(&split.coverage.records),
        "groupedDistinctBoardOccurrence": grouped_coverage(&split.coverage.distinct)
    })
}

fn coverage_domain_json(counts: &[u64], activations: u64) -> Value {
    json!({
        "observedRows": counts.iter().filter(|&&n| n != 0).count(),
        "coveragePercent": percent(counts.iter().filter(|&&n| n != 0).count() as u64, ROWS as u64),
        "activations": activations,
        "countHistogram": count_histogram(counts)
    })
}

fn count_histogram(counts: &[u64]) -> Value {
    let mut bins = [0u64; 6];
    for &count in counts {
        bins[match count {
            0 => 0,
            1 => 1,
            2..=7 => 2,
            8..=31 => 3,
            32..=127 => 4,
            _ => 5,
        }] += 1;
    }
    json!({"0": bins[0], "1": bins[1], "2-7": bins[2], "8-31": bins[3], "32-127": bins[4], "128+": bins[5]})
}

fn grouped_coverage(counts: &[u64]) -> Value {
    let dimensions = [
        ("orientedReceiverSquare", 81usize),
        ("relativeDonorColor", 2),
        ("receiverNativeType", 10),
        ("effectiveDonorType", 10),
    ];
    let mut result = serde_json::Map::new();
    for (name, size) in dimensions {
        let mut rows = Vec::new();
        for value in 0..size {
            let selected: Vec<u64> = counts
                .iter()
                .enumerate()
                .filter_map(|(row, &count)| (row_component(row, name) == value).then_some(count))
                .collect();
            rows.push(json!({"value": value, "rows": selected.len(), "observedRows": selected.iter().filter(|&&n| n != 0).count(), "activations": selected.iter().sum::<u64>(), "countHistogram": count_histogram(&selected)}));
        }
        result.insert(name.to_string(), Value::Array(rows));
    }
    Value::Object(result)
}

fn row_component(row: usize, name: &str) -> usize {
    match name {
        "orientedReceiverSquare" => row % 81,
        "relativeDonorColor" => (row / 81) % 2,
        "receiverNativeType" => (row / 162) % 10,
        "effectiveDonorType" => row / 1620,
        _ => unreachable!(),
    }
}

fn structurally_unreachable(row: usize) -> bool {
    let square = row % 81;
    let relative = (row / 81) % 2;
    let receiver = (row / 162) % 10;
    let effective = row / 1620;
    let rank = square / 9;
    (relative == 0 && rank == 8)
        || (relative == 1 && rank == 0)
        || (receiver == 9 && effective == 9)
}

fn dispersion_report(
    full: &FullPrecision,
    quant_ft: &[i16],
    quant_psqt: &[i32],
    train_counts: &[u64],
) -> Result<Value> {
    let mut groups = Vec::with_capacity(GROUPS);
    let mut any_quantized = false;
    let mut fp_survivable = 0u64;
    let mut fp_survived = 0u64;
    let mut unweighted_ft_rms = Vec::with_capacity(GROUPS);
    let mut unweighted_psqt_rms = Vec::with_capacity(GROUPS);
    let mut group_weights = Vec::with_capacity(GROUPS);
    for effective in 0..10 {
        for relative in 0..2 {
            for square in 0..81 {
                let rows: Vec<usize> = (0..10)
                    .map(|native| square + relative * 81 + native * 162 + effective * 1620)
                    .collect();
                let fp_ft = dimension_ranges_f32(&full.relation, &rows, 0, FT_DIMS, FULL_DIMS);
                let fp_psqt =
                    dimension_ranges_f32(&full.relation, &rows, FT_DIMS, PSQT_DIMS, FULL_DIMS);
                let q_ft = dimension_ranges_i16(quant_ft, &rows, FT_DIMS);
                let q_psqt = dimension_ranges_i32(quant_psqt, &rows, PSQT_DIMS);
                any_quantized |= q_ft.iter().any(|&v| v != 0) || q_psqt.iter().any(|&v| v != 0);
                let (possible_ft, survived_ft) = pairwise_survival_f32_i16(
                    &full.relation,
                    quant_ft,
                    &rows,
                    0,
                    FT_DIMS,
                    FULL_DIMS,
                );
                let (possible_psqt, survived_psqt) = pairwise_survival_f32_i32(
                    &full.relation,
                    quant_psqt,
                    &rows,
                    FT_DIMS,
                    PSQT_DIMS,
                    FULL_DIMS,
                );
                fp_survivable += possible_ft + possible_psqt;
                fp_survived += survived_ft + survived_psqt;
                let occurrences = rows.iter().map(|&row| train_counts[row]).sum::<u64>();
                unweighted_ft_rms.push(rms_f64(&fp_ft));
                unweighted_psqt_rms.push(rms_f64(&fp_psqt));
                group_weights.push(occurrences);
                groups.push(json!({
                    "orientedReceiverSquare": square,
                    "relativeDonorColor": relative,
                    "effectiveDonorType": effective,
                    "trainOccurrences": occurrences,
                    "fullPrecision": {"transformer": numeric_summary_f64(&fp_ft), "psqt": numeric_summary_f64(&fp_psqt)},
                    "quantizedNativeUnits": {"transformer": numeric_summary_i64(&q_ft), "psqt": numeric_summary_i64(&q_psqt)},
                    "survival": {
                        "fullPrecisionNonzeroPairDifferences": possible_ft + possible_psqt,
                        "nonzeroAfterQuantization": survived_ft + survived_psqt,
                        "ratio": ratio(survived_ft + survived_psqt, possible_ft + possible_psqt)
                    }
                }));
            }
        }
    }
    Ok(json!({
        "definition": "per-dimension max-minus-min across ten receiver-native slices",
        "groups": groups,
        "summaries": {
            "unweighted": {
                "transformerGroupRms": numeric_summary_f64(&unweighted_ft_rms),
                "psqtGroupRms": numeric_summary_f64(&unweighted_psqt_rms)
            },
            "trainOccurrenceWeighted": {
                "transformerGroupRms": weighted_summary(&unweighted_ft_rms, &group_weights),
                "psqtGroupRms": weighted_summary(&unweighted_psqt_rms, &group_weights)
            },
            "zeroCoverageGroupsRetained": group_weights.iter().filter(|&&weight| weight == 0).count()
        },
        "survival": {
            "fullPrecisionNonzeroPairDifferences": fp_survivable,
            "nonzeroAfterQuantization": fp_survived,
            "ratio": ratio(fp_survived, fp_survivable),
            "anyQuantizedSliceDifference": any_quantized
        }
    }))
}

fn dimension_ranges_f32(
    values: &[f32],
    rows: &[usize],
    dim_start: usize,
    dims: usize,
    stride: usize,
) -> Vec<f64> {
    (0..dims)
        .map(|dim| {
            let mut min = f32::INFINITY;
            let mut max = f32::NEG_INFINITY;
            for &row in rows {
                let value = values[row * stride + dim_start + dim];
                min = min.min(value);
                max = max.max(value);
            }
            f64::from(max - min)
        })
        .collect()
}

fn dimension_ranges_i16(values: &[i16], rows: &[usize], dims: usize) -> Vec<i64> {
    (0..dims)
        .map(|dim| {
            let mut min = i16::MAX;
            let mut max = i16::MIN;
            for &row in rows {
                let v = values[row * dims + dim];
                min = min.min(v);
                max = max.max(v);
            }
            i64::from(max) - i64::from(min)
        })
        .collect()
}

fn dimension_ranges_i32(values: &[i32], rows: &[usize], dims: usize) -> Vec<i64> {
    (0..dims)
        .map(|dim| {
            let mut min = i32::MAX;
            let mut max = i32::MIN;
            for &row in rows {
                let v = values[row * dims + dim];
                min = min.min(v);
                max = max.max(v);
            }
            i64::from(max) - i64::from(min)
        })
        .collect()
}

fn pairwise_survival_f32_i16(
    fp: &[f32],
    q: &[i16],
    rows: &[usize],
    start: usize,
    dims: usize,
    fp_stride: usize,
) -> (u64, u64) {
    let mut possible = 0;
    let mut survived = 0;
    for a in 0..rows.len() {
        for b in a + 1..rows.len() {
            for d in 0..dims {
                if fp[rows[a] * fp_stride + start + d] != fp[rows[b] * fp_stride + start + d] {
                    possible += 1;
                    survived += u64::from(q[rows[a] * dims + d] != q[rows[b] * dims + d]);
                }
            }
        }
    }
    (possible, survived)
}

fn pairwise_survival_f32_i32(
    fp: &[f32],
    q: &[i32],
    rows: &[usize],
    start: usize,
    dims: usize,
    fp_stride: usize,
) -> (u64, u64) {
    let mut possible = 0;
    let mut survived = 0;
    for a in 0..rows.len() {
        for b in a + 1..rows.len() {
            for d in 0..dims {
                if fp[rows[a] * fp_stride + start + d] != fp[rows[b] * fp_stride + start + d] {
                    possible += 1;
                    survived += u64::from(q[rows[a] * dims + d] != q[rows[b] * dims + d]);
                }
            }
        }
    }
    (possible, survived)
}

fn compare_boards(
    boards: &BTreeSet<[u8; PACKED_SFEN_BYTES]>,
    v1: &NnueModel,
    v2: &NnueModel,
    collapsed: &NnueModel,
) -> Result<Value> {
    let mut v2_collapsed = Vec::with_capacity(boards.len());
    let mut v1_v2 = Vec::with_capacity(boards.len());
    for packed in boards {
        let board = unpack_board_from_training(packed)?;
        let v1_score = v1.evaluate(&board);
        let v2_score = v2.evaluate(&board);
        let collapsed_score = collapsed.evaluate(&board);
        v2_collapsed.push(v2_score - collapsed_score);
        v1_v2.push(v1_score - v2_score);
    }
    Ok(
        json!({"positions": boards.len(), "originalV2MinusCollapsedV2": delta_summary(&v2_collapsed), "v1MinusV2": delta_summary(&v1_v2)}),
    )
}

fn compare_replay_boards(
    boards: &BTreeMap<[u8; PACKED_SFEN_BYTES], Board>,
    v1: &NnueModel,
    v2: &NnueModel,
    collapsed: &NnueModel,
) -> Value {
    let mut v2_collapsed = Vec::with_capacity(boards.len());
    let mut v1_v2 = Vec::with_capacity(boards.len());
    for board in boards.values() {
        let v1_score = v1.evaluate(board);
        let v2_score = v2.evaluate(board);
        let collapsed_score = collapsed.evaluate(board);
        v2_collapsed.push(v2_score - collapsed_score);
        v1_v2.push(v1_score - v2_score);
    }
    json!({"positions": boards.len(), "originalV2MinusCollapsedV2": delta_summary(&v2_collapsed), "v1MinusV2": delta_summary(&v1_v2)})
}

fn replay_batches(
    args: &Phase11cArgs,
) -> Result<(BTreeMap<[u8; PACKED_SFEN_BYTES], Board>, Value)> {
    let first_report: Value = serde_json::from_slice(&fs::read(&args.batch_1024_report)?)?;
    let second_report: Value = serde_json::from_slice(&fs::read(&args.batch_3072_report)?)?;
    validate_replay_report(&first_report, 1_180, 1_024)?;
    validate_replay_report(&second_report, 12_180, 3_072)?;
    ensure!(
        sha256_file(&args.batch_1024_games)? != sha256_file(&args.batch_3072_games)?,
        "duplicate replay batch identities"
    );
    let mut boards = BTreeMap::new();
    let first = replay_one_batch(&args.batch_1024_games, 1024, &mut boards)?;
    let second = replay_one_batch(&args.batch_3072_games, 3072, &mut boards)?;
    ensure!(
        first + second == 4096,
        "final replay game count is not 4,096"
    );
    let distinct_legal_positions = boards.len();
    Ok((
        boards,
        json!({"games": first+second, "distinctLegalPositions": distinct_legal_positions, "batches": [{"games": first, "seed": 1180}, {"games": second, "seed": 12180}]}),
    ))
}

fn validate_replay_report(report: &Value, seed: u64, games: u64) -> Result<()> {
    ensure!(
        report["schema"] == "haitaka-self-play-report" && report["schemaVersion"] == 3,
        "replay report schema mismatch"
    );
    ensure!(
        report["ruleset"] == "anhoku",
        "replay report ruleset mismatch"
    );
    let command = &report["command"];
    ensure!(
        command["seed"] == seed && command["games"] == games,
        "replay report seed/game identity mismatch"
    );
    ensure!(
        command["movetimeMs"] == 100
            && command["openingRandomPlies"] == 4
            && command["maxPlies"] == 200,
        "replay report protocol mismatch"
    );
    let engines = report["engines"]
        .as_array()
        .ok_or_else(|| anyhow!("replay report engines missing"))?;
    ensure!(engines.len() == 2, "replay report must contain two engines");
    ensure!(
        engines[0]["label"] == "A" && engines[0]["nnueSha256"] == EXPECTED_V2_NNUE,
        "replay engine A identity mismatch"
    );
    ensure!(
        engines[1]["label"] == "B" && engines[1]["nnueSha256"] == EXPECTED_V1_NNUE,
        "replay engine B identity mismatch"
    );
    Ok(())
}

fn replay_one_batch(
    path: &Path,
    expected: u32,
    boards: &mut BTreeMap<[u8; PACKED_SFEN_BYTES], Board>,
) -> Result<u32> {
    let mut games = Vec::new();
    for (line_index, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        let game: ReplayGame = serde_json::from_str(&line).with_context(|| {
            format!(
                "malformed replay JSON {}:{}",
                path.display(),
                line_index + 1
            )
        })?;
        ensure!(
            game.schema == "haitaka-self-play-game" && game.schema_version == 2,
            "wrong replay schema"
        );
        ensure!(
            game.failure_state.is_none(),
            "replay game {} records failure",
            game.game_index
        );
        ensure!(
            game.pair_index == (game.game_index - 1) / 2,
            "replay pair index mismatch"
        );
        games.push(game);
    }
    ensure!(
        games.len() == expected as usize,
        "{} has {} games, expected {expected}",
        path.display(),
        games.len()
    );
    games.sort_by_key(|game| game.game_index);
    for (offset, game) in games.iter().enumerate() {
        ensure!(
            game.game_index == offset as u32 + 1,
            "duplicate or missing game index in {}",
            path.display()
        );
    }
    for game in games {
        let mut board = Board::from_sfen(&game.start_sfen)
            .map_err(|err| anyhow!("invalid replay start SFEN game {}: {err}", game.game_index))?;
        boards
            .entry(pack_board_for_training(&board)?)
            .or_insert_with(|| board.clone());
        for move_text in game.moves {
            let mv = Move::from_str(&move_text).map_err(|err| {
                anyhow!(
                    "malformed move {move_text} in game {}: {err}",
                    game.game_index
                )
            })?;
            board
                .try_play(mv)
                .map_err(|_| anyhow!("illegal move {move_text} in game {}", game.game_index))?;
            boards
                .entry(pack_board_for_training(&board)?)
                .or_insert_with(|| board.clone());
        }
    }
    Ok(expected)
}

fn select_search_positions(
    boards: &BTreeMap<[u8; PACKED_SFEN_BYTES], Board>,
) -> Result<(Value, Vec<Board>)> {
    ensure!(
        boards.len() >= SEARCH_POSITIONS,
        "not enough distinct replay positions"
    );
    let mut ranked: Vec<_> = boards
        .keys()
        .map(|packed| (Sha256::digest(packed).to_vec(), *packed))
        .collect();
    ranked.sort();
    ranked.truncate(SEARCH_POSITIONS);
    let mut concatenated = Sha256::new();
    let mut entries = Vec::with_capacity(SEARCH_POSITIONS);
    let mut selected = Vec::with_capacity(SEARCH_POSITIONS);
    for (digest, packed) in ranked {
        concatenated.update(packed);
        let board = boards
            .get(&packed)
            .expect("selected replay board exists")
            .clone();
        entries.push(json!({"boardSha256": hex(&digest), "packedBoardHex": hex(&packed), "sfen": board.to_string()}));
        selected.push(board);
    }
    Ok((
        json!({"schema":"haitaka-anhoku-phase11c-replay-selection", "schemaVersion":1, "selectionPolicy":"smallest-sha256-of-canonical-packed-board-v1", "positions":SEARCH_POSITIONS, "selectionHash":format!("{:x}", concatenated.finalize()), "entries":entries}),
        selected,
    ))
}

fn search_sensitivity(
    boards: &[Board],
    v2: &Arc<NnueModel>,
    collapsed: &Arc<NnueModel>,
) -> Result<Value> {
    let mut move_divergences = 0u64;
    let mut score_divergences = 0u64;
    let mut examples = Vec::new();
    for board in boards {
        let sfen = board.to_string();
        let original =
            search_board_impl_with_eval_mode(board, 2, v2.clone(), SearchEvalMode::Incremental)
                .map_err(|err| anyhow!("original V2 depth-2 search failed: {err}"))?;
        let counterfactual = search_board_impl_with_eval_mode(
            board,
            2,
            collapsed.clone(),
            SearchEvalMode::Incremental,
        )
        .map_err(|err| anyhow!("collapsed V2 depth-2 search failed: {err}"))?;
        let move_diff = original.best_move != counterfactual.best_move;
        let score_diff = original.best_score != counterfactual.best_score;
        move_divergences += u64::from(move_diff);
        score_divergences += u64::from(score_diff);
        if (move_diff || score_diff) && examples.len() < 32 {
            examples.push(json!({"sfen":sfen,"originalBestMove":original.best_move,"collapsedBestMove":counterfactual.best_move,"originalBestScore":original.best_score,"collapsedBestScore":counterfactual.best_score}));
        }
    }
    Ok(
        json!({"positions":boards.len(),"depth":2,"bestMoveDivergences":move_divergences,"bestScoreDivergences":score_divergences,"examples":examples,"notElo":true,"notASelector":true}),
    )
}

fn tactical_report(path: &Path, v2: &Arc<NnueModel>, collapsed: &Arc<NnueModel>) -> Result<Value> {
    let suite: TacticalSuite = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        suite.ruleset == "anhoku" && suite.fixtures.len() == 6,
        "frozen tactical suite identity mismatch"
    );
    let mut rows = Vec::new();
    let mut passed = true;
    for fixture in suite.fixtures {
        let original = search_impl_with_eval_mode(
            &fixture.sfen,
            fixture.depth,
            v2.clone(),
            SearchEvalMode::Incremental,
        )
        .map_err(|err| anyhow!("tactical original {}: {err}", fixture.id))?;
        let collapsed_result = search_impl_with_eval_mode(
            &fixture.sfen,
            fixture.depth,
            collapsed.clone(),
            SearchEvalMode::Incremental,
        )
        .map_err(|err| anyhow!("tactical collapsed {}: {err}", fixture.id))?;
        let row_passed = original.best_move.as_deref() == Some(&fixture.expected_bestmove)
            && collapsed_result.best_move.as_deref() == Some(&fixture.expected_bestmove);
        passed &= row_passed;
        rows.push(json!({"id":fixture.id,"purpose":fixture.purpose,"expectedBestmove":fixture.expected_bestmove,"originalBestmove":original.best_move,"collapsedBestmove":collapsed_result.best_move,"originalScore":original.best_score,"collapsedScore":collapsed_result.best_score,"passed":row_passed}));
    }
    Ok(
        json!({"schema":suite.schema,"schemaVersion":suite.schema_version,"sha256":sha256_file(path)?,"fixtures":rows,"passed":passed,"regressionOnly":true}),
    )
}

fn classify(
    evidence_complete: bool,
    quantized_difference: bool,
    score_difference: bool,
    search_difference: bool,
) -> &'static str {
    if !evidence_complete {
        "AUDIT_INCONCLUSIVE"
    } else if quantized_difference || score_difference || search_difference {
        "EXPRESSED_NOT_RETAINED"
    } else {
        "QUANTIZATION_ERASED"
    }
}

fn delta_summary(values: &[i32]) -> Value {
    let floats: Vec<f64> = values.iter().map(|&v| f64::from(v)).collect();
    let absolute: Vec<f64> = values.iter().map(|&v| f64::from(v.abs())).collect();
    let zeros = values.iter().filter(|&&v| v == 0).count() as u64;
    let mut signed = [0u64; 9];
    let mut abs_bins = [0u64; 6];
    for &value in values {
        signed[match value {
            i32::MIN..=-100 => 0,
            -99..=-32 => 1,
            -31..=-8 => 2,
            -7..=-1 => 3,
            0 => 4,
            1..=7 => 5,
            8..=31 => 6,
            32..=99 => 7,
            _ => 8,
        }] += 1;
        let a = value.abs();
        abs_bins[match a {
            0 => 0,
            1 => 1,
            2..=7 => 2,
            8..=31 => 3,
            32..=127 => 4,
            _ => 5,
        }] += 1;
    }
    json!({"count":values.len(),"zeroDeltaCount":zeros,"zeroDeltaRate":ratio(zeros,values.len() as u64),"signed":numeric_summary_f64(&floats),"absolute":numeric_summary_f64(&absolute),"signedHistogram":{"<=-100":signed[0],"-99--32":signed[1],"-31--8":signed[2],"-7--1":signed[3],"0":signed[4],"1-7":signed[5],"8-31":signed[6],"32-99":signed[7],"100+":signed[8]},"absoluteHistogram":{"0":abs_bins[0],"1":abs_bins[1],"2-7":abs_bins[2],"8-31":abs_bins[3],"32-127":abs_bins[4],"128+":abs_bins[5]}})
}

fn numeric_summary_f64(values: &[f64]) -> Value {
    if values.is_empty() {
        return json!({"count":0});
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let l1 = values.iter().map(|v| v.abs()).sum::<f64>();
    let l2 = values.iter().map(|v| v * v).sum::<f64>().sqrt();
    json!({"count":values.len(),"zeroCount":values.iter().filter(|&&v|v==0.0).count(),"nonzeroCount":values.iter().filter(|&&v|v!=0.0).count(),"mean":values.iter().sum::<f64>()/values.len() as f64,"l1":l1,"l2":l2,"rms":l2/(values.len() as f64).sqrt(),"min":sorted[0],"p50":quantile(&sorted,50),"p90":quantile(&sorted,90),"p95":quantile(&sorted,95),"p99":quantile(&sorted,99),"max":sorted[sorted.len()-1]})
}

fn numeric_summary_i64(values: &[i64]) -> Value {
    numeric_summary_f64(&values.iter().map(|&v| v as f64).collect::<Vec<_>>())
}
fn weighted_summary(values: &[f64], weights: &[u64]) -> Value {
    assert_eq!(values.len(), weights.len());
    let total_weight = weights.iter().sum::<u64>();
    let weighted_sum = values
        .iter()
        .zip(weights)
        .map(|(value, weight)| value * *weight as f64)
        .sum::<f64>();
    let weighted_square_sum = values
        .iter()
        .zip(weights)
        .map(|(value, weight)| value * value * *weight as f64)
        .sum::<f64>();
    let mut ordered: Vec<_> = values
        .iter()
        .copied()
        .zip(weights.iter().copied())
        .collect();
    ordered.sort_by(|a, b| a.0.total_cmp(&b.0));
    let weighted_quantile = |percent: u64| {
        if total_weight == 0 {
            return 0.0;
        }
        let target = (total_weight.saturating_sub(1) * percent + 50) / 100;
        let mut cumulative = 0u64;
        for &(value, weight) in &ordered {
            cumulative += weight;
            if cumulative > target {
                return value;
            }
        }
        ordered.last().map_or(0.0, |entry| entry.0)
    };
    json!({
        "groups": values.len(),
        "positiveWeightGroups": weights.iter().filter(|&&weight| weight != 0).count(),
        "zeroCoverageGroupsRetained": weights.iter().filter(|&&weight| weight == 0).count(),
        "totalOccurrenceWeight": total_weight,
        "mean": if total_weight == 0 { 0.0 } else { weighted_sum / total_weight as f64 },
        "rms": if total_weight == 0 { 0.0 } else { (weighted_square_sum / total_weight as f64).sqrt() },
        "p50": weighted_quantile(50),
        "p90": weighted_quantile(90),
        "p95": weighted_quantile(95),
        "p99": weighted_quantile(99),
        "max": values.iter().copied().max_by(f64::total_cmp).unwrap_or(0.0)
    })
}
fn rms_f64(values: &[f64]) -> f64 {
    (values.iter().map(|v| v * v).sum::<f64>() / values.len() as f64).sqrt()
}
fn quantile(sorted: &[f64], percent: usize) -> f64 {
    sorted[((sorted.len() - 1) * percent + 50) / 100]
}
fn percent(numerator: u64, denominator: u64) -> f64 {
    ratio(numerator, denominator) * 100.0
}
fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}
fn artifact_json(path: &Path) -> Result<Value> {
    Ok(json!({"path":path,"sha256":sha256_file(path)?,"bytes":fs::metadata(path)?.len()}))
}
fn sha256_file(path: &Path) -> Result<String> {
    let mut reader =
        BufReader::new(File::open(path).with_context(|| format!("open {}", path.display()))?);
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}
fn hex_bytes(value: &str) -> Result<Vec<u8>> {
    ensure!(value.len().is_multiple_of(2), "odd hex length");
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(Into::into))
        .collect()
}
fn git_output(args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).output()?;
    ensure!(output.status.success(), "git command failed");
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_decoding_and_grouping_match_frozen_geometry() {
        let row = 40 + 81 + 3 * 162 + 5 * 1620;
        assert_eq!(row_component(row, "orientedReceiverSquare"), 40);
        assert_eq!(row_component(row, "relativeDonorColor"), 1);
        assert_eq!(row_component(row, "receiverNativeType"), 3);
        assert_eq!(row_component(row, "effectiveDonorType"), 5);
        let group = 40 + 81 + 5 * 162;
        assert_eq!(group, 931);
    }

    #[test]
    fn runtime_v2_activation_reports_both_perspective_rows() {
        let board = Board::startpos();
        let black = donor_receiver_pair_v2_active_rows(&board, Color::Black);
        let white = donor_receiver_pair_v2_active_rows(&board, Color::White);
        assert!(!black.is_empty());
        assert_eq!(black.len(), white.len());
        for row in black.iter().chain(&white) {
            assert_eq!(
                row.index,
                row.oriented_square
                    + row.relative_color * 81
                    + row.receiver_type * 162
                    + row.effective_type * 1_620
            );
        }
    }

    #[test]
    fn collapse_quantization_fixture_uses_ties_to_even() {
        // PyTorch production round: 0.5 -> 0, 1.5 -> 2.
        let values = [0.5f32, 1.5, -0.5, -1.5];
        let expected = [0f32, 2.0, -0.0, -2.0];
        for (value, expected) in values.into_iter().zip(expected) {
            assert_eq!(value.round_ties_even(), expected);
        }
    }

    #[test]
    fn hash_stable_selection_uses_digest_then_packed_bytes() {
        let a = [0u8; PACKED_SFEN_BYTES];
        let mut b = a;
        b[0] = 1;
        let mut first = vec![
            (Sha256::digest(a).to_vec(), a),
            (Sha256::digest(b).to_vec(), b),
        ];
        let mut second = first.clone();
        second.reverse();
        first.sort();
        second.sort();
        assert_eq!(first, second);
    }

    #[test]
    fn illegal_replay_is_rejected() {
        let mut board = Board::startpos();
        let mv = Move::from_str("5e5d").unwrap();
        assert!(board.try_play(mv).is_err());
    }

    #[test]
    fn all_three_route_states_are_exact() {
        assert_eq!(classify(true, false, false, false), "QUANTIZATION_ERASED");
        assert_eq!(classify(true, true, false, false), "EXPRESSED_NOT_RETAINED");
        assert_eq!(classify(false, true, true, true), "AUDIT_INCONCLUSIVE");
    }
}

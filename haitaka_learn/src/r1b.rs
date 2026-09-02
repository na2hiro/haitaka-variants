use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::ops::Range;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, ensure};
use haitaka::{Board, Move};
use haitaka_wasm::{NnueModel, R1InferenceTrace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const EXPECT_MAGIC: &[u8; 16] = b"HTK-R1B-EXP-V1\0\0";
const CHECKPOINT_MAGIC: &[u8; 16] = b"HTK-R1B-FP-V1\0\0\0";
const FT_DIMENSIONS: usize = 512;
const PSQT_BUCKETS: usize = 8;
const TRANSFORMED_DIMENSIONS: usize = 1024;
const HIDDEN1_DIMENSIONS: usize = 16;
const HIDDEN2_DIMENSIONS: usize = 32;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorpusRow {
    id: String,
    sfen: String,
    parent_id: Option<String>,
    move_from_parent: Option<String>,
    label_score_side_to_move: i16,
    output_bucket: usize,
}

#[derive(Debug)]
struct ExpectedTrace {
    full_score: f64,
    bucket: usize,
    black_accumulator: [i16; FT_DIMENSIONS],
    white_accumulator: [i16; FT_DIMENSIONS],
    black_psqt: [i32; PSQT_BUCKETS],
    white_psqt: [i32; PSQT_BUCKETS],
    transformed: [u8; TRANSFORMED_DIMENSIONS],
    hidden1: [i32; HIDDEN1_DIMENSIONS],
    hidden1_relu: [u8; HIDDEN1_DIMENSIONS],
    hidden2: [i32; HIDDEN2_DIMENSIONS],
    hidden2_relu: [u8; HIDDEN2_DIMENSIONS],
    output: i32,
    psqt: i32,
    score: i32,
}

#[derive(Debug, Deserialize, Serialize)]
struct QuantizationLimits {
    schema: String,
    splits: BTreeMap<String, String>,
    score_buckets: Vec<ScoreBucket>,
    limits: AbsoluteLimits,
}

#[derive(Debug, Deserialize, Serialize)]
struct ScoreBucket {
    id: String,
    minimum_inclusive: Option<i32>,
    maximum_inclusive: Option<i32>,
}

#[derive(Debug, Deserialize, Serialize)]
struct AbsoluteLimits {
    mean_absolute_score_delta: f64,
    p99_absolute_score_delta: f64,
    maximum_absolute_score_delta: f64,
    maximum_positive_loss_degradation: f64,
    serializer_clamped_weights: u64,
    accumulator_overflows: u64,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaseResult {
    positions: usize,
    transitions: usize,
    integer_score_mismatches: usize,
    integer_activation_mismatches: usize,
    full_incremental_accumulator_mismatches: usize,
    full_incremental_score_mismatches: usize,
    restored_parent_snapshot_mismatches: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GroupMetrics {
    count: usize,
    mean_absolute_score_delta: f64,
    p95_absolute_score_delta: f64,
    p99_absolute_score_delta: f64,
    maximum_absolute_score_delta: f64,
    full_precision_loss: f64,
    quantized_loss: f64,
    loss_degradation: f64,
    passed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactIdentity {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct R1bReport {
    schema: &'static str,
    schema_version: u32,
    ruleset: &'static str,
    feature_family: &'static str,
    corpus_positions: usize,
    deterministic_order: &'static str,
    python_integer_emulator: &'static str,
    full_precision_framework: &'static str,
    cases: BTreeMap<String, CaseResult>,
    quantization_by_group: BTreeMap<String, GroupMetrics>,
    activation_clamp_counts: serde_json::Value,
    serializer_clamped_weights: u64,
    accumulator_overflows: u64,
    artifacts: BTreeMap<String, ArtifactIdentity>,
    gates: BTreeMap<String, bool>,
    pub(crate) passed: bool,
}

pub(crate) fn run(
    r1a_dir: &Path,
    output_dir: &Path,
    limits_path: &Path,
    python: &Path,
    source_identity_path: &Path,
    workspace_root: &Path,
) -> Result<R1bReport> {
    let r1a_report_path = r1a_dir.join("r1a-gate-report.json");
    let r1a_report: serde_json::Value = read_json(&r1a_report_path)?;
    ensure!(
        r1a_report
            .get("passed")
            .and_then(serde_json::Value::as_bool)
            == Some(true),
        "R1-B requires a passing R1-A report"
    );
    let corpus_path = r1a_dir.join("parity-corpus.jsonl");
    let features_path = r1a_dir.join("rust-features.jsonl");
    let corpus = read_corpus(&corpus_path)?;
    ensure!(corpus.len() >= 10_000, "R1-B parity corpus is too small");
    for (index, row) in corpus.iter().enumerate() {
        ensure!(
            row.id == format!("r1a-{index:05}"),
            "unstable R1-B fixture order"
        );
        ensure!(
            row.output_bucket < 8,
            "invalid output bucket for {}",
            row.id
        );
    }
    let limits: QuantizationLimits = read_json(limits_path)?;
    ensure!(
        limits.schema == "haitaka-r1b-quantization-limits-v1",
        "wrong R1-B quantization-limit schema"
    );
    ensure!(limits.splits.len() == 2 && limits.score_buckets.len() == 5);

    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;
    let helper = workspace_root.join("scripts/r1b-parity-oracle.py");
    let status = Command::new(python)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .arg(&helper)
        .arg("--corpus")
        .arg(&corpus_path)
        .arg("--features")
        .arg(&features_path)
        .arg("--limits")
        .arg(limits_path)
        .arg("--output-dir")
        .arg(output_dir)
        .current_dir(workspace_root)
        .status()
        .with_context(|| format!("failed to run {}", helper.display()))?;
    ensure!(status.success(), "Python R1-B parity oracle failed");

    let metadata_path = output_dir.join("sentinel-network-metadata.json");
    let metadata: serde_json::Value = read_json(&metadata_path)?;
    ensure!(
        metadata.get("schema").and_then(serde_json::Value::as_str)
            == Some("haitaka-r1b-sentinel-network-v1")
    );
    let source_hash = metadata
        .get("generatorSourceSha256")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("sentinel metadata lacks generatorSourceSha256"))?;
    ensure!(source_hash == sha256_file(&helper)?);
    let checkpoint_a = output_dir.join("sentinel-checkpoint-a.r1fp");
    let checkpoint_b = output_dir.join("sentinel-checkpoint-b.r1fp");
    let checkpoint_metadata = validate_checkpoint(&checkpoint_a)?;
    ensure!(checkpoint_metadata == validate_checkpoint(&checkpoint_b)?);
    let checkpoint_a_identity = artifact_identity(&checkpoint_a)?;
    let checkpoint_b_identity = artifact_identity(&checkpoint_b)?;
    let checkpoint_repeat_identical = checkpoint_a_identity.bytes == checkpoint_b_identity.bytes
        && checkpoint_a_identity.sha256 == checkpoint_b_identity.sha256;
    let sentinel_a = output_dir.join("sentinel-a.nnue");
    let sentinel_b = output_dir.join("sentinel-b.nnue");
    let sentinel_a_identity = artifact_identity(&sentinel_a)?;
    let sentinel_b_identity = artifact_identity(&sentinel_b)?;
    let repeat_export_identical = sentinel_a_identity.bytes == sentinel_b_identity.bytes
        && sentinel_a_identity.sha256 == sentinel_b_identity.sha256;
    let export_metadata_a = output_dir.join("sentinel-export-metadata-a.json");
    let export_metadata_b = output_dir.join("sentinel-export-metadata-b.json");
    let repeat_metadata_identical = fs::read(&export_metadata_a)? == fs::read(&export_metadata_b)?;

    let mut cases = BTreeMap::new();
    let mut sentinel_full_scores = Vec::new();
    let mut sentinel_quantized_scores = Vec::new();
    for (name, network_path, expectations_path) in [
        (
            "zero",
            output_dir.join("zero.nnue"),
            output_dir.join("zero-expectations.bin"),
        ),
        (
            "bias-only",
            output_dir.join("bias-only.nnue"),
            output_dir.join("bias-only-expectations.bin"),
        ),
        (
            "sentinel",
            sentinel_a.clone(),
            output_dir.join("sentinel-expectations.bin"),
        ),
    ] {
        let capture_scores = name == "sentinel";
        let result = check_case(
            &network_path,
            &expectations_path,
            &corpus,
            if capture_scores {
                Some((&mut sentinel_full_scores, &mut sentinel_quantized_scores))
            } else {
                None
            },
        )?;
        cases.insert(name.to_string(), result);
    }
    ensure!(sentinel_full_scores.len() == corpus.len());

    let quantization_by_group = quantization_metrics(
        &corpus,
        &sentinel_full_scores,
        &sentinel_quantized_scores,
        &limits,
    )?;
    let serializer_clamped_weights = metadata
        .get("serializerClampedWeights")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow!("sentinel metadata lacks serializerClampedWeights"))?;
    let integer_audit = metadata
        .get("integerAudit")
        .ok_or_else(|| anyhow!("sentinel metadata lacks integerAudit"))?;
    let accumulator_overflows = integer_audit
        .as_object()
        .ok_or_else(|| anyhow!("integerAudit is not an object"))?
        .values()
        .map(|value| {
            value
                .get("accumulatorOverflows")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| anyhow!("integerAudit case lacks accumulatorOverflows"))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .sum();
    let activation_clamp_counts = integer_audit.clone();
    let serialized_patterns_verified = audit_serialized_patterns(output_dir, &checkpoint_metadata)?;
    let activation_boundaries_exercised = activation_boundaries_exercised(integer_audit);

    let all_exact = cases.values().all(|case| {
        case.integer_score_mismatches == 0
            && case.integer_activation_mismatches == 0
            && case.full_incremental_accumulator_mismatches == 0
            && case.full_incremental_score_mismatches == 0
            && case.restored_parent_snapshot_mismatches == 0
    });
    let quantization_limits_passed = quantization_by_group.values().all(|metrics| metrics.passed);
    let mut gates = BTreeMap::new();
    gates.insert(
        "checkpointRegenerationByteIdentical".to_string(),
        checkpoint_repeat_identical,
    );
    gates.insert(
        "repeatExportByteIdentical".to_string(),
        repeat_export_identical,
    );
    gates.insert(
        "repeatExportMetadataByteIdentical".to_string(),
        repeat_metadata_identical,
    );
    gates.insert("pythonIntegerEqualsRustFullRefresh".to_string(), all_exact);
    gates.insert(
        "rustFullRefreshEqualsIncremental".to_string(),
        cases.values().all(|case| {
            case.full_incremental_accumulator_mismatches == 0
                && case.full_incremental_score_mismatches == 0
                && case.restored_parent_snapshot_mismatches == 0
        }),
    );
    gates.insert(
        "absoluteQuantizationLimits".to_string(),
        quantization_limits_passed,
    );
    gates.insert(
        "noSerializerWeightClamping".to_string(),
        serializer_clamped_weights == limits.limits.serializer_clamped_weights,
    );
    gates.insert(
        "noAccumulatorOverflow".to_string(),
        accumulator_overflows == limits.limits.accumulator_overflows,
    );
    gates.insert(
        "requiredSentinelPatterns".to_string(),
        required_patterns_present(&metadata)
            && serialized_patterns_verified
            && activation_boundaries_exercised,
    );
    let passed = gates.values().all(|value| *value);

    let mut artifacts = BTreeMap::new();
    for (name, path) in [
        ("r1aReport", r1a_report_path.as_path()),
        ("corpus", corpus_path.as_path()),
        ("features", features_path.as_path()),
        ("frozenLimits", limits_path),
        ("pythonOracleSource", helper.as_path()),
        ("sentinelMetadata", metadata_path.as_path()),
        ("checkpoint", checkpoint_a.as_path()),
        ("checkpointRepeat", checkpoint_b.as_path()),
        ("sentinelNetwork", sentinel_a.as_path()),
        ("sentinelNetworkRepeat", sentinel_b.as_path()),
        ("zeroNetwork", output_dir.join("zero.nnue").as_path()),
        (
            "biasOnlyNetwork",
            output_dir.join("bias-only.nnue").as_path(),
        ),
        (
            "sentinelExpectations",
            output_dir.join("sentinel-expectations.bin").as_path(),
        ),
        (
            "zeroExpectations",
            output_dir.join("zero-expectations.bin").as_path(),
        ),
        (
            "biasOnlyExpectations",
            output_dir.join("bias-only-expectations.bin").as_path(),
        ),
        ("exportMetadata", export_metadata_a.as_path()),
        ("exportMetadataRepeat", export_metadata_b.as_path()),
    ] {
        artifacts.insert(name.to_string(), artifact_identity(path)?);
    }
    let executable = std::env::current_exe()?;
    artifacts.insert(
        "gateExecutable".to_string(),
        artifact_identity(&executable)?,
    );
    artifacts.insert(
        "gateSource".to_string(),
        artifact_identity(&workspace_root.join("haitaka_learn/src/r1b.rs"))?,
    );
    artifacts.insert(
        "runtimeSource".to_string(),
        artifact_identity(&workspace_root.join("haitaka_wasm/src/nnue.rs"))?,
    );
    artifacts.insert(
        "sourceIdentity".to_string(),
        artifact_identity(source_identity_path)?,
    );

    let report = R1bReport {
        schema: "haitaka-anhoku-r1b-gate",
        schema_version: 1,
        ruleset: "anhoku",
        feature_family: "HalfKAv2^+DonorSingleEff",
        corpus_positions: corpus.len(),
        deterministic_order: "R1-A stable fixture-id order; no RNG, filtering, cycling, or sampling",
        python_integer_emulator: "independent parser and exact integer graph in scripts/r1b-parity-oracle.py",
        full_precision_framework: "PyTorch CPU tensors loaded from deterministic full-precision checkpoint",
        cases,
        quantization_by_group,
        activation_clamp_counts,
        serializer_clamped_weights,
        accumulator_overflows,
        artifacts,
        gates,
        passed,
    };
    let report_path = output_dir.join("r1b-gate-report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    ensure!(passed, "R1-B gate failed; see {}", report_path.display());
    Ok(report)
}

fn check_case(
    network_path: &Path,
    expectations_path: &Path,
    corpus: &[CorpusRow],
    mut scores: Option<(&mut Vec<f64>, &mut Vec<i32>)>,
) -> Result<CaseResult> {
    let network_bytes = fs::read(network_path)?;
    let model = NnueModel::from_bytes(&network_bytes)
        .map_err(|error| anyhow!("failed to parse {}: {error}", network_path.display()))?;
    let mut reader = BufReader::new(File::open(expectations_path)?);
    let mut magic = [0u8; 16];
    reader.read_exact(&mut magic)?;
    ensure!(&magic == EXPECT_MAGIC, "wrong R1-B expectation magic");
    let count = read_u32(&mut reader)? as usize;
    ensure!(count == corpus.len(), "R1-B expectation count mismatch");
    let boards = corpus
        .iter()
        .map(|row| {
            Board::from_sfen(&row.sfen)
                .map_err(|error| anyhow!("invalid R1-B SFEN {}: {error}", row.id))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut states = Vec::with_capacity(corpus.len());
    let mut result = CaseResult {
        positions: corpus.len(),
        ..CaseResult::default()
    };
    for (index, (row, board)) in corpus.iter().zip(&boards).enumerate() {
        let expected = read_expected(&mut reader)?;
        ensure!(
            expected.bucket == row.output_bucket,
            "expectation bucket mismatch for {}",
            row.id
        );
        let full_state = model.build_position_state_full(board);
        let actual = model.r1_inference_trace_from_state(board, &full_state);
        let full_refresh_score = model.evaluate_from_state(board, &full_state);
        result.integer_score_mismatches +=
            usize::from(actual.score != expected.score || full_refresh_score != expected.score);
        result.integer_activation_mismatches += usize::from(!trace_matches(&actual, &expected));
        if let Some((full_scores, quantized_scores)) = scores.as_mut() {
            full_scores.push(expected.full_score);
            quantized_scores.push(actual.score);
        }
        if let (Some(parent_id), Some(move_text)) = (&row.parent_id, &row.move_from_parent) {
            let parent = fixture_index(parent_id)?;
            ensure!(
                parent < index,
                "R1-B parent is not earlier than child for {}",
                row.id
            );
            let mv = Move::from_str(move_text)
                .map_err(|error| anyhow!("invalid R1-B transition {move_text}: {error}"))?;
            let saved_parent = states[parent];
            let incremental = model.apply_move(&boards[parent], board, &states[parent], mv);
            result.transitions += 1;
            result.full_incremental_accumulator_mismatches +=
                usize::from(incremental != full_state);
            result.full_incremental_score_mismatches +=
                usize::from(model.evaluate_from_state(board, &incremental) != full_refresh_score);
            result.restored_parent_snapshot_mismatches +=
                usize::from(states[parent] != saved_parent);
        }
        states.push(full_state);
    }
    let mut trailing = [0u8; 1];
    ensure!(
        reader.read(&mut trailing)? == 0,
        "trailing R1-B expectation bytes"
    );
    Ok(result)
}

fn trace_matches(actual: &R1InferenceTrace, expected: &ExpectedTrace) -> bool {
    actual.bucket == expected.bucket
        && actual.black_accumulator == expected.black_accumulator
        && actual.white_accumulator == expected.white_accumulator
        && actual.black_psqt == expected.black_psqt
        && actual.white_psqt == expected.white_psqt
        && actual.transformed == expected.transformed
        && actual.hidden1 == expected.hidden1
        && actual.hidden1_relu == expected.hidden1_relu
        && actual.hidden2 == expected.hidden2
        && actual.hidden2_relu == expected.hidden2_relu
        && actual.output == expected.output
        && actual.psqt == expected.psqt
        && actual.score == expected.score
}

fn read_expected(reader: &mut impl Read) -> Result<ExpectedTrace> {
    let full_score = f64::from_le_bytes(read_array(reader)?);
    let mut bucket = [0u8; 1];
    reader.read_exact(&mut bucket)?;
    let mut padding = [0u8; 3];
    reader.read_exact(&mut padding)?;
    ensure!(padding == [0; 3]);
    Ok(ExpectedTrace {
        full_score,
        bucket: bucket[0] as usize,
        black_accumulator: read_i16_array(reader)?,
        white_accumulator: read_i16_array(reader)?,
        black_psqt: read_i32_array(reader)?,
        white_psqt: read_i32_array(reader)?,
        transformed: read_array(reader)?,
        hidden1: read_i32_array(reader)?,
        hidden1_relu: read_array(reader)?,
        hidden2: read_i32_array(reader)?,
        hidden2_relu: read_array(reader)?,
        output: read_i32(reader)?,
        psqt: read_i32(reader)?,
        score: read_i32(reader)?,
    })
}

fn read_array<const N: usize>(reader: &mut impl Read) -> Result<[u8; N]> {
    let mut values = [0u8; N];
    reader.read_exact(&mut values)?;
    Ok(values)
}

fn read_i16_array<const N: usize>(reader: &mut impl Read) -> Result<[i16; N]> {
    let mut values = [0i16; N];
    for value in &mut values {
        *value = i16::from_le_bytes(read_array(reader)?);
    }
    Ok(values)
}

fn read_i32_array<const N: usize>(reader: &mut impl Read) -> Result<[i32; N]> {
    let mut values = [0i32; N];
    for value in &mut values {
        *value = read_i32(reader)?;
    }
    Ok(values)
}

fn read_i32(reader: &mut impl Read) -> Result<i32> {
    Ok(i32::from_le_bytes(read_array(reader)?))
}

fn read_u32(reader: &mut impl Read) -> Result<u32> {
    Ok(u32::from_le_bytes(read_array(reader)?))
}

fn fixture_index(id: &str) -> Result<usize> {
    id.strip_prefix("r1a-")
        .ok_or_else(|| anyhow!("invalid R1-B fixture id {id}"))?
        .parse()
        .map_err(|error| anyhow!("invalid R1-B fixture id {id}: {error}"))
}

fn quantization_metrics(
    corpus: &[CorpusRow],
    full: &[f64],
    quantized: &[i32],
    limits: &QuantizationLimits,
) -> Result<BTreeMap<String, GroupMetrics>> {
    let mut groups = BTreeMap::<String, Vec<usize>>::new();
    for index in 0..corpus.len() {
        let split = if index % 2 == 0 {
            "even-fixture-id"
        } else {
            "odd-fixture-id"
        };
        let bucket = limits
            .score_buckets
            .iter()
            .find(|bucket| {
                bucket_contains(bucket, i32::from(corpus[index].label_score_side_to_move))
            })
            .ok_or_else(|| anyhow!("no R1-B score bucket for {}", corpus[index].id))?;
        groups.entry("all".to_string()).or_default().push(index);
        groups
            .entry(format!("split/{split}"))
            .or_default()
            .push(index);
        groups
            .entry(format!("score-bucket/{}", bucket.id))
            .or_default()
            .push(index);
        groups
            .entry(format!("split-score/{split}/{}", bucket.id))
            .or_default()
            .push(index);
    }
    for split in limits.splits.keys() {
        ensure!(
            groups.contains_key(&format!("split/{split}")),
            "empty R1-B split {split}"
        );
        for bucket in &limits.score_buckets {
            ensure!(
                groups.contains_key(&format!("split-score/{split}/{}", bucket.id)),
                "empty R1-B split/score group {split}/{}",
                bucket.id
            );
        }
    }
    for bucket in &limits.score_buckets {
        ensure!(groups.contains_key(&format!("score-bucket/{}", bucket.id)));
    }
    Ok(groups
        .into_iter()
        .map(|(name, indices)| {
            let metrics = group_metrics(&indices, corpus, full, quantized, &limits.limits);
            (name, metrics)
        })
        .collect())
}

fn bucket_contains(bucket: &ScoreBucket, score: i32) -> bool {
    bucket
        .minimum_inclusive
        .is_none_or(|minimum| score >= minimum)
        && bucket
            .maximum_inclusive
            .is_none_or(|maximum| score <= maximum)
}

fn group_metrics(
    indices: &[usize],
    corpus: &[CorpusRow],
    full: &[f64],
    quantized: &[i32],
    limits: &AbsoluteLimits,
) -> GroupMetrics {
    let mut deltas = indices
        .iter()
        .map(|&index| (full[index] - f64::from(quantized[index])).abs())
        .collect::<Vec<_>>();
    deltas.sort_by(f64::total_cmp);
    let mean = deltas.iter().sum::<f64>() / deltas.len() as f64;
    let p95 = percentile(&deltas, 0.95);
    let p99 = percentile(&deltas, 0.99);
    let maximum = *deltas.last().unwrap();
    let full_loss = indices
        .iter()
        .map(|&index| {
            probability_loss(
                full[index],
                f64::from(corpus[index].label_score_side_to_move),
            )
        })
        .sum::<f64>()
        / indices.len() as f64;
    let quantized_loss = indices
        .iter()
        .map(|&index| {
            probability_loss(
                f64::from(quantized[index]),
                f64::from(corpus[index].label_score_side_to_move),
            )
        })
        .sum::<f64>()
        / indices.len() as f64;
    let degradation = quantized_loss - full_loss;
    let passed = mean <= limits.mean_absolute_score_delta
        && p99 <= limits.p99_absolute_score_delta
        && maximum <= limits.maximum_absolute_score_delta
        && degradation <= limits.maximum_positive_loss_degradation;
    GroupMetrics {
        count: indices.len(),
        mean_absolute_score_delta: mean,
        p95_absolute_score_delta: p95,
        p99_absolute_score_delta: p99,
        maximum_absolute_score_delta: maximum,
        full_precision_loss: full_loss,
        quantized_loss,
        loss_degradation: degradation,
        passed,
    }
}

fn percentile(sorted: &[f64], percentile: f64) -> f64 {
    let rank = (percentile * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn probability_loss(prediction: f64, target: f64) -> f64 {
    let prediction = sigmoid(prediction / 410.0);
    let target = sigmoid(target / 410.0);
    (prediction - target).powi(2)
}

fn sigmoid(value: f64) -> f64 {
    if value >= 0.0 {
        1.0 / (1.0 + (-value).exp())
    } else {
        let exp = value.exp();
        exp / (1.0 + exp)
    }
}

fn required_patterns_present(metadata: &serde_json::Value) -> bool {
    let patterns = &metadata["patterns"];
    patterns["zeroNetwork"].as_bool() == Some(true)
        && patterns["biasOnlyNetwork"].as_bool() == Some(true)
        && patterns["oneHotPositiveAndNegativeRows"].as_bool() == Some(true)
        && patterns["distinctDonorReceiverAndPerspectiveSignatures"].as_bool() == Some(true)
        && patterns["activationClampBoundaries"]
            .as_array()
            .is_some_and(|values| values.len() == 9)
        && patterns["tiesToEven"]
            .as_array()
            .is_some_and(|values| values.len() == 4)
        && patterns["minimumMaximumSerializedTransformerWeights"]
            .as_array()
            .is_some_and(|values| values.len() == 2)
}

fn validate_checkpoint(path: &Path) -> Result<serde_json::Value> {
    let mut file = File::open(path)?;
    let mut magic = [0u8; 16];
    file.read_exact(&mut magic)?;
    ensure!(&magic == CHECKPOINT_MAGIC, "wrong R1-B checkpoint magic");
    let header_len = read_u32(&mut file)? as usize;
    ensure!(header_len > 0 && header_len < 4096);
    let mut header = vec![0u8; header_len];
    file.read_exact(&mut header)?;
    let metadata: serde_json::Value = serde_json::from_slice(&header)?;
    ensure!(
        metadata.get("schema").and_then(serde_json::Value::as_str)
            == Some("haitaka-r1b-full-precision-checkpoint-v1")
    );
    ensure!(
        metadata.get("bytes").and_then(serde_json::Value::as_u64)
            == Some(fs::metadata(path)?.len())
    );
    Ok(metadata)
}

#[derive(Debug)]
struct SerializedLayout {
    transformer_bias: Range<usize>,
    transformer_weights: Range<usize>,
    transformer_psqt: Range<usize>,
    affine_biases: Vec<Range<usize>>,
    affine_weights: Vec<Range<usize>>,
}

fn serialized_layout(bytes: &[u8]) -> Result<SerializedLayout> {
    const REAL_ROWS: usize = 152_523;
    ensure!(bytes.len() >= 16, "truncated R1-B serialized network");
    let description_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut cursor = 12usize
        .checked_add(description_len)
        .and_then(|value| value.checked_add(4))
        .ok_or_else(|| anyhow!("R1-B serialized layout overflow"))?;
    let transformer_bias = cursor..cursor + FT_DIMENSIONS * 2;
    cursor = transformer_bias.end;
    let transformer_weights = cursor..cursor + REAL_ROWS * FT_DIMENSIONS * 2;
    cursor = transformer_weights.end;
    let transformer_psqt = cursor..cursor + REAL_ROWS * PSQT_BUCKETS * 4;
    cursor = transformer_psqt.end;
    let mut affine_biases = Vec::new();
    let mut affine_weights = Vec::new();
    for _ in 0..8 {
        cursor += 4;
        for (outputs, padded_inputs) in [(16, 1024), (32, 32), (1, 32)] {
            let biases = cursor..cursor + outputs * 4;
            cursor = biases.end;
            let weights = cursor..cursor + outputs * padded_inputs;
            cursor = weights.end;
            affine_biases.push(biases);
            affine_weights.push(weights);
        }
    }
    ensure!(
        cursor == bytes.len(),
        "unexpected R1-B serialized network length"
    );
    Ok(SerializedLayout {
        transformer_bias,
        transformer_weights,
        transformer_psqt,
        affine_biases,
        affine_weights,
    })
}

fn audit_serialized_patterns(
    output_dir: &Path,
    checkpoint_metadata: &serde_json::Value,
) -> Result<bool> {
    let zero = fs::read(output_dir.join("zero.nnue"))?;
    let zero_layout = serialized_layout(&zero)?;
    let zero_payload = [
        zero_layout.transformer_bias.clone(),
        zero_layout.transformer_weights.clone(),
        zero_layout.transformer_psqt.clone(),
    ]
    .into_iter()
    .chain(zero_layout.affine_biases.iter().cloned())
    .chain(zero_layout.affine_weights.iter().cloned())
    .all(|range| zero[range].iter().all(|byte| *byte == 0));

    let bias_only = fs::read(output_dir.join("bias-only.nnue"))?;
    let bias_layout = serialized_layout(&bias_only)?;
    let bias_weights_zero = [
        bias_layout.transformer_weights.clone(),
        bias_layout.transformer_psqt.clone(),
    ]
    .into_iter()
    .chain(bias_layout.affine_weights.iter().cloned())
    .all(|range| bias_only[range].iter().all(|byte| *byte == 0));
    let bias_values_present = bias_only[bias_layout.transformer_bias.clone()]
        .iter()
        .any(|byte| *byte != 0)
        && bias_layout
            .affine_biases
            .iter()
            .any(|range| bias_only[range.clone()].iter().any(|byte| *byte != 0));

    let sentinel = fs::read(output_dir.join("sentinel-a.nnue"))?;
    let sentinel_layout = serialized_layout(&sentinel)?;
    let read_bias = |dimension: usize| {
        read_i16_at(
            &sentinel,
            sentinel_layout.transformer_bias.start + dimension * 2,
        )
    };
    let clamp_biases = [-1, 0, 63, 64, 127, 128, 8127, 8128, 8129];
    let clamp_boundaries_present = clamp_biases
        .iter()
        .enumerate()
        .all(|(dimension, expected)| read_bias(dimension) == *expected);
    let ties_to_even_present = [0, 2, 0, -2]
        .iter()
        .enumerate()
        .all(|(offset, expected)| read_bias(11 + offset) == *expected);
    let rows = checkpoint_metadata
        .get("sentinelRows")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow!("R1-B checkpoint lacks sentinelRows"))?;
    let row_value = |name: &str| -> Result<usize> {
        Ok(rows
            .get(name)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| anyhow!("R1-B checkpoint lacks sentinel row {name}"))?
            as usize)
    };
    let weight = |row: usize, dimension: usize| {
        read_i16_at(
            &sentinel,
            sentinel_layout.transformer_weights.start + (row * FT_DIMENSIONS + dimension) * 2,
        )
    };
    let row_patterns_present = weight(row_value("oneHotPositive")?, 15) == 63
        && weight(row_value("oneHotNegative")?, 15) == -63
        && weight(row_value("maximumI16")?, 9) == i16::MAX
        && weight(row_value("minimumI16")?, 10) == i16::MIN;
    let first_h1 = &sentinel[sentinel_layout.affine_weights[0].clone()];
    let perspectives_are_asymmetric = first_h1[..FT_DIMENSIONS].iter().any(|byte| *byte != 0)
        && first_h1[FT_DIMENSIONS..FT_DIMENSIONS * 2]
            .iter()
            .any(|byte| *byte != 0)
        && first_h1[..FT_DIMENSIONS] != first_h1[FT_DIMENSIONS..FT_DIMENSIONS * 2];

    Ok(zero_payload
        && bias_weights_zero
        && bias_values_present
        && clamp_boundaries_present
        && ties_to_even_present
        && row_patterns_present
        && perspectives_are_asymmetric)
}

fn read_i16_at(bytes: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn activation_boundaries_exercised(integer_audit: &serde_json::Value) -> bool {
    let clamps = &integer_audit["sentinel"]["clamps"];
    [
        "transformerLower",
        "transformerUpper",
        "hidden1Lower",
        "hidden1Upper",
        "hidden2Lower",
        "hidden2Upper",
    ]
    .iter()
    .all(|name| clamps[*name].as_u64().is_some_and(|count| count > 0))
}

fn read_corpus(path: &Path) -> Result<Vec<CorpusRow>> {
    BufReader::new(File::open(path)?)
        .lines()
        .map(|line| Ok(serde_json::from_str(&line?)?))
        .collect()
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("failed to parse {}", path.display()))
}

fn artifact_identity(path: &Path) -> Result<ArtifactIdentity> {
    Ok(ArtifactIdentity {
        path: path.display().to_string(),
        bytes: fs::metadata(path)?.len(),
        sha256: sha256_file(path)?,
    })
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_score_buckets_are_exhaustive_at_boundaries() {
        let buckets: QuantizationLimits = read_json(Path::new(
            "../r0/anhoku-reboot/r1b-quantization-limits.json",
        ))
        .unwrap();
        for score in [
            -10_000, -501, -500, -101, -100, 0, 100, 101, 500, 501, 10_000,
        ] {
            assert_eq!(
                buckets
                    .score_buckets
                    .iter()
                    .filter(|bucket| bucket_contains(bucket, score))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn percentile_uses_nearest_rank() {
        let values = (1..=100).map(f64::from).collect::<Vec<_>>();
        assert_eq!(percentile(&values, 0.95), 95.0);
        assert_eq!(percentile(&values, 0.99), 99.0);
    }
}

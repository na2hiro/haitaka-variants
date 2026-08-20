use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::config::LoadedConfig;
use crate::dataset::ENTRY_BYTES;

const SCORE_OFFSET: usize = 64;
const TEACHER_MOVE_OFFSET: usize = 66;
const PLY_OFFSET: usize = 68;
const RESULT_OFFSET: usize = 70;
// Search mate scores are distance-adjusted below 30,000.
const MATE_SCORE_THRESHOLD: i32 = 29_000;

#[derive(Debug, Serialize)]
pub struct AuditReport {
    schema: &'static str,
    file: AuditFile,
    identity: AuditIdentity,
    side_to_move: BinaryCounts,
    ply_parity: ParityCounts,
    outcomes_relative_to_side_to_move: OutcomeCounts,
    scores: ScoreStats,
    teacher_moves: TeacherMoveStats,
    samples_before_opening: u64,
    position_trace: PositionTraceStats,
    groups: GroupStats,
}

#[derive(Debug, Serialize)]
struct AuditFile {
    path: String,
    bytes: u64,
    entry_bytes: usize,
    entries: u64,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct AuditIdentity {
    config_hash: Option<String>,
    seed: Option<u64>,
    ruleset: Option<String>,
    feature_family: Option<String>,
    sampling_phase: Option<String>,
    sample_after_opening: Option<bool>,
    teacher_move_encoding: Option<String>,
    opening_policy: Option<String>,
    opening_suite_id: Option<String>,
    opening_suite_sha256: Option<String>,
    opening_transformation: Option<String>,
    split_policy: Option<String>,
    split_seed: Option<u64>,
    shuffle_policy: Option<String>,
    shuffle_seed: Option<u64>,
    shuffle_chunk_records: Option<u64>,
    self_play_move_policy: Option<String>,
    position_policy: Option<String>,
    training_trace_version: Option<String>,
    incomplete_label_policy: Option<String>,
    position_selection_audit_version: Option<String>,
}

#[derive(Debug, Serialize)]
struct PositionTraceStats {
    root_ply_min: u16,
    root_ply_max: u16,
    leaf_distance_min: Option<u64>,
    leaf_distance_max: Option<u64>,
    leaf_distance_mean: f64,
    candidate_positions: u64,
    rejected_incomplete_label_positions: u64,
    rejected_terminal_positions: u64,
    rejected_mate_score_positions: u64,
    selection_by_side_parity_and_result: Value,
    selection_by_opening: Value,
}

#[derive(Debug, Serialize)]
struct GroupStats {
    game_count: u64,
    unique_game_ids: u64,
    opening_group_count: u64,
    train_opening_group_count: u64,
    validation_opening_group_count: u64,
    opening_group_overlap_count: u64,
    opening_group_overlap_ids: Vec<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct BinaryCounts {
    black: u64,
    white: u64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct ParityCounts {
    even: u64,
    odd: u64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct OutcomeCounts {
    win: u64,
    loss: u64,
    draw: u64,
}

#[derive(Debug, Serialize)]
struct ScoreStats {
    min: i16,
    max: i16,
    mean: f64,
    absolute_mean: f64,
    quantiles: ScoreQuantiles,
    mate_rate_count: u64,
    clamp_rate_count: u64,
}

#[derive(Debug, Serialize)]
struct ScoreQuantiles {
    p01: i16,
    p05: i16,
    p25: i16,
    p50: i16,
    p75: i16,
    p95: i16,
    p99: i16,
}

#[derive(Debug, Serialize)]
struct TeacherMoveStats {
    nonzero: u64,
    zero: u64,
}

pub fn audit_dataset(
    bin_path: &Path,
    manifest_path: &Path,
    config_path: Option<&Path>,
) -> Result<AuditReport> {
    let manifest: Value = serde_json::from_slice(
        &fs::read(manifest_path)
            .with_context(|| format!("failed to read manifest {}", manifest_path.display()))?,
    )
    .with_context(|| format!("failed to parse manifest {}", manifest_path.display()))?;
    let manifest_entry_bytes = manifest
        .get("entry_bytes")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("manifest is missing integer entry_bytes"))?
        as usize;
    if manifest_entry_bytes != ENTRY_BYTES {
        bail!(
            "manifest entry_bytes is {manifest_entry_bytes}, expected {ENTRY_BYTES} for the current 72-byte ABI"
        );
    }
    let expected_entries = manifest
        .get("sampled_positions")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("manifest is missing integer sampled_positions"))?;
    let expected_bytes = expected_entries
        .checked_mul(ENTRY_BYTES as u64)
        .ok_or_else(|| anyhow!("dataset byte length overflow"))?;
    let actual_bytes = fs::metadata(bin_path)
        .with_context(|| format!("failed to stat dataset {}", bin_path.display()))?
        .len();
    if actual_bytes != expected_bytes {
        let kind = if actual_bytes < expected_bytes {
            "truncated"
        } else {
            "overlong"
        };
        bail!(
            "dataset {} is {kind}: found {actual_bytes} bytes, expected exactly {expected_bytes} bytes ({expected_entries} records x {ENTRY_BYTES})",
            bin_path.display()
        );
    }
    if expected_entries == 0 {
        bail!("dataset contains no entries; score statistics are undefined");
    }

    let loaded = config_path.map(LoadedConfig::from_path).transpose()?;
    let opening_random_plies = value_u64(&manifest, "opening_random_plies").or_else(|| {
        loaded
            .as_ref()
            .map(|c| u64::from(c.config.data.opening_random_plies))
    });
    let mut reader = BufReader::new(
        File::open(bin_path)
            .with_context(|| format!("failed to open dataset {}", bin_path.display()))?,
    );
    let mut hash = Sha256::new();
    let mut record = [0u8; ENTRY_BYTES];
    let mut sides = BinaryCounts { black: 0, white: 0 };
    let mut parity = ParityCounts { even: 0, odd: 0 };
    let mut outcomes = OutcomeCounts {
        win: 0,
        loss: 0,
        draw: 0,
    };
    let mut scores = Vec::with_capacity(expected_entries as usize);
    let mut score_sum = 0i128;
    let mut absolute_score_sum = 0u128;
    let mut mate_rate_count = 0;
    let mut clamp_rate_count = 0;
    let mut nonzero_moves = 0;
    let mut samples_before_opening = 0;
    let mut root_ply_min = u16::MAX;
    let mut root_ply_max = 0u16;

    for _ in 0..expected_entries {
        reader.read_exact(&mut record)?;
        hash.update(record);
        // The packer mirrors Haitaka colors for the trainer: bit 0 means original White.
        if record[0] & 1 == 0 {
            sides.black += 1;
        } else {
            sides.white += 1;
        }
        let score = i16::from_le_bytes([record[SCORE_OFFSET], record[SCORE_OFFSET + 1]]);
        let teacher_move =
            u16::from_le_bytes([record[TEACHER_MOVE_OFFSET], record[TEACHER_MOVE_OFFSET + 1]]);
        let ply = u16::from_le_bytes([record[PLY_OFFSET], record[PLY_OFFSET + 1]]);
        root_ply_min = root_ply_min.min(ply);
        root_ply_max = root_ply_max.max(ply);
        let result = record[RESULT_OFFSET] as i8;
        if ply % 2 == 0 {
            parity.even += 1;
        } else {
            parity.odd += 1;
        }
        match result {
            1 => outcomes.win += 1,
            -1 => outcomes.loss += 1,
            0 => outcomes.draw += 1,
            other => bail!("record has invalid game_result {other}; expected -1, 0, or 1"),
        }
        let score_i32 = i32::from(score);
        score_sum += i128::from(score_i32);
        absolute_score_sum += score_i32.unsigned_abs() as u128;
        mate_rate_count += u64::from(score_i32.abs() >= MATE_SCORE_THRESHOLD);
        clamp_rate_count += u64::from(matches!(score, i16::MIN | i16::MAX));
        nonzero_moves += u64::from(teacher_move != 0);
        if opening_random_plies.is_some_and(|opening| u64::from(ply) < opening) {
            samples_before_opening += 1;
        }
        scores.push(score);
    }
    scores.sort_unstable();
    let count = expected_entries as f64;
    let q = |percent: u32| -> i16 {
        let index = (((scores.len() - 1) as u128 * u128::from(percent) + 50) / 100) as usize;
        scores[index]
    };
    let identity = AuditIdentity {
        config_hash: value_string(&manifest, "config_hash")
            .or_else(|| loaded.as_ref().map(|c| c.hash_hex.clone())),
        seed: value_u64(&manifest, "seed").or_else(|| loaded.as_ref().map(|c| c.config.data.seed)),
        ruleset: value_string(&manifest, "ruleset").or_else(|| {
            loaded
                .as_ref()
                .map(|c| c.config.rules.ruleset.as_str().to_string())
        }),
        feature_family: value_string(&manifest, "feature_family")
            .or_else(|| loaded.as_ref().map(|c| c.training_features().to_string())),
        sampling_phase: value_string(&manifest, "sampling_phase").or_else(|| {
            loaded
                .as_ref()
                .map(|c| c.config.data.sampling_policy.manifest_name().to_string())
        }),
        sample_after_opening: manifest
            .get("sample_after_opening")
            .and_then(Value::as_bool)
            .or_else(|| {
                loaded
                    .as_ref()
                    .map(|c| c.config.data.sampling_policy.samples_after_opening())
            }),
        teacher_move_encoding: Some(
            value_string(&manifest, "teacher_move_encoding")
                .unwrap_or_else(|| "legacy-unspecified".to_string()),
        ),
        opening_policy: value_string(&manifest, "opening_policy"),
        opening_suite_id: value_string(&manifest, "opening_suite_id"),
        opening_suite_sha256: value_string(&manifest, "opening_suite_sha256"),
        opening_transformation: value_string(&manifest, "opening_transformation"),
        split_policy: value_string(&manifest, "split_policy"),
        split_seed: value_u64(&manifest, "split_seed"),
        shuffle_policy: value_string(&manifest, "shuffle_policy"),
        shuffle_seed: value_u64(&manifest, "shuffle_seed"),
        shuffle_chunk_records: value_u64(&manifest, "shuffle_chunk_records"),
        self_play_move_policy: value_string(&manifest, "self_play_move_policy"),
        position_policy: Some(
            value_string(&manifest, "position_policy")
                .unwrap_or_else(|| "root-position".to_string()),
        ),
        training_trace_version: value_string(&manifest, "training_trace_version"),
        incomplete_label_policy: Some(
            value_string(&manifest, "incomplete_label_policy")
                .unwrap_or_else(|| "error".to_string()),
        ),
        position_selection_audit_version: value_string(
            &manifest,
            "position_selection_audit_version",
        ),
    };
    let train_opening_ids = value_string_vec(&manifest, "train_opening_ids");
    let validation_opening_ids = value_string_vec(&manifest, "validation_opening_ids");
    let validation_set = validation_opening_ids
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    let opening_group_overlap_ids = train_opening_ids
        .iter()
        .filter(|id| validation_set.contains(id))
        .cloned()
        .collect::<Vec<_>>();
    let games = manifest.get("games").and_then(Value::as_array);
    let unique_game_ids = games
        .into_iter()
        .flatten()
        .filter_map(|game| game.get("game_id").and_then(Value::as_str))
        .collect::<std::collections::BTreeSet<_>>()
        .len() as u64;
    Ok(AuditReport {
        schema: "haitaka-dataset-audit-v1",
        file: AuditFile {
            path: bin_path.display().to_string(),
            bytes: actual_bytes,
            entry_bytes: ENTRY_BYTES,
            entries: expected_entries,
            sha256: format!("{:x}", hash.finalize()),
        },
        identity,
        side_to_move: sides,
        ply_parity: parity,
        outcomes_relative_to_side_to_move: outcomes,
        scores: ScoreStats {
            min: scores[0],
            max: scores[scores.len() - 1],
            mean: score_sum as f64 / count,
            absolute_mean: absolute_score_sum as f64 / count,
            quantiles: ScoreQuantiles {
                p01: q(1),
                p05: q(5),
                p25: q(25),
                p50: q(50),
                p75: q(75),
                p95: q(95),
                p99: q(99),
            },
            mate_rate_count,
            clamp_rate_count,
        },
        teacher_moves: TeacherMoveStats {
            nonzero: nonzero_moves,
            zero: expected_entries - nonzero_moves,
        },
        samples_before_opening,
        position_trace: PositionTraceStats {
            root_ply_min,
            root_ply_max,
            leaf_distance_min: value_u64(&manifest, "leaf_distance_min"),
            leaf_distance_max: value_u64(&manifest, "leaf_distance_max"),
            leaf_distance_mean: value_f64(&manifest, "leaf_distance_mean").unwrap_or(0.0),
            candidate_positions: value_u64(&manifest, "candidate_positions")
                .unwrap_or(expected_entries),
            rejected_incomplete_label_positions: value_u64(
                &manifest,
                "rejected_incomplete_label_positions",
            )
            .unwrap_or(0),
            rejected_terminal_positions: value_u64(&manifest, "rejected_terminal_positions")
                .unwrap_or(0),
            rejected_mate_score_positions: value_u64(&manifest, "rejected_mate_score_positions")
                .unwrap_or(0),
            selection_by_side_parity_and_result: manifest
                .get("position_selection")
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default())),
            selection_by_opening: manifest
                .get("opening_position_selection")
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default())),
        },
        groups: GroupStats {
            game_count: games.map_or(0, |games| games.len() as u64),
            unique_game_ids,
            opening_group_count: value_u64(&manifest, "opening_group_count").unwrap_or(0),
            train_opening_group_count: train_opening_ids.len() as u64,
            validation_opening_group_count: validation_opening_ids.len() as u64,
            opening_group_overlap_count: opening_group_overlap_ids.len() as u64,
            opening_group_overlap_ids,
        },
    })
}

pub fn write_report(report: &AuditReport, output: Option<&Path>) -> Result<Option<PathBuf>> {
    let mut bytes = serde_json::to_vec_pretty(report)?;
    bytes.push(b'\n');
    if let Some(path) = output {
        fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
        Ok(Some(path.to_path_buf()))
    } else {
        print!("{}", String::from_utf8(bytes).expect("JSON is UTF-8"));
        Ok(None)
    }
}

fn value_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn value_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn value_f64(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

fn value_string_vec(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn entry(white_to_move: bool, score: i16, mv: u16, ply: u16, result: i8) -> [u8; 72] {
        let mut entry = [0u8; 72];
        entry[0] = u8::from(white_to_move);
        entry[64..66].copy_from_slice(&score.to_le_bytes());
        entry[66..68].copy_from_slice(&mv.to_le_bytes());
        entry[68..70].copy_from_slice(&ply.to_le_bytes());
        entry[70] = result as u8;
        entry
    }

    #[test]
    fn fixture_reports_exact_counters_for_sides_outcomes_and_moves() {
        let temp = tempdir().unwrap();
        let bin = temp.path().join("fixture.bin");
        let manifest = temp.path().join("fixture.json");
        let records = [
            entry(false, -30_000, 0, 2, -1),
            entry(true, 0, 42, 3, 0),
            entry(false, i16::MAX, 0, 4, 1),
        ];
        fs::write(&bin, records.concat()).unwrap();
        fs::write(
            &manifest,
            r#"{"sampled_positions":3,"entry_bytes":72,"opening_random_plies":4,"position_policy":"qsearch-pv-leaf","training_trace_version":"qsearch-pv-v1","candidate_positions":5,"rejected_terminal_positions":1,"rejected_mate_score_positions":1,"leaf_distance_min":1,"leaf_distance_max":3,"leaf_distance_mean":2.0}"#,
        )
        .unwrap();
        let report = audit_dataset(&bin, &manifest, None).unwrap();
        assert_eq!(report.side_to_move, BinaryCounts { black: 2, white: 1 });
        assert_eq!(report.ply_parity, ParityCounts { even: 2, odd: 1 });
        assert_eq!(
            report.outcomes_relative_to_side_to_move,
            OutcomeCounts {
                win: 1,
                loss: 1,
                draw: 1
            }
        );
        assert_eq!(report.teacher_moves.nonzero, 1);
        assert_eq!(report.samples_before_opening, 2);
        assert_eq!(report.scores.mate_rate_count, 2);
        assert_eq!(report.scores.clamp_rate_count, 1);
        assert_eq!(
            report.identity.position_policy.as_deref(),
            Some("qsearch-pv-leaf")
        );
        assert_eq!(report.position_trace.root_ply_min, 2);
        assert_eq!(report.position_trace.root_ply_max, 4);
        assert_eq!(report.position_trace.leaf_distance_min, Some(1));
        assert_eq!(report.position_trace.leaf_distance_max, Some(3));
        assert_eq!(report.position_trace.leaf_distance_mean, 2.0);
        assert_eq!(report.position_trace.candidate_positions, 5);
        assert_eq!(report.position_trace.rejected_terminal_positions, 1);
        assert_eq!(report.position_trace.rejected_mate_score_positions, 1);
    }

    #[test]
    fn rejects_truncated_and_overlong_files() {
        let temp = tempdir().unwrap();
        let bin = temp.path().join("fixture.bin");
        let manifest = temp.path().join("fixture.json");
        fs::write(&manifest, r#"{"sampled_positions":1,"entry_bytes":72}"#).unwrap();
        for (size, expected) in [(71, "truncated"), (73, "overlong")] {
            fs::write(&bin, vec![0u8; size]).unwrap();
            let error = audit_dataset(&bin, &manifest, None).unwrap_err();
            assert!(format!("{error:#}").contains(expected));
        }
    }

    #[test]
    fn report_serialization_is_byte_identical() {
        let temp = tempdir().unwrap();
        let bin = temp.path().join("fixture.bin");
        let manifest = temp.path().join("fixture.json");
        fs::write(&bin, entry(false, 10, 0, 8, 1)).unwrap();
        fs::write(&manifest, r#"{"sampled_positions":1,"entry_bytes":72}"#).unwrap();
        let one =
            serde_json::to_vec_pretty(&audit_dataset(&bin, &manifest, None).unwrap()).unwrap();
        let two =
            serde_json::to_vec_pretty(&audit_dataset(&bin, &manifest, None).unwrap()).unwrap();
        assert_eq!(one, two);
    }
}

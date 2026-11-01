//! Independent replay validator for the Step 12 opening freeze.
//!
//! The validator reads the producer's candidate log as data, but recomputes
//! the RNG stream, every move, every rejection decision, all family sets, the
//! published rows, and the explicit 512-game schedule itself.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, ensure};
use haitaka::Board;
use rand::SeedableRng;
use rand::rngs::StdRng;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::openings::color_swap_anhoku_sfen;
use crate::step12_openings::{
    CandidateTrajectory, FREEZE_SCHEMA, GENERATION_SCHEMA, PAIR_TRANSFORMATION, VALIDATION_SCHEMA,
    canonical_family, load_exclusions, load_spec, pairing_fixture, replay_candidate_with_rng,
    trajectory_digest,
};

pub(crate) fn validate_openings(
    spec_path: &Path,
    output_dir: &Path,
    report_path: &Path,
) -> Result<PathBuf> {
    let spec = load_spec(spec_path)?;
    let exclusions = load_exclusions(&spec)?;
    ensure!(
        output_dir.is_dir(),
        "missing opening output directory {}",
        output_dir.display()
    );
    let generation_path = output_dir.join("generation.json");
    let generation = load_json(&generation_path)?;
    ensure!(
        generation["schema"] == GENERATION_SCHEMA,
        "opening generation schema differs"
    );
    ensure!(
        generation["status"] == "passed",
        "opening generation did not pass"
    );
    ensure!(
        generation["seed"] == 2026091606u64,
        "opening generation seed differs"
    );
    ensure!(
        generation["randomPlies"] == 16u64,
        "opening generation random plies differ"
    );
    ensure!(
        generation["rng"] == crate::step12_openings::RNG_IDENTITY,
        "opening generation RNG differs"
    );
    ensure!(
        generation["modelBlind"] == true,
        "opening generation is not model blind"
    );
    ensure!(
        generation["outcomeInputsUsed"] == false,
        "opening generation used outcomes"
    );
    verify_receipt(&generation["spec"])?;
    verify_receipt(&generation["exclusions"])?;
    let candidate_path = output_dir.join("candidates.jsonl");
    verify_receipt(&generation["candidateLog"])?;
    ensure!(
        PathBuf::from(generation["candidateLog"]["path"].as_str().unwrap()) == candidate_path,
        "generation candidate log path differs"
    );

    let accepted = replay_and_validate_candidates(&spec, &exclusions, &candidate_path)?;
    ensure!(
        accepted.len() == 128,
        "independent replay accepted {} openings, expected 128",
        accepted.len()
    );
    verify_suite_outputs(output_dir, &accepted)?;
    let exclusions_path = output_dir.join("exclusions.json");
    verify_exclusions_receipt(&exclusions_path, &exclusions, &accepted)?;
    let schedule_path = output_dir.join("schedule.json");
    let schedule = load_json(&schedule_path)?;
    let fixture = pairing_fixture(&spec)?;
    validate_schedule(&spec, &schedule, &accepted, &fixture)?;
    verify_generation_counts(&generation, &candidate_path, accepted.len())?;

    let checks = json!({
        "specAndInputHashesAtCompletion": true,
        "historicalFamilyInventories": true,
        "newTrainingFamilyExclusion": true,
        "newValidationFamilyExclusion": true,
        "corpusFamilySplitDisjoint": true,
        "candidateOrdinalsContiguous": true,
        "candidateReplay": true,
        "candidateRejectionPrecedence": true,
        "firstEligibleAcceptance": true,
        "candidateTrajectoryHashes": true,
        "sourceGameIdsUnique": true,
        "exactAcceptedQuota": true,
        "developmentSuiteBytes": true,
        "developmentSfenBytes": true,
        "blockSlicesAndOrder": true,
        "scheduleRows": 512,
        "pairingFixture": true,
        "pairingTransformation": PAIR_TRANSFORMATION,
        "modelsBound": false,
        "outcomesOpened": false,
    });
    let report = json!({
        "schema": VALIDATION_SCHEMA,
        "status": "passed",
        "experimentId": spec["experimentId"],
        "checks": checks,
        "spec": file_receipt(spec_path)?,
        "generation": file_receipt(&generation_path)?,
        "candidateLog": file_receipt(&candidate_path)?,
        "exclusions": file_receipt(&exclusions_path)?,
        "schedule": file_receipt(&schedule_path)?,
        "acceptedPositions": accepted.len(),
        "attemptedCandidates": generation["attemptedCandidates"],
        "rejectedCandidates": generation["rejectedCandidates"],
        "lastCandidateOrdinal": generation["lastCandidateOrdinal"],
        "accepted": accepted
            .iter()
            .enumerate()
            .map(|(row, item)| json!({
                "rowIndex": row,
                "openingId": item.opening_id,
                "sourceGameId": item.source_game_id,
                "candidateOrdinal": item.candidate_ordinal,
                "sfen": item.sfen,
                "canonicalFamily": item.family,
            }))
            .collect::<Vec<_>>(),
        "pairingFixture": fixture,
        "modelBlind": true,
        "trainingOrGpuAttempted": false,
        "outcomesOpened": false,
    });
    write_json(report_path, &report)?;

    let freeze_path = output_dir.join("freeze.json");
    let mut freeze = load_json(&freeze_path)?;
    ensure!(
        freeze["schema"] == FREEZE_SCHEMA,
        "opening freeze schema differs"
    );
    ensure!(
        freeze["status"] == "validated-before-publication",
        "opening producer freeze marker is invalid"
    );
    freeze["independentValidation"] = file_receipt(report_path)?;
    freeze["validator"] = json!({
        "implementation": "haitaka_learn/src/step12_opening_validation.rs::validate_openings",
        "binary": file_receipt(&std::env::current_exe()?)?,
    });
    freeze["validationCompleted"] = Value::Bool(true);
    write_json(&freeze_path, &freeze)?;
    Ok(report_path.to_path_buf())
}

#[derive(Clone)]
struct AcceptedOpening {
    candidate_ordinal: u64,
    source_game_id: String,
    opening_id: String,
    sfen: String,
    family: String,
}

fn replay_and_validate_candidates(
    spec: &Value,
    exclusions: &crate::step12_openings::ExclusionState,
    candidate_path: &Path,
) -> Result<Vec<AcceptedOpening>> {
    let seed = spec["seed"].as_u64().unwrap();
    let random_plies = spec["randomPlies"].as_u64().unwrap() as u16;
    let target = spec["acceptedPositions"].as_u64().unwrap() as usize;
    let maximum_attempts = spec["maximumCandidateAttempts"].as_u64().unwrap();
    let candidate_prefix = spec["candidateIdPrefix"].as_str().unwrap();
    let opening_prefix = spec["openingIdPrefix"].as_str().unwrap();
    let mut rng = StdRng::seed_from_u64(seed);
    let reader = BufReader::new(File::open(candidate_path)?);
    let mut accepted = Vec::with_capacity(target);
    let mut accepted_families = HashSet::with_capacity(target);
    let mut expected_ordinal = 0u64;
    let mut after_target = false;

    for (line_number, raw) in reader.lines().enumerate() {
        let line = raw?;
        if line.trim().is_empty() {
            continue;
        }
        ensure!(
            !after_target,
            "candidate log continues after the 128th accepted candidate"
        );
        let value: Value = serde_json::from_str(&line).with_context(|| {
            format!(
                "invalid candidate JSON at {}:{}",
                candidate_path.display(),
                line_number + 1
            )
        })?;
        ensure!(
            value["candidateOrdinal"] == expected_ordinal,
            "candidate ordinal gap at line {}",
            line_number + 1
        );
        let source_game_id = format!("{candidate_prefix}{expected_ordinal:06}");
        ensure!(
            value["sourceGameId"] == source_game_id,
            "candidate source-game ID differs at ordinal {}",
            expected_ordinal
        );
        let trajectory = replay_candidate_with_rng(&mut rng, random_plies);
        let start_sfen = value["startSfen"]
            .as_str()
            .ok_or_else(|| anyhow!("candidate start SFEN is missing at {}", expected_ordinal))?;
        ensure!(
            start_sfen == trajectory.start_sfen,
            "candidate start SFEN differs at {}",
            expected_ordinal
        );
        let moves = value["moves"]
            .as_array()
            .ok_or_else(|| anyhow!("candidate moves are missing at {}", expected_ordinal))?;
        let move_strings = moves
            .iter()
            .map(|mv| {
                mv.as_str().map(str::to_string).ok_or_else(|| {
                    anyhow!("candidate move is not a string at {}", expected_ordinal)
                })
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            move_strings == trajectory.moves,
            "candidate move sequence differs at {}",
            expected_ordinal
        );
        let endpoint = value["endpointSfen"].as_str().map(str::to_string);
        ensure!(
            endpoint == trajectory.endpoint_sfen,
            "candidate endpoint differs at {}",
            expected_ordinal
        );
        let family = if trajectory.valid_endpoint {
            Some(canonical_family(
                &Board::from_sfen(trajectory.endpoint_sfen.as_deref().unwrap())
                    .map_err(|error| anyhow!("invalid candidate endpoint SFEN: {error:?}"))?,
            )?)
        } else {
            None
        };
        let (rejection, accepted_row) = expected_decision(
            &trajectory,
            family.as_deref(),
            exclusions,
            &accepted_families,
            accepted.len(),
        );
        ensure!(
            value["rejection"] == rejection.map(Value::from).unwrap_or(Value::Null),
            "candidate rejection differs at {}",
            expected_ordinal
        );
        ensure!(
            value["acceptedRow"] == accepted_row.map(Value::from).unwrap_or(Value::Null),
            "candidate accepted row differs at {}",
            expected_ordinal
        );
        ensure!(
            value["canonicalFamily"] == family.clone().map(Value::from).unwrap_or(Value::Null),
            "candidate family differs at {}",
            expected_ordinal
        );
        ensure!(
            value["trajectorySha256"]
                == trajectory_digest(
                    expected_ordinal,
                    &source_game_id,
                    &trajectory.start_sfen,
                    &trajectory.moves,
                    trajectory.endpoint_sfen.as_deref()
                ),
            "candidate trajectory hash differs at {}",
            expected_ordinal
        );
        if let Some(row) = accepted_row {
            let family = family.unwrap();
            accepted_families.insert(family.clone());
            accepted.push(AcceptedOpening {
                candidate_ordinal: expected_ordinal,
                source_game_id,
                opening_id: format!("{opening_prefix}{:03}", row + 1),
                sfen: trajectory.endpoint_sfen.unwrap(),
                family,
            });
            if accepted.len() == target {
                after_target = true;
            }
        }
        expected_ordinal += 1;
    }
    ensure!(
        accepted.len() == target,
        "candidate log accepted {}/{} openings",
        accepted.len(),
        target
    );
    ensure!(
        expected_ordinal <= maximum_attempts,
        "candidate log exceeds the 100,000 candidate cap"
    );
    Ok(accepted)
}

fn expected_decision(
    trajectory: &CandidateTrajectory,
    family: Option<&str>,
    exclusions: &crate::step12_openings::ExclusionState,
    accepted_families: &HashSet<String>,
    accepted_count: usize,
) -> (Option<&'static str>, Option<u64>) {
    if !trajectory.valid_endpoint {
        return (
            Some(if trajectory.moves.len() < 16 {
                "incomplete-trajectory"
            } else {
                "invalid-terminal-endpoint"
            }),
            None,
        );
    }
    let family = family.expect("valid candidate has a family");
    if exclusions.historical.contains(family) {
        (Some("historical-excluded-family"), None)
    } else if exclusions.training.contains(family) {
        (Some("new-training-family"), None)
    } else if exclusions.validation.contains(family) {
        (Some("new-validation-family"), None)
    } else if accepted_families.contains(family) {
        (Some("accepted-family-duplicate"), None)
    } else {
        (None, Some(accepted_count as u64))
    }
}

fn verify_suite_outputs(output_dir: &Path, accepted: &[AcceptedOpening]) -> Result<()> {
    let suite = output_dir.join("development.tsv");
    let text = fs::read_to_string(&suite)?;
    let mut rows = Vec::new();
    for (line_number, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() || raw.starts_with('#') {
            continue;
        }
        ensure!(
            raw.matches('\t').count() == 1,
            "development.tsv row {} has the wrong shape",
            line_number + 1
        );
        let (id, sfen) = raw.split_once('\t').unwrap();
        rows.push((id.to_string(), sfen.to_string()));
    }
    ensure!(
        rows.len() == accepted.len(),
        "development.tsv row count differs"
    );
    for (index, ((id, sfen), expected)) in rows.iter().zip(accepted).enumerate() {
        ensure!(
            id == &expected.opening_id,
            "development.tsv ID differs at row {}",
            index
        );
        ensure!(
            sfen == &expected.sfen,
            "development.tsv SFEN differs at row {}",
            index
        );
        ensure!(
            Board::from_sfen(sfen)
                .map_err(|error| anyhow!("invalid development SFEN: {error:?}"))?
                .to_string()
                == *sfen,
            "development.tsv SFEN is not canonical at row {}",
            index
        );
    }
    let sfen_lines = fs::read_to_string(output_dir.join("development.sfen"))?
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    ensure!(
        sfen_lines
            == accepted
                .iter()
                .map(|item| item.sfen.clone())
                .collect::<Vec<_>>(),
        "development.sfen differs from accepted rows"
    );
    for block in 0..4 {
        let lines = fs::read_to_string(output_dir.join(format!("blocks/block-{block:02}.sfen")))?
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let expected = accepted[block * 32..(block + 1) * 32]
            .iter()
            .map(|item| item.sfen.clone())
            .collect::<Vec<_>>();
        ensure!(
            lines == expected,
            "opening block {} differs from the accepted row slice",
            block
        );
    }
    Ok(())
}

fn verify_exclusions_receipt(
    path: &Path,
    exclusions: &crate::step12_openings::ExclusionState,
    accepted: &[AcceptedOpening],
) -> Result<()> {
    let value = load_json(path)?;
    ensure!(
        value["schema"] == crate::step12_openings::EXCLUSION_SCHEMA,
        "opening exclusions schema differs"
    );
    ensure!(
        value["status"] == "passed",
        "opening exclusions did not pass"
    );
    ensure!(
        value["familyDefinition"] == crate::step12_openings::FAMILY_DEFINITION,
        "opening family definition differs"
    );
    ensure!(
        value["historicalInventoryCount"] == exclusions.historical_input_count as u64,
        "opening historical inventory count differs"
    );
    ensure!(
        value["historicalUniqueFamilies"] == exclusions.historical.len() as u64,
        "opening historical family count differs"
    );
    ensure!(
        value["newTrainingUniqueFamilies"] == exclusions.training.len() as u64,
        "opening training family count differs"
    );
    ensure!(
        value["newValidationUniqueFamilies"] == exclusions.validation.len() as u64,
        "opening validation family count differs"
    );
    ensure!(
        value["acceptedOverlapCount"] == 0u64,
        "opening exclusions reports accepted overlap"
    );
    let listed = value["acceptedOpeningFamilies"]
        .as_array()
        .ok_or_else(|| anyhow!("opening exclusions accepted families are missing"))?;
    ensure!(
        listed
            == &accepted
                .iter()
                .map(|item| Value::from(item.family.clone()))
                .collect::<Vec<_>>(),
        "opening exclusions accepted families differ"
    );
    Ok(())
}

fn validate_schedule(
    spec: &Value,
    schedule: &Value,
    accepted: &[AcceptedOpening],
    fixture: &Value,
) -> Result<()> {
    ensure!(
        schedule["schema"] == "haitaka-anhoku-strength-first-step12-opening-schedule-v1",
        "opening schedule schema differs"
    );
    ensure!(
        schedule["status"] == "planned-before-training",
        "opening schedule status differs"
    );
    ensure!(
        schedule["totalGames"] == 512u64,
        "opening schedule total differs"
    );
    ensure!(
        schedule["pairTransformation"] == PAIR_TRANSFORMATION,
        "opening schedule transformation differs"
    );
    ensure!(
        schedule["pairingFixture"] == *fixture,
        "opening schedule fixture differs"
    );
    let rows = schedule["rows"]
        .as_array()
        .ok_or_else(|| anyhow!("opening schedule rows are missing"))?;
    ensure!(
        rows.len() == 512,
        "opening schedule has {} rows",
        rows.len()
    );
    let mut ordinal = 0u64;
    for block in 0..4 {
        for cell in ["early-selected", "u2688-control"] {
            for block_row in 0..32 {
                let row_index = block * 32 + block_row;
                let accepted_row = &accepted[row_index];
                let transformed = color_swap_anhoku_sfen(&accepted_row.sfen)?;
                for leg in 0..2 {
                    let row = &rows[ordinal as usize];
                    let expected_sfen = if leg == 0 {
                        accepted_row.sfen.clone()
                    } else {
                        transformed.clone()
                    };
                    let expected_a = if leg == 0 { "black" } else { "white" };
                    let expected_b = if leg == 0 { "white" } else { "black" };
                    ensure!(
                        row["executionOrdinal"] == ordinal,
                        "schedule execution ordinal differs at {}",
                        ordinal
                    );
                    ensure!(row["cell"] == cell, "schedule cell differs at {}", ordinal);
                    ensure!(
                        row["block"] == block as u64,
                        "schedule block differs at {}",
                        ordinal
                    );
                    ensure!(
                        row["openingRow"] == row_index as u64,
                        "schedule opening row differs at {}",
                        ordinal
                    );
                    ensure!(
                        row["blockRow"] == block_row as u64,
                        "schedule block row differs at {}",
                        ordinal
                    );
                    ensure!(
                        row["pairIndex"] == row_index as u64,
                        "schedule pair index differs at {}",
                        ordinal
                    );
                    ensure!(
                        row["leg"] == leg as u64,
                        "schedule leg differs at {}",
                        ordinal
                    );
                    ensure!(
                        row["openingId"] == accepted_row.opening_id,
                        "schedule opening ID differs at {}",
                        ordinal
                    );
                    ensure!(
                        row["sourceGameId"] == accepted_row.source_game_id,
                        "schedule source ID differs at {}",
                        ordinal
                    );
                    ensure!(
                        row["candidateOrdinal"] == accepted_row.candidate_ordinal,
                        "schedule candidate ordinal differs at {}",
                        ordinal
                    );
                    ensure!(
                        row["startSfen"] == expected_sfen,
                        "schedule SFEN differs at {}",
                        ordinal
                    );
                    ensure!(
                        row["startSfenSha256"] == sha256_bytes(expected_sfen.as_bytes()),
                        "schedule SFEN hash differs at {}",
                        ordinal
                    );
                    ensure!(
                        row["aColor"] == expected_a,
                        "schedule A color differs at {}",
                        ordinal
                    );
                    ensure!(
                        row["bColor"] == expected_b,
                        "schedule B color differs at {}",
                        ordinal
                    );
                    ensure!(
                        row["aEvaluator"] == "model-role" && row["bEvaluator"] == "handcrafted",
                        "schedule evaluator role differs at {}",
                        ordinal
                    );
                    ensure!(
                        row["movetimeMs"] == 100u64
                            && row["threads"] == 1u64
                            && row["concurrentGames"] == 1u64,
                        "schedule resource settings differ at {}",
                        ordinal
                    );
                    ensure!(
                        row["openingRandomPlies"] == 0u64
                            && row["openingOrder"] == "sequential"
                            && row["matchSeed"] == spec["matchSeed"]
                            && row["maxPlies"] == 200u64,
                        "schedule search settings differ at {}",
                        ordinal
                    );
                    ensure!(
                        row["pairTransformation"] == PAIR_TRANSFORMATION
                            && row["modelsBound"] == false,
                        "schedule binding differs at {}",
                        ordinal
                    );
                    ordinal += 1;
                }
            }
        }
    }
    Ok(())
}

fn verify_generation_counts(
    generation: &Value,
    candidate_path: &Path,
    accepted: usize,
) -> Result<()> {
    let lines = BufReader::new(File::open(candidate_path)?)
        .lines()
        .filter_map(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .count() as u64;
    ensure!(
        generation["attemptedCandidates"] == lines,
        "generation attempted count differs from candidate log"
    );
    ensure!(
        generation["acceptedPositions"] == accepted as u64,
        "generation accepted count differs"
    );
    ensure!(
        generation["rejectedCandidates"] == lines - accepted as u64,
        "generation rejected count differs"
    );
    ensure!(
        generation["lastCandidateOrdinal"] == lines - 1,
        "generation last ordinal differs"
    );
    Ok(())
}

fn load_json(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path).with_context(
        || format!("failed to read {}", path.display()),
    )?)?)
}

fn verify_receipt(value: &Value) -> Result<()> {
    let path = PathBuf::from(
        value["path"]
            .as_str()
            .ok_or_else(|| anyhow!("artifact receipt has no path"))?,
    );
    ensure!(
        path.is_file(),
        "receipt artifact is missing: {}",
        path.display()
    );
    ensure!(
        fs::metadata(&path)?.len()
            == value["bytes"]
                .as_u64()
                .ok_or_else(|| anyhow!("artifact receipt has no byte count"))?,
        "receipt byte count differs: {}",
        path.display()
    );
    ensure!(
        file_sha256(&path)? == value["sha256"].as_str().unwrap(),
        "receipt SHA-256 differs: {}",
        path.display()
    );
    Ok(())
}

fn file_receipt(path: &Path) -> Result<Value> {
    Ok(json!({"path": path, "bytes": fs::metadata(path)?.len(), "sha256": file_sha256(path)?}))
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes)?;
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

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

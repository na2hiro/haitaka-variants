//! Step 12 model-blind development-opening producer.
//!
//! This is deliberately separate from the historical opening producer.  It
//! consumes only the frozen family inventories and frozen corpus binaries,
//! uses one continuous locked RNG stream, and leaves a complete audit trail
//! for every candidate before publishing the four match blocks.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use haitaka::{Board, Color, Move, Piece};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::dataset::{ENTRY_BYTES, PACKED_SFEN_BYTES, unpack_board_from_training};
use crate::openings::color_swap_anhoku_sfen;

pub(crate) const SPEC_SCHEMA: &str = "haitaka-anhoku-strength-first-step12-opening-spec-v1";
pub(crate) const GENERATION_SCHEMA: &str =
    "haitaka-anhoku-strength-first-step12-opening-generation-v1";
pub(crate) const VALIDATION_SCHEMA: &str =
    "haitaka-anhoku-strength-first-step12-opening-validation-v1";
pub(crate) const FREEZE_SCHEMA: &str = "haitaka-anhoku-strength-first-step12-opening-freeze-v1";
pub(crate) const EXCLUSION_SCHEMA: &str =
    "haitaka-anhoku-strength-first-step12-opening-exclusions-v1";
pub(crate) const RNG_IDENTITY: &str = "rand-std-rng-0.9-lockfile-v1";
pub(crate) const FAMILY_DEFINITION: &str = "anhoku-canonical-color-swap-family-v1";
pub(crate) const PAIR_TRANSFORMATION: &str = "anhoku-rotate-180-and-swap-colors-v1";
pub(crate) const TRAJECTORY_DOMAIN: &[u8] =
    b"haitaka-anhoku-strength-first-step12-opening-trajectory-v1\0";

#[derive(Clone, Debug)]
pub(crate) struct CandidateTrajectory {
    pub(crate) start_sfen: String,
    pub(crate) moves: Vec<String>,
    pub(crate) endpoint_sfen: Option<String>,
    pub(crate) valid_endpoint: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ExclusionState {
    pub(crate) historical: HashSet<String>,
    pub(crate) training: HashSet<String>,
    pub(crate) validation: HashSet<String>,
    pub(crate) all: HashSet<String>,
    pub(crate) receipts: Vec<Value>,
    pub(crate) historical_input_count: usize,
    pub(crate) historical_rows: u64,
    pub(crate) training_records: u64,
    pub(crate) validation_records: u64,
}

pub(crate) fn generate_openings(spec_path: &Path, output_dir: &Path) -> Result<PathBuf> {
    let spec = load_spec(spec_path)?;
    validate_spec(&spec)?;
    ensure_output_absent(output_dir)?;
    let exclusions = load_exclusions(&spec)?;
    let pairing_fixture = pairing_fixture(&spec)?;

    fs::create_dir_all(output_dir)?;
    let candidates_path = output_dir.join("candidates.jsonl");
    let mut candidate_writer = BufWriter::new(File::create(&candidates_path)?);
    let seed = required_u64(&spec, "seed")?;
    let random_plies = required_u64(&spec, "randomPlies")? as u16;
    let target = required_u64(&spec, "acceptedPositions")? as usize;
    let maximum_attempts = required_u64(&spec, "maximumCandidateAttempts")?;
    let candidate_prefix = required_string(&spec, "candidateIdPrefix")?;
    let opening_prefix = required_string(&spec, "openingIdPrefix")?;
    let mut accepted = Vec::<AcceptedOpening>::with_capacity(target);
    let mut accepted_families = HashSet::with_capacity(target);
    let mut rng = StdRng::seed_from_u64(seed);
    let mut attempted = 0u64;

    while accepted.len() < target && attempted < maximum_attempts {
        let ordinal = attempted;
        attempted += 1;
        let source_game_id = format!("{candidate_prefix}{ordinal:06}");
        let trajectory = replay_candidate_with_rng(&mut rng, random_plies);
        let family = if trajectory.valid_endpoint {
            Some(canonical_family(
                &Board::from_sfen(trajectory.endpoint_sfen.as_deref().unwrap())
                    .map_err(|error| anyhow!("invalid generated endpoint SFEN: {error:?}"))?,
            )?)
        } else {
            None
        };
        let mut rejection = None;
        let mut accepted_row = None;
        if !trajectory.valid_endpoint {
            rejection = Some(if trajectory.moves.len() < random_plies as usize {
                "incomplete-trajectory"
            } else {
                "invalid-terminal-endpoint"
            });
        } else if let Some(family) = &family {
            if exclusions.historical.contains(family) {
                rejection = Some("historical-excluded-family");
            } else if exclusions.training.contains(family) {
                rejection = Some("new-training-family");
            } else if exclusions.validation.contains(family) {
                rejection = Some("new-validation-family");
            } else if accepted_families.contains(family) {
                rejection = Some("accepted-family-duplicate");
            } else {
                accepted_families.insert(family.clone());
                accepted_row = Some(accepted.len() as u64);
                accepted.push(AcceptedOpening {
                    candidate_ordinal: ordinal,
                    source_game_id: source_game_id.clone(),
                    opening_id: format!("{opening_prefix}{:03}", accepted.len() + 1),
                    sfen: trajectory.endpoint_sfen.clone().unwrap(),
                    family: family.clone(),
                });
            }
        }
        let candidate = json!({
            "candidateOrdinal": ordinal,
            "sourceGameId": source_game_id,
            "startSfen": trajectory.start_sfen,
            "moves": trajectory.moves,
            "endpointSfen": trajectory.endpoint_sfen,
            "canonicalFamily": family,
            "rejection": rejection,
            "acceptedRow": accepted_row,
            "trajectorySha256": trajectory_digest(
                ordinal,
                &source_game_id,
                &trajectory.start_sfen,
                &trajectory.moves,
                trajectory.endpoint_sfen.as_deref(),
            ),
        });
        serde_json::to_writer(&mut candidate_writer, &candidate)?;
        candidate_writer.write_all(b"\n")?;
    }
    candidate_writer.flush()?;
    ensure!(
        accepted.len() == target,
        "opening-shortfall: accepted {}/{} candidates after {} attempts",
        accepted.len(),
        target,
        attempted
    );

    let output_artifacts = write_opening_outputs(output_dir, &accepted)?;
    let exclusions_value = json!({
        "schema": EXCLUSION_SCHEMA,
        "status": "passed",
        "familyDefinition": FAMILY_DEFINITION,
        "historicalInventoryCount": exclusions.historical_input_count,
        "historicalRows": exclusions.historical_rows,
        "historicalUniqueFamilies": exclusions.historical.len(),
        "newTrainingRecords": exclusions.training_records,
        "newTrainingUniqueFamilies": exclusions.training.len(),
        "newValidationRecords": exclusions.validation_records,
        "newValidationUniqueFamilies": exclusions.validation.len(),
        "unionUniqueFamilies": exclusions.all.len(),
        "overlaps": {
            "historicalTraining": exclusions.historical.intersection(&exclusions.training).count(),
            "historicalValidation": exclusions.historical.intersection(&exclusions.validation).count(),
            "trainingValidation": exclusions.training.intersection(&exclusions.validation).count(),
        },
        "inputs": exclusions.receipts,
        "acceptedOpeningFamilies": accepted.iter().map(|row| row.family.clone()).collect::<Vec<_>>(),
        "acceptedOverlapCount": 0,
        "recursiveDiscoveryUsed": false,
    });
    let exclusions_path = output_dir.join("exclusions.json");
    write_json(&exclusions_path, &exclusions_value)?;

    let schedule = build_schedule(&spec, &accepted, &pairing_fixture)?;
    let schedule_path = output_dir.join("schedule.json");
    write_json(&schedule_path, &schedule)?;

    let generation = json!({
        "schema": GENERATION_SCHEMA,
        "status": "passed",
        "experimentId": spec["experimentId"],
        "seed": seed,
        "randomPlies": random_plies,
        "rng": RNG_IDENTITY,
        "legalMoveOrder": "Board::generate_moves callback order; no sorting",
        "maximumCandidateAttempts": maximum_attempts,
        "attemptedCandidates": attempted,
        "acceptedPositions": accepted.len(),
        "rejectedCandidates": attempted - accepted.len() as u64,
        "lastCandidateOrdinal": attempted - 1,
        "candidateLog": file_receipt(&candidates_path)?,
        "spec": file_receipt(spec_path)?,
        "exclusions": file_receipt(&exclusions_path)?,
        "outputs": output_artifacts,
        "pairingFixture": pairing_fixture,
        "modelBlind": true,
        "outcomeInputsUsed": false,
        "producer": {
            "implementation": "haitaka_learn/src/step12_openings.rs::generate_openings",
            "binary": file_receipt(&std::env::current_exe()?)?,
        },
    });
    let generation_path = output_dir.join("generation.json");
    write_json(&generation_path, &generation)?;

    let freeze = json!({
        "schema": FREEZE_SCHEMA,
        "status": "validated-before-publication",
        "experimentId": spec["experimentId"],
        "generation": file_receipt(&generation_path)?,
        "exclusions": file_receipt(&exclusions_path)?,
        "independentValidation": Value::Null,
        "schedule": file_receipt(&schedule_path)?,
        "outputs": output_artifacts,
        "pairingFixture": pairing_fixture,
        "modelsBound": false,
        "writtenLast": true,
        "flags": {
            "trainingOrGpuAttempted": false,
            "outcomesOpened": false,
            "confirmationAuthorized": false,
            "deploymentAuthorized": false,
            "handcraftedRemainsProduction": true,
        },
    });
    let freeze_path = output_dir.join("freeze.json");
    write_json(&freeze_path, &freeze)?;
    Ok(freeze_path)
}

#[derive(Clone)]
struct AcceptedOpening {
    candidate_ordinal: u64,
    source_game_id: String,
    opening_id: String,
    sfen: String,
    family: String,
}

pub(crate) fn load_spec(path: &Path) -> Result<Value> {
    let value: Value = serde_json::from_slice(
        &fs::read(path)
            .with_context(|| format!("failed to read opening spec {}", path.display()))?,
    )?;
    validate_spec(&value)?;
    Ok(value)
}

pub(crate) fn validate_spec(spec: &Value) -> Result<()> {
    ensure!(
        spec["schema"] == SPEC_SCHEMA,
        "unexpected Step 12 opening spec schema"
    );
    ensure!(
        spec["seed"] == 2026091606u64,
        "opening seed differs from the Step 12 contract"
    );
    ensure!(
        spec["acceptedPositions"] == 128u64,
        "opening quota differs from the Step 12 contract"
    );
    ensure!(
        spec["randomPlies"] == 16u64,
        "opening random plies differ from the Step 12 contract"
    );
    ensure!(
        spec["maximumCandidateAttempts"] == 100000u64,
        "opening candidate cap differs from the Step 12 contract"
    );
    ensure!(
        spec["rng"] == RNG_IDENTITY,
        "opening RNG identity differs from the Step 12 contract"
    );
    ensure!(
        spec["candidateIdPrefix"].as_str().is_some(),
        "candidate ID prefix is missing"
    );
    ensure!(
        spec["openingIdPrefix"].as_str().is_some(),
        "opening ID prefix is missing"
    );
    let historical = spec["inputs"]["historicalFamilyInventories"]
        .as_array()
        .ok_or_else(|| anyhow!("opening spec has no historical family inventories"))?;
    ensure!(
        !historical.is_empty(),
        "opening spec has no historical family inventories"
    );
    let corpora = spec["inputs"]["newCorpora"]
        .as_array()
        .ok_or_else(|| anyhow!("opening spec has no new corpus inputs"))?;
    ensure!(
        corpora.len() == 2,
        "opening spec must bind training and validation corpora"
    );
    for item in historical.iter().chain(corpora.iter()) {
        verify_artifact_value(item)?;
    }
    verify_artifact_value(&spec["inputs"]["canonicalizationSource"])?;
    verify_artifact_value(&spec["pairing"]["implementation"])?;
    if spec["pairing"]["documentation"].is_object() {
        verify_artifact_value(&spec["pairing"]["documentation"])?;
    }
    verify_artifact_value(&spec["build"]["binary"])?;
    verify_artifact_value(&spec["build"]["lockfile"])?;
    let sources = spec["build"]["sources"]
        .as_array()
        .ok_or_else(|| anyhow!("opening spec build sources are missing"))?;
    ensure!(!sources.is_empty(), "opening spec has no build sources");
    for source in sources {
        verify_artifact_value(source)?;
    }
    ensure!(
        spec["pairing"]["transformation"] == PAIR_TRANSFORMATION,
        "opening pair transformation differs from the contract"
    );
    Ok(())
}

pub(crate) fn load_exclusions(spec: &Value) -> Result<ExclusionState> {
    let mut historical = HashSet::new();
    let mut receipts = Vec::new();
    let mut historical_rows = 0u64;
    for item in spec["inputs"]["historicalFamilyInventories"]
        .as_array()
        .unwrap()
    {
        let path = artifact_path(item)?;
        let before = historical.len();
        let mut inventory_families = HashSet::new();
        let mut metadata = None;
        let mut family_rows = 0u64;
        let reader = BufReader::new(File::open(&path)?);
        for (line_number, raw) in reader.lines().enumerate() {
            let line = raw?;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(&line).with_context(|| {
                format!(
                    "invalid family inventory JSON {}:{}",
                    path.display(),
                    line_number + 1
                )
            })?;
            match value["kind"].as_str() {
                Some("metadata") => {
                    ensure!(
                        metadata.is_none(),
                        "duplicate inventory metadata in {}",
                        path.display()
                    );
                    ensure!(
                        value["schema"]
                            == "haitaka-anhoku-strength-first-step12-family-inventory-v1",
                        "unexpected family inventory schema in {}",
                        path.display()
                    );
                    metadata = Some(value);
                }
                Some("family") => {
                    let family = value["canonicalFamily"].as_str().ok_or_else(|| {
                        anyhow!(
                            "family inventory row has no canonicalFamily: {}",
                            path.display()
                        )
                    })?;
                    ensure!(
                        !family.is_empty(),
                        "family inventory contains an empty family: {}",
                        path.display()
                    );
                    inventory_families.insert(family.to_string());
                    historical.insert(family.to_string());
                    family_rows += 1;
                }
                other => bail!(
                    "unexpected family inventory row kind {other:?} in {}",
                    path.display()
                ),
            }
        }
        let metadata = metadata
            .ok_or_else(|| anyhow!("family inventory has no metadata: {}", path.display()))?;
        let input_rows = metadata["inputs"]
            .as_array()
            .ok_or_else(|| anyhow!("family inventory inputs are missing: {}", path.display()))?
            .iter()
            .try_fold(0u64, |total, input| {
                let rows = input["rows"].as_u64().ok_or_else(|| {
                    anyhow!(
                        "family inventory input rows are missing: {}",
                        path.display()
                    )
                })?;
                total
                    .checked_add(rows)
                    .ok_or_else(|| anyhow!("historical inventory row count overflow"))
            })?;
        validate_inventory_metadata(
            &metadata,
            &inventory_families,
            input_rows,
            family_rows,
            &path,
        )?;
        historical_rows = historical_rows
            .checked_add(input_rows)
            .ok_or_else(|| anyhow!("historical inventory row count overflow"))?;
        let mut receipt = file_receipt(&path)?;
        receipt["id"] = item["id"].clone();
        receipt["rows"] = input_rows.into();
        receipt["newCanonicalFamilies"] = (historical.len() - before).into();
        receipts.push(receipt);
    }

    let mut training = HashSet::new();
    let mut validation = HashSet::new();
    let mut training_records = 0u64;
    let mut validation_records = 0u64;
    for item in spec["inputs"]["newCorpora"].as_array().unwrap() {
        let role = required_string(item, "role")?;
        let path = artifact_path(item)?;
        let records = required_u64(item, "records")?;
        ensure!(
            fs::metadata(&path)?.len() == records * ENTRY_BYTES as u64,
            "new corpus byte count differs: {}",
            path.display()
        );
        let mut reader = BufReader::new(File::open(&path)?);
        let mut record = [0u8; ENTRY_BYTES];
        let target = if role == "training" {
            &mut training
        } else if role == "validation" {
            &mut validation
        } else {
            bail!("unknown new corpus role {role}")
        };
        for index in 0..records {
            reader.read_exact(&mut record).with_context(|| {
                format!("short new corpus record {} in {}", index, path.display())
            })?;
            let packed: [u8; PACKED_SFEN_BYTES] = record[..PACKED_SFEN_BYTES].try_into().unwrap();
            let board = unpack_board_from_training(&packed).with_context(|| {
                format!("invalid new corpus board {} in {}", index, path.display())
            })?;
            target.insert(canonical_family(&board)?);
        }
        ensure!(
            reader.fill_buf()?.is_empty(),
            "new corpus has trailing bytes: {}",
            path.display()
        );
        if role == "training" {
            training_records = records;
        } else {
            validation_records = records;
        }
        let mut receipt = file_receipt(&path)?;
        receipt["id"] = item["id"].clone();
        receipt["role"] = role.into();
        receipt["records"] = records.into();
        receipt["uniqueCanonicalFamilies"] = target.len().into();
        receipts.push(receipt);
    }
    ensure!(
        training_records == 880000,
        "new training corpus record count differs"
    );
    ensure!(
        validation_records == 60000,
        "new validation corpus record count differs"
    );
    ensure!(
        training.is_disjoint(&validation),
        "new training and validation corpus families overlap"
    );
    let mut all = historical.clone();
    all.extend(training.iter().cloned());
    all.extend(validation.iter().cloned());
    Ok(ExclusionState {
        historical_input_count: spec["inputs"]["historicalFamilyInventories"]
            .as_array()
            .unwrap()
            .len(),
        historical_rows,
        training_records,
        validation_records,
        historical,
        training,
        validation,
        all,
        receipts,
    })
}

fn validate_inventory_metadata(
    metadata: &Value,
    file_families: &HashSet<String>,
    input_rows: u64,
    family_rows: u64,
    path: &Path,
) -> Result<()> {
    ensure!(
        metadata["rows"].as_u64() == Some(input_rows),
        "family inventory input rows field differs: {}",
        path.display()
    );
    ensure!(
        metadata["uniqueCanonicalFamilies"] == file_families.len() as u64,
        "family inventory unique count differs: {}",
        path.display()
    );
    ensure!(
        family_rows == file_families.len() as u64,
        "family inventory family rows differ: {}",
        path.display()
    );
    Ok(())
}

pub(crate) fn replay_candidate(seed: u64, ordinal: u64, random_plies: u16) -> CandidateTrajectory {
    let mut rng = StdRng::seed_from_u64(seed);
    for _ in 0..ordinal {
        let _ = replay_candidate_with_rng(&mut rng, random_plies);
    }
    replay_candidate_with_rng(&mut rng, random_plies)
}

pub(crate) fn replay_candidate_with_rng(
    rng: &mut StdRng,
    random_plies: u16,
) -> CandidateTrajectory {
    let mut board = Board::startpos();
    let start_sfen = board.to_string();
    let mut moves = Vec::with_capacity(random_plies as usize);
    while moves.len() < random_plies as usize {
        if !has_both_kings(&board) {
            return CandidateTrajectory {
                start_sfen,
                moves,
                endpoint_sfen: None,
                valid_endpoint: false,
            };
        }
        let legal = legal_moves(&board);
        if legal.is_empty() {
            return CandidateTrajectory {
                start_sfen,
                moves,
                endpoint_sfen: None,
                valid_endpoint: false,
            };
        }
        let mv = legal[rng.random_range(0..legal.len())];
        moves.push(mv.to_string());
        board.play_unchecked(mv);
    }
    let endpoint_sfen = board.to_string();
    let valid_endpoint = has_both_kings(&board) && !legal_moves(&board).is_empty();
    CandidateTrajectory {
        start_sfen,
        moves,
        endpoint_sfen: Some(endpoint_sfen),
        valid_endpoint,
    }
}

pub(crate) fn trajectory_digest(
    ordinal: u64,
    source_game_id: &str,
    start_sfen: &str,
    moves: &[String],
    endpoint_sfen: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(TRAJECTORY_DOMAIN);
    hasher.update(ordinal.to_le_bytes());
    hasher.update(source_game_id.as_bytes());
    hasher.update([0]);
    hasher.update(start_sfen.as_bytes());
    hasher.update([0]);
    for mv in moves {
        hasher.update(mv.as_bytes());
        hasher.update([0]);
    }
    if let Some(endpoint) = endpoint_sfen {
        hasher.update(endpoint.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn write_opening_outputs(output_dir: &Path, accepted: &[AcceptedOpening]) -> Result<Value> {
    let suite_path = output_dir.join("development.tsv");
    let mut suite = String::from(
        "# Step 12 model-blind development opening suite\n# 16 legal random plies from startpos; seed 2026091606.\n# Format: <opening-id><TAB><SFEN>\n",
    );
    for row in accepted {
        suite.push_str(&format!("{}\t{}\n", row.opening_id, row.sfen));
    }
    fs::write(&suite_path, suite.as_bytes())?;

    let sfen_path = output_dir.join("development.sfen");
    fs::write(
        &sfen_path,
        accepted
            .iter()
            .map(|row| format!("{}\n", row.sfen))
            .collect::<String>(),
    )?;
    let blocks_dir = output_dir.join("blocks");
    fs::create_dir_all(&blocks_dir)?;
    let mut blocks = Vec::new();
    for block in 0..4 {
        let path = blocks_dir.join(format!("block-{block:02}.sfen"));
        let text = accepted[block * 32..(block + 1) * 32]
            .iter()
            .map(|row| format!("{}\n", row.sfen))
            .collect::<String>();
        fs::write(&path, text.as_bytes())?;
        blocks.push(json!({
            "block": block,
            "rowRange": [block * 32, (block + 1) * 32],
            "artifact": file_receipt(&path)?,
        }));
    }
    Ok(json!({
        "suite": file_receipt(&suite_path)?,
        "sfen": file_receipt(&sfen_path)?,
        "blocks": blocks,
        "rows": accepted.len(),
        "sourceGameIds": accepted.iter().map(|row| row.source_game_id.clone()).collect::<Vec<_>>(),
        "candidateOrdinals": accepted.iter().map(|row| row.candidate_ordinal).collect::<Vec<_>>(),
    }))
}

fn build_schedule(spec: &Value, accepted: &[AcceptedOpening], fixture: &Value) -> Result<Value> {
    let seed = required_u64(spec, "matchSeed")?;
    let mut rows = Vec::with_capacity(512);
    let mut execution_ordinal = 0u64;
    for block in 0..4 {
        for cell in ["early-selected", "u2688-control"] {
            for block_row in 0..32 {
                let row_index = block * 32 + block_row;
                let opening = &accepted[row_index];
                let transformed = color_swap_anhoku_sfen(&opening.sfen)?;
                for leg in 0..2 {
                    let start_sfen = if leg == 0 {
                        opening.sfen.clone()
                    } else {
                        transformed.clone()
                    };
                    let start_board = Board::from_sfen(&start_sfen)
                        .map_err(|error| anyhow!("invalid scheduled opening SFEN: {error:?}"))?;
                    let a_color = if leg == 0 { "black" } else { "white" };
                    let b_color = if leg == 0 { "white" } else { "black" };
                    rows.push(json!({
                        "executionOrdinal": execution_ordinal,
                        "cell": cell,
                        "block": block,
                        "openingRow": row_index,
                        "blockRow": block_row,
                        "pairIndex": row_index,
                        "leg": leg,
                        "gameIndexInCell": block * 64 + block_row * 2 + leg,
                        "openingId": opening.opening_id,
                        "sourceGameId": opening.source_game_id,
                        "candidateOrdinal": opening.candidate_ordinal,
                        "startSfen": start_sfen,
                        "startSfenSha256": sha256_bytes(start_sfen.as_bytes()),
                        "sideToMove": if start_board.side_to_move() == Color::Black { "black" } else { "white" },
                        "aEvaluator": "model-role",
                        "bEvaluator": "handcrafted",
                        "aColor": a_color,
                        "bColor": b_color,
                        "movetimeMs": 100,
                        "threads": 1,
                        "concurrentGames": 1,
                        "openingRandomPlies": 0,
                        "openingOrder": "sequential",
                        "matchSeed": seed,
                        "maxPlies": 200,
                        "pairTransformation": PAIR_TRANSFORMATION,
                        "modelsBound": false,
                    }));
                    execution_ordinal += 1;
                }
            }
        }
    }
    ensure!(rows.len() == 512, "opening schedule row count differs");
    Ok(json!({
        "schema": "haitaka-anhoku-strength-first-step12-opening-schedule-v1",
        "status": "planned-before-training",
        "cells": ["early-selected", "u2688-control"],
        "blocks": 4,
        "pairsPerCell": 128,
        "gamesPerCell": 256,
        "totalGames": rows.len(),
        "pairTransformation": PAIR_TRANSFORMATION,
        "pairingFixture": fixture,
        "rows": rows,
        "modelsBound": false,
        "outcomesOpened": false,
    }))
}

pub(crate) fn pairing_fixture(spec: &Value) -> Result<Value> {
    let pairing = &spec["pairing"];
    ensure!(
        pairing["transformation"] == PAIR_TRANSFORMATION,
        "pairing transformation is not the required contract"
    );
    let base = pairing["fixture"]["baseSfen"]
        .as_str()
        .ok_or_else(|| anyhow!("pairing fixture base SFEN is missing"))?;
    let board = Board::from_sfen(base)
        .map_err(|error| anyhow!("invalid pairing fixture SFEN: {error:?}"))?;
    ensure!(
        has_both_kings(&board),
        "pairing fixture base has no two kings"
    );
    let transformed = color_swap_anhoku_sfen(base)?;
    let swapped = Board::from_sfen(&transformed)
        .map_err(|error| anyhow!("invalid transformed pairing fixture SFEN: {error:?}"))?;
    ensure!(
        board.side_to_move() != swapped.side_to_move(),
        "pairing transformation did not change side to move"
    );
    ensure!(
        color_swap_anhoku_sfen(&transformed)? == base,
        "pairing transformation is not reversible"
    );
    if let Some(expected) = pairing["fixture"]["expectedTransformedSfen"].as_str() {
        ensure!(
            expected == transformed,
            "pairing fixture transformed SFEN differs from implementation"
        );
    }
    Ok(json!({
        "status": "passed",
        "contract": PAIR_TRANSFORMATION,
        "implementation": pairing["implementation"],
        "fixtureBaseSfen": base,
        "fixtureBaseSfenSha256": sha256_bytes(base.as_bytes()),
        "fixtureTransformedSfen": transformed,
        "fixtureTransformedSfenSha256": sha256_bytes(transformed.as_bytes()),
        "baseSideToMove": if board.side_to_move() == Color::Black { "black" } else { "white" },
        "transformedSideToMove": if swapped.side_to_move() == Color::Black { "black" } else { "white" },
        "baseLeg": {"aColor": "black", "bColor": "white"},
        "transformedLeg": {"aColor": "white", "bColor": "black"},
        "reversible": true,
        "nonPlaying": true,
    }))
}

pub(crate) fn canonical_family(board: &Board) -> Result<String> {
    crate::step11_opening_gate::canonical_family(board)
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

fn ensure_output_absent(path: &Path) -> Result<()> {
    ensure!(
        !path.exists(),
        "opening output directory already exists: {}",
        path.display()
    );
    ensure!(
        !path
            .to_string_lossy()
            .contains("step9-independent-confirmation-20260913-abandoned-1000ms-partial-unopened"),
        "protected historical path is forbidden"
    );
    Ok(())
}

fn required_u64(value: &Value, key: &str) -> Result<u64> {
    value[key]
        .as_u64()
        .ok_or_else(|| anyhow!("missing numeric {key}"))
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| anyhow!("missing string {key}"))
}

fn artifact_path(value: &Value) -> Result<PathBuf> {
    let path = value["path"]
        .as_str()
        .ok_or_else(|| anyhow!("artifact path is missing"))?;
    ensure!(
        !path.contains("step9-independent-confirmation-20260913-abandoned-1000ms-partial-unopened"),
        "protected historical path is forbidden"
    );
    Ok(PathBuf::from(path))
}

fn verify_artifact_value(value: &Value) -> Result<()> {
    let path = artifact_path(value)?;
    let bytes = required_u64(value, "bytes")?;
    let sha = required_string(value, "sha256")?;
    ensure!(
        path.is_file(),
        "missing opening input artifact {}",
        path.display()
    );
    ensure!(
        fs::metadata(&path)?.len() == bytes,
        "opening input byte count differs: {}",
        path.display()
    );
    ensure!(
        file_sha256(&path)? == sha,
        "opening input SHA-256 differs: {}",
        path.display()
    );
    Ok(())
}

fn file_receipt(path: &Path) -> Result<Value> {
    Ok(json!({
        "path": path,
        "bytes": fs::metadata(path)?.len(),
        "sha256": file_sha256(path)?,
    }))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step12_opening_replay_is_deterministic() {
        let first = replay_candidate(2026091606, 17, 16);
        let second = replay_candidate(2026091606, 17, 16);
        assert_eq!(first.start_sfen, second.start_sfen);
        assert_eq!(first.moves, second.moves);
        assert_eq!(first.endpoint_sfen, second.endpoint_sfen);
    }

    #[test]
    fn step12_opening_pair_fixture_rotates_and_swaps_side() {
        let base = Board::startpos();
        let transformed =
            Board::from_sfen(&color_swap_anhoku_sfen(&base.to_string()).unwrap()).unwrap();
        assert_ne!(base.side_to_move(), transformed.side_to_move());
        assert_eq!(
            color_swap_anhoku_sfen(&transformed.to_string()).unwrap(),
            base.to_string()
        );
    }

    #[test]
    fn step12_opening_trajectory_digest_changes_with_moves() {
        let a = trajectory_digest(1, "source", "start", &["7g7f".to_string()], Some("end"));
        let b = trajectory_digest(1, "source", "start", &["2g2f".to_string()], Some("end"));
        assert_ne!(a, b);
    }

    #[test]
    fn step12_opening_historical_inventory_overlap_uses_per_file_counts() {
        let first: HashSet<String> = ["family-a", "family-b"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let second: HashSet<String> = ["family-b", "family-c"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let first_metadata =
            json!({"inputs": [{"rows": 2}], "rows": 2, "uniqueCanonicalFamilies": 2});
        let second_metadata =
            json!({"inputs": [{"rows": 2}], "rows": 2, "uniqueCanonicalFamilies": 2});
        let path = Path::new("overlapping-fixture.jsonl");
        validate_inventory_metadata(&first_metadata, &first, 2, 2, path).unwrap();
        validate_inventory_metadata(&second_metadata, &second, 2, 2, path).unwrap();

        let mut global = first.clone();
        global.extend(second.iter().cloned());
        assert_eq!(global.len(), 3);

        let incorrect = json!({"inputs": [{"rows": 2}], "rows": 2, "uniqueCanonicalFamilies": 3});
        assert!(validate_inventory_metadata(&incorrect, &second, 2, 2, path).is_err());
    }
}

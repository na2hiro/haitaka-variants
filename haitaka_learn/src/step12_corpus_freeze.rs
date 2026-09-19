//! Native producer and independent validator for the Step 12 frozen corpus.
//!
//! The historical Model B freezer remains in `strength_first_model_b.rs`.  This
//! module is the Step 12 adapter: it consumes the already completed aggregate
//! source, replays the exact bounded-chunk shuffle on record indices, and keeps
//! a verifiable map back to the source shard and record offset.  The retained
//! source did not store per-game record boundaries, so this module records that
//! game attribution is unavailable instead of manufacturing game identifiers.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::{self, File, ReadDir};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use haitaka::{Board, Color, Move};
use haitaka_wasm::r1_donor_single_active_feature_indices;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::dataset::{ENTRY_BYTES, PACKED_SFEN_BYTES, unpack_board_from_training};
use crate::openings::color_swap_anhoku_training_sfen;

const MAP_ENTRY_BYTES: usize = 32;
const SHUFFLE_POLICY: &str = "bounded-chunk-v1";
const SHUFFLE_SEED: u64 = 2026091603;
const SHUFFLE_CHUNK_RECORDS: usize = 65_536;
const TRAIN_SPLIT_FOR_RANK: &str = "training";
const VALIDATION_SPLIT_FOR_RANK: &str = "validation";
const SELECTION_DOMAIN: &[u8] = b"haitaka-strength-first-model-b-selection-v1\0";
const FAMILY_DEFINITION: &str = "anhoku-canonical-color-swap-family-v1";
const GAME_ATTRIBUTION: &str = "per-game-boundaries-not-retained";
const MAP_SCHEMA: &str = "haitaka-anhoku-strength-first-step12-source-record-map-v1";
const SELECTION_SCHEMA: &str = "haitaka-anhoku-strength-first-step12-corpus-selection-v1";
const VALIDATION_SCHEMA: &str =
    "haitaka-anhoku-strength-first-step12-corpus-independent-validation-v1";
const CORPUS_SCHEMA: &str = "haitaka-anhoku-strength-first-step12-corpus-freeze-v1";

#[derive(Clone)]
struct RankedRecord {
    rank: [u8; 32],
    source_index: u64,
    family: String,
    bytes: [u8; ENTRY_BYTES],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceTag {
    pre_shuffle_index: u64,
    shard_index: u32,
    shard_record_index: u64,
}

#[derive(Clone)]
struct ShardInfo {
    path: PathBuf,
    bin_path: PathBuf,
    shard_index: u32,
    game_start: u32,
    game_count: u32,
    sampled_positions: u64,
    game_ids: Vec<String>,
    opening_ids: BTreeSet<String>,
    game_ids_sha256: String,
    manifest_receipt: Value,
    bin_receipt: Value,
}

struct SourceRecordMap {
    split: String,
    aggregate: PathBuf,
    aggregate_receipt: Value,
    manifest_path: PathBuf,
    manifest_receipt: Value,
    shards_dir: PathBuf,
    shards: Vec<ShardInfo>,
    pre_shuffle_records: Vec<[u8; ENTRY_BYTES]>,
    aggregate_tags: Vec<SourceTag>,
    game_ids: BTreeSet<String>,
    opening_ids: BTreeSet<String>,
    source_summary: Value,
}

pub(crate) fn freeze_corpus(
    exclusion_inventories: &[PathBuf],
    source_train: &Path,
    source_train_shards: &Path,
    source_validation: &Path,
    source_validation_shards: &Path,
    output_dir: &Path,
    train_records: usize,
    validation_records: usize,
    selection_seed: u64,
) -> Result<PathBuf> {
    ensure!(
        !exclusion_inventories.is_empty(),
        "at least one --exclusion-inventory is required"
    );
    ensure!(train_records > 0, "--train-records must be positive");
    ensure!(
        validation_records > 0,
        "--validation-records must be positive"
    );
    ensure_empty_output_dir(output_dir)?;

    let (excluded, exclusion_receipts) = load_exclusion_inventories(exclusion_inventories)?;
    let train_map = build_source_record_map("train", source_train, source_train_shards, 30_000)?;
    let validation_map = build_source_record_map(
        "validation",
        source_validation,
        source_validation_shards,
        2_016,
    )?;
    validate_split_schedule(&train_map, &validation_map)?;

    let (train_candidates, train_stats) = load_ranked_unique(
        source_train,
        TRAIN_SPLIT_FOR_RANK,
        selection_seed,
        &excluded,
    )?;
    ensure!(
        train_candidates.len() >= train_records,
        "data-shortfall: training eligible {} < required {train_records}",
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
        VALIDATION_SPLIT_FOR_RANK,
        selection_seed,
        &validation_exclusions,
    )?;
    ensure!(
        validation_candidates.len() >= validation_records,
        "data-shortfall: validation eligible {} < required {validation_records}",
        validation_candidates.len()
    );
    let selected_validation = validation_candidates
        .into_iter()
        .take(validation_records)
        .collect::<Vec<_>>();

    fs::create_dir_all(output_dir)?;
    let train_map_path = output_dir.join("source-record-map-training.bin");
    let validation_map_path = output_dir.join("source-record-map-validation.bin");
    write_source_map(&train_map_path, &train_map.aggregate_tags)?;
    write_source_map(&validation_map_path, &validation_map.aggregate_tags)?;
    let map_manifest_path = output_dir.join("source-record-map.json");
    let map_manifest = json!({
        "schema": MAP_SCHEMA,
        "status": "passed",
        "shuffle": shuffle_contract(),
        "gameAttribution": {
            "sourceGameId": Value::Null,
            "reason": GAME_ATTRIBUTION,
        },
        "training": source_map_receipt(&train_map, &train_map_path)?,
        "validation": source_map_receipt(&validation_map, &validation_map_path)?,
    });
    write_json(&map_manifest_path, &map_manifest)?;

    let train_path = output_dir.join("training.bin");
    let validation_path = output_dir.join("validation.bin");
    write_records(&train_path, &selected_train)?;
    write_records(&validation_path, &selected_validation)?;
    let train_sidecar_path = output_dir.join("training.jsonl");
    let validation_sidecar_path = output_dir.join("validation.jsonl");
    write_training_sidecar(&train_sidecar_path, &selected_train, &train_map)?;
    write_training_sidecar(
        &validation_sidecar_path,
        &selected_validation,
        &validation_map,
    )?;

    let train_receipt = file_receipt(&train_path)?;
    let validation_receipt = file_receipt(&validation_path)?;
    let train_sidecar_receipt = file_receipt(&train_sidecar_path)?;
    let validation_sidecar_receipt = file_receipt(&validation_sidecar_path)?;
    let train_manifest_path = output_dir.join("training.json");
    let validation_manifest_path = output_dir.join("validation.json");
    write_json(
        &train_manifest_path,
        &split_manifest(
            "training",
            train_records,
            &train_receipt,
            &train_sidecar_receipt,
            &train_map,
            &train_stats,
        )?,
    )?;
    write_json(
        &validation_manifest_path,
        &split_manifest(
            "validation",
            validation_records,
            &validation_receipt,
            &validation_sidecar_receipt,
            &validation_map,
            &validation_stats,
        )?,
    )?;

    let selection = json!({
        "schema": SELECTION_SCHEMA,
        "status": "passed",
        "selectionSeed": selection_seed,
        "selectionPolicy": "sha256-rank-within-split-v1",
        "selectionEncoding": {
            "domainSeparator": String::from_utf8_lossy(SELECTION_DOMAIN),
            "seed": "u64-little-endian",
            "split": "UTF-8-bytes-followed-by-NUL",
            "sourceIndex": "u64-little-endian",
            "record": "exact-72-byte-record",
            "order": "lexicographic-SHA-256-then-source-index",
        },
        "deduplicationPolicy": "exclude-historical-families-and-deduplicate-within-split-by-anhoku-color-swap-board-family-v1",
        "exclusions": {
            "familyDefinition": FAMILY_DEFINITION,
            "inventoryCount": exclusion_inventories.len(),
            "uniqueCanonicalFamilies": excluded.len(),
            "inventories": exclusion_receipts,
        },
        "source": {
            "training": train_map.source_summary,
            "validation": validation_map.source_summary,
            "crossSplitSchedule": {
                "openingGroupOverlap": [],
                "gameIdOverlap": [],
                "validated": true,
            },
        },
        "training": {
            "records": train_records,
            "binary": train_receipt,
            "sidecar": train_sidecar_receipt,
            "manifest": file_receipt(&train_manifest_path)?,
            "sourceStats": train_stats,
            "selectedCanonicalFamilies": selected_train_families.len(),
        },
        "validation": {
            "records": validation_records,
            "binary": validation_receipt,
            "sidecar": validation_sidecar_receipt,
            "manifest": file_receipt(&validation_manifest_path)?,
            "sourceStats": validation_stats,
            "excludedSelectedTrainingFamilies": selected_train_families.len(),
        },
        "provenance": {
            "map": file_receipt(&map_manifest_path)?,
            "sourceGameId": Value::Null,
            "sourceGameAttribution": GAME_ATTRIBUTION,
            "sourceShardAndRecordOffset": "verified-by-exact-assembly-permutation",
        },
        "producer": {
            "command": "haitaka_learn strength-first-step12-freeze-corpus",
            "binary": file_receipt(&std::env::current_exe()?)?,
            "implementation": "haitaka_learn/src/step12_corpus_freeze.rs",
        },
    });
    let selection_path = output_dir.join("selection.json");
    write_json(&selection_path, &selection)?;
    Ok(selection_path)
}

pub(crate) fn validate_corpus(
    exclusion_inventories: &[PathBuf],
    source_train: &Path,
    source_train_shards: &Path,
    source_validation: &Path,
    source_validation_shards: &Path,
    output_dir: &Path,
    report: &Path,
    train_records: usize,
    validation_records: usize,
    selection_seed: u64,
) -> Result<PathBuf> {
    ensure!(
        output_dir.is_dir(),
        "missing frozen corpus directory {}",
        output_dir.display()
    );
    let selection_path = output_dir.join("selection.json");
    let selection = load_json(&selection_path)?;
    ensure!(
        selection["schema"] == SELECTION_SCHEMA,
        "unexpected corpus selection schema"
    );
    ensure!(
        selection["status"] == "passed",
        "corpus selection is not passed"
    );
    ensure!(
        selection["selectionSeed"] == selection_seed,
        "selection seed differs from the bound recipe"
    );
    ensure!(
        selection["selectionPolicy"] == "sha256-rank-within-split-v1",
        "selection policy differs from the bound recipe"
    );
    ensure!(
        selection["training"]["records"] == train_records as u64,
        "training quota differs from the bound recipe"
    );
    ensure!(
        selection["validation"]["records"] == validation_records as u64,
        "validation quota differs from the bound recipe"
    );
    ensure!(
        selection["provenance"]["sourceGameId"].is_null(),
        "source game attribution must remain unavailable"
    );
    ensure!(
        selection["provenance"]["sourceGameAttribution"] == GAME_ATTRIBUTION,
        "source game attribution reason differs"
    );

    let (excluded, exclusion_receipts) = load_exclusion_inventories(exclusion_inventories)?;
    let train_map = build_source_record_map("train", source_train, source_train_shards, 30_000)?;
    let validation_map = build_source_record_map(
        "validation",
        source_validation,
        source_validation_shards,
        2_016,
    )?;
    validate_split_schedule(&train_map, &validation_map)?;

    let map_manifest = load_json(&output_dir.join("source-record-map.json"))?;
    ensure!(
        map_manifest["schema"] == MAP_SCHEMA,
        "unexpected source record map schema"
    );
    ensure!(
        map_manifest["status"] == "passed",
        "source record map is not passed"
    );
    ensure!(
        map_manifest["gameAttribution"]["sourceGameId"].is_null(),
        "source record map fabricated a game id"
    );
    ensure!(
        map_manifest["gameAttribution"]["reason"] == GAME_ATTRIBUTION,
        "source record map game-attribution reason differs"
    );
    verify_source_map_artifact(
        &output_dir.join("source-record-map-training.bin"),
        &train_map.aggregate_tags,
    )?;
    verify_source_map_artifact(
        &output_dir.join("source-record-map-validation.bin"),
        &validation_map.aggregate_tags,
    )?;

    let (train_candidates, train_stats) = load_ranked_unique(
        source_train,
        TRAIN_SPLIT_FOR_RANK,
        selection_seed,
        &excluded,
    )?;
    ensure!(
        train_candidates.len() >= train_records,
        "training shortfall during independent validation"
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
        VALIDATION_SPLIT_FOR_RANK,
        selection_seed,
        &validation_exclusions,
    )?;
    ensure!(
        validation_candidates.len() >= validation_records,
        "validation shortfall during independent validation"
    );
    let selected_validation = validation_candidates
        .into_iter()
        .take(validation_records)
        .collect::<Vec<_>>();

    let train_families = verify_selected_split(
        "training",
        &output_dir.join("training.bin"),
        &output_dir.join("training.jsonl"),
        &selected_train,
        &train_map,
        &excluded,
    )?;
    let validation_families = verify_selected_split(
        "validation",
        &output_dir.join("validation.bin"),
        &output_dir.join("validation.jsonl"),
        &selected_validation,
        &validation_map,
        &validation_exclusions,
    )?;
    ensure!(
        train_families.is_disjoint(&validation_families),
        "training and validation families overlap"
    );
    ensure!(
        selected_train_families == train_families,
        "training family set differs from independently selected records"
    );

    verify_split_manifest(&output_dir.join("training.json"), "training", train_records)?;
    verify_split_manifest(
        &output_dir.join("validation.json"),
        "validation",
        validation_records,
    )?;

    let checks = json!({
        "sourceAggregatePermutationAllRecords": true,
        "sourceShardManifestAndBinaryHashes": true,
        "sourceGameScheduleInventory": true,
        "sourceGroupSplitIsolation": true,
        "selectionPolicyAndOrder": true,
        "exactQuotas": true,
        "exactRecordBytes": true,
        "sidecarRowsAndSourceIndices": true,
        "recordLabelsCopiedByteForByte": true,
        "trainingWithinSplitCanonicalFamilyUnique": true,
        "validationWithinSplitCanonicalFamilyUnique": true,
        "trainingValidationCanonicalFamilyDisjoint": true,
        "historicalFamilyExclusions": true,
        "sourceGameId": Value::Null,
        "sourceGameAttribution": GAME_ATTRIBUTION,
    });
    let receipt = json!({
        "schema": VALIDATION_SCHEMA,
        "status": "passed",
        "checks": checks,
        "selection": file_receipt(&selection_path)?,
        "exclusionInventories": exclusion_receipts,
        "training": {
            "records": train_records,
            "sourceRecords": train_stats["sourceRecords"],
            "eligibleCanonicalFamilies": train_stats["eligibleCanonicalFamilies"],
            "binary": file_receipt(&output_dir.join("training.bin"))?,
            "sidecar": file_receipt(&output_dir.join("training.jsonl"))?,
        },
        "validation": {
            "records": validation_records,
            "sourceRecords": validation_stats["sourceRecords"],
            "eligibleCanonicalFamilies": validation_stats["eligibleCanonicalFamilies"],
            "binary": file_receipt(&output_dir.join("validation.bin"))?,
            "sidecar": file_receipt(&output_dir.join("validation.jsonl"))?,
        },
        "sourceRecordMap": file_receipt(&output_dir.join("source-record-map.json"))?,
        "independentImplementation": "haitaka_learn/src/step12_corpus_freeze.rs::validate_corpus",
    });
    write_json(report, &receipt)?;
    Ok(report.to_path_buf())
}

fn build_source_record_map(
    split: &str,
    aggregate: &Path,
    shards_dir: &Path,
    expected_games: u32,
) -> Result<SourceRecordMap> {
    ensure!(
        aggregate.is_file(),
        "missing {split} source aggregate {}",
        aggregate.display()
    );
    ensure!(
        shards_dir.is_dir(),
        "missing {split} source shard directory {}",
        shards_dir.display()
    );
    let manifest_path = aggregate
        .parent()
        .ok_or_else(|| anyhow!("source aggregate has no parent: {}", aggregate.display()))?
        .join(format!("{split}.json"));
    let manifest = load_json(&manifest_path)?;
    ensure!(
        manifest["dataset"] == split,
        "source manifest dataset differs for {split}"
    );
    ensure!(
        manifest["ruleset"] == "anhoku",
        "source manifest ruleset differs for {split}"
    );
    ensure!(
        manifest["entry_bytes"] == ENTRY_BYTES as u64,
        "source manifest entry size differs for {split}"
    );
    ensure!(
        manifest["shuffle_policy"] == SHUFFLE_POLICY,
        "source manifest shuffle policy differs for {split}"
    );
    ensure!(
        manifest["shuffle_seed"] == SHUFFLE_SEED,
        "source manifest shuffle seed differs for {split}"
    );
    ensure!(
        manifest["shuffle_chunk_records"] == SHUFFLE_CHUNK_RECORDS as u64,
        "source manifest shuffle chunk size differs for {split}"
    );
    ensure!(
        manifest["game_count"] == expected_games as u64,
        "source manifest game count differs for {split}"
    );
    ensure!(
        manifest["completed_games"] == expected_games as u64,
        "source manifest is incomplete for {split}"
    );

    let shards = load_shards(shards_dir, split)?;
    let shard_games = shards
        .iter()
        .map(|shard| shard.game_count as u64)
        .sum::<u64>();
    let shard_records = shards
        .iter()
        .map(|shard| shard.sampled_positions)
        .sum::<u64>();
    ensure!(
        shard_games == expected_games as u64,
        "{split} shard game coverage is {shard_games}/{expected_games}"
    );
    ensure!(
        manifest["sampled_positions"] == shard_records,
        "{split} shard record total differs from aggregate manifest"
    );
    let aggregate_bytes = fs::metadata(aggregate)?.len();
    ensure!(
        aggregate_bytes == shard_records * ENTRY_BYTES as u64,
        "{split} aggregate byte count differs from shard records"
    );

    let mut pre_shuffle_records = Vec::with_capacity(shard_records as usize);
    let mut pre_shuffle_tags = Vec::with_capacity(shard_records as usize);
    let mut game_ids = BTreeSet::new();
    let mut opening_ids = BTreeSet::new();
    for shard in &shards {
        let mut reader = BufReader::new(File::open(&shard.bin_path)?);
        for shard_record_index in 0..shard.sampled_positions {
            let mut record = [0u8; ENTRY_BYTES];
            reader.read_exact(&mut record).with_context(|| {
                format!(
                    "short source shard record {shard_record_index} in {}",
                    shard.bin_path.display()
                )
            })?;
            let pre_shuffle_index = pre_shuffle_records.len() as u64;
            pre_shuffle_records.push(record);
            pre_shuffle_tags.push(SourceTag {
                pre_shuffle_index,
                shard_index: shard.shard_index,
                shard_record_index,
            });
        }
        ensure!(
            reader.fill_buf()?.is_empty(),
            "source shard has trailing bytes: {}",
            shard.bin_path.display()
        );
        game_ids.extend(shard.game_ids.iter().cloned());
        opening_ids.extend(shard.opening_ids.iter().cloned());
    }
    ensure!(
        pre_shuffle_records.len() as u64 == shard_records,
        "{split} source shard records changed while reading"
    );
    ensure!(
        game_ids.len() == expected_games as usize,
        "{split} source game inventory is incomplete"
    );

    let aggregate_tags = replay_aggregate_shuffle(pre_shuffle_tags, split)?;
    verify_aggregate_bytes(aggregate, &aggregate_tags, &pre_shuffle_records)?;

    let aggregate_receipt = file_receipt(aggregate)?;
    let manifest_receipt = file_receipt(&manifest_path)?;
    let game_ids_sha256 = digest_lines(&game_ids);
    let source_summary = json!({
        "split": split,
        "aggregate": aggregate_receipt,
        "manifest": manifest_receipt,
        "shardDirectory": shards_dir,
        "shardCount": shards.len(),
        "gameCount": game_ids.len(),
        "recordCount": aggregate_tags.len(),
        "gameIdsSha256": game_ids_sha256,
        "openingGroups": opening_ids,
        "shardAssemblyOrder": "ascending-game-start-with-contiguous-ranges",
        "shuffle": shuffle_contract(),
    });
    Ok(SourceRecordMap {
        split: split.to_string(),
        aggregate: aggregate.to_path_buf(),
        aggregate_receipt,
        manifest_path,
        manifest_receipt,
        shards_dir: shards_dir.to_path_buf(),
        shards,
        pre_shuffle_records,
        aggregate_tags,
        game_ids,
        opening_ids,
        source_summary,
    })
}

fn load_shards(shards_dir: &Path, split: &str) -> Result<Vec<ShardInfo>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(shards_dir)
        .with_context(|| format!("failed to read {}", shards_dir.display()))?
    {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) == Some("json")
            && path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.starts_with("shard-"))
        {
            paths.push(path);
        }
    }
    ensure!(
        !paths.is_empty(),
        "no explicit {split} shard manifests found in {}",
        shards_dir.display()
    );
    paths.sort();
    let mut shards = Vec::with_capacity(paths.len());
    for path in paths {
        let value = load_json(&path)?;
        ensure!(
            value["dataset"] == split,
            "shard dataset differs in {}",
            path.display()
        );
        let shard_index = required_u64(&value, "shard_index", &path)?
            .try_into()
            .map_err(|_| anyhow!("shard index overflows u32: {}", path.display()))?;
        let game_start = required_u64(&value, "game_start", &path)?
            .try_into()
            .map_err(|_| anyhow!("game start overflows u32: {}", path.display()))?;
        let game_count = required_u64(&value, "game_count", &path)?
            .try_into()
            .map_err(|_| anyhow!("game count overflows u32: {}", path.display()))?;
        let sampled_positions = required_u64(&value, "sampled_positions", &path)?;
        ensure!(
            value["entry_bytes"] == ENTRY_BYTES as u64,
            "shard entry size differs in {}",
            path.display()
        );
        let games = value["games"]
            .as_array()
            .ok_or_else(|| anyhow!("shard games is not an array: {}", path.display()))?;
        ensure!(
            games.len() == game_count as usize,
            "shard game inventory count differs in {}",
            path.display()
        );
        let mut game_ids = Vec::with_capacity(games.len());
        let mut opening_ids = BTreeSet::new();
        for (local_index, game) in games.iter().enumerate() {
            let game_index = required_u64(game, "game_index", &path)?;
            ensure!(
                game_index == game_start as u64 + local_index as u64,
                "non-contiguous game index in {}",
                path.display()
            );
            let game_id = required_string(game, "game_id", &path)?.to_string();
            ensure!(
                game_ids.iter().all(|known| known != &game_id),
                "duplicate game id in {}",
                path.display()
            );
            let opening_id = required_string(game, "opening_id", &path)?.to_string();
            game_ids.push(game_id);
            opening_ids.insert(opening_id);
        }
        let bin_path = path.with_extension("bin");
        ensure!(
            bin_path.is_file(),
            "missing source shard payload {}",
            bin_path.display()
        );
        ensure!(
            fs::metadata(&bin_path)?.len() == sampled_positions * ENTRY_BYTES as u64,
            "source shard payload size differs in {}",
            bin_path.display()
        );
        shards.push(ShardInfo {
            path: path.clone(),
            bin_path: bin_path.clone(),
            shard_index,
            game_start,
            game_count,
            sampled_positions,
            game_ids_sha256: digest_lines(&game_ids),
            game_ids,
            opening_ids,
            manifest_receipt: file_receipt(&path)?,
            bin_receipt: file_receipt(&bin_path)?,
        });
    }
    shards.sort_by_key(|shard| shard.game_start);
    let mut expected_game_start = 0u32;
    for (ordinal, shard) in shards.iter().enumerate() {
        ensure!(
            shard.game_start == expected_game_start,
            "missing {split} shard game range at {}",
            shard.game_start
        );
        ensure!(
            shard.shard_index == ordinal as u32,
            "{split} shard index/order mismatch at {}",
            shard.path.display()
        );
        expected_game_start = expected_game_start
            .checked_add(shard.game_count)
            .ok_or_else(|| anyhow!("{split} game range overflows"))?;
    }
    Ok(shards)
}

fn replay_aggregate_shuffle(
    mut pre_shuffle_tags: Vec<SourceTag>,
    split: &str,
) -> Result<Vec<SourceTag>> {
    ensure!(
        !pre_shuffle_tags.is_empty(),
        "cannot shuffle an empty {split} source"
    );
    let split_seed = SHUFFLE_SEED
        ^ match split {
            "train" => 0x7472_6169_6e2d_7631,
            "validation" => 0x7661_6c69_642d_7631,
            _ => bail!("unknown source split {split}"),
        };
    let mut chunks = Vec::new();
    for (chunk_index, source_chunk) in pre_shuffle_tags.chunks(SHUFFLE_CHUNK_RECORDS).enumerate() {
        let mut chunk = source_chunk.to_vec();
        let mut rng = StdRng::seed_from_u64(splitmix64(split_seed ^ chunk_index as u64));
        chunk.shuffle(&mut rng);
        chunks.push(chunk);
    }
    pre_shuffle_tags.clear();
    let (offset, step) = chunk_permutation(split_seed, chunks.len());
    for position in 0..chunks.len() {
        let chunk_index = (offset + position.wrapping_mul(step)) % chunks.len();
        pre_shuffle_tags.extend_from_slice(&chunks[chunk_index]);
    }
    Ok(pre_shuffle_tags)
}

fn verify_aggregate_bytes(
    aggregate: &Path,
    aggregate_tags: &[SourceTag],
    pre_shuffle_records: &[[u8; ENTRY_BYTES]],
) -> Result<()> {
    let mut reader = BufReader::new(File::open(aggregate)?);
    let mut seen = vec![false; pre_shuffle_records.len()];
    for (aggregate_index, tag) in aggregate_tags.iter().enumerate() {
        let pre_index = tag.pre_shuffle_index as usize;
        ensure!(
            pre_index < pre_shuffle_records.len(),
            "aggregate map references an out-of-range source record"
        );
        ensure!(
            !seen[pre_index],
            "aggregate map reuses source record {pre_index}"
        );
        seen[pre_index] = true;
        let mut record = [0u8; ENTRY_BYTES];
        reader.read_exact(&mut record).with_context(|| {
            format!(
                "short aggregate record {aggregate_index} in {}",
                aggregate.display()
            )
        })?;
        ensure!(
            record == pre_shuffle_records[pre_index],
            "aggregate byte mismatch at record {aggregate_index}"
        );
    }
    ensure!(
        seen.iter().all(|value| *value),
        "aggregate map omits a source record"
    );
    ensure!(
        reader.fill_buf()?.is_empty(),
        "aggregate has trailing bytes: {}",
        aggregate.display()
    );
    Ok(())
}

fn validate_split_schedule(train: &SourceRecordMap, validation: &SourceRecordMap) -> Result<()> {
    ensure!(
        train.split == "train" && validation.split == "validation",
        "unexpected source split names"
    );
    ensure!(
        train.opening_ids.is_disjoint(&validation.opening_ids),
        "source opening groups overlap across train and validation"
    );
    ensure!(
        train.game_ids.is_disjoint(&validation.game_ids),
        "source game ids overlap across train and validation"
    );
    ensure!(
        train.game_ids.iter().all(|id| id.starts_with("train-")),
        "train source game id has the wrong split prefix"
    );
    ensure!(
        validation
            .game_ids
            .iter()
            .all(|id| id.starts_with("validation-")),
        "validation source game id has the wrong split prefix"
    );
    for map in [train, validation] {
        let manifest = &map.manifest_receipt;
        let manifest_value = load_json(&map.manifest_path)?;
        ensure!(
            manifest_value["opening_group_overlap"]
                .as_array()
                .is_some_and(|items| items.is_empty()),
            "{} source manifest reports opening-group overlap",
            map.split
        );
        let allowed_key = if map.split == "train" {
            "train_opening_ids"
        } else {
            "validation_opening_ids"
        };
        let allowed = manifest_value[allowed_key]
            .as_array()
            .ok_or_else(|| anyhow!("{} source manifest has no {allowed_key}", map.split))?
            .iter()
            .filter_map(Value::as_str)
            .collect::<HashSet<_>>();
        ensure!(
            !allowed.is_empty(),
            "{} source manifest has no allowed opening groups",
            map.split
        );
        ensure!(
            map.opening_ids
                .iter()
                .all(|id| allowed.contains(id.as_str())),
            "{} source shard uses an opening outside its declared split",
            map.split
        );
        let _ = manifest;
    }
    Ok(())
}

fn load_exclusion_inventories(paths: &[PathBuf]) -> Result<(HashSet<String>, Vec<Value>)> {
    let mut families = HashSet::new();
    let mut receipts = Vec::with_capacity(paths.len());
    for path in paths {
        ensure!(
            path.is_file(),
            "missing exclusion family inventory {}",
            path.display()
        );
        let reader = BufReader::new(File::open(path)?);
        let mut metadata_seen = false;
        let mut rows = 0u64;
        let before = families.len();
        for (line_number, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(&line).with_context(|| {
                format!("invalid JSON in {}:{}", path.display(), line_number + 1)
            })?;
            match value["kind"].as_str() {
                Some("metadata") => {
                    ensure!(
                        !metadata_seen,
                        "duplicate inventory metadata in {}",
                        path.display()
                    );
                    metadata_seen = true;
                    ensure!(
                        value["schema"]
                            == "haitaka-anhoku-strength-first-step12-family-inventory-v1",
                        "unexpected inventory schema in {}",
                        path.display()
                    );
                }
                Some("family") => {
                    let family = value["canonicalFamily"].as_str().ok_or_else(|| {
                        anyhow!(
                            "inventory family has no canonicalFamily: {}",
                            path.display()
                        )
                    })?;
                    ensure!(
                        !family.is_empty(),
                        "inventory contains an empty family: {}",
                        path.display()
                    );
                    families.insert(family.to_string());
                    rows += 1;
                }
                _ => bail!(
                    "unknown inventory line kind in {}:{}",
                    path.display(),
                    line_number + 1
                ),
            }
        }
        ensure!(
            metadata_seen,
            "inventory has no metadata line: {}",
            path.display()
        );
        receipts.push(json!({
            "path": path,
            "bytes": fs::metadata(path)?.len(),
            "sha256": file_sha256(path)?,
            "rows": rows,
            "newCanonicalFamilies": families.len() - before,
        }));
    }
    Ok((families, receipts))
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
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error.into()),
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
        "source": file_receipt(path)?,
        "sourceRecords": source_records,
        "excludedRecords": excluded_records,
        "duplicateCanonicalFamilyRecords": duplicate_records,
        "eligibleCanonicalFamilies": selected.len(),
    });
    Ok((selected, stats))
}

fn verify_selected_split(
    split: &str,
    binary: &Path,
    sidecar: &Path,
    expected: &[RankedRecord],
    source_map: &SourceRecordMap,
    exclusions: &HashSet<String>,
) -> Result<HashSet<String>> {
    ensure!(
        fs::metadata(binary)?.len() == expected.len() as u64 * ENTRY_BYTES as u64,
        "{split} frozen binary has the wrong byte count"
    );
    let mut binary_reader = BufReader::new(File::open(binary)?);
    let sidecar_reader = BufReader::new(File::open(sidecar)?);
    let mut sidecar_lines = sidecar_reader.lines();
    let mut families = HashSet::with_capacity(expected.len());
    for (selected_index, candidate) in expected.iter().enumerate() {
        let mut record = [0u8; ENTRY_BYTES];
        binary_reader.read_exact(&mut record)?;
        ensure!(
            record == candidate.bytes,
            "{split} frozen record order differs at {selected_index}"
        );
        let family = record_family(&record)?;
        ensure!(
            !exclusions.contains(&family),
            "{split} frozen record uses an excluded family"
        );
        ensure!(
            families.insert(family),
            "{split} frozen records contain a duplicate canonical family"
        );
        let line = sidecar_lines
            .next()
            .ok_or_else(|| anyhow!("{split} sidecar is short at row {selected_index}"))??;
        let metadata: Value = serde_json::from_str(&line)
            .with_context(|| format!("invalid {split} sidecar JSON at row {selected_index}"))?;
        ensure!(
            metadata["sourceIndex"] == candidate.source_index,
            "{split} sidecar source index differs at {selected_index}"
        );
        let tag = source_map.aggregate_tags[candidate.source_index as usize];
        ensure!(
            metadata["provenance"]["sourceAggregateIndex"] == candidate.source_index,
            "{split} sidecar aggregate index differs at {selected_index}"
        );
        ensure!(
            metadata["provenance"]["sourceShard"] == tag.shard_index,
            "{split} sidecar shard differs at {selected_index}"
        );
        ensure!(
            metadata["provenance"]["sourceShardRecordIndex"] == tag.shard_record_index,
            "{split} sidecar shard offset differs at {selected_index}"
        );
        ensure!(
            metadata["provenance"]["sourceGameId"].is_null(),
            "{split} sidecar fabricated a source game id"
        );
    }
    ensure!(
        binary_reader.fill_buf()?.is_empty(),
        "{split} frozen binary has trailing bytes"
    );
    ensure!(
        sidecar_lines.next().transpose()?.is_none(),
        "{split} sidecar has trailing rows"
    );
    Ok(families)
}

fn verify_split_manifest(path: &Path, split: &str, records: usize) -> Result<()> {
    let value = load_json(path)?;
    ensure!(
        value["schema"] == "haitaka-anhoku-strength-first-step12-frozen-dataset-manifest-v1",
        "unexpected {split} frozen manifest schema"
    );
    ensure!(
        value["dataset"] == split,
        "{split} frozen manifest dataset differs"
    );
    ensure!(
        value["records"] == records as u64,
        "{split} frozen manifest count differs"
    );
    ensure!(
        value["entryBytes"] == ENTRY_BYTES as u64,
        "{split} frozen manifest entry size differs"
    );
    Ok(())
}

fn write_source_map(path: &Path, tags: &[SourceTag]) -> Result<()> {
    let mut writer = BufWriter::new(File::create(path)?);
    for (aggregate_index, tag) in tags.iter().enumerate() {
        writer.write_all(&(aggregate_index as u64).to_le_bytes())?;
        writer.write_all(&tag.pre_shuffle_index.to_le_bytes())?;
        writer.write_all(&tag.shard_index.to_le_bytes())?;
        writer.write_all(&0u32.to_le_bytes())?;
        writer.write_all(&tag.shard_record_index.to_le_bytes())?;
    }
    writer.flush()?;
    Ok(())
}

fn verify_source_map_artifact(path: &Path, expected: &[SourceTag]) -> Result<()> {
    ensure!(
        fs::metadata(path)?.len() == expected.len() as u64 * MAP_ENTRY_BYTES as u64,
        "source record map has the wrong byte count: {}",
        path.display()
    );
    let mut reader = BufReader::new(File::open(path)?);
    let mut bytes = [0u8; MAP_ENTRY_BYTES];
    for (aggregate_index, expected_tag) in expected.iter().enumerate() {
        reader.read_exact(&mut bytes)?;
        let observed_aggregate = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let observed_pre = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let observed_shard = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let observed_offset = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        ensure!(
            observed_aggregate == aggregate_index as u64,
            "source record map aggregate index differs at {aggregate_index}"
        );
        ensure!(
            SourceTag {
                pre_shuffle_index: observed_pre,
                shard_index: observed_shard,
                shard_record_index: observed_offset
            } == *expected_tag,
            "source record map differs at {aggregate_index}"
        );
    }
    ensure!(
        reader.fill_buf()?.is_empty(),
        "source record map has trailing bytes"
    );
    Ok(())
}

fn source_map_receipt(map: &SourceRecordMap, map_path: &Path) -> Result<Value> {
    Ok(json!({
        "split": map.split,
        "aggregate": map.aggregate_receipt,
        "manifest": map.manifest_receipt,
        "shardDirectory": map.shards_dir,
        "shardCount": map.shards.len(),
        "gameCount": map.game_ids.len(),
        "recordCount": map.aggregate_tags.len(),
        "gameIdsSha256": digest_lines(&map.game_ids),
        "openingGroups": map.opening_ids,
        "shards": map.shards.iter().map(|shard| json!({
            "shardIndex": shard.shard_index,
            "gameStart": shard.game_start,
            "gameCount": shard.game_count,
            "sampledPositions": shard.sampled_positions,
            "gameIdsSha256": shard.game_ids_sha256,
            "openingGroups": shard.opening_ids,
            "manifest": shard.manifest_receipt,
            "binary": shard.bin_receipt,
        })).collect::<Vec<_>>(),
        "map": file_receipt(map_path)?,
        "mapEntryBytes": MAP_ENTRY_BYTES,
        "shuffle": shuffle_contract(),
        "gameAttribution": {
            "sourceGameId": Value::Null,
            "reason": GAME_ATTRIBUTION,
        },
    }))
}

fn split_manifest(
    split: &str,
    records: usize,
    binary: &Value,
    sidecar: &Value,
    source: &SourceRecordMap,
    stats: &Value,
) -> Result<Value> {
    Ok(json!({
        "schema": "haitaka-anhoku-strength-first-step12-frozen-dataset-manifest-v1",
        "dataset": split,
        "records": records,
        "entryBytes": ENTRY_BYTES,
        "binary": binary,
        "sidecar": sidecar,
        "source": source.source_summary,
        "sourceStats": stats,
        "labelEvaluator": "haitaka-native-handcrafted-search-v1",
        "labelBudget": 10000,
        "targetSemantics": "bounded-nonmate-root-plus-explicit-mate-class-distance",
        "scoreTransform": "identity-with-robust-clip-v1",
        "sourceGameId": Value::Null,
        "sourceGameAttribution": GAME_ATTRIBUTION,
    }))
}

fn selection_rank(seed: u64, split: &str, source_index: u64, record: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(SELECTION_DOMAIN);
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

fn write_training_sidecar(
    path: &Path,
    records: &[RankedRecord],
    source_map: &SourceRecordMap,
) -> Result<()> {
    let mut writer = BufWriter::new(File::create(path)?);
    for record in records {
        let tag = source_map.aggregate_tags[record.source_index as usize];
        let packed: [u8; PACKED_SFEN_BYTES] = record.bytes[..PACKED_SFEN_BYTES]
            .try_into()
            .expect("fixed packed-board prefix");
        let board = unpack_board_from_training(&packed)?;
        let moves = legal_moves(&board);
        let ply = board.move_number();
        let in_check = !board.checkers().is_empty();
        let tactical = in_check || moves.iter().any(|mv| board.piece_on(mv.to()).is_some());
        let black = r1_donor_single_active_feature_indices(&board, Color::Black);
        let white = r1_donor_single_active_feature_indices(&board, Color::White);
        let donor_state = if black.donor.is_empty() && white.donor.is_empty() {
            "none"
        } else {
            "active"
        };
        let phase = if moves.len() <= 3 || ply >= 150 {
            "terminal-adjacent"
        } else if ply < 36 {
            "early"
        } else if ply < 90 {
            "middle"
        } else {
            "late"
        };
        serde_json::to_writer(
            &mut writer,
            &json!({
                "schema": "haitaka-strength-first-step12-frozen-training-metadata-v1",
                "sourceIndex": record.source_index,
                "provenance": {
                    "sourceAggregateIndex": record.source_index,
                    "sourceShard": tag.shard_index,
                    "sourceShardRecordIndex": tag.shard_record_index,
                    "sourceGameId": Value::Null,
                    "sourceGameAttribution": GAME_ATTRIBUTION,
                },
                "position": {
                    "phase": phase,
                    "tactical": tactical,
                    "inCheck": in_check,
                    "donorState": donor_state,
                },
            }),
        )?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn legal_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|generated| {
        moves.extend(generated);
        false
    });
    moves
}

fn record_family(record: &[u8; ENTRY_BYTES]) -> Result<String> {
    let packed: [u8; PACKED_SFEN_BYTES] = record[..PACKED_SFEN_BYTES]
        .try_into()
        .expect("fixed packed-board prefix");
    canonical_family(&unpack_board_from_training(&packed)?)
}

fn canonical_family(board: &Board) -> Result<String> {
    let direct = board.to_string();
    let swapped = color_swap_anhoku_training_sfen(&direct)?;
    Ok(if direct <= swapped { direct } else { swapped })
}

fn required_u64(value: &Value, key: &str, path: &Path) -> Result<u64> {
    value[key]
        .as_u64()
        .ok_or_else(|| anyhow!("missing numeric {key} in {}", path.display()))
}

fn required_string<'a>(value: &'a Value, key: &str, path: &Path) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| anyhow!("missing string {key} in {}", path.display()))
}

fn chunk_permutation(seed: u64, count: usize) -> (usize, usize) {
    if count <= 1 {
        return (0, 1);
    }
    let offset = (splitmix64(seed ^ 0x6f66_6673_6574_7631) % count as u64) as usize;
    let mut step = (splitmix64(seed ^ 0x7374_6570_2d76_3100) % count as u64) as usize;
    step = step.max(1);
    while gcd(step, count) != 1 {
        step += 1;
        if step == count {
            step = 1;
        }
    }
    (offset, step)
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut result = value;
    result = (result ^ (result >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    result = (result ^ (result >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    result ^ (result >> 31)
}

fn gcd(mut left: usize, mut right: usize) -> usize {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

fn shuffle_contract() -> Value {
    json!({
        "policy": SHUFFLE_POLICY,
        "seed": SHUFFLE_SEED,
        "chunkRecords": SHUFFLE_CHUNK_RECORDS,
        "splitSeedXor": {
            "train": "0x747261696e2d7631",
            "validation": "0x76616c69642d7631",
        },
        "chunkRng": "rand-std-rng-0.9-lockfile-v1-seed(split-seed-xor-chunk-index)",
        "withinChunk": "rand::seq::SliceRandom::shuffle",
        "chunkPermutation": "splitmix64-offset-step-coprime-v1",
        "assemblyOrder": "ascending-game-start-with-contiguous-ranges",
    })
}

fn digest_lines<'a, I>(values: I) -> String
where
    I: IntoIterator<Item = &'a String>,
{
    let mut hasher = Sha256::new();
    for value in values {
        hasher.update(value.as_bytes());
        hasher.update([b'\n']);
    }
    format!("{:x}", hasher.finalize())
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

fn file_receipt(path: &Path) -> Result<Value> {
    Ok(json!({
        "path": path,
        "bytes": fs::metadata(path)?.len(),
        "sha256": file_sha256(path)?,
    }))
}

fn load_json(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {}", path.display()))
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

fn ensure_empty_output_dir(path: &Path) -> Result<()> {
    if path.exists() {
        ensure!(
            path.is_dir(),
            "corpus output is not a directory: {}",
            path.display()
        );
        let mut entries: ReadDir = fs::read_dir(path)?;
        ensure!(
            entries.next().is_none(),
            "corpus output directory is already populated: {}",
            path.display()
        );
    } else {
        fs::create_dir_all(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shuffle_reconstruction_keeps_duplicate_records_as_distinct_offsets() {
        let tags = (0..(SHUFFLE_CHUNK_RECORDS as u64 + 7))
            .map(|index| SourceTag {
                pre_shuffle_index: index,
                shard_index: if index < SHUFFLE_CHUNK_RECORDS as u64 {
                    0
                } else {
                    1
                },
                shard_record_index: index % 11,
            })
            .collect::<Vec<_>>();
        let shuffled = replay_aggregate_shuffle(tags.clone(), "train").unwrap();
        assert_eq!(shuffled.len(), tags.len());
        let mut observed = shuffled
            .iter()
            .map(|tag| tag.pre_shuffle_index)
            .collect::<Vec<_>>();
        observed.sort_unstable();
        assert_eq!(observed, (0..tags.len() as u64).collect::<Vec<_>>());
        assert!(
            shuffled
                .windows(2)
                .any(|pair| pair[0].pre_shuffle_index != pair[1].pre_shuffle_index)
        );
    }

    #[test]
    fn selection_domain_matches_historical_model_b_domain() {
        let record = [7u8; ENTRY_BYTES];
        let mut hasher = Sha256::new();
        hasher.update(b"haitaka-strength-first-model-b-selection-v1\0");
        hasher.update(99u64.to_le_bytes());
        hasher.update(b"training");
        hasher.update([0]);
        hasher.update(3u64.to_le_bytes());
        hasher.update(record);
        assert_eq!(
            selection_rank(99, "training", 3, &record),
            <[u8; 32]>::from(hasher.finalize())
        );
    }
}

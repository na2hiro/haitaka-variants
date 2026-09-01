use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail, ensure};
use haitaka::{Board, Color, GameStatus, Move, Piece, Square};
use haitaka_wasm::{
    R1_HALFKAV2_BASE_FEATURES, R1_SENTINEL_CONSTRUCTION, R1ActiveFeatureIndices, R1SentinelNetwork,
    handcrafted_static_eval, r1_donor_single_active_feature_indices, search_impl_handcrafted,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::{FEATURE_SET_DONOR_SINGLE, LoadedTrainingConfig};
use crate::dataset::{PACKED_SFEN_BYTES, pack_board_for_training, unpack_board_from_training};
use crate::openings::color_swap_anhoku_sfen;
use crate::trainer::PreparedTrainer;

const CORPUS_POSITIONS: usize = 10_240;
const ENTRY_BYTES: usize = 72;

#[derive(Debug, Clone)]
struct RawFixture {
    board: Board,
    parent: Option<usize>,
    move_from_parent: Option<Move>,
    seed_class: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FeaturePerspective<'a> {
    base: &'a [usize],
    donor: &'a [usize],
}

#[derive(Debug, Serialize)]
struct FeatureDump<'a> {
    id: &'a str,
    black: FeaturePerspective<'a>,
    white: FeaturePerspective<'a>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CorpusDump<'a> {
    id: &'a str,
    sfen: String,
    packed_hex: String,
    parent_id: Option<String>,
    move_from_parent: Option<String>,
    seed_class: Option<&'static str>,
    label_score_side_to_move: i16,
    output_bucket: usize,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Coverage {
    sides_to_move: BTreeMap<String, u64>,
    piece_types: BTreeMap<String, u64>,
    output_buckets: BTreeMap<usize, u64>,
    captures: u64,
    promotions: u64,
    drops: u64,
    king_moves: u64,
    checks: u64,
    double_checks: u64,
    terminal_adjacent: u64,
    maximum_legal_hands: u64,
    donor_gained: u64,
    donor_removed: u64,
    donor_replaced: u64,
    receiver_moved_with_relation_change: u64,
    expected_gold_like_identity_collisions: u64,
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
pub(crate) struct R1aReport {
    schema: &'static str,
    schema_version: u32,
    ruleset: &'static str,
    feature_family: &'static str,
    deterministic_order: &'static str,
    pub(crate) corpus_positions: usize,
    distinct_packed_positions: usize,
    pub(crate) transitions_checked: usize,
    restore_round_trips_checked: usize,
    packing_round_trips: usize,
    feature_signature_checks: usize,
    rust_cpp_feature_rows_checked: usize,
    rust_cpp_feature_mismatches: usize,
    color_swap_pairs_checked: usize,
    color_swap_score_mismatches: usize,
    side_to_move_score_checks: usize,
    side_to_move_score_mismatches: usize,
    depth_one_reference_checks: usize,
    depth_one_reference_mismatches: usize,
    incremental_accumulator_mismatches: usize,
    sentinel_construction: &'static str,
    sentinel_source_sha256: String,
    coverage: Coverage,
    artifacts: BTreeMap<String, ArtifactIdentity>,
    gates: BTreeMap<String, bool>,
    pub(crate) passed: bool,
}

pub(crate) fn run(
    config_path: &Path,
    output_dir: &Path,
    workspace_root: &Path,
) -> Result<R1aReport> {
    let loaded = LoadedTrainingConfig::from_path(config_path)?;
    loaded.ruleset_requires_matching_engine()?;
    ensure!(
        loaded.training_features() == FEATURE_SET_DONOR_SINGLE,
        "R1-A requires training.features={FEATURE_SET_DONOR_SINGLE}"
    );
    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;

    let fixtures = build_corpus()?;
    ensure!(fixtures.len() == CORPUS_POSITIONS);
    let ids = (0..fixtures.len())
        .map(|index| format!("r1a-{index:05}"))
        .collect::<Vec<_>>();

    let corpus_path = output_dir.join("parity-corpus.jsonl");
    let rust_features_path = output_dir.join("rust-features.jsonl");
    let cpp_features_path = output_dir.join("cpp-features.jsonl");
    let dataset_path = output_dir.join("parity-corpus.bin");
    let ids_path = output_dir.join("fixture-labels.tsv");
    let sentinel_path = output_dir.join("sentinel-network-metadata.json");
    let report_path = output_dir.join("r1a-gate-report.json");

    let mut corpus_writer = BufWriter::new(File::create(&corpus_path)?);
    let mut feature_writer = BufWriter::new(File::create(&rust_features_path)?);
    let mut dataset_writer = BufWriter::new(File::create(&dataset_path)?);
    let mut ids_writer = BufWriter::new(File::create(&ids_path)?);
    let mut coverage = Coverage::default();
    let mut packed_seen = BTreeSet::new();
    let mut feature_rows = Vec::with_capacity(fixtures.len());
    let mut packing_round_trips = 0usize;
    let mut feature_signature_checks = 0usize;
    let mut side_to_move_score_checks = 0usize;
    let mut side_to_move_score_mismatches = 0usize;

    for (index, fixture) in fixtures.iter().enumerate() {
        let id = &ids[index];
        let packed = pack_board_for_training(&fixture.board)?;
        ensure!(
            packed_seen.insert(packed),
            "duplicate packed board for {id}"
        );
        let decoded = unpack_board_from_training(&packed)?;
        ensure!(
            pack_board_for_training(&decoded)? == packed,
            "canonical packed round trip failed for {id}"
        );
        packing_round_trips += 1;
        let expected_signature = signature_for_board(&fixture.board);
        let decoded_signature = decode_signature_independently(&packed)?;
        ensure!(
            expected_signature == decoded_signature,
            "independent packed signature mismatch for {id}"
        );
        feature_signature_checks += 1;

        let black = r1_donor_single_active_feature_indices(&fixture.board, Color::Black);
        let white = r1_donor_single_active_feature_indices(&fixture.board, Color::White);
        serde_json::to_writer(
            &mut feature_writer,
            &FeatureDump {
                id,
                black: feature_perspective(&black),
                white: feature_perspective(&white),
            },
        )?;
        feature_writer.write_all(b"\n")?;
        feature_rows.push((black, white));

        let independent_score = independent_handcrafted_static_eval(&fixture.board);
        let runtime_score = handcrafted_static_eval(&fixture.board);
        side_to_move_score_checks += 1;
        side_to_move_score_mismatches += usize::from(independent_score != runtime_score);
        let stored_score = independent_score.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        write_r1a_training_entry(
            &mut dataset_writer,
            &packed,
            stored_score,
            fixture.board.move_number(),
        )?;
        // Packed runtime Black is Trainer White in the C++ ABI.
        let trainer_is_white = usize::from(fixture.board.side_to_move() == Color::Black);
        writeln!(ids_writer, "{id}\t{stored_score}\t{trainer_is_white}")?;

        update_position_coverage(&mut coverage, &fixture.board);
        if let (Some(parent), Some(mv)) = (fixture.parent, fixture.move_from_parent) {
            update_transition_coverage(
                &mut coverage,
                &fixtures[parent].board,
                &fixture.board,
                mv,
                &feature_rows[parent],
                &feature_rows[index],
            );
        }
        serde_json::to_writer(
            &mut corpus_writer,
            &CorpusDump {
                id,
                sfen: fixture.board.to_string(),
                packed_hex: hex(&packed),
                parent_id: fixture.parent.map(|parent| ids[parent].clone()),
                move_from_parent: fixture.move_from_parent.map(|mv| mv.to_string()),
                seed_class: fixture.seed_class,
                label_score_side_to_move: stored_score,
                output_bucket: output_bucket(&fixture.board),
            },
        )?;
        corpus_writer.write_all(b"\n")?;
    }
    corpus_writer.flush()?;
    feature_writer.flush()?;
    dataset_writer.flush()?;
    ids_writer.flush()?;
    ensure!(
        fs::metadata(&dataset_path)?.len() == (fixtures.len() * ENTRY_BYTES) as u64,
        "R1-A packed dataset has an unexpected byte length"
    );

    let mut color_swap_pairs_checked = 0usize;
    let mut color_swap_score_mismatches = 0usize;
    let packed_to_index = fixtures
        .iter()
        .enumerate()
        .map(|(index, fixture)| Ok((pack_board_for_training(&fixture.board)?, index)))
        .collect::<Result<HashMap<_, _>>>()?;
    for fixture in &fixtures {
        let swapped = Board::from_sfen(&color_swap_anhoku_sfen(&fixture.board.to_string())?)
            .map_err(|err| anyhow!("invalid generated color swap: {err}"))?;
        let swapped_packed = pack_board_for_training(&swapped)?;
        if packed_to_index.contains_key(&swapped_packed) {
            color_swap_pairs_checked += 1;
            if independent_handcrafted_static_eval(&fixture.board)
                != independent_handcrafted_static_eval(&swapped)
            {
                color_swap_score_mismatches += 1;
            }
        }
    }

    let (depth_one_reference_checks, depth_one_reference_mismatches) = depth_one_oracle()?;

    let sentinel_source = format!(
        "{}\n{}",
        R1_SENTINEL_CONSTRUCTION,
        fs::read_to_string(workspace_root.join("haitaka_wasm/src/nnue.rs"))?
    );
    let sentinel_source_sha256 = sha256_bytes(sentinel_source.as_bytes());
    fs::write(
        &sentinel_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "haitaka-r1a-sentinel-network-v1",
            "construction": R1_SENTINEL_CONSTRUCTION,
            "featureFamily": FEATURE_SET_DONOR_SINGLE,
            "baseRows": R1_HALFKAV2_BASE_FEATURES,
            "rowIdentityDimensions": 8,
            "sourceSha256": sentinel_source_sha256,
            "serialized": false,
            "scope": "R1-A feature-transformer accumulator parity; serialized parity is R1-B"
        }))?,
    )?;

    let sentinel = R1SentinelNetwork::donor_single();
    let mut states = Vec::with_capacity(fixtures.len());
    let mut transitions_checked = 0usize;
    let mut restore_round_trips_checked = 0usize;
    let mut incremental_accumulator_mismatches = 0usize;
    for (index, fixture) in fixtures.iter().enumerate() {
        let full = sentinel.build_position_state_full(&fixture.board);
        if let (Some(parent), Some(mv)) = (fixture.parent, fixture.move_from_parent) {
            let incremental =
                sentinel.apply_move(&fixtures[parent].board, &fixture.board, &states[parent], mv);
            transitions_checked += 1;
            incremental_accumulator_mismatches += usize::from(incremental != full);
            ensure!(
                pack_board_for_training(&fixtures[parent].board)?
                    == pack_board_for_training(&unpack_board_from_training(
                        &pack_board_for_training(&fixtures[parent].board)?
                    )?)?,
                "parent restore round trip failed at {}",
                ids[index]
            );
            restore_round_trips_checked += 1;
        }
        states.push(full);
    }
    drop(states);
    drop(sentinel);

    let trainer_checkout = loaded.trainer_checkout()?;
    let _prepared = PreparedTrainer::new(&loaded, &trainer_checkout)?;
    let loader_library = find_loader_library(&trainer_checkout)?;
    let helper = workspace_root.join("scripts/r1a-cpp-feature-oracle.py");
    let status = Command::new(&loaded.config.paths.python)
        .arg(&helper)
        .arg("--library")
        .arg(&loader_library)
        .arg("--dataset")
        .arg(&dataset_path)
        .arg("--ids")
        .arg(&ids_path)
        .arg("--output")
        .arg(&cpp_features_path)
        .arg("--base-rows")
        .arg(R1_HALFKAV2_BASE_FEATURES.to_string())
        .current_dir(workspace_root)
        .status()
        .with_context(|| format!("failed to run {}", helper.display()))?;
    ensure!(status.success(), "C++ R1-A feature oracle failed");
    let rust_feature_bytes = fs::read(&rust_features_path)?;
    let cpp_feature_bytes = fs::read(&cpp_features_path)?;
    let rust_cpp_feature_mismatches = usize::from(rust_feature_bytes != cpp_feature_bytes);

    validate_coverage(&coverage)?;
    let mut gates = BTreeMap::new();
    gates.insert(
        "atLeast10000LegalPositions".to_string(),
        fixtures.len() >= 10_000,
    );
    gates.insert(
        "canonicalPackingRoundTrip".to_string(),
        packing_round_trips == fixtures.len(),
    );
    gates.insert(
        "independentFeatureSignature".to_string(),
        feature_signature_checks == fixtures.len(),
    );
    gates.insert(
        "rustCppExactFeatureIndices".to_string(),
        rust_cpp_feature_mismatches == 0,
    );
    gates.insert(
        "colorSwapScoreTransform".to_string(),
        color_swap_pairs_checked >= 10_000 && color_swap_score_mismatches == 0,
    );
    gates.insert(
        "sideToMoveScoreOrientation".to_string(),
        side_to_move_score_mismatches == 0,
    );
    gates.insert(
        "depthOneReference".to_string(),
        depth_one_reference_checks > 0 && depth_one_reference_mismatches == 0,
    );
    gates.insert(
        "incrementalEqualsFullRefresh".to_string(),
        transitions_checked > 0 && incremental_accumulator_mismatches == 0,
    );
    gates.insert("requiredCoverage".to_string(), true);
    let passed = gates.values().all(|passed| *passed);

    let mut artifacts = BTreeMap::new();
    let gate_executable = std::env::current_exe().context("failed to identify R1-A executable")?;
    let gate_source = workspace_root.join("haitaka_learn/src/r1a.rs");
    let loader_overlay_source =
        workspace_root.join("haitaka_learn/trainer_overlay/training_data_loader.cpp");
    for (name, path) in [
        ("corpus", corpus_path.as_path()),
        ("packedDataset", dataset_path.as_path()),
        ("fixtureLabels", ids_path.as_path()),
        ("rustFeatures", rust_features_path.as_path()),
        ("cppFeatures", cpp_features_path.as_path()),
        ("sentinelMetadata", sentinel_path.as_path()),
        ("gateExecutable", gate_executable.as_path()),
        ("gateSource", gate_source.as_path()),
        ("cppOracleSource", helper.as_path()),
        ("cppLoaderOverlaySource", loader_overlay_source.as_path()),
        ("cppLoader", loader_library.as_path()),
    ] {
        artifacts.insert(name.to_string(), artifact_identity(path)?);
    }
    let report = R1aReport {
        schema: "haitaka-anhoku-r1a-gate",
        schema_version: 1,
        ruleset: "anhoku",
        feature_family: FEATURE_SET_DONOR_SINGLE,
        deterministic_order: "stable-id insertion order from curated seeds then UTF-8 move-sorted breadth-first expansion; no RNG, filtering, cycling, or sampling",
        corpus_positions: fixtures.len(),
        distinct_packed_positions: packed_seen.len(),
        transitions_checked,
        restore_round_trips_checked,
        packing_round_trips,
        feature_signature_checks,
        rust_cpp_feature_rows_checked: fixtures.len() * 2,
        rust_cpp_feature_mismatches,
        color_swap_pairs_checked,
        color_swap_score_mismatches,
        side_to_move_score_checks,
        side_to_move_score_mismatches,
        depth_one_reference_checks,
        depth_one_reference_mismatches,
        incremental_accumulator_mismatches,
        sentinel_construction: R1_SENTINEL_CONSTRUCTION,
        sentinel_source_sha256,
        coverage,
        artifacts,
        gates,
        passed,
    };
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    ensure!(passed, "R1-A gate failed; see {}", report_path.display());
    Ok(report)
}

fn feature_perspective(indices: &R1ActiveFeatureIndices) -> FeaturePerspective<'_> {
    FeaturePerspective {
        base: &indices.base,
        donor: &indices.donor,
    }
}

fn write_r1a_training_entry(
    writer: &mut impl Write,
    packed: &[u8; PACKED_SFEN_BYTES],
    score: i16,
    game_ply: u16,
) -> Result<()> {
    writer.write_all(packed)?;
    writer.write_all(&score.to_le_bytes())?;
    writer.write_all(&0u16.to_le_bytes())?;
    writer.write_all(&game_ply.to_le_bytes())?;
    writer.write_all(&[0, 0])?;
    Ok(())
}

fn build_corpus() -> Result<Vec<RawFixture>> {
    let mut fixtures = Vec::with_capacity(CORPUS_POSITIONS);
    let mut seen = HashMap::<[u8; PACKED_SFEN_BYTES], usize>::new();
    let mut queue = VecDeque::new();
    for (class, board) in curated_seeds()? {
        insert_with_swap(
            &mut fixtures,
            &mut seen,
            &mut queue,
            board,
            None,
            None,
            Some(class),
        )?;
    }
    while fixtures.len() < CORPUS_POSITIONS {
        let parent = queue
            .pop_front()
            .ok_or_else(|| anyhow!("deterministic corpus frontier exhausted"))?;
        let board = fixtures[parent].board.clone();
        let mut moves = legal_moves(&board);
        moves.sort_unstable_by_key(ToString::to_string);
        for mv in moves {
            let mut child = board.clone();
            child.play_unchecked(mv);
            insert_with_swap(
                &mut fixtures,
                &mut seen,
                &mut queue,
                child,
                Some(parent),
                Some(mv),
                None,
            )?;
            if fixtures.len() >= CORPUS_POSITIONS {
                break;
            }
        }
    }
    fixtures.truncate(CORPUS_POSITIONS);
    Ok(fixtures)
}

#[allow(clippy::too_many_arguments)]
fn insert_with_swap(
    fixtures: &mut Vec<RawFixture>,
    seen: &mut HashMap<[u8; PACKED_SFEN_BYTES], usize>,
    queue: &mut VecDeque<usize>,
    board: Board,
    parent: Option<usize>,
    mv: Option<Move>,
    seed_class: Option<&'static str>,
) -> Result<()> {
    let base_index = insert_fixture(fixtures, seen, queue, board.clone(), parent, mv, seed_class)?;
    if fixtures.len() >= CORPUS_POSITIONS {
        return Ok(());
    }
    let swapped = Board::from_sfen(&color_swap_anhoku_sfen(&board.to_string())?)
        .map_err(|err| anyhow!("invalid deterministic color swap: {err}"))?;
    let swapped_parent = parent.and_then(|parent| {
        let parent_swapped = color_swap_anhoku_sfen(&fixtures[parent].board.to_string()).ok()?;
        let parent_swapped = Board::from_sfen(&parent_swapped).ok()?;
        let packed = pack_board_for_training(&parent_swapped).ok()?;
        seen.get(&packed).copied()
    });
    let swapped_mv = mv.map(transform_move);
    let _ = insert_fixture(
        fixtures,
        seen,
        queue,
        swapped,
        swapped_parent,
        swapped_mv,
        seed_class,
    )?;
    let _ = base_index;
    Ok(())
}

fn insert_fixture(
    fixtures: &mut Vec<RawFixture>,
    seen: &mut HashMap<[u8; PACKED_SFEN_BYTES], usize>,
    queue: &mut VecDeque<usize>,
    board: Board,
    parent: Option<usize>,
    move_from_parent: Option<Move>,
    seed_class: Option<&'static str>,
) -> Result<Option<usize>> {
    let packed = pack_board_for_training(&board)?;
    if seen.contains_key(&packed) {
        return Ok(None);
    }
    let index = fixtures.len();
    seen.insert(packed, index);
    fixtures.push(RawFixture {
        board,
        parent,
        move_from_parent,
        seed_class,
    });
    queue.push_back(index);
    Ok(Some(index))
}

fn curated_seeds() -> Result<Vec<(&'static str, Board)>> {
    let sfens = [
        ("start-position", haitaka::SFEN_STARTPOS),
        ("capture", "9/9/k8/9/4Rr3/9/9/9/4K4 b - 1"),
        ("promotion", "k8/4P4/9/9/9/9/9/9/4K4 b - 1"),
        ("drop", "4k4/9/9/9/9/9/9/9/4K4 b P 1"),
        ("king-move", "4k4/9/9/9/9/9/9/9/4K4 b - 1"),
        ("donor-receiver", "4k4/9/9/4B4/4R4/9/9/9/4K4 b - 1"),
        ("check", "9/9/9/9/9/9/9/8k/3rK4 b - 1"),
        (
            "double-check",
            "ln2+r1r2/5s+Pkl/3+B1p1p1/p4B2p/2P6/P6PP/1PNP1P3/2G3SK1/L4G1NL w 2GSN3Ps3p 76",
        ),
        (
            "pinned",
            "ln3gsn1/7kl/3+B1p1p1/p4s2p/2P6/P2B3PP/1PNP+rPP2/2G3SK1/L4G1NL b G3Prs3p 65",
        ),
        (
            "terminal-adjacent",
            "lns4+Rl/1r1g5/p1p1pSp1p/1p1p1p3/8k/7NG/PPPPPPP1P/1B7/LNSGKGSNL w B2p 26",
        ),
        (
            "all-promoted-types",
            "4k4/9/9/9/9/9/+P+L+N+S+B+R3/9/4K4 b - 1",
        ),
        ("maximum-hand", "4k4/9/9/9/9/9/9/9/4K4 b 2R2B4G4S4N4L18P 1"),
    ];
    let mut seeds = Vec::new();
    for (class, sfen) in sfens {
        let board = Board::from_sfen(sfen)
            .map_err(|err| anyhow!("invalid curated R1-A seed `{class}`: {err}; {sfen}"))?;
        seeds.push((class, board));
    }
    for target_bucket in 0..8 {
        seeds.push(("output-bucket", output_bucket_seed(target_bucket)?));
    }
    Ok(seeds)
}

fn output_bucket_seed(target_bucket: usize) -> Result<Board> {
    let target_count = [2usize, 6, 11, 16, 21, 26, 31, 36][target_bucket];
    let start = Board::startpos();
    let pieces = Square::ALL
        .iter()
        .filter_map(|&square| start.colored_piece_on(square).map(|piece| (square, piece)))
        .collect::<Vec<_>>();
    let kings = pieces
        .iter()
        .copied()
        .filter(|(_, piece)| piece.piece == Piece::King)
        .collect::<Vec<_>>();
    let others = pieces
        .iter()
        .copied()
        .filter(|(_, piece)| piece.piece != Piece::King)
        .collect::<Vec<_>>();
    for shift in 0..others.len() {
        let mut candidate = Board::default();
        candidate.set_move_number(1);
        for (square, piece) in &kings {
            candidate.unchecked_put(piece.color, piece.piece, *square);
        }
        for offset in 0..target_count - 2 {
            let (square, piece) = others[(shift + offset) % others.len()];
            candidate.unchecked_put(piece.color, piece.piece, square);
        }
        if let Ok(board) = Board::from_sfen(&candidate.to_string())
            && output_bucket(&board) == target_bucket
        {
            return Ok(board);
        }
    }
    bail!("could not construct legal output-bucket {target_bucket} seed")
}

fn legal_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|piece_moves| {
        moves.extend(piece_moves);
        false
    });
    moves
}

fn transform_move(mv: Move) -> Move {
    match mv {
        Move::Drop { piece, to } => Move::Drop {
            piece,
            to: to.flip(),
        },
        Move::BoardMove {
            from,
            to,
            promotion,
        } => Move::BoardMove {
            from: from.flip(),
            to: to.flip(),
            promotion,
        },
    }
}

fn output_bucket(board: &Board) -> usize {
    let count = usize::try_from(board.occupied().len()).unwrap_or(0).max(1);
    (((count - 1) * 8) / 40).min(7)
}

fn update_position_coverage(coverage: &mut Coverage, board: &Board) {
    *coverage
        .sides_to_move
        .entry(format!("{:?}", board.side_to_move()).to_lowercase())
        .or_default() += 1;
    *coverage
        .output_buckets
        .entry(output_bucket(board))
        .or_default() += 1;
    for &piece in &Piece::ALL {
        let count = board.pieces(piece).len() as u64
            + u64::from(board.num_in_hand(Color::Black, piece))
            + u64::from(board.num_in_hand(Color::White, piece));
        if count > 0 {
            *coverage
                .piece_types
                .entry(format!("{piece:?}"))
                .or_default() += count;
        }
    }
    if !board.checkers().is_empty() {
        coverage.checks += 1;
    }
    if board.checkers().len() >= 2 {
        coverage.double_checks += 1;
    }
    let moves = legal_moves(board);
    if moves.len() <= 1
        || moves.iter().any(|&mv| {
            let mut child = board.clone();
            child.play_unchecked(mv);
            child.status() != GameStatus::Ongoing
        })
    {
        coverage.terminal_adjacent += 1;
    }
    if [Color::Black, Color::White].iter().any(|&color| {
        Piece::ALL[..Piece::HAND_NUM]
            .iter()
            .all(|&piece| board.num_in_hand(color, piece) == Piece::MAX_HAND[piece as usize])
    }) {
        coverage.maximum_legal_hands += 1;
    }
    coverage.expected_gold_like_identity_collisions +=
        [Piece::Tokin, Piece::PLance, Piece::PKnight, Piece::PSilver]
            .iter()
            .map(|&piece| board.pieces(piece).len() as u64)
            .sum::<u64>();
}

fn update_transition_coverage(
    coverage: &mut Coverage,
    parent: &Board,
    child: &Board,
    mv: Move,
    parent_features: &(R1ActiveFeatureIndices, R1ActiveFeatureIndices),
    child_features: &(R1ActiveFeatureIndices, R1ActiveFeatureIndices),
) {
    match mv {
        Move::Drop { .. } => coverage.drops += 1,
        Move::BoardMove {
            from,
            to,
            promotion,
        } => {
            coverage.captures += u64::from(parent.piece_on(to).is_some());
            coverage.promotions += u64::from(promotion);
            coverage.king_moves += u64::from(parent.piece_on(from) == Some(Piece::King));
        }
    }
    let parent_donor = parent_features
        .0
        .donor
        .iter()
        .chain(&parent_features.1.donor)
        .copied()
        .collect::<BTreeSet<_>>();
    let child_donor = child_features
        .0
        .donor
        .iter()
        .chain(&child_features.1.donor)
        .copied()
        .collect::<BTreeSet<_>>();
    let removed = parent_donor.difference(&child_donor).count();
    let added = child_donor.difference(&parent_donor).count();
    coverage.donor_gained += added as u64;
    coverage.donor_removed += removed as u64;
    coverage.donor_replaced += usize::min(removed, added) as u64;
    if matches!(mv, Move::BoardMove { .. }) && (removed > 0 || added > 0) {
        coverage.receiver_moved_with_relation_change += 1;
    }
    let _ = child;
}

fn validate_coverage(coverage: &Coverage) -> Result<()> {
    ensure!(
        coverage.sides_to_move.len() == 2,
        "both sides to move were not covered"
    );
    ensure!(
        coverage.piece_types.len() == Piece::NUM,
        "not every piece type was covered"
    );
    ensure!(
        coverage.output_buckets.len() == 8,
        "not every output bucket was covered"
    );
    for (name, value) in [
        ("captures", coverage.captures),
        ("promotions", coverage.promotions),
        ("drops", coverage.drops),
        ("king moves", coverage.king_moves),
        ("checks", coverage.checks),
        ("double checks", coverage.double_checks),
        ("terminal-adjacent positions", coverage.terminal_adjacent),
        ("maximum legal hands", coverage.maximum_legal_hands),
        ("donor gained", coverage.donor_gained),
        ("donor removed", coverage.donor_removed),
        ("donor replaced", coverage.donor_replaced),
        (
            "receiver relation movement",
            coverage.receiver_moved_with_relation_change,
        ),
        (
            "expected gold-like identity collisions",
            coverage.expected_gold_like_identity_collisions,
        ),
    ] {
        ensure!(value > 0, "R1-A corpus did not cover {name}");
    }
    Ok(())
}

fn independent_handcrafted_static_eval(board: &Board) -> i32 {
    let us = board.side_to_move();
    let them = !us;
    independent_material(board, us) - independent_material(board, them)
        // The frozen handcrafted evaluator defines mobility as destination
        // targets, not expanded promotion choices. Keep the independent
        // reference explicit about that subtle but intentional convention.
        + 2 * (mobility_targets(board) as i32
            - board
                .null_move()
                .map_or(0, |other| mobility_targets(&other)) as i32)
}

fn mobility_targets(board: &Board) -> usize {
    let mut count = 0;
    board.generate_moves(|moves| {
        count += moves.len();
        false
    });
    count
}

fn independent_material(board: &Board, color: Color) -> i32 {
    Piece::ALL
        .iter()
        .map(|&piece| {
            let value = match piece {
                Piece::Pawn => 100,
                Piece::Lance | Piece::Knight => 300,
                Piece::Silver => 400,
                Piece::Gold => 500,
                Piece::Bishop => 700,
                Piece::Rook => 800,
                Piece::King => 0,
                Piece::Tokin | Piece::PLance | Piece::PKnight | Piece::PSilver => 550,
                Piece::PBishop => 900,
                Piece::PRook => 1000,
            };
            value
                * (board.colored_pieces(color, piece).len() as i32
                    + i32::from(board.num_in_hand(color, piece)))
        })
        .sum()
}

fn depth_one_oracle() -> Result<(usize, usize)> {
    let boards = [
        Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b - 1").unwrap(),
        Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 w - 1").unwrap(),
    ];
    let mut checks = 0;
    let mut mismatches = 0;
    for board in boards {
        let expected = legal_moves(&board)
            .into_iter()
            .map(|mv| {
                let mut child = board.clone();
                child.play_unchecked(mv);
                -independent_handcrafted_static_eval(&child)
            })
            .max();
        let actual = search_impl_handcrafted(&board.to_string(), 1)
            .map_err(|err| anyhow!("depth-1 orientation search failed: {err}"))?
            .best_score;
        checks += 1;
        mismatches += usize::from(expected != actual);
    }
    Ok((checks, mismatches))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PackedSignature {
    side_to_move: u8,
    white_king: usize,
    black_king: usize,
    board: [Option<(u8, usize)>; 81],
    hands: [[u8; 10]; 2],
    fullmove: u16,
}

fn signature_for_board(board: &Board) -> PackedSignature {
    let mut trainer_board = [None; 81];
    for &square in &Square::ALL {
        if let Some(piece) = board.colored_piece_on(square)
            && piece.piece != Piece::King
        {
            trainer_board[trainer_square(square)] =
                Some((trainer_color(piece.color), trainer_piece_type(piece.piece)));
        }
    }
    let mut hands = [[0u8; 10]; 2];
    for &color in &[Color::Black, Color::White] {
        for &piece in &Piece::ALL[..Piece::HAND_NUM] {
            hands[trainer_color(color) as usize][trainer_piece_type(piece)] =
                board.num_in_hand(color, piece);
        }
    }
    PackedSignature {
        side_to_move: trainer_color(board.side_to_move()),
        white_king: trainer_square(board.king(Color::Black)),
        black_king: trainer_square(board.king(Color::White)),
        board: trainer_board,
        hands,
        fullmove: board.move_number(),
    }
}

fn decode_signature_independently(packed: &[u8; PACKED_SFEN_BYTES]) -> Result<PackedSignature> {
    let mut reader = IndependentBitReader::new(packed);
    let side_to_move = u8::from(reader.bit()?);
    let white_king = reader.bits(7)? as usize;
    let black_king = reader.bits(7)? as usize;
    ensure!(white_king < 81 && black_king < 81 && white_king != black_king);
    let mut board = [None; 81];
    for rank in (0..9).rev() {
        for file in 0..9 {
            let square = rank * 9 + file;
            if square == white_king || square == black_king {
                continue;
            }
            if reader.bit()? {
                let mut code = 1u32;
                for shift in 1..5 {
                    code |= u32::from(reader.bit()?) << shift;
                }
                ensure!(
                    code <= 19 && code % 2 == 1,
                    "invalid independent piece code"
                );
                let color = u8::from(reader.bit()?);
                board[square] = Some((color, ((code - 1) / 2) as usize));
            }
        }
    }
    let mut hands = [[0u8; 10]; 2];
    for color in &mut hands {
        for count in color {
            *count = reader.bits(5)? as u8;
        }
    }
    for _ in 0..5 {
        ensure!(!reader.bit()?, "unexpected castling/en-passant bit");
    }
    ensure!(reader.bits(6)? == 0, "unexpected rule50 bits");
    let low = reader.bits(8)? as u16;
    let high = reader.bits(8)? as u16;
    ensure!(!reader.bit()?, "unexpected rule50 high bit");
    Ok(PackedSignature {
        side_to_move,
        white_king,
        black_king,
        board,
        hands,
        fullmove: low | (high << 8),
    })
}

struct IndependentBitReader<'a> {
    bytes: &'a [u8; PACKED_SFEN_BYTES],
    cursor: usize,
}

impl<'a> IndependentBitReader<'a> {
    fn new(bytes: &'a [u8; PACKED_SFEN_BYTES]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn bit(&mut self) -> Result<bool> {
        ensure!(
            self.cursor < self.bytes.len() * 8,
            "independent decoder overflow"
        );
        let value = ((self.bytes[self.cursor / 8] >> (self.cursor % 8)) & 1) != 0;
        self.cursor += 1;
        Ok(value)
    }

    fn bits(&mut self, count: usize) -> Result<u32> {
        let mut value = 0;
        for shift in 0..count {
            value |= u32::from(self.bit()?) << shift;
        }
        Ok(value)
    }
}

fn trainer_color(color: Color) -> u8 {
    match color {
        Color::Black => 0,
        Color::White => 1,
    }
}

fn trainer_square(square: Square) -> usize {
    (8 - square.file() as usize) + (8 - square.rank() as usize) * 9
}

fn trainer_piece_type(piece: Piece) -> usize {
    match piece {
        Piece::Bishop => 0,
        Piece::Rook => 1,
        Piece::Silver => 2,
        Piece::PRook => 3,
        Piece::Pawn => 4,
        Piece::Lance => 5,
        Piece::Knight => 6,
        Piece::Gold | Piece::Tokin | Piece::PLance | Piece::PKnight | Piece::PSilver => 7,
        Piece::PBishop => 8,
        Piece::King => 9,
    }
}

fn find_loader_library(checkout: &Path) -> Result<PathBuf> {
    for name in [
        "libtraining_data_loader.so",
        "libtraining_data_loader.dylib",
        "training_data_loader.dll",
    ] {
        let path = checkout.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    bail!(
        "built C++ loader library not found under {}",
        checkout.display()
    )
}

fn artifact_identity(path: &Path) -> Result<ArtifactIdentity> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(ArtifactIdentity {
        path: path.display().to_string(),
        bytes: bytes.len() as u64,
        sha256: sha256_bytes(&bytes),
    })
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_signature_matches_start_position() {
        let board = Board::startpos();
        let packed = pack_board_for_training(&board).unwrap();
        assert_eq!(
            decode_signature_independently(&packed).unwrap(),
            signature_for_board(&board)
        );
    }

    #[test]
    fn curated_seeds_cover_every_output_bucket() {
        let seeds = curated_seeds().unwrap();
        let buckets = seeds
            .iter()
            .map(|(_, board)| output_bucket(board))
            .collect::<BTreeSet<_>>();
        assert_eq!(buckets, (0..8).collect());
    }

    #[test]
    fn corpus_is_deterministic_and_large_enough() {
        let first = build_corpus().unwrap();
        let second = build_corpus().unwrap();
        assert_eq!(first.len(), CORPUS_POSITIONS);
        assert_eq!(
            first
                .iter()
                .map(|fixture| pack_board_for_training(&fixture.board).unwrap())
                .collect::<Vec<_>>(),
            second
                .iter()
                .map(|fixture| pack_board_for_training(&fixture.board).unwrap())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn independent_handcrafted_oracle_matches_runtime() {
        for fixture in build_corpus().unwrap() {
            assert_eq!(
                independent_handcrafted_static_eval(&fixture.board),
                handcrafted_static_eval(&fixture.board),
                "{}",
                fixture.board
            );
        }
    }
}

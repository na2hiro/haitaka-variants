use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, ensure};
use haitaka::{Board, Move};
use haitaka_wasm::{
    NnueModel, SearchEvalMode, donor_receiver_pair_v2_stats,
    migrate_donor_single_to_receiver_pair_v2, search_impl_with_eval_mode,
};
use rand::prelude::IndexedRandom;
use rand::{SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const RANDOM_SEED: u64 = 0x11a0_d0a0_2026_0829;
const RANDOM_GAMES: usize = 16;
const RANDOM_PLIES: usize = 48;
const BENCH_ROUNDS: usize = 9;
const BENCH_REPETITIONS: usize = 200;

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
    #[serde(default = "default_depth")]
    depth: u8,
    expected_bestmove: String,
    purpose: String,
}

const fn default_depth() -> u8 {
    2
}

#[derive(Debug, Serialize)]
pub struct Phase11aReport {
    schema: &'static str,
    schema_version: u32,
    source: ArtifactIdentity,
    migrated: ArtifactIdentity,
    feature_geometry: FeatureGeometry,
    equivalence: EquivalenceReport,
    tactical_suite: TacticalReport,
    inference: InferenceReport,
    gates: GateReport,
}

impl Phase11aReport {
    pub fn phase11b_go(&self) -> bool {
        self.gates.phase11b_go
    }
}

#[derive(Debug, Serialize)]
struct ArtifactIdentity {
    path: String,
    sha256: String,
    bytes: u64,
    feature_family: String,
}

#[derive(Debug, Serialize)]
struct FeatureGeometry {
    v1_real_features: usize,
    v2_real_features: usize,
    increase_percent: f64,
    donor_features_per_influenced_piece: usize,
}

#[derive(Debug, Serialize)]
struct EquivalenceReport {
    representative_positions: usize,
    randomized_games: usize,
    randomized_positions: usize,
    incremental_transitions: usize,
    fixed_depth_searches: usize,
    score_mismatches: usize,
    accumulator_mismatches: usize,
    search_mismatches: usize,
    passed: bool,
}

#[derive(Debug, Serialize)]
struct TacticalReport {
    suite_path: String,
    suite_sha256: String,
    schema: String,
    schema_version: u32,
    fixtures: Vec<TacticalResult>,
    passed: bool,
}

#[derive(Debug, Serialize)]
struct TacticalResult {
    id: String,
    purpose: String,
    depth: u8,
    expected_bestmove: String,
    v1_bestmove: Option<String>,
    v2_bestmove: Option<String>,
    v1_best_score: Option<i32>,
    v2_best_score: Option<i32>,
    passed: bool,
}

#[derive(Debug, Serialize)]
struct InferenceReport {
    corpus_positions: usize,
    rounds: usize,
    repetitions_per_round: usize,
    v1_median_ns_per_position: f64,
    v2_median_ns_per_position: f64,
    regression_percent: f64,
}

#[derive(Debug, Serialize)]
struct GateReport {
    model_size_increase_percent: f64,
    model_size_at_most_10_percent: bool,
    inference_regression_at_most_5_percent: bool,
    equivalence_passed: bool,
    tactical_suite_passed: bool,
    phase11b_go: bool,
}

pub fn run(
    source_path: &Path,
    migrated_path: &Path,
    suite_path: &Path,
    report_path: &Path,
) -> Result<Phase11aReport> {
    let source_bytes = fs::read(source_path)
        .with_context(|| format!("failed to read {}", source_path.display()))?;
    let migrated_bytes = migrate_donor_single_to_receiver_pair_v2(&source_bytes)
        .map_err(|err| anyhow!("failed to migrate V1 network: {err}"))?;
    if let Some(parent) = migrated_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(migrated_path, &migrated_bytes)
        .with_context(|| format!("failed to write {}", migrated_path.display()))?;

    let source_model = Arc::new(
        NnueModel::from_bytes(&source_bytes)
            .map_err(|err| anyhow!("failed to load V1 network: {err}"))?,
    );
    let migrated_model = Arc::new(
        NnueModel::from_bytes(&migrated_bytes)
            .map_err(|err| anyhow!("failed to load migrated V2 network: {err}"))?,
    );
    ensure!(
        source_model.feature_family_name() == "HalfKAv2^+DonorSingleEff",
        "source network has unexpected family {}",
        source_model.feature_family_name()
    );
    ensure!(
        migrated_model.feature_family_name() == "HalfKAv2^+DonorReceiverPairV2",
        "migrated network has unexpected family {}",
        migrated_model.feature_family_name()
    );

    let suite_bytes =
        fs::read(suite_path).with_context(|| format!("failed to read {}", suite_path.display()))?;
    let suite: TacticalSuite = serde_json::from_slice(&suite_bytes)
        .with_context(|| format!("failed to parse {}", suite_path.display()))?;
    ensure!(
        suite.ruleset == "anhoku",
        "tactical suite must target anhoku"
    );
    ensure!(
        !suite.fixtures.is_empty(),
        "tactical suite must not be empty"
    );

    let representative: Vec<Board> = suite
        .fixtures
        .iter()
        .map(|fixture| {
            Board::from_sfen(&fixture.sfen)
                .map_err(|err| anyhow!("invalid tactical SFEN {}: {err}", fixture.id))
        })
        .collect::<Result<_>>()?;
    let mut score_mismatches = 0;
    let mut accumulator_mismatches = 0;
    for board in &representative {
        let v1 = source_model.build_position_state_full(board);
        let v2 = migrated_model.build_position_state_full(board);
        accumulator_mismatches += usize::from(v1 != v2);
        score_mismatches += usize::from(
            source_model.evaluate_from_state(board, &v1)
                != migrated_model.evaluate_from_state(board, &v2),
        );
    }

    let mut rng = StdRng::seed_from_u64(RANDOM_SEED);
    let mut randomized_positions = 0;
    let mut incremental_transitions = 0;
    for _ in 0..RANDOM_GAMES {
        let mut board = Board::startpos();
        let mut v1_state = source_model.build_position_state_full(&board);
        let mut v2_state = migrated_model.build_position_state_full(&board);
        for _ in 0..RANDOM_PLIES {
            let legal = legal_moves(&board);
            let Some(&mv) = legal.choose(&mut rng) else {
                break;
            };
            let mut child = board.clone();
            child.play_unchecked(mv);
            let v1_incremental = source_model.apply_move(&board, &child, &v1_state, mv);
            let v2_incremental = migrated_model.apply_move(&board, &child, &v2_state, mv);
            let v1_full = source_model.build_position_state_full(&child);
            let v2_full = migrated_model.build_position_state_full(&child);
            accumulator_mismatches += usize::from(
                v1_incremental != v1_full
                    || v2_incremental != v2_full
                    || v1_incremental != v2_incremental,
            );
            let v1_score = source_model.evaluate_from_state(&child, &v1_incremental);
            let v2_score = migrated_model.evaluate_from_state(&child, &v2_incremental);
            score_mismatches += usize::from(v1_score != v2_score);
            randomized_positions += 1;
            incremental_transitions += 1;
            board = child;
            v1_state = v1_incremental;
            v2_state = v2_incremental;
        }
    }

    let mut search_mismatches = 0;
    let mut tactical_rows = Vec::with_capacity(suite.fixtures.len());
    for fixture in &suite.fixtures {
        let v1 = search_impl_with_eval_mode(
            &fixture.sfen,
            fixture.depth,
            source_model.clone(),
            SearchEvalMode::Incremental,
        )
        .map_err(|err| anyhow!("V1 search failed for {}: {err}", fixture.id))?;
        let v2 = search_impl_with_eval_mode(
            &fixture.sfen,
            fixture.depth,
            migrated_model.clone(),
            SearchEvalMode::Incremental,
        )
        .map_err(|err| anyhow!("V2 search failed for {}: {err}", fixture.id))?;
        let equivalent = v1.best_move == v2.best_move && v1.best_score == v2.best_score;
        search_mismatches += usize::from(!equivalent);
        let passed =
            equivalent && v1.best_move.as_deref() == Some(fixture.expected_bestmove.as_str());
        tactical_rows.push(TacticalResult {
            id: fixture.id.clone(),
            purpose: fixture.purpose.clone(),
            depth: fixture.depth,
            expected_bestmove: fixture.expected_bestmove.clone(),
            v1_bestmove: v1.best_move,
            v2_bestmove: v2.best_move,
            v1_best_score: v1.best_score,
            v2_best_score: v2.best_score,
            passed,
        });
    }

    let (v1_ns, v2_ns) = benchmark_full_refresh(&source_model, &migrated_model, &representative);
    let inference_regression = (v2_ns / v1_ns - 1.0) * 100.0;
    let source_len = source_bytes.len() as u64;
    let migrated_len = migrated_bytes.len() as u64;
    let model_size_increase = (migrated_len as f64 / source_len as f64 - 1.0) * 100.0;
    let equivalence_passed =
        score_mismatches == 0 && accumulator_mismatches == 0 && search_mismatches == 0;
    let tactical_passed = tactical_rows.iter().all(|row| row.passed);
    let size_passed = model_size_increase <= 10.0;
    let inference_passed = inference_regression <= 5.0;
    let phase11b_go = equivalence_passed && tactical_passed && size_passed && inference_passed;
    let stats = donor_receiver_pair_v2_stats();

    let report = Phase11aReport {
        schema: "haitaka-anhoku-phase11a-gate",
        schema_version: 1,
        source: artifact_identity(source_path, &source_bytes, &source_model),
        migrated: artifact_identity(migrated_path, &migrated_bytes, &migrated_model),
        feature_geometry: FeatureGeometry {
            v1_real_features: stats.v1_real_features,
            v2_real_features: stats.v2_real_features,
            increase_percent: (stats.v2_real_features as f64 / stats.v1_real_features as f64 - 1.0)
                * 100.0,
            donor_features_per_influenced_piece: 1,
        },
        equivalence: EquivalenceReport {
            representative_positions: representative.len(),
            randomized_games: RANDOM_GAMES,
            randomized_positions,
            incremental_transitions,
            fixed_depth_searches: suite.fixtures.len(),
            score_mismatches,
            accumulator_mismatches,
            search_mismatches,
            passed: equivalence_passed,
        },
        tactical_suite: TacticalReport {
            suite_path: suite_path.display().to_string(),
            suite_sha256: sha256_hex(&suite_bytes),
            schema: suite.schema,
            schema_version: suite.schema_version,
            fixtures: tactical_rows,
            passed: tactical_passed,
        },
        inference: InferenceReport {
            corpus_positions: representative.len(),
            rounds: BENCH_ROUNDS,
            repetitions_per_round: BENCH_REPETITIONS,
            v1_median_ns_per_position: v1_ns,
            v2_median_ns_per_position: v2_ns,
            regression_percent: inference_regression,
        },
        gates: GateReport {
            model_size_increase_percent: model_size_increase,
            model_size_at_most_10_percent: size_passed,
            inference_regression_at_most_5_percent: inference_passed,
            equivalence_passed,
            tactical_suite_passed: tactical_passed,
            phase11b_go,
        },
    };
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(report_path, serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("failed to write {}", report_path.display()))?;
    Ok(report)
}

fn artifact_identity(path: &Path, bytes: &[u8], model: &NnueModel) -> ArtifactIdentity {
    ArtifactIdentity {
        path: path.display().to_string(),
        sha256: sha256_hex(bytes),
        bytes: bytes.len() as u64,
        feature_family: model.feature_family_name().to_string(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn legal_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|piece_moves| {
        moves.extend(piece_moves);
        false
    });
    moves
}

fn benchmark_full_refresh(v1: &NnueModel, v2: &NnueModel, positions: &[Board]) -> (f64, f64) {
    let mut v1_rounds = Vec::with_capacity(BENCH_ROUNDS);
    let mut v2_rounds = Vec::with_capacity(BENCH_ROUNDS);
    for round in 0..BENCH_ROUNDS {
        let measure = |model: &NnueModel| {
            let started = Instant::now();
            for _ in 0..BENCH_REPETITIONS {
                for board in positions {
                    black_box(model.evaluate_full_refresh(black_box(board)));
                }
            }
            started.elapsed().as_nanos() as f64 / (BENCH_REPETITIONS * positions.len()) as f64
        };
        if round % 2 == 0 {
            v1_rounds.push(measure(v1));
            v2_rounds.push(measure(v2));
        } else {
            v2_rounds.push(measure(v2));
            v1_rounds.push(measure(v1));
        }
    }
    v1_rounds.sort_by(f64::total_cmp);
    v2_rounds.sort_by(f64::total_cmp);
    (v1_rounds[BENCH_ROUNDS / 2], v2_rounds[BENCH_ROUNDS / 2])
}

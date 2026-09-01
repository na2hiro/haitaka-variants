use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, ensure};
use haitaka::{Board, Color, Move, Piece};
use haitaka_wasm::{NnueModel, r1_donor_single_active_feature_indices};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::dataset::pack_board_for_training;

#[derive(Debug, Deserialize)]
struct Contract {
    schema: String,
    device: String,
    gpu_allowed: bool,
    input_artifacts: BTreeMap<String, FrozenArtifact>,
    overfit_corpus: OverfitCorpus,
}

#[derive(Debug, Deserialize)]
struct FrozenArtifact {
    path: PathBuf,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct OverfitCorpus {
    positions: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorpusRow {
    id: String,
    sfen: String,
    parent_id: Option<String>,
    move_from_parent: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeExpectation {
    id: String,
    integer_score: i32,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeParity {
    positions: usize,
    python_integer_score_mismatches: usize,
    transitions: usize,
    full_incremental_accumulator_mismatches: usize,
    full_incremental_score_mismatches: usize,
    restored_parent_snapshot_mismatches: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CollisionPair {
    id: String,
    base_sfen: String,
    promoted_sfen: String,
    packed_bytes_equal: bool,
    black_features_equal: bool,
    white_features_equal: bool,
    absolute_handcrafted_material_delta: i32,
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
pub(crate) struct R1cReport {
    schema: &'static str,
    schema_version: u32,
    ruleset: &'static str,
    feature_family: &'static str,
    device: &'static str,
    corpus_positions: usize,
    python_oracle: serde_json::Value,
    runtime_parity: RuntimeParity,
    collision_pairs: Vec<CollisionPair>,
    artifacts: BTreeMap<String, ArtifactIdentity>,
    gates: BTreeMap<String, bool>,
    pub(crate) passed: bool,
}

pub(crate) struct RunArgs<'a> {
    pub r1a_dir: &'a Path,
    pub r1b_dir: &'a Path,
    pub output_dir: &'a Path,
    pub contract_path: &'a Path,
    pub limits_path: &'a Path,
    pub python: &'a Path,
    pub workspace_root: &'a Path,
}

pub(crate) fn run(args: RunArgs<'_>) -> Result<R1cReport> {
    let contract: Contract = read_json(args.contract_path)?;
    ensure!(
        contract.schema == "haitaka-r1c-learnability-contract-v1",
        "wrong R1-C contract schema"
    );
    ensure!(contract.device == "cpu" && !contract.gpu_allowed);
    ensure!(contract.overfit_corpus.positions == 8_192);
    validate_frozen_inputs(&contract, args.workspace_root)?;

    let r1a_report_path = args.r1a_dir.join("r1a-gate-report.json");
    let r1b_report_path = args.r1b_dir.join("r1b-gate-report.json");
    ensure_passing_report(&r1a_report_path, "R1-A")?;
    ensure_passing_report(&r1b_report_path, "R1-B")?;
    fs::create_dir_all(args.output_dir)
        .with_context(|| format!("failed to create {}", args.output_dir.display()))?;

    let helper = args
        .workspace_root
        .join("scripts/r1c-learnability-oracle.py");
    let status = Command::new(args.python)
        .arg(&helper)
        .arg("--workspace-root")
        .arg(args.workspace_root)
        .arg("--contract")
        .arg(args.contract_path)
        .arg("--r1a-dir")
        .arg(args.r1a_dir)
        .arg("--limits")
        .arg(args.limits_path)
        .arg("--output-dir")
        .arg(args.output_dir)
        .current_dir(args.workspace_root)
        .status()
        .with_context(|| format!("failed to run {}", helper.display()))?;
    ensure!(status.success(), "Python R1-C learnability oracle failed");

    let python_results_path = args.output_dir.join("python-oracle-results.json");
    let python_oracle: serde_json::Value = read_json(&python_results_path)?;
    ensure!(
        python_oracle
            .get("schema")
            .and_then(serde_json::Value::as_str)
            == Some("haitaka-r1c-python-oracle-results-v1")
    );
    let python_passed = python_oracle
        .get("passed")
        .and_then(serde_json::Value::as_bool)
        == Some(true);
    ensure!(python_passed, "R1-C Python report is not passing");

    let corpus_path = args.r1a_dir.join("parity-corpus.jsonl");
    let expectations_path = args.output_dir.join("runtime-expectations.jsonl");
    let network_path = args.output_dir.join("overfit.nnue");
    let runtime_parity = check_runtime_parity(
        &corpus_path,
        &expectations_path,
        &network_path,
        contract.overfit_corpus.positions,
    )?;
    let collision_pairs = collision_diagnostic()?;
    let runtime_exact = runtime_parity.python_integer_score_mismatches == 0
        && runtime_parity.full_incremental_accumulator_mismatches == 0
        && runtime_parity.full_incremental_score_mismatches == 0
        && runtime_parity.restored_parent_snapshot_mismatches == 0;
    let collisions_exact = collision_pairs.iter().all(|pair| pair.passed);

    let mut gates = BTreeMap::new();
    gates.insert("r1aReportPassing".to_string(), true);
    gates.insert("r1bReportPassing".to_string(), true);
    gates.insert("pythonLearnabilityOracle".to_string(), python_passed);
    gates.insert("pythonIntegerEqualsRustRuntime".to_string(), runtime_exact);
    gates.insert(
        "rustFullRefreshEqualsIncremental".to_string(),
        runtime_parity.full_incremental_accumulator_mismatches == 0
            && runtime_parity.full_incremental_score_mismatches == 0
            && runtime_parity.restored_parent_snapshot_mismatches == 0,
    );
    gates.insert("identityCollisionDiagnostic".to_string(), collisions_exact);
    let passed = gates.values().all(|value| *value);

    let mut artifacts = BTreeMap::new();
    for (name, path) in [
        ("contract", args.contract_path.to_path_buf()),
        ("r1aReport", r1a_report_path),
        ("r1bReport", r1b_report_path),
        ("corpus", corpus_path),
        ("features", args.r1a_dir.join("rust-features.jsonl")),
        ("frozenQuantizationLimits", args.limits_path.to_path_buf()),
        ("pythonOracleSource", helper),
        ("pythonOracleResults", python_results_path),
        (
            "trainingMetadata",
            args.output_dir.join("overfit-training-metadata.json"),
        ),
        (
            "checkpoint",
            args.output_dir.join("overfit-checkpoint.r1fp"),
        ),
        ("serializedNetwork", network_path),
        (
            "serializedNetworkRepeat",
            args.output_dir.join("overfit-repeat.nnue"),
        ),
        ("runtimeExpectations", expectations_path),
        (
            "gateSource",
            args.workspace_root.join("haitaka_learn/src/r1c.rs"),
        ),
        (
            "runtimeSource",
            args.workspace_root.join("haitaka_wasm/src/nnue.rs"),
        ),
    ] {
        artifacts.insert(name.to_string(), artifact_identity(&path)?);
    }
    artifacts.insert(
        "gateExecutable".to_string(),
        artifact_identity(&std::env::current_exe()?)?,
    );

    let report = R1cReport {
        schema: "haitaka-anhoku-r1c-gate",
        schema_version: 1,
        ruleset: "anhoku",
        feature_family: "HalfKAv2^+DonorSingleEff",
        device: "cpu",
        corpus_positions: contract.overfit_corpus.positions,
        python_oracle,
        runtime_parity,
        collision_pairs,
        artifacts,
        gates,
        passed,
    };
    let report_path = args.output_dir.join("r1c-gate-report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    ensure!(passed, "R1-C gate failed; see {}", report_path.display());
    Ok(report)
}

fn validate_frozen_inputs(contract: &Contract, workspace_root: &Path) -> Result<()> {
    for (name, artifact) in &contract.input_artifacts {
        let path = workspace_root.join(&artifact.path);
        ensure!(
            sha256_file(&path)? == artifact.sha256,
            "frozen R1-C input identity mismatch for {name}: {}",
            path.display()
        );
    }
    Ok(())
}

fn ensure_passing_report(path: &Path, phase: &str) -> Result<()> {
    let report: serde_json::Value = read_json(path)?;
    ensure!(
        report.get("passed").and_then(serde_json::Value::as_bool) == Some(true),
        "R1-C requires a passing {phase} report"
    );
    Ok(())
}

fn check_runtime_parity(
    corpus_path: &Path,
    expectations_path: &Path,
    network_path: &Path,
    count: usize,
) -> Result<RuntimeParity> {
    let corpus = read_json_lines::<CorpusRow>(corpus_path)?;
    let expectations = read_json_lines::<RuntimeExpectation>(expectations_path)?;
    ensure!(corpus.len() >= count && expectations.len() == count);
    let bytes = fs::read(network_path)?;
    let model = NnueModel::from_bytes(&bytes)
        .map_err(|error| anyhow!("failed to parse {}: {error}", network_path.display()))?;
    let mut states = Vec::with_capacity(count);
    let mut boards = Vec::with_capacity(count);
    let mut parity = RuntimeParity {
        positions: count,
        ..RuntimeParity::default()
    };
    for index in 0..count {
        let row = &corpus[index];
        let expectation = &expectations[index];
        ensure!(row.id == format!("r1a-{index:05}"));
        ensure!(expectation.id == row.id);
        let board = Board::from_sfen(&row.sfen)
            .map_err(|error| anyhow!("invalid R1-C SFEN {}: {error}", row.id))?;
        let full = model.build_position_state_full(&board);
        let full_score = model.evaluate_from_state(&board, &full);
        parity.python_integer_score_mismatches +=
            usize::from(full_score != expectation.integer_score);
        if let (Some(parent_id), Some(move_text)) = (&row.parent_id, &row.move_from_parent) {
            let parent = fixture_index(parent_id)?;
            ensure!(parent < index);
            let mv = Move::from_str(move_text)
                .map_err(|error| anyhow!("invalid R1-C move {move_text}: {error}"))?;
            let saved_parent = states[parent];
            let incremental = model.apply_move(&boards[parent], &board, &states[parent], mv);
            parity.transitions += 1;
            parity.full_incremental_accumulator_mismatches += usize::from(incremental != full);
            parity.full_incremental_score_mismatches +=
                usize::from(model.evaluate_from_state(&board, &incremental) != full_score);
            parity.restored_parent_snapshot_mismatches +=
                usize::from(states[parent] != saved_parent);
        }
        boards.push(board);
        states.push(full);
    }
    Ok(parity)
}

fn collision_diagnostic() -> Result<Vec<CollisionPair>> {
    let definitions = [
        (
            "black-gold-tokin",
            "4k4/9/9/9/9/9/9/4G4/4K4 b - 1",
            "4k4/9/9/9/9/9/9/4+P4/4K4 b - 1",
        ),
        (
            "black-gold-promoted-lance",
            "4k4/9/9/9/9/9/9/4G4/4K4 b - 1",
            "4k4/9/9/9/9/9/9/4+L4/4K4 b - 1",
        ),
        (
            "black-gold-promoted-knight",
            "4k4/9/9/9/9/9/9/4G4/4K4 b - 1",
            "4k4/9/9/9/9/9/9/4+N4/4K4 b - 1",
        ),
        (
            "black-gold-promoted-silver",
            "4k4/9/9/9/9/9/9/4G4/4K4 b - 1",
            "4k4/9/9/9/9/9/9/4+S4/4K4 b - 1",
        ),
        (
            "white-gold-tokin",
            "4k4/4g4/9/9/9/9/9/9/4K4 w - 1",
            "4k4/4+p4/9/9/9/9/9/9/4K4 w - 1",
        ),
        (
            "white-gold-promoted-lance",
            "4k4/4g4/9/9/9/9/9/9/4K4 w - 1",
            "4k4/4+l4/9/9/9/9/9/9/4K4 w - 1",
        ),
        (
            "white-gold-promoted-knight",
            "4k4/4g4/9/9/9/9/9/9/4K4 w - 1",
            "4k4/4+n4/9/9/9/9/9/9/4K4 w - 1",
        ),
        (
            "white-gold-promoted-silver",
            "4k4/4g4/9/9/9/9/9/9/4K4 w - 1",
            "4k4/4+s4/9/9/9/9/9/9/4K4 w - 1",
        ),
    ];
    definitions
        .into_iter()
        .map(|(id, base_sfen, promoted_sfen)| {
            let base = Board::from_sfen(base_sfen)
                .map_err(|error| anyhow!("invalid collision fixture {id}: {error}"))?;
            let promoted = Board::from_sfen(promoted_sfen)
                .map_err(|error| anyhow!("invalid collision fixture {id}: {error}"))?;
            let packed_bytes_equal =
                pack_board_for_training(&base)? == pack_board_for_training(&promoted)?;
            let black_features_equal = r1_donor_single_active_feature_indices(&base, Color::Black)
                == r1_donor_single_active_feature_indices(&promoted, Color::Black);
            let white_features_equal = r1_donor_single_active_feature_indices(&base, Color::White)
                == r1_donor_single_active_feature_indices(&promoted, Color::White);
            let absolute_handcrafted_material_delta =
                (handcrafted_material_score(&base) - handcrafted_material_score(&promoted)).abs();
            let passed = packed_bytes_equal
                && black_features_equal
                && white_features_equal
                && absolute_handcrafted_material_delta == 50;
            Ok(CollisionPair {
                id: id.to_string(),
                base_sfen: base_sfen.to_string(),
                promoted_sfen: promoted_sfen.to_string(),
                packed_bytes_equal,
                black_features_equal,
                white_features_equal,
                absolute_handcrafted_material_delta,
                passed,
            })
        })
        .collect()
}

fn handcrafted_material_score(board: &Board) -> i32 {
    let us = board.side_to_move();
    material_for(board, us) - material_for(board, !us)
}

fn material_for(board: &Board, color: Color) -> i32 {
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

fn fixture_index(id: &str) -> Result<usize> {
    id.strip_prefix("r1a-")
        .ok_or_else(|| anyhow!("invalid R1-C fixture id {id}"))?
        .parse()
        .map_err(|error| anyhow!("invalid R1-C fixture id {id}: {error}"))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {}", path.display()))
}

fn read_json_lines<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>> {
    BufReader::new(File::open(path)?)
        .lines()
        .enumerate()
        .map(|(index, line)| {
            serde_json::from_str(&line?)
                .with_context(|| format!("failed to parse {} line {}", path.display(), index + 1))
        })
        .collect()
}

fn artifact_identity(path: &Path) -> Result<ArtifactIdentity> {
    Ok(ArtifactIdentity {
        path: path.display().to_string(),
        bytes: fs::metadata(path)?.len(),
        sha256: sha256_file(path)?,
    })
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(all(test, feature = "anhoku"))]
mod tests {
    use super::*;

    #[test]
    fn gold_like_collision_fixtures_are_exact() {
        let pairs = collision_diagnostic().unwrap();
        assert_eq!(pairs.len(), 8);
        assert!(pairs.iter().all(|pair| pair.passed));
    }
}

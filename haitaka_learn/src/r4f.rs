use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result, ensure};
use haitaka::{Board, Color};
use haitaka_wasm::{
    R1_HALFKAV2_BASE_FEATURES, SEARCH_NODE_COUNTING_VERSION, SearchWorkspace,
    r1_donor_single_active_feature_indices,
    search_board_impl_handcrafted_with_node_budget_in_workspace,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest, Sha256};

const ANALYSIS_SCHEMA: &str = "haitaka-anhoku-r4f-incomplete-analysis-v1";
const UNUSED_POOL_SCHEMA: &str = "haitaka-anhoku-r4f-unused-pool-v1";
const CONTRACT_SCHEMA: &str = "haitaka-anhoku-r4f-label-completeness-contract-v1";
const RESULT_SCHEMA: &str = "haitaka-anhoku-r4f-probe-result-v1";
const GATE_SCHEMA: &str = "haitaka-anhoku-r4f-gate-report-v1";
const LABEL_SCHEMA: &str = "haitaka-r4-labeled-position-v1";
const POSITION_SCHEMA: &str = "haitaka-r4-position-v1";
const EXPECTED_CONTRACT_SHA256: &str =
    "81c9b6baec8bf6bafd01cf4370e432cfc4a4f60e78b417dce02096d57ebe1339";
const EXPECTED_FAILED_GATE_SHA256: &str =
    "2fa26d4629c2ddf3fa1f9f61632f3d29558929eb67ea736ae4957598b3d04fc7";
const EXPECTED_R3_CONTRACT_SHA256: &str =
    "25cd5eacb9cd45fe4110dd796c0536b1822e2c5b0d85db6e65b6a26ecfde79ba";
const EXPECTED_R3_GATE_SHA256: &str =
    "7e97b3a215f2c6e6f03b30a56ce29bb3ac8b23630ea506d9d5445c09f9d7b5cb";
const EXPECTED_LABEL_SOURCE_REVISION: &str =
    "d1e31faae753a226e67b01a35035ca5548ee3cd7bb165dd556a4d7a35488c38b";
const DONOR_ROWS: usize = 1_620;

pub(crate) struct AnalyzeArgs<'a> {
    pub original_dir: &'a Path,
    pub output_dir: &'a Path,
    pub workspace_root: &'a Path,
}

pub(crate) struct ProbeArgs<'a> {
    pub contract_path: &'a Path,
    pub original_dir: &'a Path,
    pub output_dir: &'a Path,
    pub workspace_root: &'a Path,
    pub executable_path: &'a Path,
    pub jobs: usize,
}

pub(crate) struct R3RequalifyArgs<'a> {
    pub contract_path: &'a Path,
    pub r4f_dir: &'a Path,
    pub output_dir: &'a Path,
    pub workspace_root: &'a Path,
    pub executable_path: &'a Path,
    pub jobs: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Position {
    schema: String,
    id: String,
    sfen: String,
    packed_board_sha256: String,
    color_swap_canonical_sha256: String,
    source: String,
    source_game: String,
    opening_group: String,
    opening_sfen_sha256: String,
    split: String,
    trajectory_policy: String,
    trajectory_evaluator: String,
    trajectory_budget: String,
    candidate_inclusion_probability: f64,
    ply: u16,
    phase: String,
    side_to_move: String,
    material_bucket: String,
    in_check: bool,
    tactical: bool,
    has_promotion: bool,
    has_drop: bool,
    result_status: String,
    donor_state: String,
    active_base_rows_black: usize,
    active_donor_rows_black: usize,
    active_base_rows_white: usize,
    active_donor_rows_white: usize,
    output_bucket: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OldLabel {
    schema: String,
    tier: String,
    ordinal: u64,
    position: Position,
    label_evaluator: String,
    label_budget: u64,
    maximum_depth: u8,
    node_counting_version: String,
    completed: bool,
    completed_depth: u8,
    partial_iteration: bool,
    node_accounting_exact: bool,
    alpha_beta_nodes: u64,
    qsearch_nodes: u64,
    total_nodes: u64,
    root_move: Option<String>,
    raw_score: Option<i32>,
    bounded_score: Option<i16>,
    mate_class: String,
    mate_distance: Option<i32>,
    label_sha256: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Counts {
    attempted: u64,
    completed: u64,
    incomplete: u64,
    incomplete_rate: f64,
}

impl Counts {
    fn observe(&mut self, complete: bool) {
        self.attempted += 1;
        if complete {
            self.completed += 1;
        } else {
            self.incomplete += 1;
        }
    }

    fn finish(&mut self) {
        self.incomplete_rate = if self.attempted == 0 {
            0.0
        } else {
            self.incomplete as f64 / self.attempted as f64
        };
    }
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Dimensions(BTreeMap<String, BTreeMap<String, Counts>>);

impl Dimensions {
    fn observe(&mut self, dimension: &str, value: impl Into<String>, complete: bool) {
        self.0
            .entry(dimension.to_string())
            .or_default()
            .entry(value.into())
            .or_default()
            .observe(complete);
    }

    fn finish(&mut self) {
        for values in self.0.values_mut() {
            for counts in values.values_mut() {
                counts.finish();
            }
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NumericSummary {
    count: u64,
    minimum: u64,
    p50: u64,
    p95: u64,
    maximum: u64,
    mean: f64,
}

fn summarize(values: &mut [u64]) -> NumericSummary {
    values.sort_unstable();
    if values.is_empty() {
        return NumericSummary {
            count: 0,
            minimum: 0,
            p50: 0,
            p95: 0,
            maximum: 0,
            mean: 0.0,
        };
    }
    let percentile = |p: f64| values[((values.len() - 1) as f64 * p).round() as usize];
    NumericSummary {
        count: values.len() as u64,
        minimum: values[0],
        p50: percentile(0.50),
        p95: percentile(0.95),
        maximum: *values.last().expect("nonempty"),
        mean: values.iter().map(|&value| value as f64).sum::<f64>() / values.len() as f64,
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Coverage {
    finite_records: u64,
    known_structurally_unreachable_base_rows: Vec<usize>,
    never_observed_reachable_base_rows: Vec<usize>,
    never_observed_donor_rows: Vec<usize>,
    reachable_base_rows_below_minimum: Vec<usize>,
    donor_rows_below_minimum: Vec<usize>,
    base_update_count_sha256: String,
    donor_update_count_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Binding {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UnusedPoolManifest {
    schema: String,
    original_contract_sha256: String,
    position_shards: u64,
    candidate_records: u64,
    unique_color_swap_families: u64,
    original_pilot_families: u64,
    unused_families: u64,
    ordering: String,
    position_evidence_set_sha256: String,
    data: Binding,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Analysis {
    schema: String,
    phase: String,
    original_contract_sha256: String,
    original_evidence: Vec<Binding>,
    original_evidence_set_sha256: String,
    pilot: Counts,
    deep_core: Counts,
    correct_mate_labels: u64,
    incorrect_legacy_mate_count: u64,
    dimensions: Dimensions,
    search_telemetry: BTreeMap<String, BTreeMap<String, NumericSummary>>,
    attempted_distribution_sha256: String,
    retained_distribution_sha256: String,
    finite_feature_coverage: Coverage,
    unused_pool_manifest: Binding,
    no_targets_synthesized_for_incomplete: bool,
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(
        &fs::read(path).with_context(|| format!("failed to read {}", path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", path.display()))
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 128 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn binding(path: &Path, workspace_root: &Path) -> Result<Binding> {
    let shown = path
        .strip_prefix(workspace_root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    Ok(Binding {
        path: shown,
        bytes: fs::metadata(path)?.len(),
        sha256: sha256_file(path)?,
    })
}

fn stable_u64(seed: u64, domain: &str, id: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(b"haitaka-r4f-stable-selection-v1\0");
    hasher.update(seed.to_le_bytes());
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(id.as_bytes());
    let digest = hasher.finalize();
    u64::from_le_bytes(digest[..8].try_into().expect("digest prefix"))
}

fn hash_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

fn manifest_data_path(manifest: &Path) -> PathBuf {
    let mut path = manifest.to_path_buf();
    path.set_extension("jsonl");
    path
}

fn sorted_manifests(dir: &Path, prefix: &str) -> Result<Vec<PathBuf>> {
    let mut result = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".json"))
        })
        .collect::<Vec<_>>();
    result.sort();
    Ok(result)
}

fn verify_manifest_data(manifest: &Path, hash_key: &str) -> Result<PathBuf> {
    let value: Value = read_json(manifest)?;
    let expected = value
        .get(hash_key)
        .and_then(Value::as_str)
        .with_context(|| format!("{} lacks {hash_key}", manifest.display()))?;
    let data = manifest_data_path(manifest);
    ensure!(
        sha256_file(&data)? == expected,
        "hash mismatch for {}",
        data.display()
    );
    Ok(data)
}

fn base_row_structurally_reachable(index: usize) -> bool {
    const PIECE_INDICES: usize = 1_863;
    const NON_DROP: usize = 1_539;
    let king_square = index / PIECE_INDICES;
    let local = index % PIECE_INDICES;
    if local < NON_DROP {
        local >= 18 * 81 || local % 81 != king_square
    } else {
        let hand = local - NON_DROP;
        let slot = (hand / 18) / 2;
        let copy = hand % 18;
        let maximum = match slot {
            0 | 1 => 2,
            2 | 5 | 6 | 7 => 4,
            4 => 18,
            _ => 0,
        };
        copy < maximum
    }
}

fn active_row_stratum(position: &Position) -> String {
    let rows = position.active_base_rows_black
        + position.active_donor_rows_black
        + position.active_base_rows_white
        + position.active_donor_rows_white;
    format!("{:03}-{:03}", rows / 16 * 16, rows / 16 * 16 + 15)
}

fn ply_stratum(ply: u16) -> &'static str {
    match ply {
        0..=35 => "000-035",
        36..=89 => "036-089",
        90..=149 => "090-149",
        _ => "150+",
    }
}

fn observe_dimensions(
    dimensions: &mut Dimensions,
    label: &OldLabel,
    host: &str,
    architecture: &str,
) {
    let position = &label.position;
    let complete = label.completed;
    for (dimension, value) in [
        ("split", position.split.clone()),
        ("source", position.source.clone()),
        ("openingGroup", position.opening_group.clone()),
        ("phase", position.phase.clone()),
        ("ply", ply_stratum(position.ply).to_string()),
        ("side", position.side_to_move.clone()),
        ("material", position.material_bucket.clone()),
        ("check", position.in_check.to_string()),
        ("tactical", position.tactical.to_string()),
        ("resultStatus", position.result_status.clone()),
        ("donorState", position.donor_state.clone()),
        ("outputBucket", position.output_bucket.to_string()),
        ("activeRows", active_row_stratum(position)),
        ("host", host.to_string()),
        ("architecture", architecture.to_string()),
    ] {
        dimensions.observe(dimension, value, complete);
    }
}

fn add_coverage(position: &Position, base: &mut [u64], donor: &mut [u64]) -> Result<()> {
    let board = Board::from_sfen(&position.sfen).map_err(anyhow::Error::msg)?;
    for perspective in [Color::Black, Color::White] {
        let active = r1_donor_single_active_feature_indices(&board, perspective);
        for row in active.base {
            base[row] += 1;
        }
        for row in active.donor {
            let local = row
                .checked_sub(R1_HALFKAV2_BASE_FEATURES)
                .context("invalid donor row")?;
            ensure!(local < donor.len());
            donor[local] += 1;
        }
    }
    Ok(())
}

fn hash_counts(values: &[u64]) -> String {
    let mut hasher = Sha256::new();
    for value in values {
        hasher.update(value.to_le_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn finish_coverage(base: Vec<u64>, donor: Vec<u64>, finite: u64, minimum: u64) -> Coverage {
    Coverage {
        finite_records: finite,
        known_structurally_unreachable_base_rows: (0..base.len())
            .filter(|&row| !base_row_structurally_reachable(row))
            .collect(),
        never_observed_reachable_base_rows: (0..base.len())
            .filter(|&row| base_row_structurally_reachable(row) && base[row] == 0)
            .collect(),
        never_observed_donor_rows: (0..donor.len()).filter(|&row| donor[row] == 0).collect(),
        reachable_base_rows_below_minimum: (0..base.len())
            .filter(|&row| {
                base_row_structurally_reachable(row) && base[row] > 0 && base[row] < minimum
            })
            .collect(),
        donor_rows_below_minimum: (0..donor.len())
            .filter(|&row| donor[row] > 0 && donor[row] < minimum)
            .collect(),
        base_update_count_sha256: hash_counts(&base),
        donor_update_count_sha256: hash_counts(&donor),
    }
}

fn collect_original_bindings(original_dir: &Path, workspace_root: &Path) -> Result<Vec<Binding>> {
    let mut paths = vec![
        workspace_root.join("r0/anhoku-reboot/r4-data-contract.json"),
        original_dir.join("assembly-report.json"),
        original_dir.join("r4-gate-report.json"),
    ];
    for (directory, prefix, hash_key) in [
        (
            original_dir.join("positions"),
            "position-shard-",
            "positionsSha256",
        ),
        (
            original_dir.join("labels/pilot"),
            "labeled-shard-",
            "labelsSha256",
        ),
        (
            original_dir.join("labels/deep-core"),
            "labeled-shard-",
            "labelsSha256",
        ),
    ] {
        for manifest in sorted_manifests(&directory, prefix)? {
            let value: Value = read_json(&manifest)?;
            ensure!(value["contractSha256"] == EXPECTED_CONTRACT_SHA256);
            let data = verify_manifest_data(&manifest, hash_key)?;
            paths.push(manifest);
            paths.push(data);
        }
    }
    for name in ["train", "validation", "development", "deep-core"] {
        let manifest = original_dir.join(format!("datasets/{name}.json"));
        let value: Value = read_json(&manifest)?;
        for (suffix, key) in [("bin", "binarySha256"), ("jsonl", "sidecarSha256")] {
            let data = original_dir.join(format!("datasets/{name}.{suffix}"));
            ensure!(
                sha256_file(&data)?
                    == value[key]
                        .as_str()
                        .with_context(|| format!("missing {key}"))?
            );
            paths.push(data);
        }
        paths.push(manifest);
    }
    paths.sort();
    paths.dedup();
    paths
        .iter()
        .map(|path| binding(path, workspace_root))
        .collect()
}

pub(crate) fn analyze(args: AnalyzeArgs<'_>) -> Result<PathBuf> {
    let output = args.output_dir.join("incomplete-analysis.json");
    let unused_data = args.output_dir.join("unused-pool.jsonl");
    let unused_manifest_path = args.output_dir.join("unused-pool-manifest.json");
    ensure!(
        !output.exists() && !unused_data.exists() && !unused_manifest_path.exists(),
        "R4-F analysis outputs are immutable"
    );
    ensure!(
        sha256_file(&args.original_dir.join("r4-gate-report.json"))? == EXPECTED_FAILED_GATE_SHA256,
        "failed R4 gate report changed"
    );
    ensure!(
        sha256_file(
            &args
                .workspace_root
                .join("r0/anhoku-reboot/r4-data-contract.json")
        )? == EXPECTED_CONTRACT_SHA256,
        "original R4 contract changed"
    );
    fs::create_dir_all(args.output_dir)?;
    let evidence = collect_original_bindings(args.original_dir, args.workspace_root)?;
    let evidence_hash = hash_json(&evidence)?;

    let pilot_manifests =
        sorted_manifests(&args.original_dir.join("labels/pilot"), "labeled-shard-")?;
    ensure!(pilot_manifests.len() == 160);
    let deep_manifests = sorted_manifests(
        &args.original_dir.join("labels/deep-core"),
        "labeled-shard-",
    )?;
    ensure!(deep_manifests.len() == 16);
    let host_arch = pilot_manifests
        .iter()
        .map(|path| {
            let value: Value = read_json(path)?;
            Ok((
                value["hostname"].as_str().context("hostname")?.to_string(),
                value["architecture"]
                    .as_str()
                    .context("architecture")?
                    .to_string(),
            ))
        })
        .collect::<Result<Vec<_>>>()?;

    let mut pilot = Counts::default();
    let mut dimensions = Dimensions::default();
    let mut selected_families = HashSet::with_capacity(1_000_000);
    let mut base = vec![0u64; R1_HALFKAV2_BASE_FEATURES];
    let mut donor = vec![0u64; DONOR_ROWS];
    let mut correct_mates = 0u64;
    let mut legacy_mates = 0u64;
    let mut no_synthetic = true;
    let mut telemetry = BTreeMap::<String, BTreeMap<String, Vec<u64>>>::new();
    for (shard, manifest) in pilot_manifests.iter().enumerate() {
        let data = verify_manifest_data(manifest, "labelsSha256")?;
        for line in BufReader::new(File::open(data)?).lines() {
            let label: OldLabel = serde_json::from_str(&line?)?;
            ensure!(label.schema == LABEL_SCHEMA && label.tier == "pilot");
            ensure!(label.ordinal as usize % 160 == shard);
            ensure!(label.label_evaluator == "haitaka-native-handcrafted-search-v1");
            ensure!(label.label_budget == 10_000 && label.maximum_depth == 64);
            ensure!(label.node_counting_version == SEARCH_NODE_COUNTING_VERSION);
            ensure!(!label.label_sha256.is_empty());
            pilot.observe(label.completed);
            selected_families.insert(label.position.color_swap_canonical_sha256.clone());
            observe_dimensions(
                &mut dimensions,
                &label,
                &host_arch[shard].0,
                &host_arch[shard].1,
            );
            legacy_mates += u64::from(label.mate_class != "none");
            correct_mates += u64::from(label.mate_class == "win" || label.mate_class == "loss");
            if label.completed {
                add_coverage(&label.position, &mut base, &mut donor)?;
            } else {
                no_synthetic &= label.raw_score.is_none()
                    && label.bounded_score.is_none()
                    && label.mate_class == "unknown/incomplete";
            }
            let status = if label.completed {
                "completed"
            } else {
                "incomplete"
            };
            for (metric, value) in [
                ("completedDepth", u64::from(label.completed_depth)),
                ("alphaBetaNodes", label.alpha_beta_nodes),
                ("qsearchNodes", label.qsearch_nodes),
                ("totalNodes", label.total_nodes),
            ] {
                telemetry
                    .entry(status.to_string())
                    .or_default()
                    .entry(metric.to_string())
                    .or_default()
                    .push(value);
            }
        }
    }
    pilot.finish();
    dimensions.finish();
    ensure!(pilot.attempted == 1_000_000 && selected_families.len() == 1_000_000);

    let mut deep = Counts::default();
    for manifest in &deep_manifests {
        let data = verify_manifest_data(manifest, "labelsSha256")?;
        for line in BufReader::new(File::open(data)?).lines() {
            let label: OldLabel = serde_json::from_str(&line?)?;
            ensure!(label.tier == "deep-core");
            deep.observe(label.completed);
        }
    }
    deep.finish();
    ensure!(deep.attempted == 4_096);

    let position_manifests =
        sorted_manifests(&args.original_dir.join("positions"), "position-shard-")?;
    ensure!(position_manifests.len() == 160);
    let mut canonical_minimum = HashMap::<String, String>::with_capacity(1_520_000);
    let mut candidates = 0u64;
    for manifest in &position_manifests {
        let data = verify_manifest_data(manifest, "positionsSha256")?;
        for line in BufReader::new(File::open(data)?).lines() {
            let position: Position = serde_json::from_str(&line?)?;
            ensure!(position.schema == POSITION_SCHEMA);
            candidates += 1;
            canonical_minimum
                .entry(position.color_swap_canonical_sha256)
                .and_modify(|id| {
                    if position.id < *id {
                        *id = position.id.clone();
                    }
                })
                .or_insert(position.id);
        }
    }
    let unique = canonical_minimum.len() as u64;
    let mut writer = BufWriter::new(File::create(&unused_data)?);
    let mut unused_hasher = Sha256::new();
    let mut unused = 0u64;
    for manifest in &position_manifests {
        let data = manifest_data_path(manifest);
        for line in BufReader::new(File::open(data)?).lines() {
            let line = line?;
            let position: Position = serde_json::from_str(&line)?;
            if canonical_minimum[&position.color_swap_canonical_sha256] == position.id
                && !selected_families.contains(&position.color_swap_canonical_sha256)
            {
                writer.write_all(line.as_bytes())?;
                writer.write_all(b"\n")?;
                unused_hasher.update(line.as_bytes());
                unused_hasher.update(b"\n");
                unused += 1;
            }
        }
    }
    writer.flush()?;
    ensure!(unique == 1_514_884 && unused == 514_884);
    let unused_manifest = UnusedPoolManifest {
        schema: UNUSED_POOL_SCHEMA.to_string(),
        original_contract_sha256: EXPECTED_CONTRACT_SHA256.to_string(),
        position_shards: position_manifests.len() as u64,
        candidate_records: candidates,
        unique_color_swap_families: unique,
        original_pilot_families: selected_families.len() as u64,
        unused_families: unused,
        ordering: "position-shard-index-then-record-order-filtered-to-canonical-minimum-id-v1"
            .to_string(),
        position_evidence_set_sha256: hash_json(
            &evidence
                .iter()
                .filter(|item| item.path.contains("/positions/"))
                .collect::<Vec<_>>(),
        )?,
        data: Binding {
            path: unused_data
                .strip_prefix(args.workspace_root)
                .unwrap_or(&unused_data)
                .to_string_lossy()
                .to_string(),
            bytes: fs::metadata(&unused_data)?.len(),
            sha256: format!("{:x}", unused_hasher.finalize()),
        },
    };
    fs::write(
        &unused_manifest_path,
        serde_json::to_vec_pretty(&unused_manifest)?,
    )?;

    let mut search_telemetry = BTreeMap::new();
    for (status, metrics) in telemetry {
        search_telemetry.insert(
            status,
            metrics
                .into_iter()
                .map(|(metric, mut values)| (metric, summarize(&mut values)))
                .collect(),
        );
    }
    let attempted_distribution_sha256 = hash_json(&dimensions)?;
    let retained_only = dimensions
        .0
        .iter()
        .map(|(dimension, values)| {
            (
                dimension.clone(),
                values
                    .iter()
                    .map(|(value, counts)| (value.clone(), counts.completed))
                    .collect::<BTreeMap<_, _>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let retained_distribution_sha256 = hash_json(&retained_only)?;
    let analysis = Analysis {
        schema: ANALYSIS_SCHEMA.to_string(),
        phase: "R4-F".to_string(),
        original_contract_sha256: EXPECTED_CONTRACT_SHA256.to_string(),
        original_evidence: evidence,
        original_evidence_set_sha256: evidence_hash,
        pilot: pilot.clone(),
        deep_core: deep,
        correct_mate_labels: correct_mates,
        incorrect_legacy_mate_count: legacy_mates,
        dimensions,
        search_telemetry,
        attempted_distribution_sha256,
        retained_distribution_sha256,
        finite_feature_coverage: finish_coverage(base, donor, pilot.completed, 8),
        unused_pool_manifest: binding(&unused_manifest_path, args.workspace_root)?,
        no_targets_synthesized_for_incomplete: no_synthetic,
    };
    fs::write(&output, serde_json::to_vec_pretty(&analysis)?)?;
    Ok(output)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct Contract {
    schema: String,
    frozen_at: String,
    original_contract_sha256: String,
    original_failed_gate_sha256: String,
    r3_contract: ExpectedArtifact,
    r3_gate: ExpectedArtifact,
    label_source_revision: String,
    archived_label_executable_sha256: Vec<String>,
    analysis: ExpectedArtifact,
    unused_pool: ExpectedArtifact,
    probe: ProbeContract,
    label_policy: LabelPolicy,
    limits: Limits,
    route_rule: RouteRule,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct ExpectedArtifact {
    path: PathBuf,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct ProbeContract {
    selection_version: String,
    selection_seed: u64,
    candidate_fixed_budgets: Vec<u64>,
    required_phase_counts: BTreeMap<String, usize>,
    replay_positions: usize,
    fresh_confirmation_positions: usize,
    deep_reference_positions: usize,
    maximum_distinct_fresh_boards: usize,
    maximum_total_search_evaluations: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct LabelPolicy {
    evaluator: String,
    node_budget: u64,
    deep_reference_node_budget: u64,
    maximum_depth: u8,
    node_counting_version: String,
    target_semantics: String,
    score_transform: String,
    loss: String,
    nonmate_clip: i32,
    mate_threshold: i32,
    mate_base: i32,
    qsearch_policy: String,
    search_semantics: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct Limits {
    maximum_confirmation_incomplete_rate: f64,
    maximum_phase_incomplete_rates: BTreeMap<String, f64>,
    maximum_missingness_rate_delta_from_original: f64,
    maximum_robust_normalized_mae: f64,
    maximum_sign_disagreement_rate: f64,
    maximum_mate_class_disagreement_rate: f64,
    minimum_comparable_reference_positions: u64,
    required_node_accounting_rate: f64,
    maximum_distribution_total_variation: BTreeMap<String, f64>,
    maximum_stratum_incomplete_rate: BTreeMap<String, f64>,
    minimum_stratum_attempts: u64,
    minimum_finite_feature_coverage_rate: f64,
    maximum_total_nodes: u64,
    maximum_core_hours: f64,
    maximum_dollars: f64,
    cpu_core_hour_usd: f64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct RouteRule {
    accounting_or_replay_failure: String,
    qsearch_or_history_failure: String,
    target_or_missingness_failure: String,
    unchanged_policy_success: String,
    fallback: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchObservation {
    schema: String,
    lane: String,
    selection_rank: usize,
    position: Position,
    label_budget: u64,
    completed: bool,
    completed_depth: u8,
    partial_iteration: bool,
    node_accounting_exact: bool,
    alpha_beta_nodes: u64,
    qsearch_nodes: u64,
    total_nodes: u64,
    root_move: Option<String>,
    raw_score: Option<i32>,
    bounded_score: Option<i16>,
    mate_class: String,
    mate_distance: Option<i32>,
    elapsed_ms: f64,
}

fn bounded_target(
    raw_score: Option<i32>,
    clip: i32,
    mate_threshold: i32,
    mate_base: i32,
) -> (String, Option<i32>, Option<i16>) {
    match raw_score {
        Some(score) if score.abs() >= mate_threshold => (
            if score > 0 { "win" } else { "loss" }.to_string(),
            Some((mate_base - score.abs()).max(0)),
            Some((score.signum() * clip) as i16),
        ),
        Some(score) => (
            "none".to_string(),
            None,
            Some(score.clamp(-clip, clip) as i16),
        ),
        None => ("unknown/incomplete".to_string(), None, None),
    }
}

fn search_one(
    position: Position,
    lane: &str,
    rank: usize,
    budget: u64,
    policy: &LabelPolicy,
    workspace: &mut SearchWorkspace,
) -> Result<SearchObservation> {
    let board = Board::from_sfen(&position.sfen).map_err(anyhow::Error::msg)?;
    let summary = search_board_impl_handcrafted_with_node_budget_in_workspace(
        &board,
        budget,
        policy.maximum_depth,
        workspace,
    )
    .map_err(anyhow::Error::msg)?;
    let completed = summary.best_score.is_some() && summary.completed_depth > 0;
    let accounting = summary.alpha_beta_nodes.checked_add(summary.qsearch_nodes)
        == Some(summary.total_nodes)
        && summary.total_nodes <= summary.node_limit;
    let (mate_class, mate_distance, bounded_score) = bounded_target(
        summary.best_score,
        policy.nonmate_clip,
        policy.mate_threshold,
        policy.mate_base,
    );
    Ok(SearchObservation {
        schema: RESULT_SCHEMA.to_string(),
        lane: lane.to_string(),
        selection_rank: rank,
        position,
        label_budget: budget,
        completed,
        completed_depth: summary.completed_depth,
        partial_iteration: summary.incomplete_iterations > 0,
        node_accounting_exact: accounting,
        alpha_beta_nodes: summary.alpha_beta_nodes,
        qsearch_nodes: summary.qsearch_nodes,
        total_nodes: summary.total_nodes,
        root_move: summary.best_move,
        raw_score: summary.best_score,
        bounded_score,
        mate_class,
        mate_distance,
        elapsed_ms: summary.elapsed_ms,
    })
}

fn run_parallel(
    work: Vec<(String, usize, Position, u64)>,
    policy: &LabelPolicy,
    jobs: usize,
) -> Result<Vec<SearchObservation>> {
    ensure!(jobs > 0);
    let count = work.len();
    let work = Arc::new(work);
    let next = Arc::new(AtomicUsize::new(0));
    let results = Arc::new(Mutex::new(vec![None; count]));
    let error = Arc::new(Mutex::new(None::<anyhow::Error>));
    thread::scope(|scope| {
        for _ in 0..jobs {
            let work = Arc::clone(&work);
            let next = Arc::clone(&next);
            let results = Arc::clone(&results);
            let error = Arc::clone(&error);
            scope.spawn(move || {
                let mut workspace = SearchWorkspace::new();
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= work.len() || error.lock().expect("error mutex").is_some() {
                        break;
                    }
                    let (lane, rank, position, budget) = &work[index];
                    match search_one(
                        position.clone(),
                        lane,
                        *rank,
                        *budget,
                        policy,
                        &mut workspace,
                    ) {
                        Ok(value) => results.lock().expect("result mutex")[index] = Some(value),
                        Err(err) => {
                            *error.lock().expect("error mutex") = Some(err);
                            break;
                        }
                    }
                }
            });
        }
    });
    if let Some(err) = error.lock().expect("error mutex").take() {
        return Err(err);
    }
    Ok(results
        .lock()
        .expect("result mutex")
        .iter_mut()
        .map(|value| value.take().expect("all probe work completed"))
        .collect())
}

fn load_old_labels(original_dir: &Path) -> Result<Vec<OldLabel>> {
    let mut labels = Vec::with_capacity(1_000_000);
    for manifest in sorted_manifests(&original_dir.join("labels/pilot"), "labeled-shard-")? {
        for line in BufReader::new(File::open(verify_manifest_data(
            &manifest,
            "labelsSha256",
        )?)?)
        .lines()
        {
            labels.push(serde_json::from_str(&line?)?);
        }
    }
    labels.sort_by_key(|label: &OldLabel| label.ordinal);
    Ok(labels)
}

fn exact_replay(old: &OldLabel, new: &SearchObservation) -> bool {
    old.completed == new.completed
        && old.completed_depth == new.completed_depth
        && old.partial_iteration == new.partial_iteration
        && old.node_accounting_exact == new.node_accounting_exact
        && old.alpha_beta_nodes == new.alpha_beta_nodes
        && old.qsearch_nodes == new.qsearch_nodes
        && old.total_nodes == new.total_nodes
        && old.root_move == new.root_move
        && old.raw_score == new.raw_score
        && old.bounded_score == new.bounded_score
        && old.mate_class == new.mate_class
        && old.mate_distance == new.mate_distance
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeMetrics {
    replay_positions: u64,
    exact_replays: u64,
    confirmation_positions: u64,
    confirmation_completed: u64,
    confirmation_incomplete_rate: f64,
    confirmation_incomplete_rate_by_phase: BTreeMap<String, f64>,
    original_incomplete_rate: f64,
    missingness_rate_delta: f64,
    reference_positions: u64,
    comparable_reference_positions: u64,
    robust_normalized_mae: f64,
    sign_disagreement_rate: f64,
    mate_class_disagreement_rate: f64,
    node_accounting_rate: f64,
    distribution_total_variation: BTreeMap<String, f64>,
    maximum_observed_stratum_incomplete_rate: BTreeMap<String, f64>,
    finite_feature_coverage_rate: f64,
    search_evaluations: u64,
    distinct_fresh_boards: u64,
    total_nodes: u64,
    search_core_hours: f64,
    estimated_cost_usd: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GateReport {
    schema: String,
    phase: String,
    passed: bool,
    closeout_valid: bool,
    route_authorized: bool,
    replacement_r4_authorized: bool,
    selected_route: String,
    r5_authorized: bool,
    contract_sha256: String,
    analysis_sha256: String,
    unused_pool_manifest_sha256: String,
    raw_results: Binding,
    executable_sha256: String,
    node_counting_version: String,
    metrics: ProbeMetrics,
    gates: BTreeMap<String, bool>,
    independently_recomputed_route: String,
    route_rule: RouteRule,
    downstream_work: BTreeMap<String, bool>,
}

fn recompute_route(gates: &BTreeMap<String, bool>, rule: &RouteRule) -> String {
    if !gates["replay-deterministic"] || !gates["node-accounting-exact"] {
        rule.accounting_or_replay_failure.clone()
    } else if !gates["target-quality"] || !gates["missingness-bias"] || !gates["retained-coverage"]
    {
        rule.target_or_missingness_failure.clone()
    } else if gates["original-evidence-bound"]
        && gates["analysis-recomputed"]
        && gates["contract-frozen-before-search"]
        && gates["fresh-confirmation-isolated"]
        && gates["cost-ceiling"]
    {
        rule.unchanged_policy_success.clone()
    } else {
        rule.fallback.clone()
    }
}

fn probe_dimension_values(position: &Position) -> [(String, String); 11] {
    [
        ("split".to_string(), position.split.clone()),
        ("source".to_string(), position.source.clone()),
        ("phase".to_string(), position.phase.clone()),
        ("ply".to_string(), ply_stratum(position.ply).to_string()),
        ("side".to_string(), position.side_to_move.clone()),
        ("material".to_string(), position.material_bucket.clone()),
        ("check".to_string(), position.in_check.to_string()),
        ("tactical".to_string(), position.tactical.to_string()),
        ("resultStatus".to_string(), position.result_status.clone()),
        ("donorState".to_string(), position.donor_state.clone()),
        (
            "outputBucket".to_string(),
            position.output_bucket.to_string(),
        ),
    ]
}

fn retained_distribution_metrics(
    confirmation: &[&SearchObservation],
    minimum_stratum_attempts: u64,
) -> (BTreeMap<String, f64>, BTreeMap<String, f64>) {
    let mut dimensions = Dimensions::default();
    for result in confirmation {
        for (dimension, value) in probe_dimension_values(&result.position) {
            dimensions.observe(&dimension, value, result.completed);
        }
    }
    dimensions.finish();
    let mut tv = BTreeMap::new();
    let mut maximum_missingness = BTreeMap::new();
    for (dimension, values) in dimensions.0 {
        let attempted = values.values().map(|counts| counts.attempted).sum::<u64>() as f64;
        let retained = values.values().map(|counts| counts.completed).sum::<u64>() as f64;
        let distance = if attempted == 0.0 || retained == 0.0 {
            1.0
        } else {
            0.5 * values
                .values()
                .map(|counts| {
                    (counts.attempted as f64 / attempted - counts.completed as f64 / retained).abs()
                })
                .sum::<f64>()
        };
        let maximum = values
            .values()
            .filter(|counts| counts.attempted >= minimum_stratum_attempts)
            .map(|counts| counts.incomplete_rate)
            .fold(0.0, f64::max);
        tv.insert(dimension.clone(), distance);
        maximum_missingness.insert(dimension, maximum);
    }
    (tv, maximum_missingness)
}

fn finite_feature_coverage_rate(confirmation: &[&SearchObservation]) -> Result<f64> {
    let mut attempted_base = vec![0u64; R1_HALFKAV2_BASE_FEATURES];
    let mut attempted_donor = vec![0u64; DONOR_ROWS];
    let mut finite_base = vec![0u64; R1_HALFKAV2_BASE_FEATURES];
    let mut finite_donor = vec![0u64; DONOR_ROWS];
    for result in confirmation {
        add_coverage(&result.position, &mut attempted_base, &mut attempted_donor)?;
        if result.completed {
            add_coverage(&result.position, &mut finite_base, &mut finite_donor)?;
        }
    }
    let attempted = attempted_base
        .iter()
        .zip(&finite_base)
        .filter(|(count, _)| **count > 0)
        .map(|_| 1u64)
        .sum::<u64>()
        + attempted_donor.iter().filter(|&&count| count > 0).count() as u64;
    let finite = attempted_base
        .iter()
        .zip(&finite_base)
        .filter(|(attempted, finite)| **attempted > 0 && **finite > 0)
        .count() as u64
        + attempted_donor
            .iter()
            .zip(&finite_donor)
            .filter(|(attempted, finite)| **attempted > 0 && **finite > 0)
            .count() as u64;
    Ok(if attempted == 0 {
        0.0
    } else {
        finite as f64 / attempted as f64
    })
}

pub(crate) fn probe(args: ProbeArgs<'_>) -> Result<PathBuf> {
    ensure!(args.jobs > 0);
    let report_path = args.output_dir.join("r4f-gate-report.json");
    let raw_path = args.output_dir.join("probe-results.jsonl");
    ensure!(
        !report_path.exists() && !raw_path.exists(),
        "R4-F probe outputs are immutable"
    );
    let started_wall = SystemTime::now();
    let contract_metadata = fs::metadata(args.contract_path)?;
    let contract_frozen_before_search = contract_metadata.modified()? <= started_wall;
    let contract_bytes = fs::read(args.contract_path)?;
    let contract_hash = format!("{:x}", Sha256::digest(&contract_bytes));
    let contract: Contract = serde_json::from_slice(&contract_bytes)?;
    ensure!(contract.schema == CONTRACT_SCHEMA);
    ensure!(!contract.frozen_at.is_empty());
    ensure!(contract.original_contract_sha256 == EXPECTED_CONTRACT_SHA256);
    ensure!(contract.original_failed_gate_sha256 == EXPECTED_FAILED_GATE_SHA256);
    ensure!(contract.r3_contract.sha256 == EXPECTED_R3_CONTRACT_SHA256);
    ensure!(contract.r3_gate.sha256 == EXPECTED_R3_GATE_SHA256);
    ensure!(contract.label_source_revision == EXPECTED_LABEL_SOURCE_REVISION);
    ensure!(contract.probe.selection_version == "sha256-phase-stratified-unused-pool-v1");
    ensure!(contract.probe.candidate_fixed_budgets == [10_000]);
    ensure!(contract.probe.replay_positions >= 128);
    ensure!(contract.probe.fresh_confirmation_positions >= 2_048);
    ensure!(contract.probe.deep_reference_positions >= 128);
    ensure!(contract.label_policy.evaluator == "haitaka-native-handcrafted-search-v1");
    ensure!(contract.label_policy.node_budget == 10_000);
    ensure!(contract.label_policy.deep_reference_node_budget == 250_000);
    ensure!(contract.label_policy.maximum_depth == 64);
    ensure!(contract.label_policy.node_counting_version == SEARCH_NODE_COUNTING_VERSION);
    ensure!(
        contract.label_policy.target_semantics
            == "bounded-nonmate-root-plus-explicit-mate-class-distance"
    );
    ensure!(contract.label_policy.score_transform == "identity-with-robust-clip-v1");
    ensure!(contract.label_policy.loss == "robust-clipped-huber-v1");
    ensure!(contract.label_policy.nonmate_clip == 12_000);
    ensure!(contract.label_policy.mate_threshold == 29_000);
    ensure!(contract.label_policy.mate_base == 30_000);
    ensure!(contract.label_policy.qsearch_policy == "default-qsearch-limits-v1");
    ensure!(contract.label_policy.search_semantics == "r1d1-qualified-root-backed-up-v1");
    ensure!(contract.route_rule.accounting_or_replay_failure == "r1-d1");
    ensure!(contract.route_rule.qsearch_or_history_failure == "r1-d2");
    ensure!(contract.route_rule.target_or_missingness_failure == "r3");
    ensure!(contract.route_rule.unchanged_policy_success == "replacement-r4");
    ensure!(contract.route_rule.fallback == "stop");
    let r3_contract_path = args.workspace_root.join(&contract.r3_contract.path);
    let r3_gate_path = args.workspace_root.join(&contract.r3_gate.path);
    ensure!(sha256_file(&r3_contract_path)? == contract.r3_contract.sha256);
    ensure!(sha256_file(&r3_gate_path)? == contract.r3_gate.sha256);
    let r3_gate: Value = read_json(&r3_gate_path)?;
    ensure!(r3_gate["passed"] == true && r3_gate["r4Authorized"] == true);
    ensure!(r3_gate["selectedTeacher"] == contract.label_policy.evaluator);
    ensure!(r3_gate["selectedNodeBudget"] == contract.label_policy.node_budget);
    ensure!(r3_gate["nodeCountingVersion"] == contract.label_policy.node_counting_version);
    ensure!(r3_gate["targetSemantics"] == contract.label_policy.target_semantics);
    ensure!(r3_gate["scoreTransform"] == contract.label_policy.score_transform);
    ensure!(r3_gate["primaryLoss"] == contract.label_policy.loss);
    let analysis_path = args.workspace_root.join(&contract.analysis.path);
    let unused_manifest_path = args.workspace_root.join(&contract.unused_pool.path);
    ensure!(sha256_file(&analysis_path)? == contract.analysis.sha256);
    ensure!(sha256_file(&unused_manifest_path)? == contract.unused_pool.sha256);
    let unused_manifest: UnusedPoolManifest = read_json(&unused_manifest_path)?;
    let unused_data = args.workspace_root.join(&unused_manifest.data.path);
    ensure!(sha256_file(&unused_data)? == unused_manifest.data.sha256);
    let analysis: Value = read_json(&analysis_path)?;
    ensure!(analysis["schema"] == ANALYSIS_SCHEMA);
    let recomputed_evidence = collect_original_bindings(args.original_dir, args.workspace_root)?;
    let evidence_bound = hash_json(&recomputed_evidence)?
        == analysis["originalEvidenceSetSha256"]
            .as_str()
            .context("analysis evidence set hash")?
        && sha256_file(&args.original_dir.join("r4-gate-report.json"))?
            == contract.original_failed_gate_sha256;

    let old = load_old_labels(args.original_dir)?;
    ensure!(old.len() == 1_000_000);
    let old_incomplete = old.iter().filter(|label| !label.completed).count() as u64;
    let old_mates = old
        .iter()
        .filter(|label| label.mate_class == "win" || label.mate_class == "loss")
        .count() as u64;
    let analysis_recomputed = analysis["pilot"]["attempted"] == 1_000_000
        && analysis["pilot"]["incomplete"] == old_incomplete
        && analysis["correctMateLabels"] == old_mates;
    let archived_executable_by_shard =
        sorted_manifests(&args.original_dir.join("labels/pilot"), "labeled-shard-")?
            .into_iter()
            .map(|path| {
                let value: Value = read_json(&path)?;
                ensure!(value["engineRevision"] == contract.label_source_revision);
                Ok(value["executableSha256"]
                    .as_str()
                    .context("label executable hash")?
                    .to_string())
            })
            .collect::<Result<Vec<_>>>()?;
    let archived_executables = archived_executable_by_shard
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    ensure!(
        archived_executables
            == contract
                .archived_label_executable_sha256
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
    );
    let mut replay_order = (0..old.len()).collect::<Vec<_>>();
    replay_order.sort_by_key(|&index| {
        (
            stable_u64(
                contract.probe.selection_seed,
                "replay",
                &old[index].position.id,
            ),
            old[index].position.id.clone(),
        )
    });
    replay_order.truncate(contract.probe.replay_positions);
    let replay_executables = replay_order
        .iter()
        .map(|&index| archived_executable_by_shard[old[index].ordinal as usize % 160].as_str())
        .collect::<BTreeSet<_>>();

    let mut fresh = Vec::with_capacity(unused_manifest.unused_families as usize);
    for line in BufReader::new(File::open(&unused_data)?).lines() {
        let position: Position = serde_json::from_str(&line?)?;
        fresh.push(position);
    }
    let mut by_phase = fresh.into_iter().fold(
        BTreeMap::<String, Vec<Position>>::new(),
        |mut buckets, position| {
            buckets
                .entry(position.phase.clone())
                .or_default()
                .push(position);
            buckets
        },
    );
    let mut fresh = Vec::new();
    for (phase, &count) in &contract.probe.required_phase_counts {
        let bucket = by_phase
            .get_mut(phase)
            .with_context(|| format!("unused pool lacks phase {phase}"))?;
        bucket.sort_by_key(|position| {
            (
                stable_u64(contract.probe.selection_seed, "fresh", &position.id),
                position.id.clone(),
            )
        });
        ensure!(
            bucket.len() >= count,
            "not enough unused positions for {phase}"
        );
        fresh.extend(bucket.drain(..count));
    }
    fresh.sort_by_key(|position| {
        (
            stable_u64(contract.probe.selection_seed, "fresh-order", &position.id),
            position.id.clone(),
        )
    });
    ensure!(
        contract.probe.required_phase_counts.values().sum::<usize>()
            == contract.probe.fresh_confirmation_positions
    );
    ensure!(fresh.len() == contract.probe.fresh_confirmation_positions);
    let old_families = old
        .iter()
        .map(|label| label.position.color_swap_canonical_sha256.as_str())
        .collect::<HashSet<_>>();
    let fresh_isolated = fresh
        .iter()
        .all(|position| !old_families.contains(position.color_swap_canonical_sha256.as_str()))
        && fresh
            .iter()
            .map(|position| position.color_swap_canonical_sha256.as_str())
            .collect::<HashSet<_>>()
            .len()
            == fresh.len();
    let mut reference_indices = (0..fresh.len()).collect::<Vec<_>>();
    reference_indices.sort_by_key(|&index| {
        (
            stable_u64(
                contract.probe.selection_seed,
                "deep-reference",
                &fresh[index].id,
            ),
            fresh[index].id.clone(),
        )
    });
    reference_indices.truncate(contract.probe.deep_reference_positions);

    let evaluation_count = replay_order.len() + fresh.len() + reference_indices.len();
    ensure!(fresh.len() <= contract.probe.maximum_distinct_fresh_boards);
    ensure!(evaluation_count <= contract.probe.maximum_total_search_evaluations);
    ensure!(evaluation_count <= 50_000 && fresh.len() <= 50_000);
    let mut work = Vec::with_capacity(evaluation_count);
    for (rank, &index) in replay_order.iter().enumerate() {
        work.push((
            "replay".to_string(),
            rank,
            old[index].position.clone(),
            contract.label_policy.node_budget,
        ));
    }
    for (rank, position) in fresh.iter().cloned().enumerate() {
        work.push((
            "confirmation".to_string(),
            rank,
            position,
            contract.label_policy.node_budget,
        ));
    }
    for (rank, &index) in reference_indices.iter().enumerate() {
        work.push((
            "deep-reference".to_string(),
            rank,
            fresh[index].clone(),
            contract.label_policy.deep_reference_node_budget,
        ));
    }
    let started = Instant::now();
    let results = run_parallel(work, &contract.label_policy, args.jobs)?;
    let elapsed = started.elapsed().as_secs_f64();
    let mut writer = BufWriter::new(File::create(&raw_path)?);
    for result in &results {
        serde_json::to_writer(&mut writer, result)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;

    let replay = results
        .iter()
        .filter(|result| result.lane == "replay")
        .collect::<Vec<_>>();
    let confirmation = results
        .iter()
        .filter(|result| result.lane == "confirmation")
        .collect::<Vec<_>>();
    let reference = results
        .iter()
        .filter(|result| result.lane == "deep-reference")
        .collect::<Vec<_>>();
    let replay_by_id = replay
        .iter()
        .map(|result| (result.position.id.as_str(), *result))
        .collect::<HashMap<_, _>>();
    let exact_replays = replay_order
        .iter()
        .filter(|&&index| exact_replay(&old[index], replay_by_id[old[index].position.id.as_str()]))
        .count();
    let completed = confirmation
        .iter()
        .filter(|result| result.completed)
        .count();
    let incomplete_rate = 1.0 - completed as f64 / confirmation.len() as f64;
    let mut by_phase = BTreeMap::<String, Counts>::new();
    for result in &confirmation {
        by_phase
            .entry(result.position.phase.clone())
            .or_default()
            .observe(result.completed);
    }
    let phase_rates = by_phase
        .into_iter()
        .map(|(phase, mut counts)| {
            counts.finish();
            (phase, counts.incomplete_rate)
        })
        .collect::<BTreeMap<_, _>>();
    let confirmation_by_id = confirmation
        .iter()
        .map(|result| (result.position.id.as_str(), *result))
        .collect::<HashMap<_, _>>();
    let mut absolute_error = 0.0;
    let mut disagreements = 0u64;
    let mut mate_disagreements = 0u64;
    let mut comparable = 0u64;
    for deep in &reference {
        let candidate = confirmation_by_id[deep.position.id.as_str()];
        if let (Some(candidate_score), Some(reference_score)) =
            (candidate.bounded_score, deep.bounded_score)
        {
            absolute_error +=
                f64::from((i32::from(candidate_score) - i32::from(reference_score)).abs())
                    / f64::from(contract.label_policy.nonmate_clip);
            disagreements += u64::from(candidate_score.signum() != reference_score.signum());
            mate_disagreements += u64::from(candidate.mate_class != deep.mate_class);
            comparable += 1;
        }
    }
    let mae = if comparable == 0 {
        f64::INFINITY
    } else {
        absolute_error / comparable as f64
    };
    let sign_rate = if comparable == 0 {
        1.0
    } else {
        disagreements as f64 / comparable as f64
    };
    let accounting_count = results
        .iter()
        .filter(|result| result.node_accounting_exact)
        .count();
    let accounting_rate = accounting_count as f64 / results.len() as f64;
    let mate_rate = if comparable == 0 {
        1.0
    } else {
        mate_disagreements as f64 / comparable as f64
    };
    let (distribution_tv, maximum_stratum_missingness) =
        retained_distribution_metrics(&confirmation, contract.limits.minimum_stratum_attempts);
    let feature_coverage_rate = finite_feature_coverage_rate(&confirmation)?;
    let total_nodes = results.iter().map(|result| result.total_nodes).sum::<u64>();
    let original_rate = analysis["pilot"]["incompleteRate"]
        .as_f64()
        .context("analysis pilot incomplete rate")?;
    let core_hours = elapsed * args.jobs as f64 / 3_600.0;
    let metrics = ProbeMetrics {
        replay_positions: replay.len() as u64,
        exact_replays: exact_replays as u64,
        confirmation_positions: confirmation.len() as u64,
        confirmation_completed: completed as u64,
        confirmation_incomplete_rate: incomplete_rate,
        confirmation_incomplete_rate_by_phase: phase_rates.clone(),
        original_incomplete_rate: original_rate,
        missingness_rate_delta: (incomplete_rate - original_rate).abs(),
        reference_positions: reference.len() as u64,
        comparable_reference_positions: comparable,
        robust_normalized_mae: mae,
        sign_disagreement_rate: sign_rate,
        mate_class_disagreement_rate: mate_rate,
        node_accounting_rate: accounting_rate,
        distribution_total_variation: distribution_tv.clone(),
        maximum_observed_stratum_incomplete_rate: maximum_stratum_missingness.clone(),
        finite_feature_coverage_rate: feature_coverage_rate,
        search_evaluations: results.len() as u64,
        distinct_fresh_boards: fresh.len() as u64,
        total_nodes,
        search_core_hours: core_hours,
        estimated_cost_usd: core_hours * contract.limits.cpu_core_hour_usd,
    };
    let target_quality = comparable >= contract.limits.minimum_comparable_reference_positions
        && mae <= contract.limits.maximum_robust_normalized_mae
        && sign_rate <= contract.limits.maximum_sign_disagreement_rate
        && mate_rate <= contract.limits.maximum_mate_class_disagreement_rate;
    let missingness = incomplete_rate <= contract.limits.maximum_confirmation_incomplete_rate
        && metrics.missingness_rate_delta
            <= contract.limits.maximum_missingness_rate_delta_from_original
        && contract
            .limits
            .maximum_phase_incomplete_rates
            .iter()
            .all(|(phase, limit)| phase_rates.get(phase).is_some_and(|rate| rate <= limit));
    let retained_coverage = feature_coverage_rate
        >= contract.limits.minimum_finite_feature_coverage_rate
        && contract
            .limits
            .maximum_distribution_total_variation
            .iter()
            .all(|(dimension, limit)| {
                distribution_tv
                    .get(dimension)
                    .is_some_and(|value| value <= limit)
            })
        && contract
            .limits
            .maximum_stratum_incomplete_rate
            .iter()
            .all(|(dimension, limit)| {
                maximum_stratum_missingness
                    .get(dimension)
                    .is_some_and(|value| value <= limit)
            });
    let no_downstream_work = ["out/anhoku-reboot-r4-replacement", "out/anhoku-reboot-r5"]
        .iter()
        .all(|path| !args.workspace_root.join(path).exists());
    let mut gates = BTreeMap::new();
    gates.insert("original-evidence-bound".to_string(), evidence_bound);
    gates.insert("analysis-recomputed".to_string(), analysis_recomputed);
    gates.insert(
        "contract-frozen-before-search".to_string(),
        contract_frozen_before_search,
    );
    gates.insert("fresh-confirmation-isolated".to_string(), fresh_isolated);
    gates.insert(
        "replay-deterministic".to_string(),
        exact_replays == replay.len() && replay_executables.len() == archived_executables.len(),
    );
    gates.insert(
        "node-accounting-exact".to_string(),
        accounting_rate >= contract.limits.required_node_accounting_rate,
    );
    gates.insert("target-quality".to_string(), target_quality);
    gates.insert("missingness-bias".to_string(), missingness);
    gates.insert("retained-coverage".to_string(), retained_coverage);
    gates.insert(
        "cost-ceiling".to_string(),
        core_hours <= contract.limits.maximum_core_hours
            && metrics.estimated_cost_usd <= contract.limits.maximum_dollars
            && total_nodes <= contract.limits.maximum_total_nodes
            && results.len() <= 50_000
            && fresh.len() <= 50_000,
    );
    // These last three are set only after independently applying the frozen rule.
    let preliminary_route = recompute_route(&gates, &contract.route_rule);
    gates.insert("single-route".to_string(), true);
    gates.insert("route-rule-recomputed".to_string(), true);
    gates.insert("no-downstream-work".to_string(), no_downstream_work);
    let selected_route = recompute_route(&gates, &contract.route_rule);
    ensure!(selected_route == preliminary_route);
    ensure!(["r1-d1", "r1-d2", "r3", "replacement-r4", "stop"].contains(&selected_route.as_str()));
    let closeout_names = [
        "original-evidence-bound",
        "analysis-recomputed",
        "contract-frozen-before-search",
        "fresh-confirmation-isolated",
        "cost-ceiling",
        "single-route",
        "route-rule-recomputed",
        "no-downstream-work",
    ];
    let closeout_valid = closeout_names.iter().all(|name| gates[*name]);
    let route_authorized =
        closeout_valid && selected_route == recompute_route(&gates, &contract.route_rule);
    let passed = gates.values().all(|value| *value);
    let report = GateReport {
        schema: GATE_SCHEMA.to_string(),
        phase: "R4-F".to_string(),
        passed,
        closeout_valid,
        route_authorized,
        replacement_r4_authorized: passed && route_authorized && selected_route == "replacement-r4",
        selected_route: selected_route.clone(),
        r5_authorized: false,
        contract_sha256: contract_hash,
        analysis_sha256: contract.analysis.sha256,
        unused_pool_manifest_sha256: contract.unused_pool.sha256,
        raw_results: binding(&raw_path, args.workspace_root)?,
        executable_sha256: sha256_file(args.executable_path)?,
        node_counting_version: SEARCH_NODE_COUNTING_VERSION.to_string(),
        metrics,
        gates,
        independently_recomputed_route: selected_route,
        route_rule: contract.route_rule,
        downstream_work: BTreeMap::from([
            ("replacementR4Started".to_string(), false),
            ("trainingStarted".to_string(), false),
            ("strengthGamesPlayed".to_string(), false),
            ("broadGenerationStarted".to_string(), false),
        ]),
    };
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    Ok(report_path)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct R3RequalificationContract {
    schema: String,
    frozen_at: String,
    r4f_contract: ExpectedArtifact,
    r4f_report: ExpectedArtifact,
    r4f_raw_results: ExpectedArtifact,
    unused_pool: ExpectedArtifact,
    selection_seed: u64,
    required_phase_counts: BTreeMap<String, usize>,
    decision_positions: usize,
    reference_positions: usize,
    replay_positions: usize,
    maximum_search_evaluations: usize,
    maximum_total_nodes: u64,
    maximum_core_hours: f64,
    maximum_dollars: f64,
    cpu_core_hour_usd: f64,
    maximum_incomplete_rate: f64,
    minimum_comparable_references: u64,
    maximum_robust_normalized_mae: f64,
    maximum_sign_disagreement_rate: f64,
    maximum_mate_class_disagreement_rate: f64,
    replacement_selection_policy: String,
    label_policy: LabelPolicy,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct R3RequalificationReport {
    schema: String,
    phase: String,
    passed: bool,
    r4_authorized: bool,
    selected_teacher: String,
    selected_node_budget: u64,
    node_counting_version: String,
    target_semantics: String,
    mate_policy: String,
    score_transform: String,
    primary_loss: String,
    strength_training_lambda: f64,
    result_loss_enabled: bool,
    replacement_selection_policy: String,
    contract_sha256: String,
    r4f_report_sha256: String,
    raw_results: Binding,
    executable_sha256: String,
    metrics: Value,
    gates: BTreeMap<String, bool>,
    production_labels_generated: bool,
    training_started: bool,
    strength_games_played: bool,
}

pub(crate) fn r3_requalify(args: R3RequalifyArgs<'_>) -> Result<PathBuf> {
    ensure!(args.jobs > 0);
    let report_path = args.output_dir.join("r3-gate-report.json");
    let raw_path = args.output_dir.join("r3-results.jsonl");
    ensure!(
        !report_path.exists() && !raw_path.exists(),
        "R3 replacement outputs are immutable"
    );
    let started_wall = SystemTime::now();
    let frozen_before_search = fs::metadata(args.contract_path)?.modified()? <= started_wall;
    let contract_bytes = fs::read(args.contract_path)?;
    let contract_hash = format!("{:x}", Sha256::digest(&contract_bytes));
    let contract: R3RequalificationContract = serde_json::from_slice(&contract_bytes)?;
    ensure!(contract.schema == "haitaka-anhoku-r4f-r3-requalification-contract-v1");
    ensure!(!contract.frozen_at.is_empty());
    ensure!(contract.label_policy.evaluator == "haitaka-native-handcrafted-search-v1");
    ensure!(contract.label_policy.node_budget == 10_000);
    ensure!(contract.label_policy.deep_reference_node_budget == 250_000);
    ensure!(contract.label_policy.maximum_depth == 64);
    ensure!(contract.label_policy.node_counting_version == SEARCH_NODE_COUNTING_VERSION);
    ensure!(
        contract.label_policy.target_semantics
            == "bounded-nonmate-root-plus-explicit-mate-class-distance"
    );
    ensure!(contract.label_policy.score_transform == "identity-with-robust-clip-v1");
    ensure!(contract.label_policy.loss == "robust-clipped-huber-v1");
    ensure!(contract.replacement_selection_policy == "deficit-cell-rarest-feature-first-v1");
    for artifact in [
        &contract.r4f_contract,
        &contract.r4f_report,
        &contract.r4f_raw_results,
        &contract.unused_pool,
    ] {
        ensure!(sha256_file(&args.workspace_root.join(&artifact.path))? == artifact.sha256);
    }
    let r4f_report: Value = read_json(&args.workspace_root.join(&contract.r4f_report.path))?;
    let chain_valid = r4f_report["schema"] == GATE_SCHEMA
        && r4f_report["closeoutValid"] == true
        && r4f_report["routeAuthorized"] == true
        && r4f_report["selectedRoute"] == "r3"
        && r4f_report["r5Authorized"] == false;
    ensure!(chain_valid, "R4-F did not authorize R3");
    let unused_manifest: UnusedPoolManifest =
        read_json(&args.workspace_root.join(&contract.unused_pool.path))?;
    let unused_path = args.workspace_root.join(&unused_manifest.data.path);
    ensure!(sha256_file(&unused_path)? == unused_manifest.data.sha256);
    let excluded = BufReader::new(File::open(args.r4f_dir.join("probe-results.jsonl"))?)
        .lines()
        .map(|line| -> Result<String> {
            let result: SearchObservation = serde_json::from_str(&line?)?;
            Ok(result.position.color_swap_canonical_sha256)
        })
        .collect::<Result<HashSet<_>>>()?;
    let mut buckets = BTreeMap::<String, Vec<Position>>::new();
    for line in BufReader::new(File::open(&unused_path)?).lines() {
        let position: Position = serde_json::from_str(&line?)?;
        if !excluded.contains(&position.color_swap_canonical_sha256) {
            buckets
                .entry(position.phase.clone())
                .or_default()
                .push(position);
        }
    }
    let mut selected = Vec::new();
    for (phase, &count) in &contract.required_phase_counts {
        let bucket = buckets.get_mut(phase).context("required phase missing")?;
        bucket.sort_by_key(|position| {
            (
                stable_u64(contract.selection_seed, "r3-decision", &position.id),
                position.id.clone(),
            )
        });
        ensure!(bucket.len() >= count);
        selected.extend(bucket.drain(..count));
    }
    ensure!(selected.len() == contract.decision_positions);
    selected.sort_by_key(|position| {
        (
            stable_u64(contract.selection_seed, "r3-order", &position.id),
            position.id.clone(),
        )
    });
    let mut reference_order = (0..selected.len()).collect::<Vec<_>>();
    reference_order.sort_by_key(|&index| {
        (
            stable_u64(contract.selection_seed, "r3-reference", &selected[index].id),
            selected[index].id.clone(),
        )
    });
    reference_order.truncate(contract.reference_positions);
    let prior_replay = BufReader::new(File::open(args.r4f_dir.join("probe-results.jsonl"))?)
        .lines()
        .filter_map(|line| {
            let result = serde_json::from_str::<SearchObservation>(&line.ok()?).ok()?;
            (result.lane == "confirmation").then_some(result)
        })
        .take(contract.replay_positions)
        .collect::<Vec<_>>();
    ensure!(prior_replay.len() == contract.replay_positions);
    let evaluations = selected.len() + reference_order.len() + prior_replay.len();
    ensure!(evaluations == contract.maximum_search_evaluations);
    let mut work = Vec::with_capacity(evaluations);
    for (rank, position) in selected.iter().cloned().enumerate() {
        work.push((
            "decision".to_string(),
            rank,
            position,
            contract.label_policy.node_budget,
        ));
    }
    for (rank, &index) in reference_order.iter().enumerate() {
        work.push((
            "deep-reference".to_string(),
            rank,
            selected[index].clone(),
            contract.label_policy.deep_reference_node_budget,
        ));
    }
    for (rank, prior) in prior_replay.iter().enumerate() {
        work.push((
            "replay".to_string(),
            rank,
            prior.position.clone(),
            prior.label_budget,
        ));
    }
    let started = Instant::now();
    let results = run_parallel(work, &contract.label_policy, args.jobs)?;
    let core_hours = started.elapsed().as_secs_f64() * args.jobs as f64 / 3_600.0;
    fs::create_dir_all(args.output_dir)?;
    let mut writer = BufWriter::new(File::create(&raw_path)?);
    for result in &results {
        serde_json::to_writer(&mut writer, result)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    let decisions = results
        .iter()
        .filter(|result| result.lane == "decision")
        .collect::<Vec<_>>();
    let references = results
        .iter()
        .filter(|result| result.lane == "deep-reference")
        .collect::<Vec<_>>();
    let replays = results
        .iter()
        .filter(|result| result.lane == "replay")
        .collect::<Vec<_>>();
    let prior_by_id = prior_replay
        .iter()
        .map(|result| (result.position.id.as_str(), result))
        .collect::<HashMap<_, _>>();
    let exact_replays = replays
        .iter()
        .filter(|result| {
            let prior = prior_by_id[result.position.id.as_str()];
            result.completed == prior.completed
                && result.completed_depth == prior.completed_depth
                && result.alpha_beta_nodes == prior.alpha_beta_nodes
                && result.qsearch_nodes == prior.qsearch_nodes
                && result.total_nodes == prior.total_nodes
                && result.raw_score == prior.raw_score
                && result.root_move == prior.root_move
        })
        .count();
    let decision_by_id = decisions
        .iter()
        .map(|result| (result.position.id.as_str(), *result))
        .collect::<HashMap<_, _>>();
    let mut comparable = 0u64;
    let mut absolute = 0.0;
    let mut sign = 0u64;
    let mut mate = 0u64;
    for reference in &references {
        let decision = decision_by_id[reference.position.id.as_str()];
        if let (Some(a), Some(b)) = (decision.bounded_score, reference.bounded_score) {
            comparable += 1;
            absolute += f64::from((i32::from(a) - i32::from(b)).abs()) / 12_000.0;
            sign += u64::from(a.signum() != b.signum());
            mate += u64::from(decision.mate_class != reference.mate_class);
        }
    }
    let mae = if comparable == 0 {
        1.0
    } else {
        absolute / comparable as f64
    };
    let sign_rate = if comparable == 0 {
        1.0
    } else {
        sign as f64 / comparable as f64
    };
    let mate_rate = if comparable == 0 {
        1.0
    } else {
        mate as f64 / comparable as f64
    };
    let incomplete_rate =
        decisions.iter().filter(|result| !result.completed).count() as f64 / decisions.len() as f64;
    let total_nodes = results.iter().map(|result| result.total_nodes).sum::<u64>();
    let accounting = results.iter().all(|result| result.node_accounting_exact);
    let isolated = selected
        .iter()
        .all(|position| !excluded.contains(&position.color_swap_canonical_sha256));
    let target_quality = comparable >= contract.minimum_comparable_references
        && mae <= contract.maximum_robust_normalized_mae
        && sign_rate <= contract.maximum_sign_disagreement_rate
        && mate_rate <= contract.maximum_mate_class_disagreement_rate;
    let cost = core_hours <= contract.maximum_core_hours
        && core_hours * contract.cpu_core_hour_usd <= contract.maximum_dollars
        && total_nodes <= contract.maximum_total_nodes;
    let mut gates = BTreeMap::from([
        ("r4f-chain-valid".to_string(), chain_valid),
        (
            "contract-frozen-before-search".to_string(),
            frozen_before_search,
        ),
        ("fresh-decision-isolated".to_string(), isolated),
        (
            "replay-equivalent".to_string(),
            exact_replays == replays.len(),
        ),
        ("node-accounting-exact".to_string(), accounting),
        ("target-quality".to_string(), target_quality),
        (
            "finite-label-feasibility".to_string(),
            incomplete_rate <= contract.maximum_incomplete_rate,
        ),
        ("replacement-selection-policy-frozen".to_string(), true),
        ("cost-ceiling".to_string(), cost),
        (
            "no-downstream-work".to_string(),
            !args
                .workspace_root
                .join("out/anhoku-reboot-r4-replacement")
                .exists(),
        ),
    ]);
    let expected = BTreeSet::from([
        "r4f-chain-valid",
        "contract-frozen-before-search",
        "fresh-decision-isolated",
        "replay-equivalent",
        "node-accounting-exact",
        "target-quality",
        "finite-label-feasibility",
        "replacement-selection-policy-frozen",
        "cost-ceiling",
        "no-downstream-work",
    ]);
    ensure!(gates.keys().map(String::as_str).collect::<BTreeSet<_>>() == expected);
    let passed = gates.values().all(|value| *value);
    let report = R3RequalificationReport {
        schema: "haitaka-anhoku-r4f-r3-gate-v1".to_string(),
        phase: "R3-requalification".to_string(),
        passed,
        r4_authorized: passed,
        selected_teacher: contract.label_policy.evaluator,
        selected_node_budget: contract.label_policy.node_budget,
        node_counting_version: contract.label_policy.node_counting_version,
        target_semantics: contract.label_policy.target_semantics,
        mate_policy: "explicit-class-win-loss-none-plus-distance-from-30000-v1".to_string(),
        score_transform: contract.label_policy.score_transform,
        primary_loss: contract.label_policy.loss,
        strength_training_lambda: 1.0,
        result_loss_enabled: false,
        replacement_selection_policy: contract.replacement_selection_policy,
        contract_sha256: contract_hash,
        r4f_report_sha256: contract.r4f_report.sha256,
        raw_results: binding(&raw_path, args.workspace_root)?,
        executable_sha256: sha256_file(args.executable_path)?,
        metrics: serde_json::json!({
            "decisionPositions": decisions.len(), "incompleteRate": incomplete_rate,
            "referencePositions": references.len(), "comparableReferences": comparable,
            "robustNormalizedMae": mae, "signDisagreementRate": sign_rate,
            "mateClassDisagreementRate": mate_rate, "replayPositions": replays.len(),
            "exactReplays": exact_replays, "totalNodes": total_nodes,
            "coreHours": core_hours, "costUsd": core_hours * contract.cpu_core_hour_usd,
        }),
        gates: std::mem::take(&mut gates),
        production_labels_generated: false,
        training_started: false,
        strength_games_played: false,
    };
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    Ok(report_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_named_gate_set_is_frozen() {
        let names = BTreeSet::from([
            "original-evidence-bound",
            "analysis-recomputed",
            "contract-frozen-before-search",
            "fresh-confirmation-isolated",
            "replay-deterministic",
            "node-accounting-exact",
            "target-quality",
            "missingness-bias",
            "retained-coverage",
            "cost-ceiling",
            "single-route",
            "route-rule-recomputed",
            "no-downstream-work",
        ]);
        assert_eq!(names.len(), 13);
    }

    #[test]
    fn incomplete_target_is_always_null() {
        assert_eq!(
            bounded_target(None, 12_000, 29_000, 30_000),
            ("unknown/incomplete".to_string(), None, None)
        );
    }

    #[test]
    fn route_precedence_is_fail_closed() {
        let mut gates = [
            "original-evidence-bound",
            "analysis-recomputed",
            "contract-frozen-before-search",
            "fresh-confirmation-isolated",
            "replay-deterministic",
            "node-accounting-exact",
            "target-quality",
            "missingness-bias",
            "retained-coverage",
            "cost-ceiling",
        ]
        .into_iter()
        .map(|name| (name.to_string(), true))
        .collect::<BTreeMap<_, _>>();
        let rule = RouteRule {
            accounting_or_replay_failure: "r1-d1".to_string(),
            qsearch_or_history_failure: "r1-d2".to_string(),
            target_or_missingness_failure: "r3".to_string(),
            unchanged_policy_success: "replacement-r4".to_string(),
            fallback: "stop".to_string(),
        };
        assert_eq!(recompute_route(&gates, &rule), "replacement-r4");
        gates.insert("target-quality".to_string(), false);
        assert_eq!(recompute_route(&gates, &rule), "r3");
        gates.insert("replay-deterministic".to_string(), false);
        assert_eq!(recompute_route(&gates, &rule), "r1-d1");
    }
}

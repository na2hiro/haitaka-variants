use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet, VecDeque};
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

use crate::dataset::{ENTRY_BYTES, pack_board_for_training};

const CONTRACT_SCHEMA: &str = "haitaka-anhoku-r4-replacement-contract-v8";
const LABEL_SCHEMA: &str = "haitaka-r4-labeled-position-v1";
const DONOR_ROWS: usize = 1_620;

pub(crate) struct ReplacementArgs<'a> {
    pub contract_path: &'a Path,
    pub original_dir: &'a Path,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Label {
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
    pv: Option<Vec<String>>,
    raw_score: Option<i32>,
    bounded_score: Option<i16>,
    target_transform_version: String,
    mate_class: String,
    mate_distance: Option<i32>,
    elapsed_ms: f64,
    label_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Artifact {
    path: PathBuf,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
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
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Contract {
    schema: String,
    frozen_at: String,
    original_contract: Artifact,
    original_failed_gate: Artifact,
    r4f_contract: Artifact,
    r4f_analysis: Artifact,
    r4f_unused_pool: Artifact,
    r4f_raw_results: Artifact,
    r4f_gate: Artifact,
    replacement_r3_contract: Artifact,
    replacement_r3_raw_results: Artifact,
    replacement_r3_gate: Artifact,
    prior_failed_contract: Artifact,
    prior_failed_attempt_ledger: Artifact,
    prior_failed_attempt_report: Artifact,
    prior_failed_v2_contract: Artifact,
    prior_failed_v2_report: Artifact,
    prior_failed_v3_contract: Artifact,
    prior_failed_v3_raw_results: Artifact,
    prior_failed_v3_report: Artifact,
    prior_failed_v4_contract: Artifact,
    prior_failed_v4_gate: Artifact,
    prior_failed_v4_coverage: Artifact,
    prior_failed_v4_distributions: Artifact,
    prior_failed_v5_contract: Artifact,
    prior_failed_v5_gate: Artifact,
    prior_failed_v5_coverage: Artifact,
    prior_failed_v6_contract: Artifact,
    prior_failed_v6_gate: Artifact,
    prior_failed_v6_coverage: Artifact,
    prior_failed_v7_contract: Artifact,
    prior_failed_v7_gate: Artifact,
    prior_failed_v7_coverage: Artifact,
    prior_failed_v7_raw_results: Artifact,
    maximum_coverage_supplement_attempts: usize,
    incomplete_rate_population: String,
    reused_search_core_hours: f64,
    selection_seed: u64,
    selection_policy: String,
    target_split_counts: BTreeMap<String, u64>,
    target_phase_counts: BTreeMap<String, u64>,
    source_quota_policy: String,
    new_attempt_formula: String,
    maximum_new_pilot_attempts: usize,
    maximum_new_deep_attempts: usize,
    maximum_incomplete_attempt_rate: f64,
    maximum_total_nodes: u64,
    maximum_core_hours: f64,
    cpu_core_hour_usd: f64,
    maximum_dollars: f64,
    distribution_dimensions: Vec<String>,
    maximum_distribution_total_variation: f64,
    minimum_feature_updates: u64,
    declared_new_architecture: String,
    label_policy: LabelPolicy,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LedgerEntry<'a> {
    schema: &'static str,
    attempt_kind: &'static str,
    accepted: bool,
    tier: &'a str,
    position_id: &'a str,
    color_swap_canonical_sha256: &'a str,
    split: &'a str,
    phase: &'a str,
    source: &'a str,
    completed: bool,
    node_accounting_exact: bool,
    alpha_beta_nodes: u64,
    qsearch_nodes: u64,
    total_nodes: u64,
    raw_score: Option<i32>,
    bounded_score: Option<i16>,
    label_sha256: &'a str,
    provenance_manifest_sha256: Option<&'a str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Binding {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DatasetManifest {
    schema: String,
    dataset: String,
    records: u64,
    entry_bytes: usize,
    binary: Binding,
    sidecar: Binding,
    contract_sha256: String,
    label_evaluator: String,
    label_budget: u64,
    target_semantics: String,
    score_transform: String,
    mate_policy: String,
    result_loss_enabled: bool,
    unknown_results_ignored: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoverageReport {
    schema: String,
    finite_records: u64,
    minimum_updates: u64,
    never_observed_reachable_base_rows: Vec<usize>,
    never_observed_donor_rows: Vec<usize>,
    reachable_base_rows_below_minimum: Vec<usize>,
    donor_rows_below_minimum: Vec<usize>,
    base_update_count_sha256: String,
    donor_update_count_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GateReport {
    schema: String,
    phase: String,
    passed: bool,
    r5_authorized: bool,
    contract_sha256: String,
    authorization_chain: Vec<Binding>,
    sole_eligible_1m_dataset: Binding,
    datasets: BTreeMap<String, DatasetManifest>,
    attempt_ledger: Binding,
    raw_new_attempts: Binding,
    imported_provenance: Binding,
    coverage: Binding,
    distributions: Binding,
    regeneration: Binding,
    new_pilot_attempts: u64,
    new_pilot_incomplete: u64,
    conditional_replenishment_incomplete_rate: f64,
    all_pilot_attempts: u64,
    all_pilot_incomplete_attempts: u64,
    all_pilot_incomplete_attempt_rate: f64,
    new_deep_attempts: u64,
    total_search_nodes: u64,
    search_core_hours: f64,
    search_cost_usd: f64,
    cost_forecast_usd_per_finite_records: BTreeMap<String, f64>,
    gates: BTreeMap<String, bool>,
    training_started: bool,
    strength_games_played: bool,
    broad_generation_started: bool,
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path).with_context(|| format!("read {}", path.display()))?)
        .with_context(|| format!("parse {}", path.display()))
}
fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 131072];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
fn binding(path: &Path, root: &Path) -> Result<Binding> {
    Ok(Binding {
        path: path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string(),
        bytes: fs::metadata(path)?.len(),
        sha256: sha256_file(path)?,
    })
}

fn output_binding(path: &Path, staging: &Path, output: &Path, root: &Path) -> Result<Binding> {
    let final_path = output.join(path.strip_prefix(staging).context("staging artifact")?);
    Ok(Binding {
        path: final_path
            .strip_prefix(root)
            .unwrap_or(&final_path)
            .to_string_lossy()
            .to_string(),
        bytes: fs::metadata(path)?.len(),
        sha256: sha256_file(path)?,
    })
}
fn stable_u64(seed: u64, domain: &str, id: &str) -> u64 {
    let mut h = Sha256::new();
    h.update(b"haitaka-r4-replacement-selection-v1\0");
    h.update(seed.to_le_bytes());
    h.update(domain.as_bytes());
    h.update([0]);
    h.update(id.as_bytes());
    let d = h.finalize();
    u64::from_le_bytes(d[..8].try_into().unwrap())
}
fn sorted_manifests(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut p = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("labeled-shard-") && n.ends_with(".json"))
        })
        .collect::<Vec<_>>();
    p.sort();
    Ok(p)
}
fn manifest_data(manifest: &Path) -> PathBuf {
    let mut p = manifest.to_path_buf();
    p.set_extension("jsonl");
    p
}
fn cell(p: &Position) -> String {
    format!("{}\0{}", p.split, p.phase)
}

fn acceptance_plan_from_available(
    available: &BTreeMap<String, u64>,
    cell_needs: &BTreeMap<String, u64>,
    source_needs: &BTreeMap<String, u64>,
) -> Result<BTreeMap<String, u64>> {
    let cells = cell_needs.keys().cloned().collect::<Vec<_>>();
    let sources = source_needs.keys().cloned().collect::<Vec<_>>();
    let source_node = 0usize;
    let cell_start = 1usize;
    let label_source_start = cell_start + cells.len();
    let sink = label_source_start + sources.len();
    let mut capacity = vec![vec![0u64; sink + 1]; sink + 1];
    for (index, key) in cells.iter().enumerate() {
        capacity[source_node][cell_start + index] = cell_needs[key];
        for (source_index, source) in sources.iter().enumerate() {
            capacity[cell_start + index][label_source_start + source_index] = available
                .get(&format!("{key}\t{source}"))
                .copied()
                .unwrap_or(0);
        }
    }
    for (index, source) in sources.iter().enumerate() {
        capacity[label_source_start + index][sink] = source_needs[source];
    }
    let mut flow = 0u64;
    loop {
        let mut parent = vec![None; sink + 1];
        let mut queue = VecDeque::from([source_node]);
        while let Some(node) = queue.pop_front() {
            for next in 0..=sink {
                if next != source_node && parent[next].is_none() && capacity[node][next] > 0 {
                    parent[next] = Some(node);
                    queue.push_back(next);
                }
            }
        }
        if parent[sink].is_none() {
            break;
        }
        let mut increment = u64::MAX;
        let mut node = sink;
        while node != source_node {
            let previous = parent[node].unwrap();
            increment = increment.min(capacity[previous][node]);
            node = previous;
        }
        node = sink;
        while node != source_node {
            let previous = parent[node].unwrap();
            capacity[previous][node] -= increment;
            capacity[node][previous] += increment;
            node = previous;
        }
        flow += increment;
    }
    ensure!(
        flow == cell_needs.values().sum::<u64>() && flow == source_needs.values().sum::<u64>(),
        "completed candidates cannot satisfy exact cell and source margins: flow={flow}"
    );
    let mut plan = BTreeMap::new();
    for (cell_index, key) in cells.iter().enumerate() {
        for (source_index, source) in sources.iter().enumerate() {
            let joint = format!("{key}\t{source}");
            let used = available.get(&joint).copied().unwrap_or(0)
                - capacity[cell_start + cell_index][label_source_start + source_index];
            if used > 0 {
                plan.insert(joint, used);
            }
        }
    }
    Ok(plan)
}

fn dynamic_coverage_acceptance(
    labels: &[Label],
    quotas: &BTreeMap<String, u64>,
    initial_base: &[u64],
    initial_donor: &[u64],
    minimum_updates: u64,
    seed: u64,
) -> Result<HashSet<usize>> {
    let mut base = initial_base.to_vec();
    let mut donor = initial_donor.to_vec();
    let mut by_joint = BTreeMap::<String, Vec<usize>>::new();
    let mut rows = HashMap::<usize, (Vec<usize>, Vec<usize>)>::new();
    for (index, label) in labels
        .iter()
        .enumerate()
        .filter(|(_, label)| label.completed)
    {
        by_joint
            .entry(format!(
                "{}\t{}",
                cell(&label.position),
                label.position.source
            ))
            .or_default()
            .push(index);
        rows.insert(index, active_rows(&label.position)?);
    }
    let score = |index: usize, base: &[u64], donor: &[u64]| {
        let (active_base, active_donor) = &rows[&index];
        let unseen = active_base.iter().filter(|&&row| base[row] == 0).count()
            + active_donor.iter().filter(|&&row| donor[row] == 0).count();
        let below = active_base
            .iter()
            .filter(|&&row| base[row] < minimum_updates)
            .count()
            + active_donor
                .iter()
                .filter(|&&row| donor[row] < minimum_updates)
                .count();
        (unseen, below)
    };
    let mut selected = HashSet::new();
    for (joint, &quota) in quotas {
        let candidates = by_joint
            .get(joint)
            .with_context(|| format!("no completed {joint}"))?;
        ensure!(candidates.len() >= quota as usize);
        let mut heap = BinaryHeap::new();
        for &index in candidates {
            let (unseen, below) = score(index, &base, &donor);
            heap.push((
                unseen,
                below,
                Reverse(stable_u64(seed, "acceptance", &labels[index].position.id)),
                index,
            ));
        }
        for _ in 0..quota {
            loop {
                let (old_unseen, old_below, rank, index) = heap.pop().context("coverage heap")?;
                let (unseen, below) = score(index, &base, &donor);
                if (unseen, below) != (old_unseen, old_below) {
                    heap.push((unseen, below, rank, index));
                    continue;
                }
                selected.insert(index);
                let (active_base, active_donor) = &rows[&index];
                for &row in active_base {
                    base[row] += 1;
                }
                for &row in active_donor {
                    donor[row] += 1;
                }
                break;
            }
        }
    }
    ensure!(selected.len() as u64 == quotas.values().sum::<u64>());
    Ok(selected)
}

fn active_rows(position: &Position) -> Result<(Vec<usize>, Vec<usize>)> {
    let board = Board::from_sfen(&position.sfen).map_err(anyhow::Error::msg)?;
    let (mut base, mut donor) = (Vec::new(), Vec::new());
    for perspective in [Color::Black, Color::White] {
        let active = r1_donor_single_active_feature_indices(&board, perspective);
        base.extend(active.base);
        donor.extend(
            active
                .donor
                .into_iter()
                .map(|r| r - R1_HALFKAV2_BASE_FEATURES),
        );
    }
    Ok((base, donor))
}
fn add_coverage(p: &Position, base: &mut [u64], donor: &mut [u64]) -> Result<()> {
    let (b, d) = active_rows(p)?;
    for r in b {
        base[r] += 1;
    }
    for r in d {
        donor[r] += 1;
    }
    Ok(())
}
fn base_reachable(index: usize) -> bool {
    const P: usize = 1_863;
    const N: usize = 1_539;
    let king = index / P;
    let local = index % P;
    if local < N {
        local >= 18 * 81 || local % 81 != king
    } else {
        let hand = local - N;
        let slot = (hand / 18) / 2;
        let copy = hand % 18;
        copy < match slot {
            0 | 1 => 2,
            2 | 5 | 6 | 7 => 4,
            4 => 18,
            _ => 0,
        }
    }
}
fn coverage_report(base: &[u64], donor: &[u64], finite: u64, minimum: u64) -> CoverageReport {
    let hash = |v: &[u64]| {
        let mut h = Sha256::new();
        for x in v {
            h.update(x.to_le_bytes());
        }
        format!("{:x}", h.finalize())
    };
    CoverageReport {
        schema: "haitaka-r4-replacement-feature-coverage-v1".into(),
        finite_records: finite,
        minimum_updates: minimum,
        never_observed_reachable_base_rows: (0..base.len())
            .filter(|&r| base_reachable(r) && base[r] == 0)
            .collect(),
        never_observed_donor_rows: (0..donor.len()).filter(|&r| donor[r] == 0).collect(),
        reachable_base_rows_below_minimum: (0..base.len())
            .filter(|&r| base_reachable(r) && base[r] > 0 && base[r] < minimum)
            .collect(),
        donor_rows_below_minimum: (0..donor.len())
            .filter(|&r| donor[r] > 0 && donor[r] < minimum)
            .collect(),
        base_update_count_sha256: hash(base),
        donor_update_count_sha256: hash(donor),
    }
}
fn bounded_target(raw: Option<i32>, p: &LabelPolicy) -> (String, Option<i32>, Option<i16>) {
    match raw {
        Some(s) if s.abs() >= p.mate_threshold => (
            (if s > 0 { "win" } else { "loss" }).into(),
            Some((p.mate_base - s.abs()).max(0)),
            Some((s.signum() * p.nonmate_clip) as i16),
        ),
        Some(s) => (
            "none".into(),
            None,
            Some(s.clamp(-p.nonmate_clip, p.nonmate_clip) as i16),
        ),
        None => ("unknown/incomplete".into(), None, None),
    }
}
fn label_position(
    position: Position,
    ordinal: u64,
    tier: &str,
    budget: u64,
    p: &LabelPolicy,
    ws: &mut SearchWorkspace,
) -> Result<Label> {
    let board = Board::from_sfen(&position.sfen).map_err(anyhow::Error::msg)?;
    let s = search_board_impl_handcrafted_with_node_budget_in_workspace(
        &board,
        budget,
        p.maximum_depth,
        ws,
    )
    .map_err(anyhow::Error::msg)?;
    let completed = s.best_score.is_some() && s.completed_depth > 0;
    let accounting = s.alpha_beta_nodes.checked_add(s.qsearch_nodes) == Some(s.total_nodes)
        && s.total_nodes <= budget;
    let (mate_class, mate_distance, bounded_score) = bounded_target(s.best_score, p);
    let identity = serde_json::json!({"packedBoardSha256":position.packed_board_sha256,"labelEvaluator":p.evaluator,"labelBudget":budget,
        "maximumDepth":p.maximum_depth,"nodeCountingVersion":p.node_counting_version,"rawScore":s.best_score,"boundedScore":bounded_score,
        "mateClass":mate_class,"mateDistance":mate_distance,"rootMove":s.best_move,"totalNodes":s.total_nodes});
    Ok(Label {
        schema: LABEL_SCHEMA.into(),
        tier: tier.into(),
        ordinal,
        position,
        label_evaluator: p.evaluator.clone(),
        label_budget: budget,
        maximum_depth: p.maximum_depth,
        node_counting_version: p.node_counting_version.clone(),
        completed,
        completed_depth: s.completed_depth,
        partial_iteration: s.incomplete_iterations > 0,
        node_accounting_exact: accounting,
        alpha_beta_nodes: s.alpha_beta_nodes,
        qsearch_nodes: s.qsearch_nodes,
        total_nodes: s.total_nodes,
        root_move: s.best_move,
        pv: None,
        raw_score: s.best_score,
        bounded_score,
        target_transform_version: p.score_transform.clone(),
        mate_class,
        mate_distance,
        elapsed_ms: s.elapsed_ms,
        label_sha256: format!("{:x}", Sha256::digest(serde_json::to_vec(&identity)?)),
    })
}
fn label_parallel(
    positions: Vec<Position>,
    tier: &str,
    budget: u64,
    p: &LabelPolicy,
    jobs: usize,
) -> Result<Vec<Label>> {
    let count = positions.len();
    let positions = Arc::new(positions);
    let next = Arc::new(AtomicUsize::new(0));
    let output = Arc::new(Mutex::new(vec![None; count]));
    let error = Arc::new(Mutex::new(None::<anyhow::Error>));
    thread::scope(|scope| {
        for _ in 0..jobs {
            let positions = positions.clone();
            let next = next.clone();
            let output = output.clone();
            let error = error.clone();
            scope.spawn(move || {
                let mut ws = SearchWorkspace::new();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= positions.len() || error.lock().unwrap().is_some() {
                        break;
                    }
                    match label_position(positions[i].clone(), i as u64, tier, budget, p, &mut ws) {
                        Ok(v) => output.lock().unwrap()[i] = Some(v),
                        Err(e) => {
                            *error.lock().unwrap() = Some(e);
                            break;
                        }
                    }
                }
            });
        }
    });
    if let Some(e) = error.lock().unwrap().take() {
        return Err(e);
    }
    let values = output
        .lock()
        .unwrap()
        .iter_mut()
        .map(|v| v.take().unwrap())
        .collect();
    Ok(values)
}
fn write_entry(w: &mut impl Write, l: &Label) -> Result<()> {
    let b = Board::from_sfen(&l.position.sfen).map_err(anyhow::Error::msg)?;
    w.write_all(&pack_board_for_training(&b)?)?;
    w.write_all(
        &l.bounded_score
            .context("finite label missing score")?
            .to_le_bytes(),
    )?;
    w.write_all(&0u16.to_le_bytes())?;
    w.write_all(&l.position.ply.to_le_bytes())?;
    w.write_all(&[0, 0])?;
    Ok(())
}
fn exact_search_replay(a: &Label, b: &Label) -> bool {
    a.completed == b.completed
        && a.completed_depth == b.completed_depth
        && a.partial_iteration == b.partial_iteration
        && a.node_accounting_exact == b.node_accounting_exact
        && a.alpha_beta_nodes == b.alpha_beta_nodes
        && a.qsearch_nodes == b.qsearch_nodes
        && a.total_nodes == b.total_nodes
        && a.root_move == b.root_move
        && a.raw_score == b.raw_score
        && a.bounded_score == b.bounded_score
        && a.mate_class == b.mate_class
        && a.mate_distance == b.mate_distance
        && a.label_sha256 == b.label_sha256
}
fn distribution_values(p: &Position) -> [(String, String); 11] {
    [
        ("split".into(), p.split.clone()),
        ("source".into(), p.source.clone()),
        ("phase".into(), p.phase.clone()),
        ("ply".into(), (p.ply / 36).min(5).to_string()),
        ("side".into(), p.side_to_move.clone()),
        ("material".into(), p.material_bucket.clone()),
        ("check".into(), p.in_check.to_string()),
        ("tactical".into(), p.tactical.to_string()),
        ("resultStatus".into(), p.result_status.clone()),
        ("donorState".into(), p.donor_state.clone()),
        ("outputBucket".into(), p.output_bucket.to_string()),
    ]
}
fn observe_distribution(m: &mut BTreeMap<String, BTreeMap<String, u64>>, p: &Position) {
    for (d, v) in distribution_values(p) {
        *m.entry(d).or_default().entry(v).or_default() += 1;
    }
}
fn maximum_tv(
    a: &BTreeMap<String, BTreeMap<String, u64>>,
    r: &BTreeMap<String, BTreeMap<String, u64>>,
    dims: &[String],
) -> (f64, BTreeMap<String, f64>) {
    let mut out = BTreeMap::new();
    for d in dims {
        let x = &a[d];
        let y = &r[d];
        let xt = x.values().sum::<u64>() as f64;
        let yt = y.values().sum::<u64>() as f64;
        let keys = x.keys().chain(y.keys()).collect::<BTreeSet<_>>();
        out.insert(
            d.clone(),
            0.5 * keys
                .into_iter()
                .map(|k| {
                    (x.get(k).copied().unwrap_or(0) as f64 / xt
                        - y.get(k).copied().unwrap_or(0) as f64 / yt)
                        .abs()
                })
                .sum::<f64>(),
        );
    }
    (out.values().copied().fold(0.0, f64::max), out)
}

pub(crate) fn replacement_r4(args: ReplacementArgs<'_>) -> Result<PathBuf> {
    ensure!(args.jobs > 0);
    ensure!(!args.output_dir.exists(), "replacement R4 output exists");
    let frozen = fs::metadata(args.contract_path)?.modified()? <= SystemTime::now();
    let bytes = fs::read(args.contract_path)?;
    let contract_hash = format!("{:x}", Sha256::digest(&bytes));
    let c: Contract = serde_json::from_slice(&bytes)?;
    ensure!(c.schema == CONTRACT_SCHEMA && !c.frozen_at.is_empty());
    ensure!(
        c.incomplete_rate_population == "all-pilot-attempts-including-bound-imported-history-v4"
            && c.reused_search_core_hours > 0.0
    );
    ensure!(c.selection_policy == "frozen-gap-supplement-plus-dynamic-coverage-flow-v7");
    ensure!(c.source_quota_policy == "exact-original-attempted-split-phase-and-source-margins-v3");
    ensure!(c.new_attempt_formula == "v3-flow-attempts-plus-up-to-21000-frozen-gap-supplement-v7");
    ensure!(
        c.target_split_counts
            == BTreeMap::from([
                ("training".into(), 880000),
                ("validation".into(), 60000),
                ("development".into(), 60000)
            ])
    );
    ensure!(
        c.target_phase_counts
            == BTreeMap::from([
                ("early".into(), 150000),
                ("middle".into(), 500000),
                ("late".into(), 300000),
                ("terminal-adjacent".into(), 50000)
            ])
    );
    ensure!(
        c.label_policy.evaluator == "haitaka-native-handcrafted-search-v1"
            && c.label_policy.node_budget == 10000
            && c.label_policy.deep_reference_node_budget == 250000
    );
    ensure!(
        c.label_policy.maximum_depth == 64
            && c.label_policy.node_counting_version == SEARCH_NODE_COUNTING_VERSION
    );
    ensure!(
        c.label_policy.target_semantics == "bounded-nonmate-root-plus-explicit-mate-class-distance"
            && c.label_policy.score_transform == "identity-with-robust-clip-v1"
            && c.label_policy.loss == "robust-clipped-huber-v1"
    );
    ensure!(
        c.label_policy.qsearch_policy == "default-qsearch-limits-v1"
            && c.label_policy.search_semantics == "r1d1-qualified-root-backed-up-v1"
    );
    ensure!(c.declared_new_architecture == std::env::consts::ARCH);
    let artifacts = [
        &c.original_contract,
        &c.original_failed_gate,
        &c.r4f_contract,
        &c.r4f_analysis,
        &c.r4f_unused_pool,
        &c.r4f_raw_results,
        &c.r4f_gate,
        &c.replacement_r3_contract,
        &c.replacement_r3_raw_results,
        &c.replacement_r3_gate,
        &c.prior_failed_contract,
        &c.prior_failed_attempt_ledger,
        &c.prior_failed_attempt_report,
        &c.prior_failed_v2_contract,
        &c.prior_failed_v2_report,
        &c.prior_failed_v3_contract,
        &c.prior_failed_v3_raw_results,
        &c.prior_failed_v3_report,
        &c.prior_failed_v4_contract,
        &c.prior_failed_v4_gate,
        &c.prior_failed_v4_coverage,
        &c.prior_failed_v4_distributions,
        &c.prior_failed_v5_contract,
        &c.prior_failed_v5_gate,
        &c.prior_failed_v5_coverage,
        &c.prior_failed_v6_contract,
        &c.prior_failed_v6_gate,
        &c.prior_failed_v6_coverage,
        &c.prior_failed_v7_contract,
        &c.prior_failed_v7_gate,
        &c.prior_failed_v7_coverage,
        &c.prior_failed_v7_raw_results,
    ];
    let chain = artifacts
        .iter()
        .map(|a| {
            let p = args.workspace_root.join(&a.path);
            ensure!(sha256_file(&p)? == a.sha256);
            binding(&p, args.workspace_root)
        })
        .collect::<Result<Vec<_>>>()?;
    let r4f: Value = read_json(&args.workspace_root.join(&c.r4f_gate.path))?;
    let r3: Value = read_json(&args.workspace_root.join(&c.replacement_r3_gate.path))?;
    let chain_valid = r4f["closeoutValid"] == true
        && r4f["routeAuthorized"] == true
        && r4f["selectedRoute"] == "r3"
        && r3["passed"] == true
        && r3["r4Authorized"] == true
        && r3["selectedTeacher"] == c.label_policy.evaluator
        && r3["selectedNodeBudget"] == c.label_policy.node_budget;
    ensure!(chain_valid);
    let staging = args.output_dir.with_extension("in-progress");
    ensure!(!staging.exists());
    fs::create_dir_all(staging.join("datasets"))?;
    let ledger_path = staging.join("attempt-ledger.jsonl");
    let mut ledger = BufWriter::new(File::create(&ledger_path)?);
    let mut writers = BTreeMap::new();
    for split in ["training", "validation", "development"] {
        writers.insert(
            split.to_string(),
            (
                BufWriter::new(File::create(staging.join(format!("datasets/{split}.bin")))?),
                BufWriter::new(File::create(
                    staging.join(format!("datasets/{split}.jsonl")),
                )?),
            ),
        );
    }
    let (mut targets, mut completed) = (
        BTreeMap::<String, u64>::new(),
        BTreeMap::<String, u64>::new(),
    );
    let (mut joint_targets, mut joint_completed) = (
        BTreeMap::<String, u64>::new(),
        BTreeMap::<String, u64>::new(),
    );
    let (mut attempted_dist, mut retained_dist) = (BTreeMap::new(), BTreeMap::new());
    let (mut base, mut donor) = (
        vec![0u64; R1_HALFKAV2_BASE_FEATURES],
        vec![0u64; DONOR_ROWS],
    );
    let mut accepted = BTreeMap::<String, u64>::new();
    let mut accepted_families = HashMap::<String, String>::new();
    let mut opening_splits = HashMap::<String, String>::new();
    let mut family_split_collision = false;
    let mut opening_split_collision = false;
    let mut provenance = Vec::new();
    let mut original_hashes = HashSet::with_capacity(1000000);
    for manifest in sorted_manifests(&args.original_dir.join("labels/pilot"))? {
        let mh = sha256_file(&manifest)?;
        let mv: Value = read_json(&manifest)?;
        ensure!(mv["contractSha256"] == c.original_contract.sha256);
        let data = manifest_data(&manifest);
        ensure!(sha256_file(&data)? == mv["labelsSha256"].as_str().context("hash")?);
        provenance.push(binding(&manifest, args.workspace_root)?);
        provenance.push(binding(&data, args.workspace_root)?);
        for line in BufReader::new(File::open(&data)?).lines() {
            let line = line?;
            let l: Label = serde_json::from_str(&line)?;
            let key = cell(&l.position);
            let joint_key = format!("{key}\t{}", l.position.source);
            *targets.entry(key.clone()).or_default() += 1;
            *joint_targets.entry(joint_key.clone()).or_default() += 1;
            observe_distribution(&mut attempted_dist, &l.position);
            original_hashes.insert(l.label_sha256.clone());
            if l.completed {
                *completed.entry(key).or_default() += 1;
                *joint_completed.entry(joint_key).or_default() += 1;
                *accepted.entry(l.position.split.clone()).or_default() += 1;
                observe_distribution(&mut retained_dist, &l.position);
                add_coverage(&l.position, &mut base, &mut donor)?;
                if let Some(previous) = accepted_families.insert(
                    l.position.color_swap_canonical_sha256.clone(),
                    l.position.split.clone(),
                ) && previous != l.position.split
                {
                    family_split_collision = true;
                }
                if let Some(previous) = opening_splits
                    .insert(l.position.opening_group.clone(), l.position.split.clone())
                    && previous != l.position.split
                {
                    opening_split_collision = true;
                }
                let (w, s) = writers.get_mut(&l.position.split).context("split")?;
                write_entry(w, &l)?;
                s.write_all(line.as_bytes())?;
                s.write_all(b"\n")?;
            } else {
                ensure!(l.raw_score.is_none() && l.bounded_score.is_none());
            }
            serde_json::to_writer(
                &mut ledger,
                &LedgerEntry {
                    schema: "haitaka-r4-replacement-attempt-v1",
                    attempt_kind: "imported-r4-v1",
                    accepted: l.completed,
                    tier: "pilot",
                    position_id: &l.position.id,
                    color_swap_canonical_sha256: &l.position.color_swap_canonical_sha256,
                    split: &l.position.split,
                    phase: &l.position.phase,
                    source: &l.position.source,
                    completed: l.completed,
                    node_accounting_exact: l.node_accounting_exact,
                    alpha_beta_nodes: l.alpha_beta_nodes,
                    qsearch_nodes: l.qsearch_nodes,
                    total_nodes: l.total_nodes,
                    raw_score: l.raw_score,
                    bounded_score: l.bounded_score,
                    label_sha256: &l.label_sha256,
                    provenance_manifest_sha256: Some(&mh),
                },
            )?;
            ledger.write_all(b"\n")?;
        }
    }
    ensure!(targets.values().sum::<u64>() == 1000000);
    ensure!(completed.values().sum::<u64>() == 917201);
    let um: Value = read_json(&args.workspace_root.join(&c.r4f_unused_pool.path))?;
    let unused = args
        .workspace_root
        .join(um["data"]["path"].as_str().context("unused")?);
    ensure!(sha256_file(&unused)? == um["data"]["sha256"].as_str().context("hash")?);
    let tuning = [
        args.r4f_dir.join("probe-results.jsonl"),
        args.workspace_root.join(&c.replacement_r3_raw_results.path),
    ]
    .into_iter()
    .flat_map(|p| {
        BufReader::new(File::open(p).unwrap())
            .lines()
            .filter_map(|l| {
                let v: Value = serde_json::from_str(&l.ok()?).ok()?;
                v["position"]["colorSwapCanonicalSha256"]
                    .as_str()
                    .map(str::to_string)
            })
            .collect::<Vec<_>>()
    })
    .collect::<HashSet<_>>();
    let source_targets = attempted_dist["source"].clone();
    let mut source_needed = source_targets
        .iter()
        .map(|(source, target)| {
            (
                source.clone(),
                target - retained_dist["source"].get(source).copied().unwrap_or(0),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut candidates = HashMap::<String, Vec<(u64, u64, Position)>>::new();
    for line in BufReader::new(File::open(unused)?).lines() {
        let p: Position = serde_json::from_str(&line?)?;
        if tuning.contains(&p.color_swap_canonical_sha256) {
            continue;
        }
        let key = cell(&p);
        if targets.get(&key).copied().unwrap_or(0) <= completed.get(&key).copied().unwrap_or(0) {
            continue;
        }
        let (b, d) = active_rows(&p)?;
        let rare = b
            .iter()
            .filter(|&&r| base[r] < c.minimum_feature_updates)
            .count()
            + d.iter()
                .filter(|&&r| donor[r] < c.minimum_feature_updates)
                .count();
        let rank = stable_u64(c.selection_seed, "replacement", &p.id);
        candidates
            .entry(key)
            .or_default()
            .push((rare as u64, rank, p));
    }
    let needed = targets
        .iter()
        .map(|(key, target)| {
            (
                key.clone(),
                target - completed.get(key).copied().unwrap_or(0),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut conservative_capacity = BTreeMap::new();
    for (joint_key, &target) in &joint_targets {
        let done = joint_completed.get(joint_key).copied().unwrap_or(0);
        let (key, source) = joint_key.rsplit_once('\t').context("joint cell")?;
        let candidate_count = candidates
            .get(key)
            .map(|bucket| {
                bucket
                    .iter()
                    .filter(|(_, _, position)| position.source == source)
                    .count()
            })
            .unwrap_or(0);
        let rate = done as f64 / target as f64;
        let capacity =
            (((candidate_count.saturating_sub(4)) as f64 * rate.max(0.5)) / 1.25).floor() as u64;
        conservative_capacity.insert(joint_key.clone(), capacity);
    }
    let planned_acceptance =
        acceptance_plan_from_available(&conservative_capacity, &needed, &source_needed)?;
    let mut selected = Vec::new();
    for (joint_key, &accepted_count) in &planned_acceptance {
        let target = joint_targets[joint_key];
        let done = joint_completed.get(joint_key).copied().unwrap_or(0);
        let rate = done as f64 / target as f64;
        let attempts = ((accepted_count as f64 / rate.max(0.5)) * 1.25).ceil() as usize + 4;
        let (key, source) = joint_key.rsplit_once('\t').context("joint cell")?;
        let bucket = candidates
            .get_mut(key)
            .with_context(|| format!("no candidates {key:?}"))?;
        bucket.sort_by_key(|(rare, rank, position)| {
            (
                position.tactical,
                position.in_check,
                std::cmp::Reverse(*rare),
                *rank,
                position.id.clone(),
            )
        });
        let mut taken = 0usize;
        bucket.retain(|(_, _, position)| {
            if position.source == source && taken < attempts {
                taken += 1;
                selected.push(position.clone());
                false
            } else {
                true
            }
        });
        ensure!(taken == attempts, "insufficient candidates for {joint_key}");
    }
    ensure!(selected.len() <= c.maximum_new_pilot_attempts);
    selected.sort_by_key(|p| {
        (
            stable_u64(c.selection_seed, "execution", &p.id),
            p.id.clone(),
        )
    });
    let search_started = Instant::now();
    let raw_source = args
        .workspace_root
        .join(&c.prior_failed_v3_raw_results.path);
    let mut new = BufReader::new(File::open(&raw_source)?)
        .lines()
        .map(|line| Ok(serde_json::from_str::<Label>(&line?)?))
        .collect::<Result<Vec<_>>>()?;
    ensure!(new.len() == selected.len());
    ensure!(selected.iter().zip(&new).all(|(position, label)| {
        position.id == label.position.id
            && label.label_evaluator == c.label_policy.evaluator
            && label.label_budget == c.label_policy.node_budget
            && label.node_counting_version == c.label_policy.node_counting_version
    }));
    let failed_coverage: Value =
        read_json(&args.workspace_root.join(&c.prior_failed_v6_coverage.path))?;
    let original_coverage: Value = read_json(&args.original_dir.join("feature-row-coverage.json"))?;
    let original_unseen = original_coverage["neverObservedReachableBaseRows"]
        .as_array()
        .context("original unseen rows")?
        .iter()
        .map(|row| row.as_u64().context("base row").map(|row| row as usize))
        .collect::<Result<HashSet<_>>>()?;
    let desired_rows = failed_coverage["neverObservedReachableBaseRows"]
        .as_array()
        .context("failed unseen rows")?
        .iter()
        .map(|row| row.as_u64().context("base row").map(|row| row as usize))
        .collect::<Result<HashSet<_>>>()?
        .difference(&original_unseen)
        .copied()
        .collect::<HashSet<_>>();
    ensure!(!desired_rows.is_empty());
    let mut supplement_candidates = Vec::new();
    for bucket in candidates.values() {
        for (_, rank, position) in bucket {
            let (active_base, _) = active_rows(position)?;
            let gain = active_base
                .iter()
                .filter(|row| desired_rows.contains(row))
                .count();
            if gain > 0 {
                supplement_candidates.push((
                    Reverse(gain),
                    position.tactical,
                    position.in_check,
                    *rank,
                    position.clone(),
                ));
            }
        }
    }
    supplement_candidates.sort_unstable_by_key(|candidate| {
        (
            candidate.0,
            candidate.1,
            candidate.2,
            candidate.3,
            candidate.4.id.clone(),
        )
    });
    supplement_candidates.truncate(c.maximum_coverage_supplement_attempts);
    ensure!(!supplement_candidates.is_empty());
    let supplement_positions = supplement_candidates
        .into_iter()
        .map(|candidate| candidate.4)
        .collect::<Vec<_>>();
    let supplement = BufReader::new(File::open(
        args.workspace_root
            .join(&c.prior_failed_v7_raw_results.path),
    )?)
    .lines()
    .skip(new.len())
    .map(|line| Ok(serde_json::from_str::<Label>(&line?)?))
    .collect::<Result<Vec<_>>>()?;
    ensure!(
        supplement_positions.len() == supplement.len()
            && supplement_positions
                .iter()
                .zip(&supplement)
                .all(|(position, label)| position.id == label.position.id)
    );
    new.extend(supplement);
    ensure!(new.len() <= c.maximum_new_pilot_attempts);
    let raw_new_path = staging.join("new-attempt-results.jsonl");
    let mut raw_new = BufWriter::new(File::create(&raw_new_path)?);
    for label in &new {
        serde_json::to_writer(&mut raw_new, label)?;
        raw_new.write_all(b"\n")?;
    }
    raw_new.flush()?;
    let mut needed = needed;
    let mut actual_by_joint = BTreeMap::<String, u64>::new();
    for label in new.iter().filter(|label| label.completed) {
        *actual_by_joint
            .entry(format!(
                "{}\t{}",
                cell(&label.position),
                label.position.source
            ))
            .or_default() += 1;
    }
    let mut joint_needed =
        acceptance_plan_from_available(&actual_by_joint, &needed, &source_needed)?;
    let accepted_indices = dynamic_coverage_acceptance(
        &new,
        &joint_needed,
        &base,
        &donor,
        c.minimum_feature_updates,
        c.selection_seed,
    )?;
    let (mut new_incomplete, mut total_nodes) = (0u64, 0u64);
    let mut accepted_new = Vec::new();
    for (index, l) in new.iter().enumerate() {
        total_nodes += l.total_nodes;
        if !l.completed {
            new_incomplete += 1;
            ensure!(l.raw_score.is_none() && l.bounded_score.is_none())
        }
        let key = cell(&l.position);
        let joint_key = format!("{key}\t{}", l.position.source);
        let take = accepted_indices.contains(&index);
        if take {
            *needed.get_mut(&key).unwrap() -= 1;
            *joint_needed.get_mut(&joint_key).unwrap() -= 1;
            *source_needed.get_mut(&l.position.source).unwrap() -= 1;
            *accepted.entry(l.position.split.clone()).or_default() += 1;
            observe_distribution(&mut retained_dist, &l.position);
            add_coverage(&l.position, &mut base, &mut donor)?;
            if let Some(previous) = accepted_families.insert(
                l.position.color_swap_canonical_sha256.clone(),
                l.position.split.clone(),
            ) && previous != l.position.split
            {
                family_split_collision = true;
            }
            if let Some(previous) =
                opening_splits.insert(l.position.opening_group.clone(), l.position.split.clone())
                && previous != l.position.split
            {
                opening_split_collision = true;
            }
            let (w, s) = writers.get_mut(&l.position.split).unwrap();
            write_entry(w, l)?;
            serde_json::to_writer(&mut *s, l)?;
            s.write_all(b"\n")?;
            accepted_new.push(l.position.clone());
        }
        observe_distribution(&mut attempted_dist, &l.position);
        serde_json::to_writer(
            &mut ledger,
            &LedgerEntry {
                schema: "haitaka-r4-replacement-attempt-v1",
                attempt_kind: "replacement-v3-reused",
                accepted: take,
                tier: "pilot",
                position_id: &l.position.id,
                color_swap_canonical_sha256: &l.position.color_swap_canonical_sha256,
                split: &l.position.split,
                phase: &l.position.phase,
                source: &l.position.source,
                completed: l.completed,
                node_accounting_exact: l.node_accounting_exact,
                alpha_beta_nodes: l.alpha_beta_nodes,
                qsearch_nodes: l.qsearch_nodes,
                total_nodes: l.total_nodes,
                raw_score: l.raw_score,
                bounded_score: l.bounded_score,
                label_sha256: &l.label_sha256,
                provenance_manifest_sha256: None,
            },
        )?;
        ledger.write_all(b"\n")?;
    }
    ensure!(needed.values().all(|&n| n == 0));
    ensure!(joint_needed.values().all(|&n| n == 0));
    ensure!(source_needed.values().all(|&n| n == 0));
    ensure!(accepted == c.target_split_counts);
    for v in writers.values_mut() {
        v.0.flush()?;
        v.1.flush()?;
    }
    let deep_bin = staging.join("datasets/deep-core.bin");
    let deep_side = staging.join("datasets/deep-core.jsonl");
    let (mut db, mut ds) = (
        BufWriter::new(File::create(&deep_bin)?),
        BufWriter::new(File::create(&deep_side)?),
    );
    let mut deep_count = 0u64;
    let mut deep_import_attempts = 0u64;
    for manifest in sorted_manifests(&args.original_dir.join("labels/deep-core"))? {
        let manifest_hash = sha256_file(&manifest)?;
        let data = manifest_data(&manifest);
        provenance.push(binding(&manifest, args.workspace_root)?);
        provenance.push(binding(&data, args.workspace_root)?);
        for line in BufReader::new(File::open(data)?).lines() {
            let line = line?;
            let l: Label = serde_json::from_str(&line)?;
            deep_import_attempts += 1;
            if l.completed {
                write_entry(&mut db, &l)?;
                ds.write_all(line.as_bytes())?;
                ds.write_all(b"\n")?;
                deep_count += 1
            } else {
                ensure!(l.raw_score.is_none() && l.bounded_score.is_none());
            }
            serde_json::to_writer(
                &mut ledger,
                &LedgerEntry {
                    schema: "haitaka-r4-replacement-attempt-v1",
                    attempt_kind: "imported-r4-v1",
                    accepted: l.completed,
                    tier: "deep-core",
                    position_id: &l.position.id,
                    color_swap_canonical_sha256: &l.position.color_swap_canonical_sha256,
                    split: &l.position.split,
                    phase: &l.position.phase,
                    source: &l.position.source,
                    completed: l.completed,
                    node_accounting_exact: l.node_accounting_exact,
                    alpha_beta_nodes: l.alpha_beta_nodes,
                    qsearch_nodes: l.qsearch_nodes,
                    total_nodes: l.total_nodes,
                    raw_score: l.raw_score,
                    bounded_score: l.bounded_score,
                    label_sha256: &l.label_sha256,
                    provenance_manifest_sha256: Some(&manifest_hash),
                },
            )?;
            ledger.write_all(b"\n")?;
        }
    }
    ensure!(deep_import_attempts == 4096 && deep_count == 4086);
    let dc = accepted_new
        .into_iter()
        .filter(|p| p.split == "development")
        .take(c.maximum_new_deep_attempts)
        .collect::<Vec<_>>();
    ensure!(dc.len() == c.maximum_new_deep_attempts);
    let deep = label_parallel(
        dc,
        "deep-core",
        c.label_policy.deep_reference_node_budget,
        &c.label_policy,
        args.jobs,
    )?;
    for l in &deep {
        total_nodes += l.total_nodes;
        let take = l.completed && deep_count < 4096;
        if take {
            write_entry(&mut db, l)?;
            serde_json::to_writer(&mut ds, l)?;
            ds.write_all(b"\n")?;
            deep_count += 1
        }
        serde_json::to_writer(
            &mut ledger,
            &LedgerEntry {
                schema: "haitaka-r4-replacement-attempt-v1",
                attempt_kind: "replacement-new",
                accepted: take,
                tier: "deep-core",
                position_id: &l.position.id,
                color_swap_canonical_sha256: &l.position.color_swap_canonical_sha256,
                split: &l.position.split,
                phase: &l.position.phase,
                source: &l.position.source,
                completed: l.completed,
                node_accounting_exact: l.node_accounting_exact,
                alpha_beta_nodes: l.alpha_beta_nodes,
                qsearch_nodes: l.qsearch_nodes,
                total_nodes: l.total_nodes,
                raw_score: l.raw_score,
                bounded_score: l.bounded_score,
                label_sha256: &l.label_sha256,
                provenance_manifest_sha256: None,
            },
        )?;
        ledger.write_all(b"\n")?;
    }
    db.flush()?;
    ds.flush()?;
    ledger.flush()?;
    ensure!(deep_count == 4096);
    let pilot_replay_source = new.iter().take(64).cloned().collect::<Vec<_>>();
    let pilot_replay = label_parallel(
        pilot_replay_source
            .iter()
            .map(|label| label.position.clone())
            .collect(),
        "pilot",
        c.label_policy.node_budget,
        &c.label_policy,
        args.jobs,
    )?;
    let deep_replay_source = deep.iter().take(4).cloned().collect::<Vec<_>>();
    let deep_replay = label_parallel(
        deep_replay_source
            .iter()
            .map(|label| label.position.clone())
            .collect(),
        "deep-core",
        c.label_policy.deep_reference_node_budget,
        &c.label_policy,
        args.jobs,
    )?;
    total_nodes += pilot_replay
        .iter()
        .map(|label| label.total_nodes)
        .sum::<u64>();
    total_nodes += deep_replay
        .iter()
        .map(|label| label.total_nodes)
        .sum::<u64>();
    let new_replay_exact = pilot_replay_source
        .iter()
        .zip(&pilot_replay)
        .all(|(expected, actual)| exact_search_replay(expected, actual))
        && deep_replay_source
            .iter()
            .zip(&deep_replay)
            .all(|(expected, actual)| exact_search_replay(expected, actual));
    let search_seconds = search_started.elapsed().as_secs_f64();
    let ledger_entries = 1_000_000 + new.len() as u64 + deep_import_attempts + deep.len() as u64;
    let ledger_manifest_path = staging.join("attempt-ledger.json");
    fs::write(
        &ledger_manifest_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "haitaka-r4-replacement-attempt-ledger-manifest-v1",
            "contractSha256": contract_hash,
            "entries": ledger_entries,
            "importedPilotAttempts": 1_000_000,
            "newPilotAttempts": new.len(),
            "importedDeepAttempts": deep_import_attempts,
            "newDeepAttempts": deep.len(),
            "dataSha256": sha256_file(&ledger_path)?,
            "dataBytes": fs::metadata(&ledger_path)?.len()
        }))?,
    )?;
    let mut datasets = BTreeMap::<String, DatasetManifest>::new();
    for (name, records) in c
        .target_split_counts
        .iter()
        .map(|(n, c)| (n.as_str(), *c))
        .chain(std::iter::once(("deep-core", 4096)))
    {
        let bin = staging.join(format!("datasets/{name}.bin"));
        let side = staging.join(format!("datasets/{name}.jsonl"));
        ensure!(fs::metadata(&bin)?.len() == records * ENTRY_BYTES as u64);
        let m = DatasetManifest {
            schema: "haitaka-r4-replacement-dataset-manifest-v1".into(),
            dataset: name.into(),
            records,
            entry_bytes: ENTRY_BYTES,
            binary: output_binding(&bin, &staging, args.output_dir, args.workspace_root)?,
            sidecar: output_binding(&side, &staging, args.output_dir, args.workspace_root)?,
            contract_sha256: contract_hash.clone(),
            label_evaluator: c.label_policy.evaluator.clone(),
            label_budget: if name == "deep-core" { 250000 } else { 10000 },
            target_semantics: c.label_policy.target_semantics.clone(),
            score_transform: c.label_policy.score_transform.clone(),
            mate_policy: "explicit-class-win-loss-none-plus-distance-from-30000-v1".into(),
            result_loss_enabled: false,
            unknown_results_ignored: true,
        };
        fs::write(
            staging.join(format!("datasets/{name}.json")),
            serde_json::to_vec_pretty(&m)?,
        )?;
        datasets.insert(name.into(), m);
    }
    let aggregate = staging.join("datasets/pilot-1m.json");
    let split_links=datasets.iter().filter(|(n,_)|n.as_str()!="deep-core").map(|(n,m)|(n,serde_json::json!({"records":m.records,"manifestSha256":sha256_file(&staging.join(format!("datasets/{n}.json"))).unwrap()}))).collect::<BTreeMap<_,_>>();
    fs::write(
        &aggregate,
        serde_json::to_vec_pretty(
            &serde_json::json!({"schema":"haitaka-r4-replacement-eligible-pilot-v1","contractSha256":contract_hash,"records":1000000,"splits":split_links,"eligibleForR5":true}),
        )?,
    )?;
    let coverage = coverage_report(&base, &donor, 1000000, c.minimum_feature_updates);
    let coverage_path = staging.join("feature-row-coverage.json");
    fs::write(&coverage_path, serde_json::to_vec_pretty(&coverage)?)?;
    let (max_tv, tv) = maximum_tv(&attempted_dist, &retained_dist, &c.distribution_dimensions);
    let dist_path = staging.join("distributions.json");
    fs::write(
        &dist_path,
        serde_json::to_vec_pretty(
            &serde_json::json!({"schema":"haitaka-r4-replacement-distributions-v1","attempted":attempted_dist,"retained":retained_dist,"totalVariation":tv,"maximumTotalVariation":max_tv}),
        )?,
    )?;
    let prov_path = staging.join("imported-label-provenance.json");
    fs::write(
        &prov_path,
        serde_json::to_vec_pretty(
            &serde_json::json!({"schema":"haitaka-r4-replacement-imported-provenance-v1","originalContractSha256":c.original_contract.sha256,"labelManifestsAndData":provenance,"uniqueLabelHashes":original_hashes.len()}),
        )?,
    )?;
    let regen_path = staging.join("regeneration-proof.json");
    fs::write(
        &regen_path,
        serde_json::to_vec_pretty(
            &serde_json::json!({"schema":"haitaka-r4-replacement-position-label-regeneration-v1","importedPositionProofs":[{"path":"out/anhoku-reboot-r4/regeneration-proof-local-00008.json","sha256":sha256_file(&args.original_dir.join("regeneration-proof-local-00008.json"))?},{"path":"out/anhoku-reboot-r4/regeneration-proof-mbp-00080.json","sha256":sha256_file(&args.original_dir.join("regeneration-proof-mbp-00080.json"))?}],"labelReplayProofs":{"r4fExact":r4f["metrics"]["exactReplays"],"r4fAttempted":r4f["metrics"]["replayPositions"],"r3Exact":r3["metrics"]["exactReplays"],"r3Attempted":r3["metrics"]["replayPositions"],"newPilotAttempted":pilot_replay.len(),"newDeepAttempted":deep_replay.len(),"newExact":new_replay_exact},"newArchitecture":std::env::consts::ARCH,"newExecutableSha256":sha256_file(args.executable_path)?,"positionAndLabelHashesReproduced":new_replay_exact}),
        )?,
    )?;
    let core_hours = c.reused_search_core_hours + search_seconds * args.jobs as f64 / 3600.0;
    let conditional_incomplete_rate = new_incomplete as f64 / new.len() as f64;
    let all_pilot_attempts = 1_000_000 + new.len() as u64;
    let all_pilot_incomplete = 82_799 + new_incomplete;
    let incomplete_rate = all_pilot_incomplete as f64 / all_pilot_attempts as f64;
    let analysis: Value = read_json(&args.workspace_root.join(&c.r4f_analysis.path))?;
    let old_cov = &analysis["finiteFeatureCoverage"];
    let coverage_good = coverage.never_observed_reachable_base_rows.len()
        <= old_cov["neverObservedReachableBaseRows"]
            .as_array()
            .context("coverage")?
            .len()
        && coverage.never_observed_donor_rows.len()
            <= old_cov["neverObservedDonorRows"]
                .as_array()
                .context("coverage")?
                .len()
        && coverage.reachable_base_rows_below_minimum.len()
            <= old_cov["reachableBaseRowsBelowMinimum"]
                .as_array()
                .context("coverage")?
                .len()
        && coverage.donor_rows_below_minimum.len()
            <= old_cov["donorRowsBelowMinimum"]
                .as_array()
                .context("coverage")?
                .len();
    let new_selection_unique = new
        .iter()
        .map(|label| label.position.color_swap_canonical_sha256.as_str())
        .collect::<HashSet<_>>()
        .len()
        == new.len()
        && new.iter().all(|label| {
            !tuning.contains(&label.position.color_swap_canonical_sha256)
                && !original_hashes.contains(&label.label_sha256)
        });
    let phase_counts_exact = c
        .target_phase_counts
        .iter()
        .all(|(phase, count)| retained_dist["phase"].get(phase).copied().unwrap_or(0) == *count);
    let ledger_complete = ledger_entries
        == 1_000_000 + new.len() as u64 + 4_096 + deep.len() as u64
        && read_json::<Value>(&ledger_manifest_path)?["entries"] == ledger_entries;
    let no_downstream = !args.workspace_root.join("out/anhoku-reboot-r5").exists();
    let mut gates = BTreeMap::from([
        ("authorization-chain-valid".into(), chain_valid),
        ("replacement-contract-frozen-before-work".into(), frozen),
        ("source-pool-bound".into(), new_selection_unique),
        ("label-policy-identity-valid".into(), true),
        (
            "label-executable-equivalence".into(),
            r4f["gates"]["replay-deterministic"] == true
                && r3["gates"]["replay-equivalent"] == true,
        ),
        (
            "deterministic-selection-and-replenishment".into(),
            new_selection_unique && new.len() <= c.maximum_new_pilot_attempts,
        ),
        ("attempt-ledger-complete".into(), ledger_complete),
        ("incomplete-targets-null".into(), true),
        (
            "node-accounting-exact".into(),
            new.iter().chain(&deep).all(|l| l.node_accounting_exact),
        ),
        (
            "maximum-incomplete-attempt-rate".into(),
            incomplete_rate <= c.maximum_incomplete_attempt_rate,
        ),
        (
            "accepted-finite-cohort-complete".into(),
            accepted.values().sum::<u64>() == 1000000,
        ),
        ("finite-deep-core-complete".into(), deep_count == 4096),
        (
            "split-counts-exact".into(),
            accepted == c.target_split_counts,
        ),
        (
            "declared-mixtures-preserved".into(),
            needed.values().all(|&n| n == 0)
                && source_needed.values().all(|&n| n == 0)
                && phase_counts_exact,
        ),
        ("grouped-split-isolation".into(), !opening_split_collision),
        (
            "color-swap-split-isolation".into(),
            !family_split_collision && accepted_families.len() == 1_000_000,
        ),
        (
            "retained-attempted-bias-bounded".into(),
            max_tv <= c.maximum_distribution_total_variation,
        ),
        ("retained-feature-coverage".into(), coverage_good),
        (
            "imported-label-provenance-valid".into(),
            original_hashes.len() == 1000000,
        ),
        ("position-and-label-regeneration".into(), new_replay_exact),
        (
            "cost-forecast-and-ceiling".into(),
            total_nodes <= c.maximum_total_nodes
                && core_hours <= c.maximum_core_hours
                && core_hours * c.cpu_core_hour_usd <= c.maximum_dollars,
        ),
        ("sole-r5-dataset-identified".into(), true),
        ("no-downstream-work".into(), no_downstream),
    ]);
    ensure!(gates.len() == 23);
    let passed = gates.values().all(|v| *v);
    let yield_rate = 1.0 - conditional_incomplete_rate;
    let per_million = core_hours * c.cpu_core_hour_usd / new.len() as f64 * 1000000.0 / yield_rate;
    let report = GateReport {
        schema: "haitaka-anhoku-r4-replacement-gate-v8".into(),
        phase: "R4-replacement".into(),
        passed,
        r5_authorized: passed && no_downstream,
        contract_sha256: contract_hash,
        authorization_chain: chain,
        sole_eligible_1m_dataset: output_binding(
            &aggregate,
            &staging,
            args.output_dir,
            args.workspace_root,
        )?,
        datasets,
        attempt_ledger: output_binding(
            &ledger_manifest_path,
            &staging,
            args.output_dir,
            args.workspace_root,
        )?,
        raw_new_attempts: output_binding(
            &raw_new_path,
            &staging,
            args.output_dir,
            args.workspace_root,
        )?,
        imported_provenance: output_binding(
            &prov_path,
            &staging,
            args.output_dir,
            args.workspace_root,
        )?,
        coverage: output_binding(
            &coverage_path,
            &staging,
            args.output_dir,
            args.workspace_root,
        )?,
        distributions: output_binding(&dist_path, &staging, args.output_dir, args.workspace_root)?,
        regeneration: output_binding(&regen_path, &staging, args.output_dir, args.workspace_root)?,
        new_pilot_attempts: new.len() as u64,
        new_pilot_incomplete: new_incomplete,
        conditional_replenishment_incomplete_rate: conditional_incomplete_rate,
        all_pilot_attempts,
        all_pilot_incomplete_attempts: all_pilot_incomplete,
        all_pilot_incomplete_attempt_rate: incomplete_rate,
        new_deep_attempts: deep.len() as u64,
        total_search_nodes: total_nodes,
        search_core_hours: core_hours,
        search_cost_usd: core_hours * c.cpu_core_hour_usd,
        cost_forecast_usd_per_finite_records: BTreeMap::from([
            ("10000000".into(), per_million * 10.0),
            ("50000000".into(), per_million * 50.0),
            ("100000000".into(), per_million * 100.0),
        ]),
        gates: std::mem::take(&mut gates),
        training_started: false,
        strength_games_played: false,
        broad_generation_started: false,
    };
    let report_path = staging.join("r4-gate-report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    ensure!(
        passed,
        "replacement R4 gate failed at {}",
        report_path.display()
    );
    fs::rename(&staging, args.output_dir)?;
    Ok(args.output_dir.join("r4-gate-report.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_target_null() {
        let p = LabelPolicy {
            evaluator: String::new(),
            node_budget: 10000,
            deep_reference_node_budget: 250000,
            maximum_depth: 64,
            node_counting_version: String::new(),
            target_semantics: String::new(),
            score_transform: String::new(),
            loss: String::new(),
            nonmate_clip: 12000,
            mate_threshold: 29000,
            mate_base: 30000,
            qsearch_policy: String::new(),
            search_semantics: String::new(),
        };
        assert_eq!(
            bounded_target(None, &p),
            ("unknown/incomplete".into(), None, None)
        );
    }

    #[test]
    fn acceptance_flow_satisfies_both_margins_without_greedy_ordering() {
        let available = BTreeMap::from([
            ("cell-a\tsource-a".into(), 5),
            ("cell-a\tsource-b".into(), 5),
            ("cell-b\tsource-a".into(), 5),
        ]);
        let cells = BTreeMap::from([("cell-a".into(), 5), ("cell-b".into(), 5)]);
        let sources = BTreeMap::from([("source-a".into(), 6), ("source-b".into(), 4)]);
        let plan = acceptance_plan_from_available(&available, &cells, &sources).unwrap();
        assert_eq!(plan["cell-b\tsource-a"], 5);
        assert_eq!(plan["cell-a\tsource-a"], 1);
        assert_eq!(plan["cell-a\tsource-b"], 4);
    }

    #[test]
    fn acceptance_flow_fails_closed_when_margins_are_infeasible() {
        let available = BTreeMap::from([("cell-a\tsource-a".into(), 9)]);
        let cells = BTreeMap::from([("cell-a".into(), 10)]);
        let sources = BTreeMap::from([("source-a".into(), 10)]);
        assert!(acceptance_plan_from_available(&available, &cells, &sources).is_err());
    }
}

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, IsTerminal, Read, Write, stderr, stdin};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use haitaka::{Board, Color, Move, Piece, Square};
use haitaka_wasm::{
    NnueModel, NodeBudgetSearchSummary, SEARCH_MATE_SCORE_THRESHOLD, SEARCH_NODE_COUNTING_VERSION,
    SEARCH_TRAINING_TRACE_VERSION, SearchEvalMode, SearchSummary, SearchTrainingTrace,
    SearchWorkspace, search_board_impl_handcrafted_in_workspace,
    search_board_impl_handcrafted_with_node_budget_and_training_trace_in_workspace,
    search_board_impl_handcrafted_with_node_budget_in_workspace,
    search_board_impl_handcrafted_with_training_trace_in_workspace,
    search_board_impl_with_eval_mode_and_node_budget_and_training_trace_in_workspace,
    search_board_impl_with_eval_mode_and_node_budget_in_workspace,
    search_board_impl_with_eval_mode_and_training_trace_in_workspace,
    search_board_impl_with_eval_mode_in_workspace,
};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{
    ArtifactPaths, IncompleteLabelPolicy, LabelSearchBudget, LoadedConfig, PositionPolicy, Ruleset,
    SamplingPolicy, SelfPlayMovePolicy, ShufflePolicy, TEACHER_MOVE_ENCODING,
};
use crate::openings::{GameOpeningMetadata, OpeningSource, OpeningSplit};

const PACKED_SFEN_BYTES: usize = 64;
pub(crate) const ENTRY_BYTES: usize = PACKED_SFEN_BYTES + 8;
const SHUFFLE_IO_BUFFER_BYTES: usize = 64 * 1024;
const POSITION_SELECTION_AUDIT_VERSION: &str = "side-parity-opening-result-v1";
#[cfg(all(unix, not(test)))]
const GRACEFUL_STOP_MESSAGE: &[u8] =
    "graceful stop中です。もう一度ctrl-cすることで即座に終了できます\n".as_bytes();

static GRACEFUL_STOP_STATE: AtomicU8 = AtomicU8::new(0);

#[derive(Debug, Clone)]
pub struct DatasetOutput {
    pub output_dir: PathBuf,
    pub train_positions: u64,
    pub validation_positions: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerateOptions {
    pub jobs: Option<u32>,
    pub resume: Option<bool>,
    pub shard_index: Option<u32>,
    pub shard_index_end: Option<u32>,
    pub shard_count: Option<u32>,
    pub ignore_identity_mismatch: bool,
}

impl GenerateOptions {
    pub fn from_config(loaded: &LoadedConfig) -> Self {
        Self {
            jobs: Some(loaded.config.data.jobs),
            resume: Some(loaded.config.data.resume),
            shard_index: None,
            shard_index_end: None,
            shard_count: None,
            ignore_identity_mismatch: false,
        }
    }
}

#[derive(Debug, Serialize)]
struct DatasetManifest {
    dataset: String,
    ruleset: Ruleset,
    rule_id: u16,
    opening_sfen: String,
    opening_policy: String,
    opening_suite_id: Option<String>,
    opening_suite_sha256: Option<String>,
    opening_transformation: String,
    opening_ids: Vec<String>,
    games: Vec<GameOpeningMetadata>,
    split_policy: String,
    split_seed: u64,
    train_opening_ids: Vec<String>,
    validation_opening_ids: Vec<String>,
    opening_group_count: usize,
    opening_group_overlap: Vec<String>,
    shuffle_policy: String,
    shuffle_seed: u64,
    shuffle_chunk_records: usize,
    shuffle_memory_bound_bytes: usize,
    game_count: u32,
    completed_games: u32,
    sampled_positions: u64,
    search_depth: u8,
    label_search_depth: u8,
    label_search_budget: String,
    label_search_nodes: Option<u64>,
    label_search_max_depth: u8,
    node_counting_version: String,
    position_policy: String,
    training_trace_version: String,
    incomplete_label_policy: String,
    position_selection_audit_version: String,
    candidate_positions: u64,
    rejected_incomplete_label_positions: u64,
    rejected_terminal_positions: u64,
    rejected_mate_score_positions: u64,
    position_selection: PositionSelectionStats,
    opening_position_selection: BTreeMap<String, PositionSelectionStats>,
    root_ply_min: Option<u16>,
    root_ply_max: Option<u16>,
    leaf_distance_min: Option<u16>,
    leaf_distance_max: Option<u16>,
    leaf_distance_mean: f64,
    rollout_search_depth: u8,
    self_play_move_policy: String,
    label_searches: u64,
    rollout_searches: u64,
    label_search_states: u64,
    label_search_qnodes: u64,
    label_search_total_nodes: u64,
    label_nodes_per_search: f64,
    rollout_search_states: u64,
    rollout_search_qnodes: u64,
    label_search_cpu_seconds: f64,
    rollout_search_cpu_seconds: f64,
    generation_cpu_seconds: f64,
    bootstrap_nnue: Option<String>,
    bootstrap_nnue_sha256: Option<String>,
    engine_revision: Option<String>,
    config_hash: String,
    seed: u64,
    feature_family: String,
    sampling_phase: String,
    sample_after_opening: bool,
    teacher_move_encoding: String,
    opening_random_plies: u16,
    generated_at_unix_ms: u128,
    build_mode: String,
    entry_bytes: usize,
    shard_count: usize,
    jobs: usize,
    resumed_shards: usize,
    generated_shards: usize,
    elapsed_seconds: f64,
    positions_per_second: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ShardManifest {
    dataset: String,
    ruleset: Ruleset,
    rule_id: u16,
    opening_sfen: String,
    #[serde(default = "legacy_opening_policy")]
    opening_policy: String,
    #[serde(default)]
    opening_suite_id: Option<String>,
    #[serde(default)]
    opening_suite_sha256: Option<String>,
    #[serde(default = "legacy_opening_transformation")]
    opening_transformation: String,
    #[serde(default)]
    opening_ids: Vec<String>,
    #[serde(default)]
    games: Vec<GameOpeningMetadata>,
    #[serde(default = "legacy_split_policy")]
    split_policy: String,
    #[serde(default = "legacy_split_seed")]
    split_seed: u64,
    #[serde(default)]
    train_opening_ids: Vec<String>,
    #[serde(default)]
    validation_opening_ids: Vec<String>,
    #[serde(default = "legacy_shuffle_policy")]
    shuffle_policy: String,
    #[serde(default = "legacy_shuffle_seed")]
    shuffle_seed: u64,
    #[serde(default = "legacy_shuffle_chunk_records")]
    shuffle_chunk_records: usize,
    game_start: u32,
    game_count: u32,
    sampled_positions: u64,
    search_depth: u8,
    #[serde(default)]
    label_search_depth: u8,
    #[serde(default)]
    label_search_budget: String,
    #[serde(default)]
    label_search_nodes: Option<u64>,
    #[serde(default)]
    label_search_max_depth: u8,
    #[serde(default)]
    node_counting_version: String,
    #[serde(default)]
    position_policy: String,
    #[serde(default)]
    training_trace_version: String,
    #[serde(default = "legacy_incomplete_label_policy")]
    incomplete_label_policy: String,
    #[serde(default)]
    position_selection_audit_version: String,
    #[serde(default)]
    candidate_positions: u64,
    #[serde(default)]
    rejected_incomplete_label_positions: u64,
    #[serde(default)]
    rejected_terminal_positions: u64,
    #[serde(default)]
    rejected_mate_score_positions: u64,
    #[serde(default)]
    position_selection: PositionSelectionStats,
    #[serde(default)]
    opening_position_selection: BTreeMap<String, PositionSelectionStats>,
    #[serde(default)]
    root_ply_min: Option<u16>,
    #[serde(default)]
    root_ply_max: Option<u16>,
    #[serde(default)]
    leaf_distance_min: Option<u16>,
    #[serde(default)]
    leaf_distance_max: Option<u16>,
    #[serde(default)]
    leaf_distance_total: u64,
    #[serde(default)]
    rollout_search_depth: u8,
    #[serde(default = "legacy_self_play_move_policy")]
    self_play_move_policy: String,
    #[serde(default)]
    label_searches: u64,
    #[serde(default)]
    rollout_searches: u64,
    #[serde(default)]
    label_search_states: u64,
    #[serde(default)]
    label_search_qnodes: u64,
    #[serde(default)]
    rollout_search_states: u64,
    #[serde(default)]
    rollout_search_qnodes: u64,
    #[serde(default)]
    label_search_cpu_seconds: f64,
    #[serde(default)]
    rollout_search_cpu_seconds: f64,
    bootstrap_nnue: Option<String>,
    #[serde(default)]
    bootstrap_nnue_sha256: Option<String>,
    engine_revision: Option<String>,
    config_hash: String,
    #[serde(default = "legacy_sampling_phase")]
    sampling_phase: String,
    #[serde(default)]
    sample_after_opening: bool,
    #[serde(default = "legacy_teacher_move_encoding")]
    teacher_move_encoding: String,
    generated_at_unix_ms: u128,
    build_mode: String,
    entry_bytes: usize,
    shard_index: u32,
}

fn legacy_sampling_phase() -> String {
    "fixed-phase-legacy".to_string()
}

fn legacy_self_play_move_policy() -> String {
    "label-on-sample-legacy".to_string()
}

fn legacy_incomplete_label_policy() -> String {
    "error".to_string()
}

fn legacy_opening_policy() -> String {
    "uniform-random".to_string()
}

fn legacy_opening_transformation() -> String {
    "none".to_string()
}

fn legacy_split_policy() -> String {
    "independent-legacy".to_string()
}

fn legacy_split_seed() -> u64 {
    0x7370_6c69_742d_7631
}

fn legacy_shuffle_policy() -> String {
    "game-order-legacy".to_string()
}

fn legacy_shuffle_seed() -> u64 {
    0x7368_7566_666c_6531
}

fn legacy_shuffle_chunk_records() -> usize {
    65_536
}

fn legacy_teacher_move_encoding() -> String {
    "legacy-ambiguous-u16".to_string()
}

impl ShardManifest {
    fn label_search_depth(&self) -> u8 {
        if self.label_search_depth == 0 {
            self.search_depth
        } else {
            self.label_search_depth
        }
    }

    fn rollout_search_depth(&self) -> u8 {
        if self.rollout_search_depth == 0 {
            self.search_depth
        } else {
            self.rollout_search_depth
        }
    }

    fn label_search_budget(&self) -> &str {
        if self.label_search_budget.is_empty() {
            "depth"
        } else {
            &self.label_search_budget
        }
    }

    fn label_search_max_depth(&self) -> u8 {
        if self.label_search_max_depth == 0 {
            self.label_search_depth()
        } else {
            self.label_search_max_depth
        }
    }

    fn node_counting_version_matches(&self, budget: LabelSearchBudget) -> bool {
        self.node_counting_version == SEARCH_NODE_COUNTING_VERSION
            || (self.node_counting_version.is_empty()
                && matches!(budget, LabelSearchBudget::Depth { .. }))
    }

    fn position_policy(&self) -> &str {
        if self.position_policy.is_empty() {
            PositionPolicy::RootPosition.manifest_name()
        } else {
            &self.position_policy
        }
    }

    fn training_trace_version_matches(&self, policy: PositionPolicy) -> bool {
        self.training_trace_version == SEARCH_TRAINING_TRACE_VERSION
            || (self.training_trace_version.is_empty() && !policy.uses_training_trace())
    }

    fn incomplete_label_policy(&self) -> &str {
        if self.incomplete_label_policy.is_empty() {
            "error"
        } else {
            &self.incomplete_label_policy
        }
    }
}

#[derive(Debug, Clone)]
struct PendingSample {
    board: Board,
    score: i16,
    game_ply: u16,
    side_to_move: Color,
}

#[derive(Debug, Clone)]
struct GameEntries {
    entries: Vec<u8>,
    stats: SearchUseStats,
    opening: GameOpeningMetadata,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
struct PositionSelectionStats {
    candidate_root_black: u64,
    candidate_root_white: u64,
    stored_root_black_leaf_black: u64,
    stored_root_black_leaf_white: u64,
    stored_root_white_leaf_black: u64,
    stored_root_white_leaf_white: u64,
    stored_leaf_distance_even: u64,
    stored_leaf_distance_odd: u64,
    rejected_incomplete_root_black: u64,
    rejected_incomplete_root_white: u64,
    rejected_terminal_root_black: u64,
    rejected_terminal_root_white: u64,
    rejected_terminal_leaf_black: u64,
    rejected_terminal_leaf_white: u64,
    rejected_mate_root_black: u64,
    rejected_mate_root_white: u64,
    rejected_mate_leaf_black: u64,
    rejected_mate_leaf_white: u64,
    rejected_incomplete_game_win: u64,
    rejected_incomplete_game_loss: u64,
    rejected_incomplete_game_draw: u64,
    rejected_terminal_game_win: u64,
    rejected_terminal_game_loss: u64,
    rejected_terminal_game_draw: u64,
    rejected_mate_game_win: u64,
    rejected_mate_game_loss: u64,
    rejected_mate_game_draw: u64,
}

impl PositionSelectionStats {
    fn add(&mut self, other: Self) {
        macro_rules! add_fields {
            ($($field:ident),+ $(,)?) => {
                $(self.$field += other.$field;)+
            };
        }
        add_fields!(
            candidate_root_black,
            candidate_root_white,
            stored_root_black_leaf_black,
            stored_root_black_leaf_white,
            stored_root_white_leaf_black,
            stored_root_white_leaf_white,
            stored_leaf_distance_even,
            stored_leaf_distance_odd,
            rejected_incomplete_root_black,
            rejected_incomplete_root_white,
            rejected_terminal_root_black,
            rejected_terminal_root_white,
            rejected_terminal_leaf_black,
            rejected_terminal_leaf_white,
            rejected_mate_root_black,
            rejected_mate_root_white,
            rejected_mate_leaf_black,
            rejected_mate_leaf_white,
            rejected_incomplete_game_win,
            rejected_incomplete_game_loss,
            rejected_incomplete_game_draw,
            rejected_terminal_game_win,
            rejected_terminal_game_loss,
            rejected_terminal_game_draw,
            rejected_mate_game_win,
            rejected_mate_game_loss,
            rejected_mate_game_draw,
        );
    }

    fn record_candidate(&mut self, root_side: Color) {
        match root_side {
            Color::Black => self.candidate_root_black += 1,
            Color::White => self.candidate_root_white += 1,
        }
    }

    fn record_stored(&mut self, root_side: Color, leaf_side: Color, leaf_distance: u16) {
        match (root_side, leaf_side) {
            (Color::Black, Color::Black) => self.stored_root_black_leaf_black += 1,
            (Color::Black, Color::White) => self.stored_root_black_leaf_white += 1,
            (Color::White, Color::Black) => self.stored_root_white_leaf_black += 1,
            (Color::White, Color::White) => self.stored_root_white_leaf_white += 1,
        }
        if leaf_distance % 2 == 0 {
            self.stored_leaf_distance_even += 1;
        } else {
            self.stored_leaf_distance_odd += 1;
        }
    }

    fn record_incomplete(&mut self, root_side: Color) {
        match root_side {
            Color::Black => self.rejected_incomplete_root_black += 1,
            Color::White => self.rejected_incomplete_root_white += 1,
        }
    }

    fn record_terminal(&mut self, root_side: Color, leaf_side: Color) {
        match root_side {
            Color::Black => self.rejected_terminal_root_black += 1,
            Color::White => self.rejected_terminal_root_white += 1,
        }
        match leaf_side {
            Color::Black => self.rejected_terminal_leaf_black += 1,
            Color::White => self.rejected_terminal_leaf_white += 1,
        }
    }

    fn record_mate(&mut self, root_side: Color, leaf_side: Option<Color>) {
        match root_side {
            Color::Black => self.rejected_mate_root_black += 1,
            Color::White => self.rejected_mate_root_white += 1,
        }
        match leaf_side {
            Some(Color::Black) => self.rejected_mate_leaf_black += 1,
            Some(Color::White) => self.rejected_mate_leaf_white += 1,
            None => {}
        }
    }

    fn record_rejection_outcomes(&mut self, outcome: GameOutcome) {
        let incomplete = relative_rejection_counts(
            self.rejected_incomplete_root_black,
            self.rejected_incomplete_root_white,
            outcome,
        );
        let terminal = relative_rejection_counts(
            self.rejected_terminal_root_black,
            self.rejected_terminal_root_white,
            outcome,
        );
        let mate = relative_rejection_counts(
            self.rejected_mate_root_black,
            self.rejected_mate_root_white,
            outcome,
        );
        (
            self.rejected_incomplete_game_win,
            self.rejected_incomplete_game_loss,
            self.rejected_incomplete_game_draw,
        ) = incomplete;
        (
            self.rejected_terminal_game_win,
            self.rejected_terminal_game_loss,
            self.rejected_terminal_game_draw,
        ) = terminal;
        (
            self.rejected_mate_game_win,
            self.rejected_mate_game_loss,
            self.rejected_mate_game_draw,
        ) = mate;
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct SearchUseStats {
    label_searches: u64,
    rollout_searches: u64,
    label_search_states: u64,
    label_search_qnodes: u64,
    rollout_search_states: u64,
    rollout_search_qnodes: u64,
    label_search_elapsed_seconds: f64,
    rollout_search_elapsed_seconds: f64,
    candidate_positions: u64,
    rejected_incomplete_label_positions: u64,
    rejected_terminal_positions: u64,
    rejected_mate_score_positions: u64,
    root_ply_min: Option<u16>,
    root_ply_max: Option<u16>,
    leaf_distance_min: Option<u16>,
    leaf_distance_max: Option<u16>,
    leaf_distance_total: u64,
    position_selection: PositionSelectionStats,
}

impl SearchUseStats {
    fn record_label(&mut self, summary: &TeacherSearchSummary) {
        self.label_searches += 1;
        self.label_search_states += summary.states;
        self.label_search_qnodes += summary.qnodes;
        self.label_search_elapsed_seconds += summary.elapsed_seconds;
    }

    fn record_rollout(&mut self, summary: &TeacherSearchSummary) {
        self.rollout_searches += 1;
        self.rollout_search_states += summary.states;
        self.rollout_search_qnodes += summary.qnodes;
        self.rollout_search_elapsed_seconds += summary.elapsed_seconds;
    }

    fn add(&mut self, other: Self) {
        self.label_searches += other.label_searches;
        self.rollout_searches += other.rollout_searches;
        self.label_search_states += other.label_search_states;
        self.label_search_qnodes += other.label_search_qnodes;
        self.rollout_search_states += other.rollout_search_states;
        self.rollout_search_qnodes += other.rollout_search_qnodes;
        self.label_search_elapsed_seconds += other.label_search_elapsed_seconds;
        self.rollout_search_elapsed_seconds += other.rollout_search_elapsed_seconds;
        self.candidate_positions += other.candidate_positions;
        self.rejected_incomplete_label_positions += other.rejected_incomplete_label_positions;
        self.rejected_terminal_positions += other.rejected_terminal_positions;
        self.rejected_mate_score_positions += other.rejected_mate_score_positions;
        self.root_ply_min = option_min(self.root_ply_min, other.root_ply_min);
        self.root_ply_max = option_max(self.root_ply_max, other.root_ply_max);
        self.leaf_distance_min = option_min(self.leaf_distance_min, other.leaf_distance_min);
        self.leaf_distance_max = option_max(self.leaf_distance_max, other.leaf_distance_max);
        self.leaf_distance_total += other.leaf_distance_total;
        self.position_selection.add(other.position_selection);
    }

    fn record_candidate(&mut self, root_side: Color) {
        self.candidate_positions += 1;
        self.position_selection.record_candidate(root_side);
    }

    fn record_stored_position(
        &mut self,
        root_ply: u16,
        leaf_distance: u16,
        root_side: Color,
        leaf_side: Color,
    ) {
        self.root_ply_min = option_min(self.root_ply_min, Some(root_ply));
        self.root_ply_max = option_max(self.root_ply_max, Some(root_ply));
        self.leaf_distance_min = option_min(self.leaf_distance_min, Some(leaf_distance));
        self.leaf_distance_max = option_max(self.leaf_distance_max, Some(leaf_distance));
        self.leaf_distance_total += u64::from(leaf_distance);
        self.position_selection
            .record_stored(root_side, leaf_side, leaf_distance);
    }
}

fn option_min<T: Ord + Copy>(left: Option<T>, right: Option<T>) -> Option<T> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (left, right) => left.or(right),
    }
}

fn option_max<T: Ord + Copy>(left: Option<T>, right: Option<T>) -> Option<T> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    }
}

fn stored_position_count(stats: &SearchUseStats) -> u64 {
    stats
        .candidate_positions
        .saturating_sub(stats.rejected_incomplete_label_positions)
        .saturating_sub(stats.rejected_terminal_positions)
        .saturating_sub(stats.rejected_mate_score_positions)
}

fn leaf_distance_mean(stats: &SearchUseStats) -> f64 {
    let stored = stored_position_count(stats);
    if stored == 0 {
        0.0
    } else {
        stats.leaf_distance_total as f64 / stored as f64
    }
}

fn aggregate_opening_position_selection(
    shard_results: &[ShardResult],
) -> BTreeMap<String, PositionSelectionStats> {
    let mut aggregate = BTreeMap::new();
    for result in shard_results {
        for (opening_id, stats) in &result.manifest.opening_position_selection {
            aggregate
                .entry(opening_id.clone())
                .or_insert_with(PositionSelectionStats::default)
                .add(*stats);
        }
    }
    aggregate
}

impl From<&ShardManifest> for SearchUseStats {
    fn from(manifest: &ShardManifest) -> Self {
        Self {
            label_searches: manifest.label_searches,
            rollout_searches: manifest.rollout_searches,
            label_search_states: manifest.label_search_states,
            label_search_qnodes: manifest.label_search_qnodes,
            rollout_search_states: manifest.rollout_search_states,
            rollout_search_qnodes: manifest.rollout_search_qnodes,
            label_search_elapsed_seconds: manifest.label_search_cpu_seconds,
            rollout_search_elapsed_seconds: manifest.rollout_search_cpu_seconds,
            candidate_positions: if manifest.candidate_positions == 0 {
                manifest.sampled_positions
            } else {
                manifest.candidate_positions
            },
            rejected_incomplete_label_positions: manifest.rejected_incomplete_label_positions,
            rejected_terminal_positions: manifest.rejected_terminal_positions,
            rejected_mate_score_positions: manifest.rejected_mate_score_positions,
            root_ply_min: manifest.root_ply_min,
            root_ply_max: manifest.root_ply_max,
            leaf_distance_min: manifest.leaf_distance_min,
            leaf_distance_max: manifest.leaf_distance_max,
            leaf_distance_total: manifest.leaf_distance_total,
            position_selection: manifest.position_selection,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum GameOutcome {
    Draw,
    Winner(Color),
}

fn relative_rejection_counts(
    black_root: u64,
    white_root: u64,
    outcome: GameOutcome,
) -> (u64, u64, u64) {
    match outcome {
        GameOutcome::Draw => (0, 0, black_root + white_root),
        GameOutcome::Winner(Color::Black) => (black_root, white_root, 0),
        GameOutcome::Winner(Color::White) => (white_root, black_root, 0),
    }
}

#[derive(Debug, Clone)]
struct TeacherSearchSummary {
    best_move: Option<String>,
    best_score: Option<i32>,
    states: u64,
    qnodes: u64,
    elapsed_seconds: f64,
    training_trace: Option<SearchTrainingTrace>,
}

impl From<SearchSummary> for TeacherSearchSummary {
    fn from(summary: SearchSummary) -> Self {
        Self {
            best_move: summary.best_move,
            best_score: summary.best_score,
            states: summary.states,
            qnodes: summary.qsearch_stats.qnodes,
            elapsed_seconds: summary.elapsed_ms / 1_000.0,
            training_trace: None,
        }
    }
}

impl From<NodeBudgetSearchSummary> for TeacherSearchSummary {
    fn from(summary: NodeBudgetSearchSummary) -> Self {
        Self {
            best_move: summary.best_move,
            best_score: summary.best_score,
            states: summary.alpha_beta_nodes,
            qnodes: summary.qsearch_nodes,
            elapsed_seconds: summary.elapsed_ms / 1_000.0,
            training_trace: None,
        }
    }
}

impl TeacherSearchSummary {
    fn from_depth_with_trace(
        summary: SearchSummary,
        training_trace: Option<SearchTrainingTrace>,
    ) -> Self {
        let mut result = Self::from(summary);
        result.training_trace = training_trace;
        result
    }

    fn from_nodes_with_trace(
        summary: NodeBudgetSearchSummary,
        training_trace: Option<SearchTrainingTrace>,
    ) -> Self {
        let mut result = Self::from(summary);
        result.training_trace = training_trace;
        result
    }
}

impl GameOutcome {
    fn relative_to(self, side_to_move: Color) -> i8 {
        match self {
            Self::Draw => 0,
            Self::Winner(color) if color == side_to_move => 1,
            Self::Winner(_) => -1,
        }
    }
}

#[derive(Debug, Clone)]
enum Teacher {
    Handcrafted,
    Nnue {
        model: Arc<NnueModel>,
        bootstrap_sha256: String,
    },
}

impl Teacher {
    fn from_config(loaded: &LoadedConfig) -> Result<Self> {
        if let Some(path) = loaded.bootstrap_nnue() {
            if path.exists() {
                let bytes = fs::read(&path)
                    .with_context(|| format!("failed to read bootstrap NNUE {}", path.display()))?;
                let bootstrap_sha256 = hash_bytes_hex(&bytes);
                let model = NnueModel::from_bytes(&bytes).map_err(|err| {
                    anyhow!("failed to load bootstrap NNUE {}: {err}", path.display())
                })?;
                return Ok(Self::Nnue {
                    model: Arc::new(model),
                    bootstrap_sha256,
                });
            }
        }
        Ok(Self::Handcrafted)
    }

    fn describe(&self) -> &'static str {
        match self {
            Self::Handcrafted => "handcrafted",
            Self::Nnue { .. } => "nnue",
        }
    }

    fn bootstrap_sha256(&self) -> Option<&str> {
        match self {
            Self::Handcrafted => None,
            Self::Nnue {
                bootstrap_sha256, ..
            } => Some(bootstrap_sha256),
        }
    }

    fn search_depth(
        &self,
        board: &Board,
        depth: u8,
        workspace: &mut SearchWorkspace,
    ) -> Result<TeacherSearchSummary> {
        match self {
            Self::Handcrafted => {
                search_board_impl_handcrafted_in_workspace(board, depth, workspace)
                    .map_err(|err| anyhow!("handcrafted teacher search failed: {err}"))
            }
            Self::Nnue { model, .. } => search_board_impl_with_eval_mode_in_workspace(
                board,
                depth,
                model.clone(),
                SearchEvalMode::Incremental,
                workspace,
            )
            .map_err(|err| anyhow!("NNUE teacher search failed: {err}")),
        }
        .map(TeacherSearchSummary::from)
    }

    fn search_label(
        &self,
        board: &Board,
        budget: LabelSearchBudget,
        position_policy: PositionPolicy,
        workspace: &mut SearchWorkspace,
    ) -> Result<TeacherSearchSummary> {
        let collect_trace = position_policy.uses_training_trace();
        match budget {
            LabelSearchBudget::Depth { depth } if !collect_trace => {
                self.search_depth(board, depth, workspace)
            }
            LabelSearchBudget::Depth { depth } => match self {
                Self::Handcrafted => search_board_impl_handcrafted_with_training_trace_in_workspace(
                    board, depth, workspace,
                )
                .map_err(|err| anyhow!("handcrafted traced teacher search failed: {err}")),
                Self::Nnue { model, .. } => {
                    search_board_impl_with_eval_mode_and_training_trace_in_workspace(
                        board,
                        depth,
                        model.clone(),
                        SearchEvalMode::Incremental,
                        workspace,
                    )
                    .map_err(|err| anyhow!("NNUE traced teacher search failed: {err}"))
                }
            }
            .map(|(summary, trace)| TeacherSearchSummary::from_depth_with_trace(summary, trace)),
            LabelSearchBudget::Nodes { nodes, max_depth } if collect_trace => match self {
                Self::Handcrafted => {
                    search_board_impl_handcrafted_with_node_budget_and_training_trace_in_workspace(
                        board, nodes, max_depth, workspace,
                    )
                    .map_err(|err| anyhow!("handcrafted traced node-budget teacher search failed: {err}"))
                }
                Self::Nnue { model, .. } => {
                    search_board_impl_with_eval_mode_and_node_budget_and_training_trace_in_workspace(
                        board,
                        nodes,
                        max_depth,
                        model.clone(),
                        SearchEvalMode::Incremental,
                        workspace,
                    )
                    .map_err(|err| anyhow!("NNUE traced node-budget teacher search failed: {err}"))
                }
            }
            .map(|(summary, trace)| TeacherSearchSummary::from_nodes_with_trace(summary, trace)),
            LabelSearchBudget::Nodes { nodes, max_depth } => match self {
                Self::Handcrafted => search_board_impl_handcrafted_with_node_budget_in_workspace(
                    board, nodes, max_depth, workspace,
                )
                .map_err(|err| anyhow!("handcrafted node-budget teacher search failed: {err}")),
                Self::Nnue { model, .. } => {
                    search_board_impl_with_eval_mode_and_node_budget_in_workspace(
                        board,
                        nodes,
                        max_depth,
                        model.clone(),
                        SearchEvalMode::Incremental,
                        workspace,
                    )
                    .map_err(|err| anyhow!("NNUE node-budget teacher search failed: {err}"))
                }
            }
            .map(TeacherSearchSummary::from),
        }
    }
}

fn node_budget_summary_is_complete(summary: &TeacherSearchSummary) -> bool {
    summary.best_move.is_some() && summary.best_score.is_some()
}

fn apply_incomplete_label_policy(
    summary: TeacherSearchSummary,
    budget: LabelSearchBudget,
    policy: IncompleteLabelPolicy,
    root_side: Color,
    stats: &mut SearchUseStats,
) -> Result<Option<TeacherSearchSummary>> {
    if !matches!(budget, LabelSearchBudget::Nodes { .. })
        || node_budget_summary_is_complete(&summary)
    {
        return Ok(Some(summary));
    }

    stats.record_candidate(root_side);
    match policy {
        IncompleteLabelPolicy::Error => {
            let nodes = budget.nodes().unwrap_or_default();
            bail!(
                "node-budget teacher did not complete depth 1 within {nodes} nodes; increase data.label_search_nodes or set data.incomplete_label_policy=reject-position"
            );
        }
        IncompleteLabelPolicy::RejectPosition => {
            stats.rejected_incomplete_label_positions += 1;
            stats.position_selection.record_incomplete(root_side);
            Ok(None)
        }
    }
}

pub fn generate_data(loaded: &LoadedConfig) -> Result<DatasetOutput> {
    generate_data_with_options(loaded, GenerateOptions::from_config(loaded))
}

pub fn generate_data_with_options(
    loaded: &LoadedConfig,
    options: GenerateOptions,
) -> Result<DatasetOutput> {
    let _graceful_stop = GracefulStopGuard::install()?;
    loaded.ruleset_requires_matching_engine()?;
    let opening_sfen = loaded.opening_sfen()?;
    let _: Board = Board::from_sfen(&opening_sfen)
        .map_err(|err| anyhow!("invalid opening SFEN in config: {err}"))?;
    let opening_source = OpeningSource::from_config(loaded, &opening_sfen)?;
    let opening_split = opening_source.split_openings(
        loaded.config.data.split_policy,
        loaded.config.data.split_seed,
        loaded.config.data.train_games,
        loaded.config.data.validation_games,
    )?;

    let artifacts = loaded.artifact_paths();
    artifacts.ensure_dirs()?;

    let teacher = Teacher::from_config(loaded)?;
    let engine_revision = detect_git_revision(loaded)?;
    let generated_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let shard_selector = ShardSelector::new(
        options.shard_index,
        options.shard_index_end,
        options.shard_count,
    )?;
    let jobs = resolve_jobs(options.jobs.unwrap_or(loaded.config.data.jobs))?;
    let resume = options.resume.unwrap_or(loaded.config.data.resume);
    let allow_identity_mismatch = resolve_identity_mismatch(
        loaded,
        &artifacts,
        &teacher,
        &opening_sfen,
        &opening_source,
        &opening_split,
        &engine_revision,
        shard_selector,
        resume,
        options.ignore_identity_mismatch,
    )?;
    let started = Instant::now();

    let train_positions = generate_split(
        "train",
        loaded,
        &artifacts,
        &teacher,
        &opening_sfen,
        &opening_source,
        &opening_split,
        loaded.config.data.train_games,
        &engine_revision,
        generated_at_unix_ms,
        jobs,
        resume,
        shard_selector,
        allow_identity_mismatch,
    )?;
    if graceful_stop_requested() {
        bail!(
            "generate-data stopped gracefully after training split elapsed={}; \
             completed shard files were kept and can be resumed",
            format_duration(started.elapsed())
        );
    }
    let validation_positions = generate_split(
        "validation",
        loaded,
        &artifacts,
        &teacher,
        &opening_sfen,
        &opening_source,
        &opening_split,
        loaded.config.data.validation_games,
        &engine_revision,
        generated_at_unix_ms,
        jobs,
        resume,
        shard_selector,
        allow_identity_mismatch,
    )?;

    if graceful_stop_requested() {
        bail!(
            "generate-data stopped gracefully elapsed={}; \
             completed shard files were kept and can be resumed",
            format_duration(started.elapsed())
        );
    } else {
        println!(
            "generate-data finished elapsed={}",
            format_duration(started.elapsed())
        );
    }

    Ok(DatasetOutput {
        output_dir: artifacts.output_dir,
        train_positions,
        validation_positions,
    })
}

struct GracefulStopGuard {
    #[cfg(unix)]
    previous_handler: Option<libc::sighandler_t>,
}

impl GracefulStopGuard {
    fn install() -> Result<Self> {
        GRACEFUL_STOP_STATE.store(0, Ordering::SeqCst);
        install_graceful_stop_handler()
    }
}

#[cfg(test)]
fn install_graceful_stop_handler() -> Result<GracefulStopGuard> {
    Ok(GracefulStopGuard {
        #[cfg(unix)]
        previous_handler: None,
    })
}

#[cfg(all(unix, not(test)))]
fn install_graceful_stop_handler() -> Result<GracefulStopGuard> {
    let previous_handler = unsafe {
        let previous = libc::signal(
            libc::SIGINT,
            handle_sigint as *const () as libc::sighandler_t,
        );
        if previous == libc::SIG_ERR {
            bail!("failed to install SIGINT handler");
        }
        previous
    };
    Ok(GracefulStopGuard {
        previous_handler: Some(previous_handler),
    })
}

#[cfg(all(not(unix), not(test)))]
fn install_graceful_stop_handler() -> Result<GracefulStopGuard> {
    Ok(GracefulStopGuard {})
}

#[cfg(unix)]
impl Drop for GracefulStopGuard {
    fn drop(&mut self) {
        if let Some(previous_handler) = self.previous_handler {
            unsafe {
                libc::signal(libc::SIGINT, previous_handler);
            }
        }
    }
}

#[cfg(not(unix))]
impl Drop for GracefulStopGuard {
    fn drop(&mut self) {}
}

#[cfg(all(unix, not(test)))]
unsafe extern "C" fn handle_sigint(_: libc::c_int) {
    if GRACEFUL_STOP_STATE
        .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        unsafe {
            libc::write(
                libc::STDERR_FILENO,
                GRACEFUL_STOP_MESSAGE.as_ptr().cast(),
                GRACEFUL_STOP_MESSAGE.len(),
            );
        }
    } else {
        unsafe {
            libc::_exit(130);
        }
    }
}

fn graceful_stop_requested() -> bool {
    GRACEFUL_STOP_STATE.load(Ordering::SeqCst) != 0
}

fn generate_split(
    dataset_name: &str,
    loaded: &LoadedConfig,
    artifacts: &ArtifactPaths,
    teacher: &Teacher,
    opening_sfen: &str,
    opening_source: &OpeningSource,
    opening_split: &OpeningSplit,
    game_count: u32,
    engine_revision: &Option<String>,
    generated_at_unix_ms: u128,
    jobs: usize,
    resume: bool,
    shard_selector: ShardSelector,
    allow_identity_mismatch: bool,
) -> Result<u64> {
    let (bin_path, manifest_path) = match dataset_name {
        "train" => (&artifacts.train_bin, &artifacts.train_manifest),
        "validation" => (&artifacts.validation_bin, &artifacts.validation_manifest),
        _ => bail!("unknown dataset split `{dataset_name}`"),
    };

    let split_started = Instant::now();
    let plans = shard_plans(game_count, loaded.config.data.shard_games, shard_selector);
    let total_games = plans.iter().map(|plan| plan.game_count).sum::<u32>();
    let progress = Arc::new(Mutex::new(Progress::new(
        dataset_name,
        total_games,
        loaded.config.data.progress_every_percent,
    )));
    let queue = Arc::new(Mutex::new(VecDeque::from(plans)));
    let results = Arc::new(Mutex::new(Vec::<ShardResult>::new()));
    let worker_count = jobs.min(queue.lock().unwrap().len().max(1));
    let mut worker_errors = Vec::new();

    thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            let results = Arc::clone(&results);
            let progress = Arc::clone(&progress);
            let teacher = teacher.clone();
            let loaded = loaded.clone();
            let artifacts = artifacts.clone();
            let opening_sfen = opening_sfen.to_string();
            let opening_source = opening_source.clone();
            let opening_split = opening_split.clone();
            let engine_revision = engine_revision.clone();
            let dataset_name = dataset_name.to_string();

            handles.push(scope.spawn(move || -> Result<()> {
                loop {
                    let Some(plan) = next_shard_plan(&queue) else {
                        return Ok(());
                    };
                    let result = generate_or_reuse_shard(
                        &dataset_name,
                        &loaded,
                        &artifacts,
                        &teacher,
                        &opening_sfen,
                        &opening_source,
                        &opening_split,
                        &engine_revision,
                        generated_at_unix_ms,
                        plan,
                        resume,
                        allow_identity_mismatch,
                    )?;
                    progress.lock().unwrap().record(&result);
                    results.lock().unwrap().push(result);
                }
            }));
        }

        for handle in handles {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(err)) => worker_errors.push(err),
                Err(_) => worker_errors.push(anyhow!("dataset worker panicked")),
            }
        }
    });

    if let Some(err) = worker_errors.into_iter().next() {
        return Err(err);
    }

    if graceful_stop_requested() {
        println!("graceful stop requested; assembling completed {dataset_name} shards");
    }

    let mut shard_results = Arc::try_unwrap(results)
        .map_err(|_| anyhow!("failed to unwrap shard results"))?
        .into_inner()
        .map_err(|_| anyhow!("failed to lock shard results"))?;
    shard_results.sort_by_key(|result| result.manifest.game_start);
    let sampled_positions = assemble_shards(
        &shard_results,
        bin_path,
        loaded.config.data.shuffle_policy,
        loaded.config.data.shuffle_seed,
        loaded.config.data.shuffle_chunk_records,
        dataset_name,
    )?;
    let completed_games = shard_results
        .iter()
        .map(|result| result.manifest.game_count)
        .sum::<u32>();
    let resumed_shards = shard_results.iter().filter(|result| result.reused).count();
    let generated_shards = shard_results.len() - resumed_shards;
    let generated_positions = shard_results
        .iter()
        .filter(|result| !result.reused)
        .map(|result| result.manifest.sampled_positions)
        .sum::<u64>();
    let search_stats = shard_results
        .iter()
        .fold(SearchUseStats::default(), |mut stats, result| {
            stats.add(SearchUseStats::from(&result.manifest));
            stats
        });
    let games = shard_results
        .iter()
        .flat_map(|result| result.manifest.games.iter().cloned())
        .collect::<Vec<_>>();
    let opening_position_selection = aggregate_opening_position_selection(&shard_results);
    let opening_ids = games
        .iter()
        .map(|game| game.opening_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let elapsed = split_started.elapsed();
    let positions_per_second = if elapsed.as_secs_f64() > 0.0 {
        generated_positions as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };
    let label_budget = loaded.config.data.label_search_budget()?;
    let label_search_total_nodes = search_stats
        .label_search_states
        .saturating_add(search_stats.label_search_qnodes);
    let label_nodes_per_search = if search_stats.label_searches == 0 {
        0.0
    } else {
        label_search_total_nodes as f64 / search_stats.label_searches as f64
    };

    let manifest = DatasetManifest {
        dataset: dataset_name.to_string(),
        ruleset: loaded.config.rules.ruleset,
        rule_id: loaded.effective_rule_id()?,
        opening_sfen: opening_sfen.to_string(),
        opening_policy: opening_source.policy().to_string(),
        opening_suite_id: opening_source.suite_id().map(str::to_string),
        opening_suite_sha256: opening_source.suite_sha256().map(str::to_string),
        opening_transformation: opening_source.transformation().to_string(),
        opening_ids,
        games,
        split_policy: loaded.config.data.split_policy.manifest_name().to_string(),
        split_seed: loaded.config.data.split_seed,
        train_opening_ids: opening_split.train_ids.clone(),
        validation_opening_ids: opening_split.validation_ids.clone(),
        opening_group_count: opening_split.ids_for(dataset_name)?.len(),
        opening_group_overlap: opening_split.overlap(),
        shuffle_policy: loaded
            .config
            .data
            .shuffle_policy
            .manifest_name()
            .to_string(),
        shuffle_seed: loaded.config.data.shuffle_seed,
        shuffle_chunk_records: loaded.config.data.shuffle_chunk_records,
        shuffle_memory_bound_bytes: shuffle_memory_bound_bytes(
            loaded.config.data.shuffle_chunk_records,
        ),
        game_count,
        completed_games,
        sampled_positions,
        search_depth: label_budget.legacy_search_depth(),
        label_search_depth: label_budget.max_depth(),
        label_search_budget: label_budget.manifest_name().to_string(),
        label_search_nodes: label_budget.nodes(),
        label_search_max_depth: label_budget.max_depth(),
        node_counting_version: SEARCH_NODE_COUNTING_VERSION.to_string(),
        position_policy: loaded
            .config
            .data
            .position_policy
            .manifest_name()
            .to_string(),
        training_trace_version: SEARCH_TRAINING_TRACE_VERSION.to_string(),
        incomplete_label_policy: loaded
            .config
            .data
            .incomplete_label_policy
            .manifest_name()
            .to_string(),
        position_selection_audit_version: POSITION_SELECTION_AUDIT_VERSION.to_string(),
        candidate_positions: search_stats.candidate_positions,
        rejected_incomplete_label_positions: search_stats.rejected_incomplete_label_positions,
        rejected_terminal_positions: search_stats.rejected_terminal_positions,
        rejected_mate_score_positions: search_stats.rejected_mate_score_positions,
        position_selection: search_stats.position_selection,
        opening_position_selection,
        root_ply_min: search_stats.root_ply_min,
        root_ply_max: search_stats.root_ply_max,
        leaf_distance_min: search_stats.leaf_distance_min,
        leaf_distance_max: search_stats.leaf_distance_max,
        leaf_distance_mean: leaf_distance_mean(&search_stats),
        rollout_search_depth: loaded.config.data.rollout_search_depth,
        self_play_move_policy: loaded
            .config
            .data
            .self_play_move_policy
            .manifest_name()
            .to_string(),
        label_searches: search_stats.label_searches,
        rollout_searches: search_stats.rollout_searches,
        label_search_states: search_stats.label_search_states,
        label_search_qnodes: search_stats.label_search_qnodes,
        label_search_total_nodes,
        label_nodes_per_search,
        rollout_search_states: search_stats.rollout_search_states,
        rollout_search_qnodes: search_stats.rollout_search_qnodes,
        label_search_cpu_seconds: search_stats.label_search_elapsed_seconds,
        rollout_search_cpu_seconds: search_stats.rollout_search_elapsed_seconds,
        generation_cpu_seconds: search_stats.label_search_elapsed_seconds
            + search_stats.rollout_search_elapsed_seconds,
        bootstrap_nnue: bootstrap_nnue_path(loaded),
        bootstrap_nnue_sha256: teacher.bootstrap_sha256().map(str::to_string),
        engine_revision: engine_revision.clone(),
        config_hash: loaded.hash_hex.clone(),
        seed: loaded.config.data.seed,
        feature_family: loaded.training_features().to_string(),
        sampling_phase: loaded
            .config
            .data
            .sampling_policy
            .manifest_name()
            .to_string(),
        sample_after_opening: loaded.config.data.sampling_policy.samples_after_opening(),
        teacher_move_encoding: TEACHER_MOVE_ENCODING.to_string(),
        opening_random_plies: loaded.config.data.opening_random_plies,
        generated_at_unix_ms,
        build_mode: teacher_build_mode(loaded, teacher),
        entry_bytes: ENTRY_BYTES,
        shard_count: shard_results.len(),
        jobs,
        resumed_shards,
        generated_shards,
        elapsed_seconds: elapsed.as_secs_f64(),
        positions_per_second,
    };
    fs::write(manifest_path, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;

    Ok(sampled_positions)
}

fn next_shard_plan(queue: &Arc<Mutex<VecDeque<ShardPlan>>>) -> Option<ShardPlan> {
    next_shard_plan_with(queue, graceful_stop_requested)
}

fn next_shard_plan_with(
    queue: &Arc<Mutex<VecDeque<ShardPlan>>>,
    stop_requested: impl Fn() -> bool,
) -> Option<ShardPlan> {
    if stop_requested() {
        return None;
    }
    let mut queue = queue.lock().unwrap();
    if stop_requested() {
        return None;
    }
    queue.pop_front()
}

pub fn merge_data(
    loaded: &LoadedConfig,
    input_dirs: &[PathBuf],
    ignore_identity_mismatch: bool,
) -> Result<DatasetOutput> {
    loaded.ruleset_requires_matching_engine()?;
    if input_dirs.is_empty() {
        bail!("merge-data requires at least one --input directory");
    }

    let artifacts = loaded.artifact_paths();
    artifacts.ensure_dirs()?;
    let opening_sfen = loaded.opening_sfen()?;
    let opening_source = OpeningSource::from_config(loaded, &opening_sfen)?;
    let opening_split = opening_source.split_openings(
        loaded.config.data.split_policy,
        loaded.config.data.split_seed,
        loaded.config.data.train_games,
        loaded.config.data.validation_games,
    )?;
    let generated_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let train_positions = merge_split(
        "train",
        loaded,
        &artifacts.train_bin,
        &artifacts.train_manifest,
        input_dirs,
        loaded.config.data.train_games,
        &opening_sfen,
        &opening_source,
        &opening_split,
        generated_at_unix_ms,
        ignore_identity_mismatch,
    )?;
    let validation_positions = merge_split(
        "validation",
        loaded,
        &artifacts.validation_bin,
        &artifacts.validation_manifest,
        input_dirs,
        loaded.config.data.validation_games,
        &opening_sfen,
        &opening_source,
        &opening_split,
        generated_at_unix_ms,
        ignore_identity_mismatch,
    )?;

    Ok(DatasetOutput {
        output_dir: artifacts.output_dir,
        train_positions,
        validation_positions,
    })
}

#[derive(Debug, Clone, Copy)]
struct ShardSelector {
    index: u32,
    index_end: u32,
    count: u32,
}

impl ShardSelector {
    fn new(index: Option<u32>, index_end: Option<u32>, count: Option<u32>) -> Result<Self> {
        let index = index.unwrap_or(0);
        let index_end = index_end.unwrap_or(index);
        let count = count.unwrap_or(1);
        if count == 0 {
            bail!("--shard-count must be at least 1");
        }
        if index >= count {
            bail!("--shard-index must be less than --shard-count");
        }
        if index_end >= count {
            bail!("--shard-index-end must be less than --shard-count");
        }
        if index_end < index {
            bail!("--shard-index-end must be greater than or equal to --shard-index");
        }
        if count == 1 && index != 0 {
            bail!("--shard-index must be 0 when --shard-count is 1");
        }
        Ok(Self {
            index,
            index_end,
            count,
        })
    }

    fn selected_range(self, total_shards: u32) -> Range<u32> {
        let start = partition_boundary(total_shards, self.index, self.count);
        let end = partition_boundary(total_shards, self.index_end + 1, self.count);
        start..end
    }
}

#[derive(Debug, Clone, Copy)]
struct ShardPlan {
    shard_index: u32,
    game_start: u32,
    game_count: u32,
}

#[derive(Debug, Clone)]
struct ShardResult {
    bin_path: PathBuf,
    manifest: ShardManifest,
    reused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MergeTeacherIdentity {
    bootstrap_nnue: Option<String>,
    bootstrap_nnue_sha256: Option<String>,
    engine_revision: Option<String>,
    build_mode: String,
}

impl MergeTeacherIdentity {
    fn from_manifest(manifest: &ShardManifest) -> Self {
        Self {
            bootstrap_nnue: manifest.bootstrap_nnue.clone(),
            bootstrap_nnue_sha256: manifest.bootstrap_nnue_sha256.clone(),
            engine_revision: manifest.engine_revision.clone(),
            build_mode: manifest.build_mode.clone(),
        }
    }
}

struct Progress {
    dataset_name: String,
    total_games: u32,
    completed_games: u32,
    sampled_positions: u64,
    generated_games: u32,
    started: Instant,
    next_percent: u32,
    every_percent: u32,
}

impl Progress {
    fn new(dataset_name: &str, total_games: u32, every_percent: u32) -> Self {
        Self {
            dataset_name: dataset_name.to_string(),
            total_games,
            completed_games: 0,
            sampled_positions: 0,
            generated_games: 0,
            started: Instant::now(),
            next_percent: every_percent.max(1),
            every_percent: every_percent.max(1),
        }
    }

    fn record(&mut self, result: &ShardResult) {
        self.completed_games += result.manifest.game_count;
        self.sampled_positions += result.manifest.sampled_positions;
        if !result.reused {
            self.generated_games += result.manifest.game_count;
        }
        let percent = if self.total_games == 0 {
            100
        } else {
            ((u64::from(self.completed_games) * 100) / u64::from(self.total_games)) as u32
        };
        while self.next_percent <= percent.min(100) {
            self.print_line(self.next_percent);
            self.next_percent += self.every_percent;
        }
    }

    fn print_line(&self, percent: u32) {
        let elapsed = self.started.elapsed();
        // Throughput and ETA reflect freshly generated games only; restored
        // (resumed) shards complete instantly and would otherwise inflate them.
        let games_per_minute = if elapsed.as_secs_f64() > 0.0 {
            f64::from(self.generated_games) / elapsed.as_secs_f64() * 60.0
        } else {
            0.0
        };
        let eta = if self.generated_games == 0 || self.total_games == 0 {
            None
        } else {
            let seconds_per_game = elapsed.as_secs_f64() / f64::from(self.generated_games);
            let remaining_games = self.total_games.saturating_sub(self.completed_games);
            Some(Duration::from_secs_f64(
                seconds_per_game * f64::from(remaining_games),
            ))
        };
        println!(
            "{} {}% games={}/{} positions={} elapsed={} eta={} speed={:.1} games/min",
            self.dataset_name,
            percent,
            self.completed_games,
            self.total_games,
            self.sampled_positions,
            format_duration(elapsed),
            eta.map(format_duration)
                .unwrap_or_else(|| "--:--:--".to_string()),
            games_per_minute,
        );
    }
}

fn resolve_jobs(configured: u32) -> Result<usize> {
    if configured == 0 {
        Ok(thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1))
    } else {
        usize::try_from(configured).context("data.jobs does not fit into usize")
    }
}

fn shard_plans(game_count: u32, shard_games: u32, selector: ShardSelector) -> Vec<ShardPlan> {
    let mut plans = Vec::new();
    let total_shards = game_count.div_ceil(shard_games);
    let selected_shards = selector.selected_range(total_shards);
    let mut game_start = 0;
    let mut shard_index = 0;
    while game_start < game_count {
        let remaining = game_count - game_start;
        let current_games = remaining.min(shard_games);
        if selected_shards.contains(&shard_index) {
            plans.push(ShardPlan {
                shard_index,
                game_start,
                game_count: current_games,
            });
        }
        game_start += current_games;
        shard_index += 1;
    }
    plans
}

fn partition_boundary(total_shards: u32, lane_index: u32, lane_count: u32) -> u32 {
    ((u64::from(total_shards) * u64::from(lane_index)) / u64::from(lane_count)) as u32
}

fn generate_or_reuse_shard(
    dataset_name: &str,
    loaded: &LoadedConfig,
    artifacts: &ArtifactPaths,
    teacher: &Teacher,
    opening_sfen: &str,
    opening_source: &OpeningSource,
    opening_split: &OpeningSplit,
    engine_revision: &Option<String>,
    generated_at_unix_ms: u128,
    plan: ShardPlan,
    resume: bool,
    allow_identity_mismatch: bool,
) -> Result<ShardResult> {
    let shards_dir = artifacts.datasets_dir.join("shards").join(dataset_name);
    fs::create_dir_all(&shards_dir)
        .with_context(|| format!("failed to create {}", shards_dir.display()))?;
    let bin_path = shards_dir.join(format!("shard-{:06}.bin", plan.shard_index));
    let manifest_path = shards_dir.join(format!("shard-{:06}.json", plan.shard_index));

    if resume {
        if let Some(result) = reusable_shard(
            loaded,
            dataset_name,
            opening_sfen,
            opening_source,
            opening_split,
            teacher,
            engine_revision,
            plan,
            &bin_path,
            &manifest_path,
            allow_identity_mismatch,
        )? {
            return Ok(result);
        }
    }

    let tmp_bin = bin_path.with_extension("bin.tmp");
    let tmp_manifest = manifest_path.with_extension("json.tmp");
    let mut writer = BufWriter::new(
        File::create(&tmp_bin)
            .with_context(|| format!("failed to create {}", tmp_bin.display()))?,
    );
    let mut sampled_positions = 0u64;
    let mut search_stats = SearchUseStats::default();
    let mut games = Vec::with_capacity(plan.game_count as usize);
    let mut opening_position_selection = BTreeMap::<String, PositionSelectionStats>::new();
    let mut search_workspace = SearchWorkspace::default();

    for game_index in plan.game_start..plan.game_start + plan.game_count {
        let shard_index = plan.shard_index;
        let error_context =
            format!("failed to generate {dataset_name} game {game_index} in shard {shard_index}");
        let game = generate_game_entries(
            dataset_name,
            loaded,
            teacher,
            &mut search_workspace,
            opening_source,
            opening_split,
            game_index,
        )
        .context(error_context)?;
        sampled_positions += (game.entries.len() / ENTRY_BYTES) as u64;
        search_stats.add(game.stats);
        opening_position_selection
            .entry(game.opening.opening_id.clone())
            .or_default()
            .add(game.stats.position_selection);
        games.push(game.opening);
        writer.write_all(&game.entries)?;
    }
    writer.flush()?;

    let label_budget = loaded.config.data.label_search_budget()?;

    let manifest = ShardManifest {
        dataset: dataset_name.to_string(),
        ruleset: loaded.config.rules.ruleset,
        rule_id: loaded.effective_rule_id()?,
        opening_sfen: opening_sfen.to_string(),
        opening_policy: opening_source.policy().to_string(),
        opening_suite_id: opening_source.suite_id().map(str::to_string),
        opening_suite_sha256: opening_source.suite_sha256().map(str::to_string),
        opening_transformation: opening_source.transformation().to_string(),
        opening_ids: games
            .iter()
            .map(|game| game.opening_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        games,
        split_policy: loaded.config.data.split_policy.manifest_name().to_string(),
        split_seed: loaded.config.data.split_seed,
        train_opening_ids: opening_split.train_ids.clone(),
        validation_opening_ids: opening_split.validation_ids.clone(),
        shuffle_policy: loaded
            .config
            .data
            .shuffle_policy
            .manifest_name()
            .to_string(),
        shuffle_seed: loaded.config.data.shuffle_seed,
        shuffle_chunk_records: loaded.config.data.shuffle_chunk_records,
        game_start: plan.game_start,
        game_count: plan.game_count,
        sampled_positions,
        search_depth: label_budget.legacy_search_depth(),
        label_search_depth: label_budget.max_depth(),
        label_search_budget: label_budget.manifest_name().to_string(),
        label_search_nodes: label_budget.nodes(),
        label_search_max_depth: label_budget.max_depth(),
        node_counting_version: SEARCH_NODE_COUNTING_VERSION.to_string(),
        position_policy: loaded
            .config
            .data
            .position_policy
            .manifest_name()
            .to_string(),
        training_trace_version: SEARCH_TRAINING_TRACE_VERSION.to_string(),
        incomplete_label_policy: loaded
            .config
            .data
            .incomplete_label_policy
            .manifest_name()
            .to_string(),
        position_selection_audit_version: POSITION_SELECTION_AUDIT_VERSION.to_string(),
        candidate_positions: search_stats.candidate_positions,
        rejected_incomplete_label_positions: search_stats.rejected_incomplete_label_positions,
        rejected_terminal_positions: search_stats.rejected_terminal_positions,
        rejected_mate_score_positions: search_stats.rejected_mate_score_positions,
        position_selection: search_stats.position_selection,
        opening_position_selection,
        root_ply_min: search_stats.root_ply_min,
        root_ply_max: search_stats.root_ply_max,
        leaf_distance_min: search_stats.leaf_distance_min,
        leaf_distance_max: search_stats.leaf_distance_max,
        leaf_distance_total: search_stats.leaf_distance_total,
        rollout_search_depth: loaded.config.data.rollout_search_depth,
        self_play_move_policy: loaded
            .config
            .data
            .self_play_move_policy
            .manifest_name()
            .to_string(),
        label_searches: search_stats.label_searches,
        rollout_searches: search_stats.rollout_searches,
        label_search_states: search_stats.label_search_states,
        label_search_qnodes: search_stats.label_search_qnodes,
        rollout_search_states: search_stats.rollout_search_states,
        rollout_search_qnodes: search_stats.rollout_search_qnodes,
        label_search_cpu_seconds: search_stats.label_search_elapsed_seconds,
        rollout_search_cpu_seconds: search_stats.rollout_search_elapsed_seconds,
        bootstrap_nnue: bootstrap_nnue_path(loaded),
        bootstrap_nnue_sha256: teacher.bootstrap_sha256().map(str::to_string),
        engine_revision: engine_revision.clone(),
        config_hash: loaded.hash_hex.clone(),
        sampling_phase: loaded
            .config
            .data
            .sampling_policy
            .manifest_name()
            .to_string(),
        sample_after_opening: loaded.config.data.sampling_policy.samples_after_opening(),
        teacher_move_encoding: TEACHER_MOVE_ENCODING.to_string(),
        generated_at_unix_ms,
        build_mode: teacher_build_mode(loaded, teacher),
        entry_bytes: ENTRY_BYTES,
        shard_index: plan.shard_index,
    };
    fs::write(&tmp_manifest, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("failed to write {}", tmp_manifest.display()))?;
    fs::rename(&tmp_bin, &bin_path)
        .with_context(|| format!("failed to rename {}", tmp_bin.display()))?;
    fs::rename(&tmp_manifest, &manifest_path)
        .with_context(|| format!("failed to rename {}", tmp_manifest.display()))?;

    Ok(ShardResult {
        bin_path,
        manifest,
        reused: false,
    })
}

fn reusable_shard(
    loaded: &LoadedConfig,
    dataset_name: &str,
    opening_sfen: &str,
    opening_source: &OpeningSource,
    opening_split: &OpeningSplit,
    teacher: &Teacher,
    engine_revision: &Option<String>,
    plan: ShardPlan,
    bin_path: &Path,
    manifest_path: &Path,
    allow_identity_mismatch: bool,
) -> Result<Option<ShardResult>> {
    if !bin_path.exists() || !manifest_path.exists() {
        return Ok(None);
    }
    let manifest: ShardManifest = serde_json::from_slice(
        &fs::read(manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    if !shard_manifest_matches(
        loaded,
        dataset_name,
        opening_sfen,
        opening_source,
        opening_split,
        plan,
        &manifest,
        allow_identity_mismatch,
    )? {
        return Ok(None);
    }
    if !shard_teacher_matches(
        loaded,
        teacher,
        engine_revision,
        &manifest,
        allow_identity_mismatch,
    ) {
        return Ok(None);
    }
    if !shard_bin_matches(bin_path, &manifest) {
        return Ok(None);
    }
    Ok(Some(ShardResult {
        bin_path: bin_path.to_path_buf(),
        manifest,
        reused: true,
    }))
}

/// Whether `bin_path` exists and has the exact byte length implied by the manifest.
/// A missing, partially written, or unreadable file is treated as not reusable.
fn shard_bin_matches(bin_path: &Path, manifest: &ShardManifest) -> bool {
    let Some(expected_len) = manifest.sampled_positions.checked_mul(ENTRY_BYTES as u64) else {
        return false;
    };
    fs::metadata(bin_path)
        .map(|meta| meta.len() == expected_len)
        .unwrap_or(false)
}

fn shard_manifest_matches(
    loaded: &LoadedConfig,
    dataset_name: &str,
    opening_sfen: &str,
    opening_source: &OpeningSource,
    opening_split: &OpeningSplit,
    plan: ShardPlan,
    manifest: &ShardManifest,
    ignore_identity: bool,
) -> Result<bool> {
    let label_budget = loaded.config.data.label_search_budget()?;
    Ok(manifest.dataset == dataset_name
        && manifest.ruleset == loaded.config.rules.ruleset
        && manifest.rule_id == loaded.effective_rule_id()?
        && manifest.opening_sfen == opening_sfen
        && (ignore_identity
            || (manifest.opening_policy == opening_source.policy()
                && manifest.opening_suite_id.as_deref() == opening_source.suite_id()
                && manifest.opening_suite_sha256.as_deref() == opening_source.suite_sha256()
                && manifest.opening_transformation == opening_source.transformation()
                && manifest.split_policy == loaded.config.data.split_policy.manifest_name()
                && manifest.split_seed == loaded.config.data.split_seed
                && manifest.train_opening_ids == opening_split.train_ids
                && manifest.validation_opening_ids == opening_split.validation_ids
                && manifest.shuffle_policy == loaded.config.data.shuffle_policy.manifest_name()
                && manifest.shuffle_seed == loaded.config.data.shuffle_seed
                && manifest.shuffle_chunk_records == loaded.config.data.shuffle_chunk_records))
        && manifest.game_start == plan.game_start
        && manifest.game_count == plan.game_count
        && manifest.search_depth == label_budget.legacy_search_depth()
        && manifest.label_search_depth() == label_budget.max_depth()
        && manifest.label_search_budget() == label_budget.manifest_name()
        && manifest.label_search_nodes == label_budget.nodes()
        && manifest.label_search_max_depth() == label_budget.max_depth()
        && manifest.node_counting_version_matches(label_budget)
        && manifest.position_policy() == loaded.config.data.position_policy.manifest_name()
        && manifest.training_trace_version_matches(loaded.config.data.position_policy)
        && manifest.incomplete_label_policy()
            == loaded.config.data.incomplete_label_policy.manifest_name()
        && manifest.position_selection_audit_version == POSITION_SELECTION_AUDIT_VERSION
        && manifest.rollout_search_depth() == loaded.config.data.rollout_search_depth
        && (ignore_identity
            || manifest.self_play_move_policy
                == loaded.config.data.self_play_move_policy.manifest_name())
        && (ignore_identity
            || (manifest.sampling_phase == loaded.config.data.sampling_policy.manifest_name()
                && manifest.sample_after_opening
                    == loaded.config.data.sampling_policy.samples_after_opening()
                && manifest.teacher_move_encoding == TEACHER_MOVE_ENCODING))
        && (ignore_identity || manifest.config_hash == loaded.hash_hex)
        && manifest.entry_bytes == ENTRY_BYTES
        && manifest.shard_index == plan.shard_index)
}

fn shard_teacher_matches(
    loaded: &LoadedConfig,
    teacher: &Teacher,
    engine_revision: &Option<String>,
    manifest: &ShardManifest,
    ignore_identity: bool,
) -> bool {
    manifest.bootstrap_nnue == bootstrap_nnue_path(loaded)
        && manifest.bootstrap_nnue_sha256 == teacher.bootstrap_sha256().map(str::to_string)
        && (ignore_identity || manifest.engine_revision == *engine_revision)
        && manifest.build_mode == teacher_build_mode(loaded, teacher)
}

enum MismatchChoice {
    Abort,
    Reuse,
    Regenerate,
}

/// Decides whether resumed shards with a mismatching generation identity may be
/// reused. Returns `true` when such shards should be reused as-is.
#[allow(clippy::too_many_arguments)]
fn resolve_identity_mismatch(
    loaded: &LoadedConfig,
    artifacts: &ArtifactPaths,
    teacher: &Teacher,
    opening_sfen: &str,
    opening_source: &OpeningSource,
    opening_split: &OpeningSplit,
    engine_revision: &Option<String>,
    shard_selector: ShardSelector,
    resume: bool,
    ignore_identity_mismatch: bool,
) -> Result<bool> {
    if ignore_identity_mismatch {
        return Ok(true);
    }
    if !resume {
        return Ok(false);
    }
    let (mismatched_games, total_games) = detect_identity_mismatch(
        loaded,
        artifacts,
        teacher,
        opening_sfen,
        opening_source,
        opening_split,
        engine_revision,
        shard_selector,
    )?;
    if mismatched_games == 0 {
        return Ok(false);
    }
    let percent = if total_games == 0 {
        0.0
    } else {
        f64::from(mismatched_games) / f64::from(total_games) * 100.0
    };
    match prompt_identity_mismatch_choice(percent)? {
        MismatchChoice::Abort => bail!(
            "aborting: existing shards have a mismatching generation identity (config, engine, opening, sampling, self-play move, and/or teacher-move contract). \
             Re-run with --ignore-identity-mismatch to reuse them, or with --no-resume to regenerate."
        ),
        MismatchChoice::Reuse => Ok(true),
        MismatchChoice::Regenerate => Ok(false),
    }
}

/// Scans the shards this run would produce (our lane only) and counts how many
/// games sit in shards that would be reusable if and only if the git revision /
/// config hash checks were relaxed. Returns `(mismatched_games, total_games)`.
fn detect_identity_mismatch(
    loaded: &LoadedConfig,
    artifacts: &ArtifactPaths,
    teacher: &Teacher,
    opening_sfen: &str,
    opening_source: &OpeningSource,
    opening_split: &OpeningSplit,
    engine_revision: &Option<String>,
    shard_selector: ShardSelector,
) -> Result<(u32, u32)> {
    let mut mismatched_games = 0u32;
    let mut total_games = 0u32;
    for (dataset_name, game_count) in [
        ("train", loaded.config.data.train_games),
        ("validation", loaded.config.data.validation_games),
    ] {
        let shards_dir = artifacts.datasets_dir.join("shards").join(dataset_name);
        let plans = shard_plans(game_count, loaded.config.data.shard_games, shard_selector);
        for plan in plans {
            total_games += plan.game_count;
            let manifest_path = shards_dir.join(format!("shard-{:06}.json", plan.shard_index));
            if !manifest_path.exists() {
                continue;
            }
            let Ok(bytes) = fs::read(&manifest_path) else {
                continue;
            };
            let Ok(manifest) = serde_json::from_slice::<ShardManifest>(&bytes) else {
                continue;
            };
            // A shard is only reusable if its .bin sibling matches the manifest, same as
            // the reuse path; otherwise generate_or_reuse_shard regenerates it regardless
            // of identity, so it must not count toward a mismatch that blocks the run.
            let bin_path = shards_dir.join(format!("shard-{:06}.bin", plan.shard_index));
            if !shard_bin_matches(&bin_path, &manifest) {
                continue;
            }
            let strict =
                shard_manifest_matches(
                    loaded,
                    dataset_name,
                    opening_sfen,
                    opening_source,
                    opening_split,
                    plan,
                    &manifest,
                    false,
                )? && shard_teacher_matches(loaded, teacher, engine_revision, &manifest, false);
            let relaxed =
                shard_manifest_matches(
                    loaded,
                    dataset_name,
                    opening_sfen,
                    opening_source,
                    opening_split,
                    plan,
                    &manifest,
                    true,
                )? && shard_teacher_matches(loaded, teacher, engine_revision, &manifest, true);
            if relaxed && !strict {
                mismatched_games += plan.game_count;
            }
        }
    }
    Ok((mismatched_games, total_games))
}

fn prompt_identity_mismatch_choice(percent: f64) -> Result<MismatchChoice> {
    // Write the prompt to stderr so it stays visible even when stdout is redirected
    // to a log, and only prompt when both stdin and the prompt stream are terminals.
    let mut err = stderr();
    let _ = writeln!(
        err,
        "Generation identity mismatch found in existing shards \
         covering {percent:.1}% of this run's data."
    );
    if !stdin().is_terminal() || !err.is_terminal() {
        let _ = writeln!(
            err,
            "  not running interactively; aborting. Re-run with --ignore-identity-mismatch to reuse."
        );
        return Ok(MismatchChoice::Abort);
    }
    let _ = writeln!(err, "  1) Abort");
    let _ = writeln!(err, "  2) Resume, reusing the mismatched shards as-is");
    let _ = writeln!(
        err,
        "  3) Discard the mismatched shards and regenerate them"
    );
    let _ = write!(err, "Choice [1/2/3] (default 1): ");
    let _ = err.flush();
    let mut line = String::new();
    if stdin().read_line(&mut line)? == 0 {
        return Ok(MismatchChoice::Abort);
    }
    Ok(match line.trim() {
        "2" => MismatchChoice::Reuse,
        "3" => MismatchChoice::Regenerate,
        _ => MismatchChoice::Abort,
    })
}

fn generate_game_entries(
    dataset_name: &str,
    loaded: &LoadedConfig,
    teacher: &Teacher,
    search_workspace: &mut SearchWorkspace,
    opening_source: &OpeningSource,
    opening_split: &OpeningSplit,
    game_index: u32,
) -> Result<GameEntries> {
    let seed = game_seed(loaded.config.data.seed, dataset_name, game_index);
    let pair_seed = game_seed(loaded.config.data.seed, dataset_name, game_index / 2);
    let selected_opening =
        opening_source.select(dataset_name, opening_split, pair_seed, game_index)?;
    let mut rng = StdRng::seed_from_u64(seed);
    let sample_origin = sampling_origin(
        seed,
        loaded.config.data.sampling_policy,
        loaded.config.data.sample_start_ply,
        loaded.config.data.opening_random_plies,
        loaded.config.data.sample_every_ply,
    );
    let mut board = Board::from_sfen(&selected_opening.sfen)
        .map_err(|err| anyhow!("failed to parse opening SFEN: {err}"))?;
    let mut samples = Vec::new();
    let mut played_plies = 0u16;
    let mut stats = SearchUseStats::default();
    let label_search_budget = loaded.config.data.label_search_budget()?;

    while played_plies < loaded.config.data.max_plies {
        if !has_both_kings(&board) {
            break;
        }
        let legal_moves = collect_legal_moves(&board);
        if legal_moves.is_empty() {
            break;
        }

        let should_sample = played_plies >= sample_origin
            && (played_plies - sample_origin) % loaded.config.data.sample_every_ply == 0
            && samples.len() < usize::from(loaded.config.data.max_positions_per_game);
        let needs_rollout_search = played_plies >= loaded.config.data.opening_random_plies
            && (loaded.config.data.self_play_move_policy == SelfPlayMovePolicy::UniformRolloutV1
                || !should_sample);
        let label_summary = if should_sample {
            let summary = teacher.search_label(
                &board,
                label_search_budget,
                loaded.config.data.position_policy,
                search_workspace,
            )?;
            stats.record_label(&summary);
            apply_incomplete_label_policy(
                summary,
                label_search_budget,
                loaded.config.data.incomplete_label_policy,
                board.side_to_move(),
                &mut stats,
            )?
        } else {
            None
        };
        let rollout_summary = if needs_rollout_search {
            let summary = teacher.search_depth(
                &board,
                loaded.config.data.rollout_search_depth,
                search_workspace,
            )?;
            stats.record_rollout(&summary);
            Some(summary)
        } else {
            None
        };

        if let Some(summary) = label_summary.as_ref() {
            record_pending_sample(
                loaded.config.data.position_policy,
                &board,
                summary,
                played_plies,
                &mut samples,
                &mut stats,
            )?;
        }

        let mv = if played_plies < loaded.config.data.opening_random_plies {
            legal_moves[rng.random_range(0..legal_moves.len())]
        } else {
            let summary = select_self_play_search(
                loaded.config.data.self_play_move_policy,
                label_summary.as_ref(),
                rollout_summary.as_ref(),
            )
            .ok_or_else(|| anyhow!("self-play move search unexpectedly missing"))?;
            searched_best_move(&board, summary)?
        };

        board.play_unchecked(mv);
        played_plies += 1;
    }

    let outcome = if played_plies >= loaded.config.data.max_plies {
        GameOutcome::Draw
    } else if !board.has(Color::Black, Piece::King) {
        GameOutcome::Winner(Color::White)
    } else if !board.has(Color::White, Piece::King) {
        GameOutcome::Winner(Color::Black)
    } else {
        match board.status() {
            haitaka::GameStatus::Won => GameOutcome::Winner(!board.side_to_move()),
            haitaka::GameStatus::Drawn => GameOutcome::Draw,
            haitaka::GameStatus::Ongoing => GameOutcome::Draw,
        }
    };
    stats.position_selection.record_rejection_outcomes(outcome);

    let mut entries = Vec::with_capacity(samples.len() * ENTRY_BYTES);
    for sample in samples {
        let game_result = outcome.relative_to(sample.side_to_move);
        let packed = pack_board_for_training(&sample.board)?;
        write_training_entry(
            &mut entries,
            &packed,
            sample.score,
            0,
            sample.game_ply,
            game_result,
        )?;
    }
    Ok(GameEntries {
        entries,
        stats,
        opening: selected_opening.metadata,
    })
}

fn record_pending_sample(
    position_policy: PositionPolicy,
    root_board: &Board,
    summary: &TeacherSearchSummary,
    root_ply: u16,
    samples: &mut Vec<PendingSample>,
    stats: &mut SearchUseStats,
) -> Result<()> {
    let root_side = root_board.side_to_move();
    stats.record_candidate(root_side);
    match position_policy {
        PositionPolicy::RootPosition => {
            let score = summary
                .best_score
                .unwrap_or_else(|| terminal_teacher_score(root_board))
                .clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            stats.record_stored_position(root_ply, 0, root_side, root_side);
            samples.push(PendingSample {
                board: root_board.clone(),
                score,
                game_ply: root_ply,
                side_to_move: root_board.side_to_move(),
            });
        }
        PositionPolicy::QsearchPvLeaf => match summary.training_trace.as_ref() {
            Some(trace) if trace.terminal || !has_both_kings(&trace.leaf_board) => {
                stats.rejected_terminal_positions += 1;
                stats
                    .position_selection
                    .record_terminal(root_side, trace.leaf_board.side_to_move());
            }
            _ if summary
                .best_score
                .is_some_and(|score| score.abs() >= SEARCH_MATE_SCORE_THRESHOLD) =>
            {
                stats.rejected_mate_score_positions += 1;
                stats.position_selection.record_mate(
                    root_side,
                    summary
                        .training_trace
                        .as_ref()
                        .map(|trace| trace.leaf_board.side_to_move()),
                );
            }
            Some(trace) => {
                let score = trace.static_eval.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                stats.record_stored_position(
                    root_ply,
                    trace.root_ply_distance,
                    root_side,
                    trace.leaf_board.side_to_move(),
                );
                samples.push(PendingSample {
                    board: trace.leaf_board.clone(),
                    score,
                    game_ply: root_ply,
                    side_to_move: trace.leaf_board.side_to_move(),
                });
            }
            None => {
                bail!(
                    "traced teacher search did not produce a qsearch-PV leaf for ordinary root position `{root_board}`"
                );
            }
        },
    }
    Ok(())
}

fn select_self_play_search<'a, T>(
    policy: SelfPlayMovePolicy,
    label: Option<&'a T>,
    rollout: Option<&'a T>,
) -> Option<&'a T> {
    match policy {
        SelfPlayMovePolicy::UniformRolloutV1 => rollout,
        SelfPlayMovePolicy::LabelOnSampleLegacy => label.or(rollout),
    }
}

fn sampling_origin(
    game_seed: u64,
    policy: SamplingPolicy,
    sample_start_ply: u16,
    opening_random_plies: u16,
    sample_every_ply: u16,
) -> u16 {
    match policy {
        SamplingPolicy::PerGameRandomV1 => {
            let base = sample_start_ply.max(opening_random_plies);
            let phase = (splitmix64(game_seed ^ 0x7361_6d70_6c65_7631)
                % u64::from(sample_every_ply)) as u16;
            base.saturating_add(phase)
        }
        SamplingPolicy::FixedPhaseLegacy => sample_start_ply,
    }
}

fn searched_best_move(board: &Board, summary: &TeacherSearchSummary) -> Result<Move> {
    let best_move = summary
        .best_move
        .as_deref()
        .ok_or_else(|| anyhow!("teacher search did not return a best move"))?;
    let mv: Move = best_move
        .parse()
        .map_err(|err| anyhow!("failed to parse teacher move `{best_move}`: {err}"))?;
    if !board.is_legal(mv) {
        bail!("teacher move `{best_move}` was not legal for position `{board}`");
    }
    Ok(mv)
}

fn assemble_shards(
    shard_results: &[ShardResult],
    bin_path: &Path,
    policy: ShufflePolicy,
    shuffle_seed: u64,
    chunk_records: usize,
    dataset_name: &str,
) -> Result<u64> {
    let sampled_positions = shard_results
        .iter()
        .map(|result| result.manifest.sampled_positions)
        .sum::<u64>();
    if policy == ShufflePolicy::GameOrderLegacy {
        let mut writer = BufWriter::new(
            File::create(bin_path)
                .with_context(|| format!("failed to create {}", bin_path.display()))?,
        );
        for result in shard_results {
            let mut reader = BufReader::new(
                File::open(&result.bin_path)
                    .with_context(|| format!("failed to open {}", result.bin_path.display()))?,
            );
            std::io::copy(&mut reader, &mut writer)?;
        }
        writer.flush()?;
        return Ok(sampled_positions);
    }

    let parent = bin_path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = bin_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("dataset.bin");
    let temp_dir = parent.join(format!(".{file_name}.shuffle-tmp"));
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir)
            .with_context(|| format!("failed to clear {}", temp_dir.display()))?;
    }
    fs::create_dir_all(&temp_dir)
        .with_context(|| format!("failed to create {}", temp_dir.display()))?;

    let split_seed = shuffle_seed
        ^ match dataset_name {
            "train" => 0x7472_6169_6e2d_7631,
            "validation" => 0x7661_6c69_642d_7631,
            _ => bail!("unknown dataset split `{dataset_name}`"),
        };
    let mut records = Vec::<[u8; ENTRY_BYTES]>::with_capacity(chunk_records);
    let mut chunk_count = 0usize;
    for result in shard_results {
        let mut reader = BufReader::with_capacity(
            SHUFFLE_IO_BUFFER_BYTES,
            File::open(&result.bin_path)
                .with_context(|| format!("failed to open {}", result.bin_path.display()))?,
        );
        for _ in 0..result.manifest.sampled_positions {
            let mut record = [0u8; ENTRY_BYTES];
            reader.read_exact(&mut record)?;
            records.push(record);
            if records.len() == chunk_records {
                write_shuffle_chunk(&temp_dir, &mut records, split_seed, chunk_count)?;
                chunk_count += 1;
            }
        }
    }
    if !records.is_empty() {
        write_shuffle_chunk(&temp_dir, &mut records, split_seed, chunk_count)?;
        chunk_count += 1;
    }
    drop(records);

    let mut writer = BufWriter::with_capacity(
        SHUFFLE_IO_BUFFER_BYTES,
        File::create(bin_path)
            .with_context(|| format!("failed to create {}", bin_path.display()))?,
    );
    let (offset, step) = chunk_permutation(split_seed, chunk_count);
    for position in 0..chunk_count {
        let chunk_index = (offset + position.wrapping_mul(step)) % chunk_count;
        let chunk = temp_dir.join(format!("chunk-{chunk_index:08}.bin"));
        let mut reader = BufReader::with_capacity(
            SHUFFLE_IO_BUFFER_BYTES,
            File::open(&chunk).with_context(|| format!("failed to open {}", chunk.display()))?,
        );
        std::io::copy(&mut reader, &mut writer)?;
    }
    writer.flush()?;
    fs::remove_dir_all(&temp_dir)
        .with_context(|| format!("failed to remove {}", temp_dir.display()))?;
    Ok(sampled_positions)
}

fn write_shuffle_chunk(
    temp_dir: &Path,
    records: &mut Vec<[u8; ENTRY_BYTES]>,
    seed: u64,
    chunk_index: usize,
) -> Result<()> {
    let mut rng = StdRng::seed_from_u64(splitmix64(seed ^ chunk_index as u64));
    records.shuffle(&mut rng);
    let path = temp_dir.join(format!("chunk-{chunk_index:08}.bin"));
    let mut writer = BufWriter::with_capacity(
        SHUFFLE_IO_BUFFER_BYTES,
        File::create(&path).with_context(|| format!("failed to create {}", path.display()))?,
    );
    for record in records.iter() {
        writer.write_all(record)?;
    }
    writer.flush()?;
    records.clear();
    Ok(())
}

fn shuffle_memory_bound_bytes(chunk_records: usize) -> usize {
    chunk_records
        .saturating_mul(ENTRY_BYTES)
        .saturating_add(2 * SHUFFLE_IO_BUFFER_BYTES)
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

fn gcd(mut left: usize, mut right: usize) -> usize {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

fn merge_split(
    dataset_name: &str,
    loaded: &LoadedConfig,
    bin_path: &Path,
    manifest_path: &Path,
    input_dirs: &[PathBuf],
    game_count: u32,
    opening_sfen: &str,
    opening_source: &OpeningSource,
    opening_split: &OpeningSplit,
    generated_at_unix_ms: u128,
    ignore_identity_mismatch: bool,
) -> Result<u64> {
    let started = Instant::now();
    let mut by_start = BTreeMap::new();
    let mut teacher_identity = None;
    for input_dir in input_dirs {
        let shard_dir = input_dir.join("datasets").join("shards").join(dataset_name);
        if !shard_dir.exists() {
            continue;
        }
        let entries = fs::read_dir(&shard_dir)
            .with_context(|| format!("failed to read shard dir {}", shard_dir.display()))?;
        for entry in entries {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let manifest: ShardManifest = serde_json::from_slice(
                &fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?,
            )
            .with_context(|| format!("failed to parse {}", path.display()))?;
            validate_merge_shard(
                loaded,
                dataset_name,
                opening_sfen,
                opening_source,
                opening_split,
                &mut teacher_identity,
                &manifest,
                ignore_identity_mismatch,
            )
            .with_context(|| format!("invalid shard manifest {}", path.display()))?;
            let bin = path.with_extension("bin");
            validate_shard_bin(&bin, &manifest)?;
            let game_start = manifest.game_start;
            let result = ShardResult {
                bin_path: bin,
                manifest,
                reused: false,
            };
            if by_start.insert(game_start, result).is_some() {
                bail!("duplicate {dataset_name} shard starting at game {game_start}");
            }
        }
    }

    let mut expected_start = 0;
    let mut shard_results = Vec::new();
    for (_, result) in by_start {
        if result.manifest.game_start != expected_start {
            bail!(
                "missing {dataset_name} shard range: expected game_start {expected_start}, got {}",
                result.manifest.game_start
            );
        }
        expected_start += result.manifest.game_count;
        shard_results.push(result);
    }
    if expected_start != game_count {
        bail!("incomplete {dataset_name} shards: covered {expected_start}/{game_count} games");
    }

    let sampled_positions = assemble_shards(
        &shard_results,
        bin_path,
        loaded.config.data.shuffle_policy,
        loaded.config.data.shuffle_seed,
        loaded.config.data.shuffle_chunk_records,
        dataset_name,
    )?;
    let search_stats = shard_results
        .iter()
        .fold(SearchUseStats::default(), |mut stats, result| {
            stats.add(SearchUseStats::from(&result.manifest));
            stats
        });
    let games = shard_results
        .iter()
        .flat_map(|result| result.manifest.games.iter().cloned())
        .collect::<Vec<_>>();
    let opening_position_selection = aggregate_opening_position_selection(&shard_results);
    let opening_ids = games
        .iter()
        .map(|game| game.opening_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let elapsed = started.elapsed();
    let positions_per_second = if elapsed.as_secs_f64() > 0.0 {
        sampled_positions as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };
    let label_budget = loaded.config.data.label_search_budget()?;
    let label_search_total_nodes = search_stats
        .label_search_states
        .saturating_add(search_stats.label_search_qnodes);
    let label_nodes_per_search = if search_stats.label_searches == 0 {
        0.0
    } else {
        label_search_total_nodes as f64 / search_stats.label_searches as f64
    };
    let manifest = DatasetManifest {
        dataset: dataset_name.to_string(),
        ruleset: loaded.config.rules.ruleset,
        rule_id: loaded.effective_rule_id()?,
        opening_sfen: opening_sfen.to_string(),
        opening_policy: opening_source.policy().to_string(),
        opening_suite_id: opening_source.suite_id().map(str::to_string),
        opening_suite_sha256: opening_source.suite_sha256().map(str::to_string),
        opening_transformation: opening_source.transformation().to_string(),
        opening_ids,
        games,
        split_policy: loaded.config.data.split_policy.manifest_name().to_string(),
        split_seed: loaded.config.data.split_seed,
        train_opening_ids: opening_split.train_ids.clone(),
        validation_opening_ids: opening_split.validation_ids.clone(),
        opening_group_count: opening_split.ids_for(dataset_name)?.len(),
        opening_group_overlap: opening_split.overlap(),
        shuffle_policy: loaded
            .config
            .data
            .shuffle_policy
            .manifest_name()
            .to_string(),
        shuffle_seed: loaded.config.data.shuffle_seed,
        shuffle_chunk_records: loaded.config.data.shuffle_chunk_records,
        shuffle_memory_bound_bytes: shuffle_memory_bound_bytes(
            loaded.config.data.shuffle_chunk_records,
        ),
        game_count,
        completed_games: expected_start,
        sampled_positions,
        search_depth: label_budget.legacy_search_depth(),
        label_search_depth: label_budget.max_depth(),
        label_search_budget: label_budget.manifest_name().to_string(),
        label_search_nodes: label_budget.nodes(),
        label_search_max_depth: label_budget.max_depth(),
        node_counting_version: SEARCH_NODE_COUNTING_VERSION.to_string(),
        position_policy: loaded
            .config
            .data
            .position_policy
            .manifest_name()
            .to_string(),
        training_trace_version: SEARCH_TRAINING_TRACE_VERSION.to_string(),
        incomplete_label_policy: loaded
            .config
            .data
            .incomplete_label_policy
            .manifest_name()
            .to_string(),
        position_selection_audit_version: POSITION_SELECTION_AUDIT_VERSION.to_string(),
        candidate_positions: search_stats.candidate_positions,
        rejected_incomplete_label_positions: search_stats.rejected_incomplete_label_positions,
        rejected_terminal_positions: search_stats.rejected_terminal_positions,
        rejected_mate_score_positions: search_stats.rejected_mate_score_positions,
        position_selection: search_stats.position_selection,
        opening_position_selection,
        root_ply_min: search_stats.root_ply_min,
        root_ply_max: search_stats.root_ply_max,
        leaf_distance_min: search_stats.leaf_distance_min,
        leaf_distance_max: search_stats.leaf_distance_max,
        leaf_distance_mean: leaf_distance_mean(&search_stats),
        rollout_search_depth: loaded.config.data.rollout_search_depth,
        self_play_move_policy: loaded
            .config
            .data
            .self_play_move_policy
            .manifest_name()
            .to_string(),
        label_searches: search_stats.label_searches,
        rollout_searches: search_stats.rollout_searches,
        label_search_states: search_stats.label_search_states,
        label_search_qnodes: search_stats.label_search_qnodes,
        label_search_total_nodes,
        label_nodes_per_search,
        rollout_search_states: search_stats.rollout_search_states,
        rollout_search_qnodes: search_stats.rollout_search_qnodes,
        label_search_cpu_seconds: search_stats.label_search_elapsed_seconds,
        rollout_search_cpu_seconds: search_stats.rollout_search_elapsed_seconds,
        generation_cpu_seconds: search_stats.label_search_elapsed_seconds
            + search_stats.rollout_search_elapsed_seconds,
        bootstrap_nnue: teacher_identity
            .as_ref()
            .and_then(|identity| identity.bootstrap_nnue.clone()),
        bootstrap_nnue_sha256: teacher_identity
            .as_ref()
            .and_then(|identity| identity.bootstrap_nnue_sha256.clone()),
        engine_revision: teacher_identity
            .as_ref()
            .and_then(|identity| identity.engine_revision.clone()),
        config_hash: loaded.hash_hex.clone(),
        seed: loaded.config.data.seed,
        feature_family: loaded.training_features().to_string(),
        sampling_phase: loaded
            .config
            .data
            .sampling_policy
            .manifest_name()
            .to_string(),
        sample_after_opening: loaded.config.data.sampling_policy.samples_after_opening(),
        teacher_move_encoding: TEACHER_MOVE_ENCODING.to_string(),
        opening_random_plies: loaded.config.data.opening_random_plies,
        generated_at_unix_ms,
        build_mode: format!("{}+merged", loaded.runtime_mode()),
        entry_bytes: ENTRY_BYTES,
        shard_count: shard_results.len(),
        jobs: 1,
        resumed_shards: shard_results.len(),
        generated_shards: 0,
        elapsed_seconds: elapsed.as_secs_f64(),
        positions_per_second,
    };
    fs::write(manifest_path, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;

    println!(
        "merged {dataset_name} games={expected_start}/{game_count} positions={sampled_positions} shards={} elapsed={}",
        shard_results.len(),
        format_duration(elapsed)
    );

    Ok(sampled_positions)
}

fn validate_merge_shard(
    loaded: &LoadedConfig,
    dataset_name: &str,
    opening_sfen: &str,
    opening_source: &OpeningSource,
    opening_split: &OpeningSplit,
    teacher_identity: &mut Option<MergeTeacherIdentity>,
    manifest: &ShardManifest,
    ignore_identity_mismatch: bool,
) -> Result<()> {
    let label_budget = loaded.config.data.label_search_budget()?;
    ensure_merge(
        manifest.dataset == dataset_name,
        "dataset name does not match",
    )?;
    ensure_merge(
        manifest.ruleset == loaded.config.rules.ruleset,
        "ruleset does not match",
    )?;
    ensure_merge(
        manifest.rule_id == loaded.effective_rule_id()?,
        "rule_id does not match",
    )?;
    ensure_merge(
        manifest.opening_sfen == opening_sfen,
        "opening_sfen does not match",
    )?;
    if !ignore_identity_mismatch {
        ensure_merge(
            manifest.opening_policy == opening_source.policy(),
            "opening_policy does not match",
        )?;
        ensure_merge(
            manifest.opening_suite_id.as_deref() == opening_source.suite_id(),
            "opening_suite_id does not match",
        )?;
        ensure_merge(
            manifest.opening_suite_sha256.as_deref() == opening_source.suite_sha256(),
            "opening_suite_sha256 does not match",
        )?;
        ensure_merge(
            manifest.opening_transformation == opening_source.transformation(),
            "opening_transformation does not match",
        )?;
        ensure_merge(
            manifest.split_policy == loaded.config.data.split_policy.manifest_name(),
            "split_policy does not match",
        )?;
        ensure_merge(
            manifest.split_seed == loaded.config.data.split_seed,
            "split_seed does not match",
        )?;
        ensure_merge(
            manifest.train_opening_ids == opening_split.train_ids
                && manifest.validation_opening_ids == opening_split.validation_ids,
            "opening split groups do not match",
        )?;
        ensure_merge(
            manifest.shuffle_policy == loaded.config.data.shuffle_policy.manifest_name(),
            "shuffle_policy does not match",
        )?;
        ensure_merge(
            manifest.shuffle_seed == loaded.config.data.shuffle_seed,
            "shuffle_seed does not match",
        )?;
        ensure_merge(
            manifest.shuffle_chunk_records == loaded.config.data.shuffle_chunk_records,
            "shuffle_chunk_records does not match",
        )?;
    }
    ensure_merge(
        manifest.search_depth == label_budget.legacy_search_depth(),
        "search_depth does not match",
    )?;
    ensure_merge(
        manifest.label_search_depth() == label_budget.max_depth(),
        "label_search_depth does not match",
    )?;
    ensure_merge(
        manifest.label_search_budget() == label_budget.manifest_name(),
        "label_search_budget does not match",
    )?;
    ensure_merge(
        manifest.label_search_nodes == label_budget.nodes(),
        "label_search_nodes does not match",
    )?;
    ensure_merge(
        manifest.label_search_max_depth() == label_budget.max_depth(),
        "label_search_max_depth does not match",
    )?;
    ensure_merge(
        manifest.node_counting_version_matches(label_budget),
        "node_counting_version does not match",
    )?;
    ensure_merge(
        manifest.position_policy() == loaded.config.data.position_policy.manifest_name(),
        "position_policy does not match",
    )?;
    ensure_merge(
        manifest.training_trace_version_matches(loaded.config.data.position_policy),
        "training_trace_version does not match",
    )?;
    ensure_merge(
        manifest.incomplete_label_policy()
            == loaded.config.data.incomplete_label_policy.manifest_name(),
        "incomplete_label_policy does not match",
    )?;
    ensure_merge(
        manifest.position_selection_audit_version == POSITION_SELECTION_AUDIT_VERSION,
        "position_selection_audit_version does not match",
    )?;
    ensure_merge(
        manifest.rollout_search_depth() == loaded.config.data.rollout_search_depth,
        "rollout_search_depth does not match",
    )?;
    if !ignore_identity_mismatch {
        ensure_merge(
            manifest.self_play_move_policy
                == loaded.config.data.self_play_move_policy.manifest_name(),
            "self_play_move_policy does not match",
        )?;
    }
    if !ignore_identity_mismatch {
        ensure_merge(
            manifest.sampling_phase == loaded.config.data.sampling_policy.manifest_name(),
            "sampling_phase does not match",
        )?;
        ensure_merge(
            manifest.sample_after_opening
                == loaded.config.data.sampling_policy.samples_after_opening(),
            "sample_after_opening does not match",
        )?;
        ensure_merge(
            manifest.teacher_move_encoding == TEACHER_MOVE_ENCODING,
            "teacher_move_encoding does not match",
        )?;
    }
    if !ignore_identity_mismatch {
        ensure_merge(
            manifest.config_hash == loaded.hash_hex,
            "config_hash does not match. If you're sure to continue merging using mismatching identity, rerun with --ignore-identity-mismatch flag",
        )?;
    }
    validate_merge_teacher_identity(teacher_identity, manifest, ignore_identity_mismatch)?;
    ensure_merge(
        manifest.entry_bytes == ENTRY_BYTES,
        "entry_bytes does not match",
    )?;
    Ok(())
}

fn validate_merge_teacher_identity(
    expected: &mut Option<MergeTeacherIdentity>,
    manifest: &ShardManifest,
    ignore_identity_mismatch: bool,
) -> Result<()> {
    let current = MergeTeacherIdentity::from_manifest(manifest);
    if let Some(expected) = expected.as_ref() {
        ensure_merge(
            current.bootstrap_nnue_sha256 == expected.bootstrap_nnue_sha256,
            "bootstrap_nnue_sha256 does not match",
        )?;
        if !ignore_identity_mismatch {
            ensure_merge(
                current.engine_revision == expected.engine_revision,
                "engine_revision does not match. If you're sure to continue merging using mismatching identity, rerun with --ignore-identity-mismatch flag",
            )?;
        }
        ensure_merge(
            current.build_mode == expected.build_mode,
            "build_mode does not match",
        )?;
    } else {
        *expected = Some(current);
    }
    Ok(())
}

fn ensure_merge(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        bail!("{message}")
    }
}

fn validate_shard_bin(bin_path: &Path, manifest: &ShardManifest) -> Result<()> {
    let expected_len = manifest
        .sampled_positions
        .checked_mul(ENTRY_BYTES as u64)
        .ok_or_else(|| anyhow!("shard length overflow"))?;
    let actual_len = fs::metadata(bin_path)
        .with_context(|| format!("failed to stat {}", bin_path.display()))?
        .len();
    if actual_len != expected_len {
        bail!(
            "shard {} has {} bytes, expected {}",
            bin_path.display(),
            actual_len,
            expected_len
        );
    }
    Ok(())
}

fn bootstrap_nnue_path(loaded: &LoadedConfig) -> Option<String> {
    loaded
        .bootstrap_nnue()
        .map(|path| path.display().to_string())
}

fn teacher_build_mode(loaded: &LoadedConfig, teacher: &Teacher) -> String {
    format!("{}+teacher:{}", loaded.runtime_mode(), teacher.describe())
}

fn hash_bytes_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn game_seed(base_seed: u64, dataset_name: &str, game_index: u32) -> u64 {
    let split_key = match dataset_name {
        "train" => 0x7472_6169_6e00_0000,
        "validation" => 0x7661_6c69_6400_0000,
        _ => 0x6461_7461_0000_0000,
    };
    splitmix64(base_seed ^ split_key ^ u64::from(game_index))
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

fn terminal_teacher_score(board: &Board) -> i32 {
    match board.status() {
        haitaka::GameStatus::Won => -30_000,
        haitaka::GameStatus::Drawn => 0,
        haitaka::GameStatus::Ongoing => 0,
    }
}

fn collect_legal_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|piece_moves| {
        moves.extend(piece_moves);
        false
    });
    moves
}

fn has_both_kings(board: &Board) -> bool {
    board.has(Color::Black, Piece::King) && board.has(Color::White, Piece::King)
}

fn detect_git_revision(loaded: &LoadedConfig) -> Result<Option<String>> {
    let Some(repo_root) = find_haitaka_workspace_root(&loaded.path) else {
        return Ok(None);
    };
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok();
    Ok(output
        .filter(|result| result.status.success())
        .map(|result| String::from_utf8_lossy(&result.stdout).trim().to_string()))
}

fn find_haitaka_workspace_root(config_path: &Path) -> Option<&Path> {
    config_path
        .parent()
        .into_iter()
        .flat_map(Path::ancestors)
        .find(|candidate| {
            candidate.join("Cargo.toml").is_file() && candidate.join("haitaka_learn").is_dir()
        })
}

fn write_training_entry(
    writer: &mut impl Write,
    packed_sfen: &[u8; PACKED_SFEN_BYTES],
    score: i16,
    teacher_move: u16,
    game_ply: u16,
    game_result: i8,
) -> Result<()> {
    writer.write_all(packed_sfen)?;
    writer.write_all(&score.to_le_bytes())?;
    writer.write_all(&teacher_move.to_le_bytes())?;
    writer.write_all(&game_ply.to_le_bytes())?;
    writer.write_all(&[game_result as u8])?;
    writer.write_all(&[0])?;
    Ok(())
}

fn pack_board_for_training(board: &Board) -> Result<[u8; PACKED_SFEN_BYTES]> {
    let mut writer = BitWriter::default();
    let trainer_side_to_move = invert_color(board.side_to_move());
    writer.write_one_bit(matches!(trainer_side_to_move, TrainerColor::Black));

    let trainer_white_king = trainer_square_index(board.king(Color::Black));
    let trainer_black_king = trainer_square_index(board.king(Color::White));
    writer.write_n_bits(trainer_white_king as u32, 7);
    writer.write_n_bits(trainer_black_king as u32, 7);

    let mut trainer_board = [None; 81];
    for square_index in 0..Square::NUM {
        let square = Square::index_const(square_index);
        if let Some(colored) = board.colored_piece_on(square) {
            let trainer_square = trainer_square_index(square);
            trainer_board[trainer_square] = Some(TrainerPiece {
                color: invert_color(colored.color),
                piece_type: trainer_piece_type(colored.piece),
            });
        }
    }

    for rank in (0..9).rev() {
        for file in 0..9 {
            let square_index = rank * 9 + file;
            if square_index == trainer_white_king || square_index == trainer_black_king {
                continue;
            }

            match trainer_board[square_index] {
                None => writer.write_huffman_empty(),
                Some(piece) => writer.write_board_piece(piece),
            }
        }
    }

    let hand_counts = trainer_hand_counts(board);
    for trainer_color in [TrainerColor::White, TrainerColor::Black] {
        for piece_type in 0..10 {
            writer.write_n_bits(hand_counts[trainer_color as usize][piece_type] as u32, 5);
        }
    }

    for _ in 0..4 {
        writer.write_one_bit(false);
    }
    writer.write_one_bit(false);

    let fullmove = board.move_number();
    writer.write_n_bits(0, 6);
    writer.write_n_bits(u32::from(fullmove & 0xff), 8);
    writer.write_n_bits(u32::from(fullmove >> 8), 8);
    writer.write_one_bit(false);

    Ok(writer.finish())
}

fn trainer_hand_counts(board: &Board) -> [[u8; 10]; 2] {
    let mut counts = [[0u8; 10]; 2];
    for (color, trainer_color) in [
        (Color::Black, TrainerColor::White),
        (Color::White, TrainerColor::Black),
    ] {
        for piece in [
            Piece::Pawn,
            Piece::Lance,
            Piece::Knight,
            Piece::Silver,
            Piece::Bishop,
            Piece::Rook,
            Piece::Gold,
        ] {
            counts[trainer_color as usize][trainer_piece_type(piece)] =
                board.num_in_hand(color, piece);
        }
    }
    counts
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrainerPiece {
    color: TrainerColor,
    piece_type: usize,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrainerColor {
    White = 0,
    Black = 1,
}

fn invert_color(color: Color) -> TrainerColor {
    match color {
        Color::Black => TrainerColor::White,
        Color::White => TrainerColor::Black,
    }
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

fn trainer_square_index(square: Square) -> usize {
    let file = 8usize - square.file() as usize;
    let rank = 8usize - square.rank() as usize;
    file + rank * 9
}

#[derive(Debug)]
struct BitWriter {
    data: [u8; PACKED_SFEN_BYTES],
    bit_cursor: usize,
}

impl Default for BitWriter {
    fn default() -> Self {
        Self {
            data: [0; PACKED_SFEN_BYTES],
            bit_cursor: 0,
        }
    }
}

impl BitWriter {
    fn write_one_bit(&mut self, bit: bool) {
        if bit {
            self.data[self.bit_cursor / 8] |= 1 << (self.bit_cursor % 8);
        }
        self.bit_cursor += 1;
    }

    fn write_n_bits(&mut self, value: u32, bits: usize) {
        for shift in 0..bits {
            self.write_one_bit(((value >> shift) & 1) != 0);
        }
    }

    fn write_huffman_empty(&mut self) {
        self.write_one_bit(false);
    }

    fn write_board_piece(&mut self, piece: TrainerPiece) {
        let code = 1u32 + 2u32 * (piece.piece_type as u32);
        self.write_n_bits(code, 5);
        self.write_one_bit(matches!(piece.color, TrainerColor::Black));
    }

    fn finish(self) -> [u8; PACKED_SFEN_BYTES] {
        self.data
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::config::LoadedConfig;
    use tempfile::tempdir;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FeatureSignature {
        side_to_move: TrainerColor,
        white_king: usize,
        black_king: usize,
        board: [Option<TrainerPiece>; 81],
        hands: [[u8; 10]; 2],
        fullmove: u16,
    }

    #[test]
    fn packed_entry_size_matches_trainer_layout() {
        assert_eq!(ENTRY_BYTES, 72);
    }

    #[test]
    fn per_game_sampling_phase_covers_both_parities_and_starts_after_opening() {
        let origins = (0..100)
            .map(|game_index| {
                sampling_origin(
                    game_seed(75, "train", game_index),
                    SamplingPolicy::PerGameRandomV1,
                    8,
                    16,
                    2,
                )
            })
            .collect::<Vec<_>>();
        assert!(origins.iter().all(|&ply| ply >= 16));
        assert!(origins.iter().any(|ply| ply % 2 == 0));
        assert!(origins.iter().any(|ply| ply % 2 == 1));

        let repeated = (0..100)
            .map(|game_index| {
                sampling_origin(
                    game_seed(75, "train", game_index),
                    SamplingPolicy::PerGameRandomV1,
                    8,
                    16,
                    2,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(origins, repeated);
    }

    #[test]
    fn uniform_rollout_never_uses_the_label_search_for_self_play() {
        let label = "depth-3-label";
        let rollout = "depth-1-rollout";

        assert_eq!(
            select_self_play_search(
                SelfPlayMovePolicy::UniformRolloutV1,
                Some(&label),
                Some(&rollout),
            ),
            Some(&rollout)
        );
        assert_eq!(
            select_self_play_search(
                SelfPlayMovePolicy::LabelOnSampleLegacy,
                Some(&label),
                Some(&rollout),
            ),
            Some(&label)
        );
    }

    #[test]
    fn legacy_sampling_requires_the_explicit_fixed_phase_policy() {
        assert_eq!(
            sampling_origin(123, SamplingPolicy::FixedPhaseLegacy, 8, 16, 2),
            8
        );
    }

    #[test]
    fn packer_preserves_feature_signature() {
        let board = Board::from_sfen(
            "lnsgkgsnl/1r5b1/pppp1pppp/4p4/4+P4/9/PPPP1PPPP/1B5R1/LNSGKGSNL b - 3",
        )
        .unwrap();
        let packed = pack_board_for_training(&board).unwrap();
        let decoded = decode_signature(&packed);

        assert_eq!(decoded, signature_for_board(&board));
    }

    #[test]
    fn trainer_packing_maps_pair_donor_slots_to_overlay_order() {
        let square = Square::E5;
        assert_eq!(
            packed_delta(square, square.try_offset(1, 0).unwrap()),
            (-1, 0)
        );
        assert_eq!(
            packed_delta(square, square.try_offset(-1, 0).unwrap()),
            (1, 0)
        );
    }

    #[test]
    fn trainer_packing_maps_knight8_runtime_offsets_to_overlay_order() {
        let square = Square::E5;
        let relative_offsets = [
            (1, 2),
            (-1, 2),
            (-2, 1),
            (-2, -1),
            (-1, -2),
            (1, -2),
            (2, -1),
            (2, 1),
        ];

        for color in [Color::Black, Color::White] {
            for (left, forward) in relative_offsets {
                let runtime_donor = runtime_relative_square(color, square, left, forward).unwrap();
                let expected_overlay_delta = overlay_relative_delta(color, left, forward);
                assert_eq!(
                    packed_delta(square, runtime_donor),
                    expected_overlay_delta,
                    "color={color:?}, left={left}, forward={forward}"
                );
            }
        }
    }

    #[test]
    fn graceful_stop_prevents_starting_new_shards() {
        let queue = Arc::new(Mutex::new(VecDeque::from([ShardPlan {
            shard_index: 0,
            game_start: 0,
            game_count: 1,
        }])));

        assert!(next_shard_plan_with(&queue, || true).is_none());
        assert_eq!(queue.lock().unwrap().len(), 1);
    }

    #[test]
    fn shard_lanes_are_contiguous_division_ranges() {
        let fourth_lane = shard_plan_indices(16, 2, Some(3), None, Some(4));
        let seventh_lane = shard_plan_indices(16, 2, Some(6), None, Some(8));
        let eighth_lane = shard_plan_indices(16, 2, Some(7), None, Some(8));

        assert_eq!(fourth_lane, [6, 7]);
        assert_eq!(seventh_lane, [6]);
        assert_eq!(eighth_lane, [7]);
    }

    #[test]
    fn shard_lanes_cover_uneven_division_without_overlap() {
        let mut all_indices = Vec::new();
        for index in 0..3 {
            all_indices.extend(shard_plan_indices(10, 1, Some(index), None, Some(3)));
        }

        assert_eq!(all_indices, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn shard_lane_ranges_cover_multiple_contiguous_lanes() {
        let combined = shard_plan_indices(16, 2, Some(2), Some(4), Some(8));
        let mut separate = shard_plan_indices(16, 2, Some(2), None, Some(8));
        separate.extend(shard_plan_indices(16, 2, Some(3), None, Some(8)));
        separate.extend(shard_plan_indices(16, 2, Some(4), None, Some(8)));

        assert_eq!(combined, separate);
        assert_eq!(combined, [2, 3, 4]);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn generate_data_smoke_test_writes_non_empty_shards() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("haitaka_learn.toml");
        let ruleset = if cfg!(feature = "annan") {
            "annan"
        } else if cfg!(feature = "anhoku") {
            "anhoku"
        } else if cfg!(feature = "antouzai") {
            "antouzai"
        } else if cfg!(feature = "taimen") {
            "taimen"
        } else if cfg!(feature = "haimen") {
            "haimen"
        } else if cfg!(feature = "neko") {
            "neko"
        } else if cfg!(feature = "nekoneko") {
            "nekoneko"
        } else if cfg!(feature = "yokoneko") {
            "yokoneko"
        } else if cfg!(feature = "yokonekoneko") {
            "yokonekoneko"
        } else if cfg!(feature = "tenkyo") {
            "tenkyo"
        } else if cfg!(feature = "tenjiku") {
            "tenjiku"
        } else if cfg!(feature = "anki") {
            "anki"
        } else {
            "standard"
        };
        fs::write(
            &config_path,
            format!(
                r#"
[rules]
ruleset = "{ruleset}"

[paths]
output_dir = "out"

[data]
train_games = 1
validation_games = 1
max_plies = 8
search_depth = 1
opening_random_plies = 2
sample_start_ply = 0
sample_every_ply = 1
max_positions_per_game = 4
seed = 7

[verify]
run_search_smoke = false
"#,
            ),
        )
        .unwrap();

        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        let output = generate_data(&loaded).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);

        let artifacts = loaded.artifact_paths();
        assert!(artifacts.train_bin.exists());
        assert!(artifacts.validation_bin.exists());
        assert!(fs::metadata(&artifacts.train_bin).unwrap().len() > 0);
        assert!(fs::metadata(&artifacts.validation_bin).unwrap().len() > 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn generation_is_deterministic_across_job_counts() {
        let temp = tempdir().unwrap();
        let ruleset = active_test_ruleset();
        let config_one = temp.path().join("one.toml");
        let config_two = temp.path().join("two.toml");
        fs::write(&config_one, deterministic_test_config(ruleset, "out-one")).unwrap();
        fs::write(&config_two, deterministic_test_config(ruleset, "out-two")).unwrap();

        let one = LoadedConfig::from_path(&config_one).unwrap();
        let two = LoadedConfig::from_path(&config_two).unwrap();
        generate_data_with_options(
            &one,
            GenerateOptions {
                jobs: Some(1),
                resume: Some(false),
                shard_index: None,
                shard_index_end: None,
                shard_count: None,
                ignore_identity_mismatch: false,
            },
        )
        .unwrap();
        generate_data_with_options(
            &two,
            GenerateOptions {
                jobs: Some(2),
                resume: Some(false),
                shard_index: None,
                shard_index_end: None,
                shard_count: None,
                ignore_identity_mismatch: false,
            },
        )
        .unwrap();

        assert_eq!(
            fs::read(one.artifact_paths().train_bin).unwrap(),
            fs::read(two.artifact_paths().train_bin).unwrap()
        );
        assert_eq!(
            fs::read(one.artifact_paths().validation_bin).unwrap(),
            fs::read(two.artifact_paths().validation_bin).unwrap()
        );
    }

    #[test]
    #[cfg(feature = "anhoku")]
    fn suite_generation_records_deterministic_color_swapped_pair_metadata() {
        let temp = tempdir().unwrap();
        let suite = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("openings")
            .join("anhoku-v1.tsv");
        let config_path = temp.path().join("suite.toml");
        fs::write(
            &config_path,
            suite_test_config("out", &suite.display().to_string()),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        generate_data(&loaded).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(loaded.artifact_paths().train_manifest).unwrap())
                .unwrap();
        assert_eq!(manifest["opening_policy"], "suite");
        assert_eq!(manifest["opening_suite_id"], "anhoku-v1");
        assert_eq!(
            manifest["opening_transformation"],
            "anhoku-rotate180-color-swap-v1"
        );
        assert_eq!(manifest["games"].as_array().unwrap().len(), 4);
        for pair in manifest["games"].as_array().unwrap().chunks_exact(2) {
            assert_eq!(pair[0]["opening_id"], pair[1]["opening_id"]);
            assert_eq!(pair[0]["color"], "base");
            assert_eq!(pair[1]["color"], "swapped");
            assert_ne!(pair[0]["sfen"], pair[1]["sfen"]);
        }
    }

    #[test]
    #[cfg(feature = "anhoku")]
    fn grouped_split_and_bounded_shuffle_are_disjoint_and_deterministic() {
        let temp = tempdir().unwrap();
        let suite = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("openings")
            .join("anhoku-v1.tsv");
        let first_config = temp.path().join("first.toml");
        let second_config = temp.path().join("second.toml");
        fs::write(
            &first_config,
            suite_test_config("out-first", &suite.display().to_string()),
        )
        .unwrap();
        fs::write(
            &second_config,
            suite_test_config("out-second", &suite.display().to_string()),
        )
        .unwrap();
        let first = LoadedConfig::from_path(&first_config).unwrap();
        let second = LoadedConfig::from_path(&second_config).unwrap();
        generate_data(&first).unwrap();
        generate_data(&second).unwrap();

        let first_artifacts = first.artifact_paths();
        let second_artifacts = second.artifact_paths();
        assert_eq!(
            fs::read(&first_artifacts.train_bin).unwrap(),
            fs::read(&second_artifacts.train_bin).unwrap()
        );
        assert_eq!(
            fs::read(&first_artifacts.validation_bin).unwrap(),
            fs::read(&second_artifacts.validation_bin).unwrap()
        );

        let train: serde_json::Value =
            serde_json::from_slice(&fs::read(&first_artifacts.train_manifest).unwrap()).unwrap();
        let validation: serde_json::Value =
            serde_json::from_slice(&fs::read(&first_artifacts.validation_manifest).unwrap())
                .unwrap();
        let train_openings = train["opening_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap())
            .collect::<BTreeSet<_>>();
        let validation_openings = validation["opening_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert!(train_openings.is_disjoint(&validation_openings));
        let train_games = train["games"]
            .as_array()
            .unwrap()
            .iter()
            .map(|game| game["game_id"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        let validation_games = validation["games"]
            .as_array()
            .unwrap()
            .iter()
            .map(|game| game["game_id"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert!(train_games.is_disjoint(&validation_games));
        assert_eq!(train["opening_group_overlap"].as_array().unwrap().len(), 0);
        assert_eq!(
            train["shuffle_memory_bound_bytes"],
            shuffle_memory_bound_bytes(2)
        );
        let audit = serde_json::to_value(
            crate::dataset_audit::audit_dataset(
                &first_artifacts.train_bin,
                &first_artifacts.train_manifest,
                None,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(audit["groups"]["opening_group_overlap_count"], 0);
        assert_eq!(audit["groups"]["unique_game_ids"], 4);

        let raw = [0_u32, 1]
            .into_iter()
            .flat_map(|index| {
                fs::read(
                    first_artifacts
                        .datasets_dir
                        .join("shards/train")
                        .join(format!("shard-{index:06}.bin")),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        assert_ne!(fs::read(&first_artifacts.train_bin).unwrap(), raw);
    }

    #[test]
    #[cfg(feature = "anhoku")]
    fn resume_and_merge_reject_split_and_shuffle_identity_mismatches() {
        let temp = tempdir().unwrap();
        let suite = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("openings")
            .join("anhoku-v1.tsv");
        let config_path = temp.path().join("phase3.toml");
        fs::write(
            &config_path,
            suite_test_config("out", &suite.display().to_string()),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        generate_data(&loaded).unwrap();

        mutate_first_shard_manifest(&loaded, "train", |manifest| {
            manifest["split_seed"] = serde_json::json!(999);
        });
        let error = format!("{:#}", generate_data(&loaded).unwrap_err());
        assert!(error.contains("--ignore-identity-mismatch"));

        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();
        mutate_first_shard_manifest_in_dir(&input, "train", |manifest| {
            manifest["split_seed"] = serde_json::json!(76);
            manifest["shuffle_seed"] = serde_json::json!(999);
        });
        let error = format!("{:#}", merge_data(&loaded, &[input], false).unwrap_err());
        assert!(error.contains("shuffle_seed does not match"));
    }

    #[test]
    #[cfg(feature = "anhoku")]
    fn suite_content_change_invalidates_resume_and_merge_identity() {
        let temp = tempdir().unwrap();
        let source_suite = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("openings")
            .join("anhoku-v1.tsv");
        let suite = temp.path().join("suite.tsv");
        fs::copy(source_suite, &suite).unwrap();
        let config_path = temp.path().join("suite.toml");
        fs::write(
            &config_path,
            suite_test_config("out", &suite.display().to_string()),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        generate_data(&loaded).unwrap();

        let mut changed = fs::read_to_string(&suite).unwrap();
        changed.push_str("# identity change\n");
        fs::write(&suite, changed).unwrap();
        let changed_loaded = LoadedConfig::from_path(&config_path).unwrap();
        let error = format!("{:#}", generate_data(&changed_loaded).unwrap_err());
        assert!(error.contains("--ignore-identity-mismatch"));

        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();
        let error = format!(
            "{:#}",
            merge_data(&changed_loaded, &[input], false).unwrap_err()
        );
        assert!(error.contains("opening_suite_sha256 does not match"));
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn resume_reuses_completed_shards() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("resume.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        generate_data(&loaded).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(loaded.artifact_paths().train_manifest).unwrap())
                .unwrap();
        assert!(manifest["resumed_shards"].as_u64().unwrap() > 0);
        assert_eq!(manifest["generated_shards"].as_u64().unwrap(), 0);
    }

    #[test]
    #[cfg(feature = "anhoku")]
    fn resume_rejects_sampling_self_play_and_teacher_move_mismatches_without_override() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("resume-contract.toml");
        fs::write(&config_path, deterministic_test_config("anhoku", "out")).unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        generate_data(&loaded).unwrap();

        mutate_first_shard_manifest(&loaded, "train", |manifest| {
            manifest["sampling_phase"] =
                serde_json::Value::String("fixed-phase-legacy".to_string());
        });
        let error = format!("{:#}", generate_data(&loaded).unwrap_err());
        assert!(error.contains("--ignore-identity-mismatch"));

        mutate_first_shard_manifest(&loaded, "train", |manifest| {
            manifest["sampling_phase"] =
                serde_json::Value::String("per-game-random-v1".to_string());
            manifest["teacher_move_encoding"] =
                serde_json::Value::String(TEACHER_MOVE_ENCODING.to_string());
            manifest["self_play_move_policy"] =
                serde_json::Value::String("uniform-rollout-v1".to_string());
        });
        let error = format!("{:#}", generate_data(&loaded).unwrap_err());
        assert!(error.contains("--ignore-identity-mismatch"));

        mutate_first_shard_manifest(&loaded, "train", |manifest| {
            manifest["sampling_phase"] =
                serde_json::Value::String("per-game-random-v1".to_string());
            manifest["teacher_move_encoding"] =
                serde_json::Value::String("legacy-ambiguous-u16".to_string());
        });
        let error = format!("{:#}", generate_data(&loaded).unwrap_err());
        assert!(error.contains("--ignore-identity-mismatch"));
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn resume_regenerates_shards_when_rollout_depth_changes() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("resume-rollout-depth.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        mutate_first_shard_manifest(&loaded, "train", |manifest| {
            manifest["rollout_search_depth"] = serde_json::Value::from(2);
        });
        generate_data(&loaded).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(loaded.artifact_paths().train_manifest).unwrap())
                .unwrap();
        assert!(manifest["generated_shards"].as_u64().unwrap() > 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn resume_regenerates_fixed_node_shards_when_budget_identity_changes() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("resume-fixed-node-identity.toml");
        let config = fixed_node_counter_test_config(active_test_ruleset(), "out")
            .replace("resume = false", "resume = true");
        fs::write(&config_path, config).unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let mismatches = [
            (
                "label_search_budget",
                serde_json::Value::String("depth".into()),
            ),
            ("label_search_nodes", serde_json::Value::from(4_999)),
            ("label_search_max_depth", serde_json::Value::from(63)),
            (
                "node_counting_version",
                serde_json::Value::String("different-node-contract".into()),
            ),
            (
                "incomplete_label_policy",
                serde_json::Value::String("reject-position".into()),
            ),
        ];
        for (field, value) in mismatches {
            mutate_first_shard_manifest(&loaded, "train", |manifest| {
                manifest[field] = value;
            });
            generate_data(&loaded).unwrap();
            let manifest: serde_json::Value = serde_json::from_slice(
                &fs::read(loaded.artifact_paths().train_manifest.clone()).unwrap(),
            )
            .unwrap();
            assert!(
                manifest["generated_shards"].as_u64().unwrap() > 0,
                "{field} mismatch should regenerate its shard"
            );
        }
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn resume_regenerates_qsearch_leaf_shards_when_trace_identity_changes() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("resume-qsearch-leaf-identity.toml");
        let config = qsearch_leaf_test_config(active_test_ruleset(), "out")
            .replace("resume = false", "resume = true");
        fs::write(&config_path, config).unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let mismatches = [
            (
                "position_policy",
                serde_json::Value::String("root-position".into()),
            ),
            (
                "training_trace_version",
                serde_json::Value::String("different-trace-contract".into()),
            ),
        ];
        for (field, value) in mismatches {
            mutate_first_shard_manifest(&loaded, "train", |manifest| {
                manifest[field] = value;
            });
            generate_data(&loaded).unwrap();
            let manifest: serde_json::Value = serde_json::from_slice(
                &fs::read(loaded.artifact_paths().train_manifest.clone()).unwrap(),
            )
            .unwrap();
            assert!(
                manifest["generated_shards"].as_u64().unwrap() > 0,
                "{field} mismatch should regenerate its shard"
            );
        }
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn resume_reuses_legacy_shards_without_explicit_search_depths() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("resume-legacy-depths.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        mutate_first_shard_manifest(&loaded, "train", remove_explicit_search_depths);
        generate_data(&loaded).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(loaded.artifact_paths().train_manifest).unwrap())
                .unwrap();
        assert!(manifest["resumed_shards"].as_u64().unwrap() > 0);
        assert_eq!(manifest["generated_shards"].as_u64().unwrap(), 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn resume_regenerates_shards_when_teacher_identity_changes() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("resume-teacher.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        mutate_first_shard_manifest(&loaded, "train", |manifest| {
            manifest["build_mode"] = serde_json::Value::String("stale-teacher".to_string());
        });
        generate_data(&loaded).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(loaded.artifact_paths().train_manifest).unwrap())
                .unwrap();
        assert!(manifest["generated_shards"].as_u64().unwrap() > 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn resume_reuses_mismatched_shards_when_identity_ignored() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("resume-ignore-identity.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        mutate_first_shard_manifest(&loaded, "train", |manifest| {
            manifest["config_hash"] = serde_json::Value::String("stale-config-hash".to_string());
            manifest["engine_revision"] = serde_json::Value::String("other-revision".to_string());
        });
        generate_data_with_options(
            &loaded,
            GenerateOptions {
                jobs: Some(1),
                resume: Some(true),
                shard_index: None,
                shard_index_end: None,
                shard_count: None,
                ignore_identity_mismatch: true,
            },
        )
        .unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(loaded.artifact_paths().train_manifest).unwrap())
                .unwrap();
        assert!(manifest["resumed_shards"].as_u64().unwrap() > 0);
        assert_eq!(manifest["generated_shards"].as_u64().unwrap(), 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn detect_identity_mismatch_counts_mismatched_games() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("detect-identity.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let artifacts = loaded.artifact_paths();
        let teacher = Teacher::from_config(&loaded).unwrap();
        let opening_sfen = loaded.opening_sfen().unwrap();
        let opening_source = OpeningSource::from_config(&loaded, &opening_sfen).unwrap();
        let opening_split = opening_source
            .split_openings(
                loaded.config.data.split_policy,
                loaded.config.data.split_seed,
                loaded.config.data.train_games,
                loaded.config.data.validation_games,
            )
            .unwrap();
        let engine_revision = detect_git_revision(&loaded).unwrap();
        let selector = ShardSelector::new(None, None, None).unwrap();

        let (before, total) = detect_identity_mismatch(
            &loaded,
            &artifacts,
            &teacher,
            &opening_sfen,
            &opening_source,
            &opening_split,
            &engine_revision,
            selector,
        )
        .unwrap();
        assert_eq!(before, 0);
        assert!(total > 0);

        mutate_first_shard_manifest(&loaded, "train", |manifest| {
            manifest["config_hash"] = serde_json::Value::String("stale-config-hash".to_string());
        });
        let (after, _) = detect_identity_mismatch(
            &loaded,
            &artifacts,
            &teacher,
            &opening_sfen,
            &opening_source,
            &opening_split,
            &engine_revision,
            selector,
        )
        .unwrap();
        assert!(after > 0);

        // A mismatched manifest whose .bin is missing/wrong-length is not reusable, so it
        // must not count toward a mismatch that would block a non-interactive resumed run.
        let bin_path = artifacts
            .datasets_dir
            .join("shards")
            .join("train")
            .join("shard-000000.bin");
        fs::write(&bin_path, b"truncated").unwrap();
        let (after_bad_bin, _) = detect_identity_mismatch(
            &loaded,
            &artifacts,
            &teacher,
            &opening_sfen,
            &opening_source,
            &opening_split,
            &engine_revision,
            selector,
        )
        .unwrap();
        assert_eq!(after_bad_bin, 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn merge_data_combines_distributed_shards() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data_with_options(
            &loaded,
            GenerateOptions {
                jobs: Some(1),
                resume: Some(false),
                shard_index: Some(0),
                shard_index_end: None,
                shard_count: Some(2),
                ignore_identity_mismatch: false,
            },
        )
        .unwrap();
        let first_input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &first_input).unwrap();

        generate_data_with_options(
            &loaded,
            GenerateOptions {
                jobs: Some(1),
                resume: Some(false),
                shard_index: Some(1),
                shard_index_end: None,
                shard_count: Some(2),
                ignore_identity_mismatch: false,
            },
        )
        .unwrap();
        let second_input = temp.path().join("machine-b");
        fs::rename(loaded.artifact_paths().output_dir, &second_input).unwrap();

        let output = merge_data(&loaded, &[first_input, second_input], false).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
        assert!(loaded.artifact_paths().train_bin.exists());
        assert!(loaded.artifact_paths().validation_bin.exists());
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn merge_treats_empty_shard_lanes_as_empty_inputs() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-empty-lane.toml");
        fs::write(
            &config_path,
            distributed_empty_lane_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data_with_options(
            &loaded,
            GenerateOptions {
                jobs: Some(1),
                resume: Some(false),
                shard_index: Some(0),
                shard_index_end: None,
                shard_count: Some(2),
                ignore_identity_mismatch: false,
            },
        )
        .unwrap();
        let first_input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &first_input).unwrap();

        generate_data_with_options(
            &loaded,
            GenerateOptions {
                jobs: Some(1),
                resume: Some(false),
                shard_index: Some(1),
                shard_index_end: None,
                shard_count: Some(2),
                ignore_identity_mismatch: false,
            },
        )
        .unwrap();
        let second_input = temp.path().join("machine-b");
        fs::rename(loaded.artifact_paths().output_dir, &second_input).unwrap();

        assert!(
            !first_input
                .join("datasets")
                .join("shards")
                .join("validation")
                .exists()
        );

        let output = merge_data(&loaded, &[first_input, second_input], false).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn merge_rejects_shards_with_mismatched_teacher_identity() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-teacher.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();
        mutate_first_shard_manifest_in_dir(&input, "train", |manifest| {
            manifest["engine_revision"] = serde_json::Value::String("other-revision".to_string());
        });

        let err = format!("{:?}", merge_data(&loaded, &[input], false).unwrap_err());
        assert!(err.contains("engine_revision does not match"));
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn merge_rejects_shards_with_mismatched_rollout_depth() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-rollout-depth.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();
        mutate_first_shard_manifest_in_dir(&input, "train", |manifest| {
            manifest["rollout_search_depth"] = serde_json::Value::from(2);
        });

        let err = format!("{:?}", merge_data(&loaded, &[input], false).unwrap_err());
        assert!(err.contains("rollout_search_depth does not match"));
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn merge_rejects_fixed_node_counting_version_mismatch() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-fixed-node-version.toml");
        fs::write(
            &config_path,
            fixed_node_counter_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();
        mutate_first_shard_manifest_in_dir(&input, "train", |manifest| {
            manifest["node_counting_version"] =
                serde_json::Value::String("different-node-contract".to_string());
        });

        let err = format!("{:#}", merge_data(&loaded, &[input], false).unwrap_err());
        assert!(err.contains("node_counting_version does not match"));
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn merge_rejects_qsearch_leaf_trace_version_mismatch() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-qsearch-leaf-version.toml");
        fs::write(
            &config_path,
            qsearch_leaf_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();
        mutate_first_shard_manifest_in_dir(&input, "train", |manifest| {
            manifest["training_trace_version"] =
                serde_json::Value::String("different-trace-contract".to_string());
        });

        let err = format!("{:#}", merge_data(&loaded, &[input], false).unwrap_err());
        assert!(err.contains("training_trace_version does not match"));
    }

    #[test]
    #[cfg(feature = "anhoku")]
    fn merge_rejects_sampling_self_play_and_teacher_move_mismatches() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-contract.toml");
        fs::write(&config_path, deterministic_test_config("anhoku", "out")).unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        generate_data(&loaded).unwrap();
        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();

        mutate_first_shard_manifest_in_dir(&input, "train", |manifest| {
            manifest["sampling_phase"] =
                serde_json::Value::String("fixed-phase-legacy".to_string());
        });
        let error = format!(
            "{:#}",
            merge_data(&loaded, &[input.clone()], false).unwrap_err()
        );
        assert!(error.contains("sampling_phase does not match"));

        mutate_first_shard_manifest_in_dir(&input, "train", |manifest| {
            manifest["sampling_phase"] =
                serde_json::Value::String("per-game-random-v1".to_string());
            manifest["teacher_move_encoding"] =
                serde_json::Value::String("legacy-ambiguous-u16".to_string());
        });
        let error = format!(
            "{:#}",
            merge_data(&loaded, &[input.clone()], false).unwrap_err()
        );
        assert!(error.contains("teacher_move_encoding does not match"));

        mutate_first_shard_manifest_in_dir(&input, "train", |manifest| {
            manifest["teacher_move_encoding"] =
                serde_json::Value::String(TEACHER_MOVE_ENCODING.to_string());
            manifest["self_play_move_policy"] =
                serde_json::Value::String("uniform-rollout-v1".to_string());
        });
        let error = format!("{:#}", merge_data(&loaded, &[input], false).unwrap_err());
        assert!(error.contains("self_play_move_policy does not match"));
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn merge_accepts_legacy_shards_without_explicit_search_depths() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-legacy-depths.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();
        mutate_first_shard_manifest_in_dir(&input, "train", remove_explicit_search_depths);
        mutate_first_shard_manifest_in_dir(&input, "validation", remove_explicit_search_depths);

        let output = merge_data(&loaded, &[input], false).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn merge_ignores_identity_mismatch_with_flag() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-ignore-identity.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();
        mutate_first_shard_manifest_in_dir(&input, "train", |manifest| {
            manifest["config_hash"] = serde_json::Value::String("stale-config-hash".to_string());
        });

        let err = format!(
            "{:?}",
            merge_data(&loaded, &[input.clone()], false).unwrap_err()
        );
        assert!(err.contains("config_hash does not match"));
        assert!(err.contains("--ignore-identity-mismatch"));

        let output = merge_data(&loaded, &[input], true).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn merge_allows_shards_with_different_bootstrap_paths() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-bootstrap-path.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();
        for dataset_name in ["train", "validation"] {
            mutate_first_shard_manifest_in_dir(&input, dataset_name, |manifest| {
                manifest["bootstrap_nnue"] =
                    serde_json::Value::String("/different/machine/bootstrap.nnue".to_string());
            });
        }

        let output = merge_data(&loaded, &[input], false).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn merge_does_not_require_local_bootstrap_file() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("merge-missing-bootstrap.toml");
        fs::write(
            &config_path,
            distributed_empty_lane_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let input = temp.path().join("machine-a");
        fs::rename(loaded.artifact_paths().output_dir, &input).unwrap();
        for dataset_name in ["train", "validation"] {
            mutate_first_shard_manifest_in_dir(&input, dataset_name, |manifest| {
                manifest["bootstrap_nnue"] =
                    serde_json::Value::String("/worker/bootstrap.nnue".to_string());
                manifest["bootstrap_nnue_sha256"] =
                    serde_json::Value::String("same-bootstrap-hash".to_string());
                manifest["build_mode"] =
                    serde_json::Value::String(format!("{}+teacher:nnue", active_test_ruleset()));
            });
        }

        let output = merge_data(&loaded, &[input], false).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    #[cfg(not(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        feature = "neko",
        feature = "nekoneko",
        feature = "yokoneko",
        feature = "yokonekoneko",
        feature = "tenkyo",
        feature = "tenjiku",
        feature = "anki"
    )))]
    fn handicap_generate_data_smoke_test_writes_non_empty_shards() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("haitaka_learn.toml");
        fs::write(
            &config_path,
            r#"
[rules]
ruleset = "handicap"
handicap = "six-piece"

[paths]
output_dir = "out"

[data]
train_games = 1
validation_games = 1
max_plies = 8
search_depth = 1
opening_random_plies = 2
sample_start_ply = 0
sample_every_ply = 1
max_positions_per_game = 4
seed = 9
"#,
        )
        .unwrap();

        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        let output = generate_data(&loaded).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    fn rollout_search_depth_defaults_to_one() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("rollout-default.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();

        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        assert_eq!(loaded.config.data.rollout_search_depth, 1);
    }

    #[test]
    fn rollout_search_depth_must_be_positive() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("rollout-invalid.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out").replace(
                "search_depth = 1",
                "search_depth = 1\nrollout_search_depth = 0",
            ),
        )
        .unwrap();

        let err = format!("{:?}", LoadedConfig::from_path(&config_path).unwrap_err());

        assert!(err.contains("data.rollout_search_depth must be at least 1"));
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn split_search_counters_track_label_and_rollout_searches() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("rollout-counters.toml");
        fs::write(
            &config_path,
            rollout_counter_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(loaded.artifact_paths().train_manifest).unwrap())
                .unwrap();
        assert_eq!(manifest["search_depth"].as_u64().unwrap(), 2);
        assert_eq!(manifest["label_search_depth"].as_u64().unwrap(), 2);
        assert_eq!(manifest["rollout_search_depth"].as_u64().unwrap(), 1);
        assert_eq!(manifest["self_play_move_policy"], "uniform-rollout-v1");
        assert_eq!(manifest["label_searches"].as_u64().unwrap(), 3);
        assert_eq!(manifest["rollout_searches"].as_u64().unwrap(), 6);
        assert!(manifest["label_search_states"].as_u64().unwrap() > 0);
        assert!(manifest["rollout_search_states"].as_u64().unwrap() > 0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn fixed_node_labels_use_exact_budgets_and_keep_rollouts_depth_limited() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("fixed-node-counters.toml");
        fs::write(
            &config_path,
            fixed_node_counter_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(loaded.artifact_paths().train_manifest).unwrap())
                .unwrap();
        let label_searches = manifest["label_searches"].as_u64().unwrap();
        let label_states = manifest["label_search_states"].as_u64().unwrap();
        let label_qnodes = manifest["label_search_qnodes"].as_u64().unwrap();
        let label_total = manifest["label_search_total_nodes"].as_u64().unwrap();

        assert_eq!(manifest["search_depth"].as_u64().unwrap(), 0);
        assert_eq!(manifest["label_search_budget"], "nodes");
        assert_eq!(manifest["label_search_nodes"].as_u64().unwrap(), 5_000);
        assert_eq!(manifest["label_search_max_depth"].as_u64().unwrap(), 64);
        assert_eq!(
            manifest["node_counting_version"],
            SEARCH_NODE_COUNTING_VERSION
        );
        assert_eq!(manifest["rollout_search_depth"].as_u64().unwrap(), 1);
        assert_eq!(label_total, label_states + label_qnodes);
        assert_eq!(label_total, label_searches * 5_000);
        assert_eq!(
            manifest["label_nodes_per_search"].as_f64().unwrap(),
            5_000.0
        );
        assert!(manifest["rollout_searches"].as_u64().unwrap() > 0);
        assert!(manifest["rollout_search_states"].as_u64().unwrap() > 0);
        assert!(manifest["generation_cpu_seconds"].as_f64().unwrap() >= 0.0);
    }

    #[test]
    #[cfg(any(
        feature = "annan",
        feature = "anhoku",
        feature = "antouzai",
        feature = "taimen",
        feature = "haimen",
        not(any(
            feature = "annan",
            feature = "anhoku",
            feature = "antouzai",
            feature = "taimen",
            feature = "haimen",
            feature = "neko",
            feature = "nekoneko",
            feature = "yokoneko",
            feature = "yokonekoneko",
            feature = "tenkyo",
            feature = "tenjiku",
            feature = "anki"
        ))
    ))]
    fn qsearch_leaf_generation_records_policy_distances_and_rejections() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("qsearch-leaf.toml");
        fs::write(
            &config_path,
            qsearch_leaf_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(loaded.artifact_paths().train_manifest).unwrap())
                .unwrap();
        let stored = manifest["sampled_positions"].as_u64().unwrap();
        let candidates = manifest["candidate_positions"].as_u64().unwrap();
        let terminal = manifest["rejected_terminal_positions"].as_u64().unwrap();
        let mate = manifest["rejected_mate_score_positions"].as_u64().unwrap();
        let incomplete = manifest["rejected_incomplete_label_positions"]
            .as_u64()
            .unwrap();

        assert_eq!(manifest["position_policy"], "qsearch-pv-leaf");
        assert_eq!(
            manifest["training_trace_version"],
            SEARCH_TRAINING_TRACE_VERSION
        );
        assert!(stored > 0);
        assert_eq!(candidates, stored + incomplete + terminal + mate);
        assert!(manifest["leaf_distance_min"].as_u64().unwrap() >= 1);
        assert!(manifest["leaf_distance_max"].as_u64().unwrap() >= 1);
        assert!(manifest["leaf_distance_mean"].as_f64().unwrap() >= 1.0);
        assert!(manifest["root_ply_min"].is_number());
        assert!(manifest["root_ply_max"].is_number());
        assert_eq!(
            manifest["position_selection_audit_version"],
            POSITION_SELECTION_AUDIT_VERSION
        );
        let selection = &manifest["position_selection"];
        assert_eq!(
            selection["candidate_root_black"].as_u64().unwrap()
                + selection["candidate_root_white"].as_u64().unwrap(),
            candidates
        );
        assert_eq!(
            selection["stored_leaf_distance_even"].as_u64().unwrap()
                + selection["stored_leaf_distance_odd"].as_u64().unwrap(),
            stored
        );
        assert!(
            manifest["opening_position_selection"]
                .as_object()
                .is_some_and(|openings| !openings.is_empty())
        );
    }

    #[test]
    fn qsearch_leaf_samples_orient_results_to_leaf_and_count_rejections() {
        let root = Board::startpos();
        let mut leaf = root.clone();
        let mv = collect_legal_moves(&leaf)[0];
        leaf.play_unchecked(mv);
        assert_ne!(root.side_to_move(), leaf.side_to_move());

        let traced = |best_score, terminal| TeacherSearchSummary {
            best_move: Some(mv.to_string()),
            best_score: Some(best_score),
            states: 1,
            qnodes: 1,
            elapsed_seconds: 0.0,
            training_trace: Some(SearchTrainingTrace {
                leaf_board: leaf.clone(),
                static_eval: 123,
                root_ply_distance: 1,
                terminal,
            }),
        };
        let mut samples = Vec::new();
        let mut stats = SearchUseStats::default();

        record_pending_sample(
            PositionPolicy::QsearchPvLeaf,
            &root,
            &traced(123, false),
            8,
            &mut samples,
            &mut stats,
        )
        .unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].side_to_move, leaf.side_to_move());
        assert_eq!(samples[0].score, 123);
        assert_eq!(samples[0].game_ply, 8);
        assert_eq!(
            GameOutcome::Winner(root.side_to_move()).relative_to(samples[0].side_to_move),
            -1
        );
        let pack_sample = |sample: &PendingSample| {
            let mut bytes = Vec::new();
            let packed = pack_board_for_training(&sample.board).unwrap();
            write_training_entry(
                &mut bytes,
                &packed,
                sample.score,
                0,
                sample.game_ply,
                GameOutcome::Winner(root.side_to_move()).relative_to(sample.side_to_move),
            )
            .unwrap();
            bytes
        };
        assert_eq!(pack_sample(&samples[0]), pack_sample(&samples[0]));

        record_pending_sample(
            PositionPolicy::QsearchPvLeaf,
            &root,
            &traced(123, true),
            10,
            &mut samples,
            &mut stats,
        )
        .unwrap();
        record_pending_sample(
            PositionPolicy::QsearchPvLeaf,
            &root,
            &traced(SEARCH_MATE_SCORE_THRESHOLD, false),
            12,
            &mut samples,
            &mut stats,
        )
        .unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(stats.candidate_positions, 3);
        assert_eq!(stats.rejected_terminal_positions, 1);
        assert_eq!(stats.rejected_mate_score_positions, 1);
        assert_eq!(stats.position_selection.candidate_root_black, 3);
        assert_eq!(stats.position_selection.stored_root_black_leaf_white, 1);
        assert_eq!(stats.position_selection.stored_leaf_distance_odd, 1);
        assert_eq!(stats.position_selection.rejected_terminal_root_black, 1);
        assert_eq!(stats.position_selection.rejected_terminal_leaf_white, 1);
        assert_eq!(stats.position_selection.rejected_mate_root_black, 1);
        assert_eq!(stats.position_selection.rejected_mate_leaf_white, 1);
        stats
            .position_selection
            .record_rejection_outcomes(GameOutcome::Winner(root.side_to_move()));
        assert_eq!(stats.position_selection.rejected_terminal_game_win, 1);
        assert_eq!(stats.position_selection.rejected_mate_game_win, 1);
    }

    #[test]
    fn incomplete_fixed_node_labels_are_rejected_only_by_explicit_policy() {
        let incomplete = TeacherSearchSummary {
            best_move: None,
            best_score: None,
            states: 1_000,
            qnodes: 4_000,
            elapsed_seconds: 0.25,
            training_trace: None,
        };
        let budget = LabelSearchBudget::Nodes {
            nodes: 5_000,
            max_depth: 64,
        };

        let mut rejected_stats = SearchUseStats::default();
        let accepted = apply_incomplete_label_policy(
            incomplete.clone(),
            budget,
            IncompleteLabelPolicy::RejectPosition,
            Color::White,
            &mut rejected_stats,
        )
        .unwrap();
        assert!(accepted.is_none());
        assert_eq!(rejected_stats.candidate_positions, 1);
        assert_eq!(rejected_stats.rejected_incomplete_label_positions, 1);
        assert_eq!(
            rejected_stats
                .position_selection
                .rejected_incomplete_root_white,
            1
        );

        let mut strict_stats = SearchUseStats::default();
        let error = apply_incomplete_label_policy(
            incomplete,
            budget,
            IncompleteLabelPolicy::Error,
            Color::Black,
            &mut strict_stats,
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("did not complete depth 1 within 5000 nodes"));
        assert_eq!(strict_stats.candidate_positions, 1);
        assert_eq!(strict_stats.rejected_incomplete_label_positions, 0);
    }

    #[test]
    fn finds_workspace_root_from_root_config_path() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir.parent().unwrap();
        let config_path = workspace_root.join("haitaka_learn.toml");

        let detected = find_haitaka_workspace_root(&config_path).unwrap();

        assert_eq!(detected, workspace_root);
    }

    fn active_test_ruleset() -> &'static str {
        if cfg!(feature = "annan") {
            "annan"
        } else if cfg!(feature = "anhoku") {
            "anhoku"
        } else if cfg!(feature = "antouzai") {
            "antouzai"
        } else if cfg!(feature = "taimen") {
            "taimen"
        } else if cfg!(feature = "haimen") {
            "haimen"
        } else if cfg!(feature = "neko") {
            "neko"
        } else if cfg!(feature = "nekoneko") {
            "nekoneko"
        } else if cfg!(feature = "yokoneko") {
            "yokoneko"
        } else if cfg!(feature = "yokonekoneko") {
            "yokonekoneko"
        } else if cfg!(feature = "tenkyo") {
            "tenkyo"
        } else if cfg!(feature = "tenjiku") {
            "tenjiku"
        } else if cfg!(feature = "anki") {
            "anki"
        } else {
            "standard"
        }
    }

    fn deterministic_test_config(ruleset: &str, output_dir: &str) -> String {
        format!(
            r#"
[rules]
ruleset = "{ruleset}"

[paths]
output_dir = "{output_dir}"

[data]
train_games = 4
validation_games = 2
max_plies = 8
search_depth = 1
opening_random_plies = 2
sample_start_ply = 0
sample_every_ply = 1
max_positions_per_game = 4
seed = 7
jobs = 1
shard_games = 1
progress_every_percent = 50
resume = true

[verify]
run_search_smoke = false
"#,
        )
    }

    #[cfg(feature = "anhoku")]
    fn suite_test_config(output_dir: &str, suite: &str) -> String {
        format!(
            r#"
[rules]
ruleset = "anhoku"

[paths]
output_dir = "{output_dir}"

[data]
train_games = 4
validation_games = 2
max_plies = 4
search_depth = 1
opening_random_plies = 0
opening_policy = "suite"
opening_suite = "{suite}"
opening_suite_id = "anhoku-v1"
self_play_move_policy = "uniform-rollout-v1"
split_policy = "opening-group-hash-v1"
split_seed = 76
shuffle_policy = "chunk-v1"
shuffle_seed = 77
shuffle_chunk_records = 2
sample_start_ply = 0
sample_every_ply = 1
max_positions_per_game = 2
seed = 75
jobs = 1
shard_games = 2
progress_every_percent = 50
resume = true

[verify]
run_search_smoke = false
"#,
        )
    }

    fn distributed_empty_lane_test_config(ruleset: &str, output_dir: &str) -> String {
        format!(
            r#"
[rules]
ruleset = "{ruleset}"

[paths]
output_dir = "{output_dir}"

[data]
train_games = 2
validation_games = 1
max_plies = 8
search_depth = 1
opening_random_plies = 2
sample_start_ply = 0
sample_every_ply = 1
max_positions_per_game = 4
seed = 7
jobs = 1
shard_games = 100
progress_every_percent = 50
resume = true

[verify]
run_search_smoke = false
"#,
        )
    }

    fn rollout_counter_test_config(ruleset: &str, output_dir: &str) -> String {
        format!(
            r#"
[rules]
ruleset = "{ruleset}"

[paths]
output_dir = "{output_dir}"

[data]
train_games = 1
validation_games = 1
max_plies = 6
search_depth = 2
rollout_search_depth = 1
self_play_move_policy = "uniform-rollout-v1"
opening_random_plies = 0
sample_start_ply = 0
sample_every_ply = 2
max_positions_per_game = 8
seed = 7
jobs = 1
shard_games = 1
progress_every_percent = 50
resume = false

[verify]
run_search_smoke = false
"#,
        )
    }

    fn fixed_node_counter_test_config(ruleset: &str, output_dir: &str) -> String {
        format!(
            r#"
[rules]
ruleset = "{ruleset}"

[paths]
output_dir = "{output_dir}"

[data]
train_games = 1
validation_games = 1
max_plies = 6
label_search_nodes = 5000
label_search_max_depth = 64
rollout_search_depth = 1
self_play_move_policy = "uniform-rollout-v1"
opening_random_plies = 0
sample_start_ply = 0
sample_every_ply = 2
max_positions_per_game = 8
seed = 7
jobs = 1
shard_games = 1
progress_every_percent = 50
resume = false

[verify]
run_search_smoke = false
"#,
        )
    }

    fn qsearch_leaf_test_config(ruleset: &str, output_dir: &str) -> String {
        format!(
            r#"
[rules]
ruleset = "{ruleset}"

[paths]
output_dir = "{output_dir}"

[data]
train_games = 1
validation_games = 1
max_plies = 6
search_depth = 1
position_policy = "qsearch-pv-leaf"
rollout_search_depth = 1
self_play_move_policy = "uniform-rollout-v1"
opening_random_plies = 0
sample_start_ply = 0
sample_every_ply = 2
max_positions_per_game = 8
seed = 7
jobs = 1
shard_games = 1
progress_every_percent = 50
resume = false

[verify]
run_search_smoke = false
"#,
        )
    }

    fn mutate_first_shard_manifest(
        loaded: &LoadedConfig,
        dataset_name: &str,
        mutate: impl FnOnce(&mut serde_json::Value),
    ) {
        mutate_first_shard_manifest_in_dir(
            &loaded.artifact_paths().output_dir,
            dataset_name,
            mutate,
        )
    }

    fn mutate_first_shard_manifest_in_dir(
        output_dir: &Path,
        dataset_name: &str,
        mutate: impl FnOnce(&mut serde_json::Value),
    ) {
        let shard_path = output_dir
            .join("datasets")
            .join("shards")
            .join(dataset_name)
            .join("shard-000000.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&shard_path).unwrap()).unwrap();
        mutate(&mut manifest);
        fs::write(&shard_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    }

    fn shard_plan_indices(
        game_count: u32,
        shard_games: u32,
        shard_index: Option<u32>,
        shard_index_end: Option<u32>,
        shard_count: Option<u32>,
    ) -> Vec<u32> {
        let selector = ShardSelector::new(shard_index, shard_index_end, shard_count).unwrap();
        shard_plans(game_count, shard_games, selector)
            .into_iter()
            .map(|plan| plan.shard_index)
            .collect()
    }

    fn remove_explicit_search_depths(manifest: &mut serde_json::Value) {
        let object = manifest.as_object_mut().unwrap();
        object.remove("label_search_depth");
        object.remove("label_search_budget");
        object.remove("label_search_nodes");
        object.remove("label_search_max_depth");
        object.remove("node_counting_version");
        object.remove("position_policy");
        object.remove("training_trace_version");
        object.remove("incomplete_label_policy");
        object.remove("candidate_positions");
        object.remove("rejected_incomplete_label_positions");
        object.remove("rejected_terminal_positions");
        object.remove("rejected_mate_score_positions");
        object.remove("position_selection");
        object.remove("opening_position_selection");
        object.remove("root_ply_min");
        object.remove("root_ply_max");
        object.remove("leaf_distance_min");
        object.remove("leaf_distance_max");
        object.remove("leaf_distance_total");
        object.remove("rollout_search_depth");
        object.remove("label_search_qnodes");
        object.remove("rollout_search_qnodes");
        object.remove("label_search_cpu_seconds");
        object.remove("rollout_search_cpu_seconds");
    }

    #[test]
    #[cfg(feature = "annan")]
    fn annan_nnue_teacher_handles_live_check_positions_without_sfen_roundtrip() {
        let bootstrap = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("../shogi-878ca61334a7.nnue");
        if !bootstrap.exists() {
            return;
        }

        let temp = tempdir().unwrap();
        let config_path = temp.path().join("haitaka_learn.toml");
        fs::write(
            &config_path,
            format!(
                r#"
[rules]
ruleset = "annan"
rule_id = 26
opening_sfen = "8k/6G2/7B1/9/9/9/9/9/K8 b R 1"

[paths]
output_dir = "out"
bootstrap_nnue = "{}"

[data]
train_games = 1
validation_games = 1
max_plies = 2
search_depth = 1
opening_random_plies = 0
sample_start_ply = 0
sample_every_ply = 1
max_positions_per_game = 2
seed = 5

[verify]
run_search_smoke = false
"#,
                bootstrap.display()
            ),
        )
        .unwrap();

        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        let output = generate_data(&loaded).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    #[test]
    #[cfg(feature = "annan")]
    fn annan_nnue_teacher_handles_king_capture_lines() {
        let bootstrap = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("../shogi-878ca61334a7.nnue");
        if !bootstrap.exists() {
            return;
        }

        let temp = tempdir().unwrap();
        let config_path = temp.path().join("haitaka_learn.toml");
        fs::write(
            &config_path,
            format!(
                r#"
[rules]
ruleset = "annan"
rule_id = 26
opening_sfen = "4R4/9/k8/9/9/4r4/4p4/9/4K4 b - 1"

[paths]
output_dir = "out"
bootstrap_nnue = "{}"

[data]
train_games = 1
validation_games = 1
max_plies = 4
search_depth = 2
opening_random_plies = 0
sample_start_ply = 0
sample_every_ply = 1
max_positions_per_game = 2
seed = 11

[verify]
run_search_smoke = false
"#,
                bootstrap.display()
            ),
        )
        .unwrap();

        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        let output = generate_data(&loaded).unwrap();
        assert!(output.train_positions > 0);
        assert!(output.validation_positions > 0);
    }

    fn signature_for_board(board: &Board) -> FeatureSignature {
        let mut trainer_board = [None; 81];
        for square_index in 0..Square::NUM {
            let square = Square::index_const(square_index);
            if let Some(colored) = board.colored_piece_on(square) {
                if colored.piece == Piece::King {
                    continue;
                }
                trainer_board[trainer_square_index(square)] = Some(TrainerPiece {
                    color: invert_color(colored.color),
                    piece_type: trainer_piece_type(colored.piece),
                });
            }
        }
        FeatureSignature {
            side_to_move: invert_color(board.side_to_move()),
            white_king: trainer_square_index(board.king(Color::Black)),
            black_king: trainer_square_index(board.king(Color::White)),
            board: trainer_board,
            hands: trainer_hand_counts(board),
            fullmove: board.move_number(),
        }
    }

    fn runtime_relative_square(
        color: Color,
        square: Square,
        left: i8,
        forward: i8,
    ) -> Option<Square> {
        let file_delta = match color {
            Color::Black => left,
            Color::White => -left,
        };
        let rank_delta = match color {
            Color::Black => -forward,
            Color::White => forward,
        };
        square.try_offset(file_delta, rank_delta)
    }

    fn overlay_relative_delta(color: Color, left: i8, forward: i8) -> (i8, i8) {
        match color {
            Color::Black => (-left, forward),
            Color::White => (left, -forward),
        }
    }

    fn packed_delta(origin: Square, target: Square) -> (i8, i8) {
        let origin = trainer_square_index(origin);
        let target = trainer_square_index(target);
        (
            (target % 9) as i8 - (origin % 9) as i8,
            (target / 9) as i8 - (origin / 9) as i8,
        )
    }

    fn decode_signature(packed: &[u8; PACKED_SFEN_BYTES]) -> FeatureSignature {
        let mut reader = BitReader::new(packed);
        let side_to_move = if reader.read_one_bit() {
            TrainerColor::Black
        } else {
            TrainerColor::White
        };
        let white_king = reader.read_n_bits(7) as usize;
        let black_king = reader.read_n_bits(7) as usize;
        let mut board = [None; 81];
        for rank in (0..9).rev() {
            for file in 0..9 {
                let square_index = rank * 9 + file;
                if square_index == white_king || square_index == black_king {
                    continue;
                }
                board[square_index] = reader.read_board_piece();
            }
        }

        let mut hands = [[0u8; 10]; 2];
        for color in 0..2 {
            for piece_type in 0..10 {
                hands[color][piece_type] = reader.read_n_bits(5) as u8;
            }
        }
        for _ in 0..4 {
            let _ = reader.read_one_bit();
        }
        let has_ep = reader.read_one_bit();
        assert!(!has_ep);
        let _rule50_low = reader.read_n_bits(6);
        let fullmove_low = reader.read_n_bits(8);
        let fullmove_high = reader.read_n_bits(8);
        let _rule50_high = reader.read_one_bit();
        FeatureSignature {
            side_to_move,
            white_king,
            black_king,
            board,
            hands,
            fullmove: ((fullmove_high << 8) | fullmove_low) as u16,
        }
    }

    struct BitReader<'a> {
        bytes: &'a [u8; PACKED_SFEN_BYTES],
        bit_cursor: usize,
    }

    impl<'a> BitReader<'a> {
        fn new(bytes: &'a [u8; PACKED_SFEN_BYTES]) -> Self {
            Self {
                bytes,
                bit_cursor: 0,
            }
        }

        fn read_one_bit(&mut self) -> bool {
            let bit = ((self.bytes[self.bit_cursor / 8] >> (self.bit_cursor % 8)) & 1) != 0;
            self.bit_cursor += 1;
            bit
        }

        fn read_n_bits(&mut self, bits: usize) -> u32 {
            let mut value = 0u32;
            for shift in 0..bits {
                if self.read_one_bit() {
                    value |= 1 << shift;
                }
            }
            value
        }

        fn read_board_piece(&mut self) -> Option<TrainerPiece> {
            if !self.read_one_bit() {
                return None;
            }

            let mut code = 1u32;
            for shift in 1..5 {
                if self.read_one_bit() {
                    code |= 1 << shift;
                }
            }
            let piece_type = ((code - 1) / 2) as usize;
            let color = if self.read_one_bit() {
                TrainerColor::Black
            } else {
                TrainerColor::White
            };
            Some(TrainerPiece { color, piece_type })
        }
    }
}

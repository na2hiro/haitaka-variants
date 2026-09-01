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

use anyhow::{Context, Result, anyhow, bail, ensure};
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
    ArtifactPaths, EvaluatorConfig, EvaluatorKind, IncompleteLabelPolicy, LabelRetryPolicy,
    LabelSearchBudget, LoadedGenerationConfig, LoadedGenerationConfig as LoadedConfig,
    PositionPolicy, Ruleset, SamplingPolicy, SelfPlayMovePolicy, ShufflePolicy,
    TEACHER_MOVE_ENCODING,
};
use crate::openings::{GameOpeningMetadata, OpeningSource, OpeningSplit, color_swap_anhoku_sfen};

pub(crate) const PACKED_SFEN_BYTES: usize = 64;
pub(crate) const ENTRY_BYTES: usize = PACKED_SFEN_BYTES + 8;
const SHUFFLE_IO_BUFFER_BYTES: usize = 64 * 1024;
const POSITION_SELECTION_AUDIT_VERSION: &str = "side-parity-opening-result-ply-v2";
const CANDIDATE_IDENTITY_VERSION: &str = "sample-root-sha256-v1";
const GENERATION_SEMANTIC_IDENTITY_VERSION: &str = "generation-semantic-v2";
const SCHEDULE_IDENTITY_VERSION: &str = "schedule-readiness-v1";
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
    #[cfg(test)]
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
    validation_opening_schedule: String,
    validation_opening_pairs_per_id: Option<u32>,
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
    label_retry_policy: String,
    max_label_attempts_per_game: Option<u16>,
    position_selection_audit_version: String,
    candidate_positions: u64,
    candidate_roots_per_game: Option<u16>,
    candidate_identity_version: String,
    candidate_identity_sha256: String,
    generation_semantic_identity_version: String,
    generation_semantic_identity_sha256: String,
    schedule_identity_version: String,
    schedule_identity_sha256: String,
    minimum_train_boards: Option<u64>,
    minimum_train_positions: Option<u64>,
    rejected_incomplete_label_positions: u64,
    rejected_terminal_positions: u64,
    rejected_mate_score_positions: u64,
    rejected_node_accounting_positions: u64,
    label_retry_exhausted_games: u64,
    label_retry_attempts_per_accepted_position: f64,
    rejected_incomplete_root_plies: BTreeMap<u16, u64>,
    rejected_terminal_root_plies: BTreeMap<u16, u64>,
    rejected_mate_root_plies: BTreeMap<u16, u64>,
    rejected_node_accounting_root_plies: BTreeMap<u16, u64>,
    position_selection: PositionSelectionStats,
    opening_position_selection: BTreeMap<String, PositionSelectionStats>,
    root_ply_min: Option<u16>,
    root_ply_max: Option<u16>,
    leaf_distance_min: Option<u16>,
    leaf_distance_max: Option<u16>,
    leaf_distance_mean: f64,
    rollout_search_depth: u8,
    self_play_move_policy: String,
    rollout_candidate_limit: u16,
    rollout_score_margin: i32,
    rollout_temperature: f64,
    rollout_rng_version: String,
    label_searches: u64,
    rollout_searches: u64,
    label_search_states: u64,
    label_search_qnodes: u64,
    label_search_total_nodes: u64,
    label_nodes_per_search: f64,
    rollout_search_states: u64,
    rollout_search_qnodes: u64,
    rollout_decisions: u64,
    rollout_legal_moves: u64,
    rollout_candidates_scored: u64,
    rollout_candidates_truncated: u64,
    rollout_near_best_candidates: u64,
    rollout_selected_score_gap_sum: i64,
    rollout_selected_score_gap_max: i32,
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
    #[serde(default = "legacy_validation_opening_schedule")]
    validation_opening_schedule: String,
    #[serde(default)]
    validation_opening_pairs_per_id: Option<u32>,
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
    #[serde(default = "legacy_label_retry_policy")]
    label_retry_policy: String,
    #[serde(default)]
    max_label_attempts_per_game: Option<u16>,
    #[serde(default)]
    position_selection_audit_version: String,
    #[serde(default)]
    candidate_positions: u64,
    #[serde(default)]
    candidate_roots_per_game: Option<u16>,
    #[serde(default)]
    candidate_identity_version: String,
    #[serde(default)]
    candidate_identity_sha256: String,
    #[serde(default)]
    generation_semantic_identity_version: String,
    #[serde(default)]
    generation_semantic_identity_sha256: String,
    #[serde(default)]
    schedule_identity_version: String,
    #[serde(default)]
    schedule_identity_sha256: String,
    #[serde(default)]
    minimum_train_boards: Option<u64>,
    #[serde(default)]
    minimum_train_positions: Option<u64>,
    #[serde(default)]
    rejected_incomplete_label_positions: u64,
    #[serde(default)]
    rejected_terminal_positions: u64,
    #[serde(default)]
    rejected_mate_score_positions: u64,
    #[serde(default)]
    rejected_node_accounting_positions: u64,
    #[serde(default)]
    label_retry_exhausted_games: u64,
    #[serde(default)]
    label_retry_attempts_per_accepted_position: f64,
    #[serde(default)]
    rejected_incomplete_root_plies: BTreeMap<u16, u64>,
    #[serde(default)]
    rejected_terminal_root_plies: BTreeMap<u16, u64>,
    #[serde(default)]
    rejected_mate_root_plies: BTreeMap<u16, u64>,
    #[serde(default)]
    rejected_node_accounting_root_plies: BTreeMap<u16, u64>,
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
    rollout_candidate_limit: u16,
    #[serde(default)]
    rollout_score_margin: i32,
    #[serde(default)]
    rollout_temperature: f64,
    #[serde(default)]
    rollout_rng_version: String,
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
    rollout_decisions: u64,
    #[serde(default)]
    rollout_legal_moves: u64,
    #[serde(default)]
    rollout_candidates_scored: u64,
    #[serde(default)]
    rollout_candidates_truncated: u64,
    #[serde(default)]
    rollout_near_best_candidates: u64,
    #[serde(default)]
    rollout_selected_score_gap_sum: i64,
    #[serde(default)]
    rollout_selected_score_gap_max: i32,
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
    #[serde(default)]
    feature_family: String,
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

fn legacy_label_retry_policy() -> String {
    "none".to_string()
}

fn legacy_opening_policy() -> String {
    "uniform-random".to_string()
}

fn legacy_opening_transformation() -> String {
    "none".to_string()
}

fn legacy_validation_opening_schedule() -> String {
    "hash-v1".to_string()
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
    candidate_identity_sha256: String,
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
    rejected_node_accounting_root_black: u64,
    rejected_node_accounting_root_white: u64,
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
    rejected_node_accounting_game_win: u64,
    rejected_node_accounting_game_loss: u64,
    rejected_node_accounting_game_draw: u64,
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
            rejected_node_accounting_root_black,
            rejected_node_accounting_root_white,
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
            rejected_node_accounting_game_win,
            rejected_node_accounting_game_loss,
            rejected_node_accounting_game_draw,
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

    fn record_node_accounting(&mut self, root_side: Color) {
        match root_side {
            Color::Black => self.rejected_node_accounting_root_black += 1,
            Color::White => self.rejected_node_accounting_root_white += 1,
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
        let node_accounting = relative_rejection_counts(
            self.rejected_node_accounting_root_black,
            self.rejected_node_accounting_root_white,
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
        (
            self.rejected_node_accounting_game_win,
            self.rejected_node_accounting_game_loss,
            self.rejected_node_accounting_game_draw,
        ) = node_accounting;
    }
}

#[derive(Debug, Clone, Default)]
struct SearchUseStats {
    label_searches: u64,
    rollout_searches: u64,
    label_search_states: u64,
    label_search_qnodes: u64,
    rollout_search_states: u64,
    rollout_search_qnodes: u64,
    label_search_elapsed_seconds: f64,
    rollout_search_elapsed_seconds: f64,
    rollout_decisions: u64,
    rollout_legal_moves: u64,
    rollout_candidates_scored: u64,
    rollout_candidates_truncated: u64,
    rollout_near_best_candidates: u64,
    rollout_selected_score_gap_sum: i64,
    rollout_selected_score_gap_max: i32,
    candidate_positions: u64,
    rejected_incomplete_label_positions: u64,
    rejected_terminal_positions: u64,
    rejected_mate_score_positions: u64,
    rejected_node_accounting_positions: u64,
    label_retry_exhausted_games: u64,
    rejected_incomplete_root_plies: BTreeMap<u16, u64>,
    rejected_terminal_root_plies: BTreeMap<u16, u64>,
    rejected_mate_root_plies: BTreeMap<u16, u64>,
    rejected_node_accounting_root_plies: BTreeMap<u16, u64>,
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

    fn record_rollout_decision(
        &mut self,
        legal_moves: u64,
        candidates: u64,
        near_best: u64,
        score_gap: i32,
    ) {
        self.rollout_decisions += 1;
        self.rollout_legal_moves += legal_moves;
        self.rollout_candidates_scored += candidates;
        self.rollout_candidates_truncated += legal_moves.saturating_sub(candidates);
        self.rollout_near_best_candidates += near_best;
        self.rollout_selected_score_gap_sum += i64::from(score_gap);
        self.rollout_selected_score_gap_max = self.rollout_selected_score_gap_max.max(score_gap);
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
        self.rollout_decisions += other.rollout_decisions;
        self.rollout_legal_moves += other.rollout_legal_moves;
        self.rollout_candidates_scored += other.rollout_candidates_scored;
        self.rollout_candidates_truncated += other.rollout_candidates_truncated;
        self.rollout_near_best_candidates += other.rollout_near_best_candidates;
        self.rollout_selected_score_gap_sum += other.rollout_selected_score_gap_sum;
        self.rollout_selected_score_gap_max = self
            .rollout_selected_score_gap_max
            .max(other.rollout_selected_score_gap_max);
        self.candidate_positions += other.candidate_positions;
        self.rejected_incomplete_label_positions += other.rejected_incomplete_label_positions;
        self.rejected_terminal_positions += other.rejected_terminal_positions;
        self.rejected_mate_score_positions += other.rejected_mate_score_positions;
        self.rejected_node_accounting_positions += other.rejected_node_accounting_positions;
        self.label_retry_exhausted_games += other.label_retry_exhausted_games;
        merge_count_map(
            &mut self.rejected_incomplete_root_plies,
            other.rejected_incomplete_root_plies,
        );
        merge_count_map(
            &mut self.rejected_terminal_root_plies,
            other.rejected_terminal_root_plies,
        );
        merge_count_map(
            &mut self.rejected_mate_root_plies,
            other.rejected_mate_root_plies,
        );
        merge_count_map(
            &mut self.rejected_node_accounting_root_plies,
            other.rejected_node_accounting_root_plies,
        );
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

fn merge_count_map(target: &mut BTreeMap<u16, u64>, source: BTreeMap<u16, u64>) {
    for (key, count) in source {
        *target.entry(key).or_default() += count;
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
        .saturating_sub(stats.rejected_node_accounting_positions)
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
            rollout_decisions: manifest.rollout_decisions,
            rollout_legal_moves: manifest.rollout_legal_moves,
            rollout_candidates_scored: manifest.rollout_candidates_scored,
            rollout_candidates_truncated: manifest.rollout_candidates_truncated,
            rollout_near_best_candidates: manifest.rollout_near_best_candidates,
            rollout_selected_score_gap_sum: manifest.rollout_selected_score_gap_sum,
            rollout_selected_score_gap_max: manifest.rollout_selected_score_gap_max,
            candidate_positions: if manifest.candidate_positions == 0 {
                manifest.sampled_positions
            } else {
                manifest.candidate_positions
            },
            rejected_incomplete_label_positions: manifest.rejected_incomplete_label_positions,
            rejected_terminal_positions: manifest.rejected_terminal_positions,
            rejected_mate_score_positions: manifest.rejected_mate_score_positions,
            rejected_node_accounting_positions: manifest.rejected_node_accounting_positions,
            label_retry_exhausted_games: manifest.label_retry_exhausted_games,
            rejected_incomplete_root_plies: manifest.rejected_incomplete_root_plies.clone(),
            rejected_terminal_root_plies: manifest.rejected_terminal_root_plies.clone(),
            rejected_mate_root_plies: manifest.rejected_mate_root_plies.clone(),
            rejected_node_accounting_root_plies: manifest
                .rejected_node_accounting_root_plies
                .clone(),
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
    total_nodes: u64,
    node_limit: Option<u64>,
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
            total_nodes: summary.states.saturating_add(summary.qsearch_stats.qnodes),
            node_limit: None,
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
            total_nodes: summary.total_nodes,
            node_limit: Some(summary.node_limit),
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

#[derive(Debug, Clone)]
struct GenerationTeachers {
    trajectory: Teacher,
    label: Teacher,
}

impl GenerationTeachers {
    fn from_config(loaded: &LoadedConfig) -> Result<Self> {
        Ok(Self {
            trajectory: Teacher::from_evaluator(loaded, &loaded.trajectory_evaluator().evaluator)?,
            label: Teacher::from_evaluator(loaded, &loaded.label_evaluator().evaluator)?,
        })
    }
}

#[derive(Default)]
struct GenerationSearchWorkspaces {
    trajectory: SearchWorkspace,
    label: SearchWorkspace,
}

impl Teacher {
    fn from_evaluator(loaded: &LoadedConfig, evaluator: &EvaluatorConfig) -> Result<Self> {
        if evaluator.kind == EvaluatorKind::Handcrafted {
            return Ok(Self::Handcrafted);
        }
        let path = loaded.resolve_path(
            evaluator
                .model
                .as_deref()
                .expect("validated NNUE evaluator model path"),
        );
        let bytes = fs::read(&path).with_context(|| {
            format!(
                "configured evaluator NNUE could not be read: {}",
                path.display()
            )
        })?;
        let bootstrap_sha256 = hash_bytes_hex(&bytes);
        ensure!(
            evaluator.model_sha256.as_deref() == Some(&bootstrap_sha256),
            "configured evaluator NNUE {} has sha256 {}, expected {}",
            path.display(),
            bootstrap_sha256,
            evaluator.model_sha256.as_deref().unwrap_or("missing")
        );
        let model = NnueModel::from_bytes(&bytes)
            .map_err(|err| anyhow!("failed to load evaluator NNUE {}: {err}", path.display()))?;
        Ok(Self::Nnue {
            model: Arc::new(model),
            bootstrap_sha256,
        })
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

#[derive(Debug, Clone, Copy)]
struct ScoredRolloutMove {
    move_: Move,
    score: i32,
}

#[derive(Debug, Clone, Copy)]
struct RolloutDecision {
    move_: Move,
    best_score: i32,
    selected_score: i32,
}

fn bounded_rollout_candidates(
    mut legal_moves: Vec<Move>,
    root_best_move: Move,
    candidate_limit: usize,
) -> Vec<Move> {
    debug_assert!(candidate_limit > 0);
    debug_assert!(legal_moves.contains(&root_best_move));
    legal_moves.sort_by_key(ToString::to_string);
    if legal_moves.len() <= candidate_limit {
        return legal_moves;
    }

    let mut candidates = legal_moves[..candidate_limit].to_vec();
    if !candidates.contains(&root_best_move) {
        candidates[candidate_limit - 1] = root_best_move;
        candidates.sort_by_key(ToString::to_string);
    }
    candidates
}

/// Choose one move from a bounded, cheap-search-ranked candidate set.
///
/// Candidate ordering is performed in the canonical orientation of the board
/// and its 180-degree color-swapped image.  The same pair-index/ply stream is
/// therefore used for both games in a color-swapped opening pair, while the
/// selected canonical move is transformed back for the swapped game.
fn choose_searched_stochastic_rollout_move(
    board: &Board,
    legal_moves: &[Move],
    teacher: &Teacher,
    search_depth: u8,
    candidate_limit: u16,
    score_margin: i32,
    temperature: f64,
    base_seed: u64,
    dataset_name: &str,
    pair_index: u32,
    ply: u16,
    rng_version: &str,
    workspace: &mut SearchWorkspace,
    stats: &mut SearchUseStats,
) -> Result<RolloutDecision> {
    ensure!(
        !legal_moves.is_empty(),
        "searched-stochastic rollout requires at least one legal move"
    );
    ensure!(
        candidate_limit > 0,
        "rollout candidate limit must be positive"
    );
    ensure!(
        temperature.is_finite() && temperature > 0.0,
        "rollout temperature must be finite and positive"
    );

    let current_sfen = board.to_string();
    let swapped_sfen = color_swap_anhoku_sfen(&current_sfen).with_context(|| {
        format!(
            "failed to canonicalize rollout position for dataset `{dataset_name}`, pair {pair_index}, ply {ply}"
        )
    })?;
    let use_swapped_orientation = swapped_sfen < current_sfen;
    let canonical_board = Board::from_sfen(if use_swapped_orientation {
        &swapped_sfen
    } else {
        &current_sfen
    })
    .map_err(|err| anyhow!("failed to parse canonical rollout position: {err}"))?;
    let canonical_legal_moves = legal_moves
        .iter()
        .copied()
        .map(|move_| {
            if use_swapped_orientation {
                transform_move(move_)
            } else {
                move_
            }
        })
        .collect::<Vec<_>>();
    let root_summary = teacher.search_depth(&canonical_board, search_depth, workspace)?;
    stats.record_rollout(&root_summary);
    let root_best_move = searched_best_move(&canonical_board, &root_summary)?;
    let candidates = bounded_rollout_candidates(
        canonical_legal_moves,
        root_best_move,
        usize::from(candidate_limit),
    );

    let mut scored = Vec::with_capacity(candidates.len());
    for canonical_move in candidates {
        ensure!(
            canonical_board.is_legal(canonical_move),
            "canonical rollout candidate `{canonical_move}` is not legal"
        );
        let mut child = canonical_board.clone();
        child.play_unchecked(canonical_move);
        let summary = teacher.search_depth(&child, search_depth, workspace)?;
        stats.record_rollout(&summary);
        let score = summary.best_score.map_or(0, |score| -score);
        scored.push(ScoredRolloutMove {
            move_: canonical_move,
            score,
        });
    }

    scored.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.move_.to_string().cmp(&right.move_.to_string()))
    });
    let best_score = scored[0].score;
    let cutoff = best_score.saturating_sub(score_margin);
    let near_best_candidates = scored
        .iter()
        .filter(|candidate| candidate.score >= cutoff)
        .count();
    let selected_index = weighted_choice_index(
        &scored[..near_best_candidates],
        temperature,
        rollout_stream_seed(base_seed, dataset_name, pair_index, ply, rng_version),
    );
    let selected = scored[selected_index];
    let selected_move = if use_swapped_orientation {
        transform_move(selected.move_)
    } else {
        selected.move_
    };
    ensure!(
        legal_moves.contains(&selected_move),
        "searched-stochastic rollout selected illegal move `{selected_move}`"
    );
    stats.record_rollout_decision(
        legal_moves.len() as u64,
        scored.len() as u64,
        near_best_candidates as u64,
        best_score.saturating_sub(selected.score),
    );
    Ok(RolloutDecision {
        move_: selected_move,
        best_score,
        selected_score: selected.score,
    })
}

fn weighted_choice_index(candidates: &[ScoredRolloutMove], temperature: f64, seed: u64) -> usize {
    debug_assert!(!candidates.is_empty());
    if candidates.len() == 1 {
        return 0;
    }
    let best_score = candidates[0].score;
    let weights = candidates
        .iter()
        .map(|candidate| {
            (f64::from(candidate.score.saturating_sub(best_score)) / temperature)
                .exp()
                .max(f64::MIN_POSITIVE)
        })
        .collect::<Vec<_>>();
    let total = weights.iter().sum::<f64>();
    let target = unit_interval(splitmix64(seed)) * total;
    let mut cumulative = 0.0;
    for (index, weight) in weights.iter().enumerate() {
        cumulative += weight;
        if target < cumulative || index + 1 == weights.len() {
            return index;
        }
    }
    candidates.len() - 1
}

fn unit_interval(value: u64) -> f64 {
    // Use the top 53 bits so the conversion is exact for the generated
    // double and independent of rand crate implementation details.
    (value >> 11) as f64 / (1u64 << 53) as f64
}

fn rollout_stream_seed(
    base_seed: u64,
    dataset_name: &str,
    pair_index: u32,
    ply: u16,
    rng_version: &str,
) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(b"haitaka-rollout-stream\0");
    hasher.update(rng_version.as_bytes());
    hasher.update([0]);
    hasher.update(dataset_name.as_bytes());
    hasher.update([0]);
    hasher.update(base_seed.to_le_bytes());
    hasher.update(pair_index.to_le_bytes());
    hasher.update(ply.to_le_bytes());
    let digest = hasher.finalize();
    splitmix64(u64::from_le_bytes(
        digest[..8].try_into().expect("SHA-256 prefix is 8 bytes"),
    ))
}

fn transform_move(move_: Move) -> Move {
    match move_ {
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

const TRAJECTORY_AUDIT_SCHEMA: &str = "haitaka-trajectory-audit-v2";
const LABEL_CALIBRATION_SCHEMA: &str = "haitaka-label-calibration-v2";
const TRAJECTORY_TRANCHE_GAMES: usize = 64;
const TRAJECTORY_MIN_PAIRS_PER_OPENING: u64 = 2;
const TRAJECTORY_MIN_UNIQUE_RATIO: f64 = 0.95;
const TRAJECTORY_MIN_FINAL_NEW_BOARDS_PER_GAME: f64 = 30.0;
const CALIBRATION_MAX_INCOMPLETE_PERCENT: u64 = 1;
const CALIBRATION_MAX_BIAS_DELTA: f64 = 0.05;
const CALIBRATION_BUDGETS: [u64; 3] = [50_000, 100_000, 200_000];
const CALIBRATION_MAX_OVERALL_ATTEMPTS_PER_ACCEPT: f64 = 1.25;
const CALIBRATION_MAX_SPLIT_ATTEMPTS_PER_ACCEPT: f64 = 1.50;

#[derive(Debug, Clone, Copy, Default)]
pub struct TrajectoryAuditOptions {
    pub jobs: Option<u32>,
    pub shard_index: Option<u32>,
    pub shard_index_end: Option<u32>,
    pub shard_count: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct TrajectoryAuditReport {
    schema: &'static str,
    source: TrajectorySourceIdentity,
    policy: TrajectoryPolicyIdentity,
    limits: TrajectoryAuditLimits,
    totals: TrajectoryTotals,
    opening_coverage: TrajectoryOpeningCoverage,
    tranches: Vec<TrajectoryTranche>,
    trajectories: Vec<TrajectorySummary>,
    paired_symmetry: PairedSymmetry,
    decision: TrajectoryAuditDecision,
}

#[derive(Debug, Serialize)]
struct TrajectorySourceIdentity {
    config_path: String,
    config_sha256: String,
    ruleset: String,
    rule_id: u16,
    teacher_build_mode: String,
    teacher_sha256: Option<String>,
    engine_revision: Option<String>,
    opening_suite_id: Option<String>,
    opening_suite_sha256: Option<String>,
    opening_transformation: String,
    train_opening_ids: Vec<String>,
    ood_v2_opening_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct TrajectoryPolicyIdentity {
    self_play_move_policy: String,
    rollout_search_depth: u8,
    rollout_candidate_limit: u16,
    rollout_score_margin: i32,
    rollout_temperature: f64,
    rollout_rng_version: String,
    opening_random_plies: u16,
    stream_key: String,
}

#[derive(Debug, Serialize)]
struct TrajectoryAuditLimits {
    games_requested: u32,
    games_audited: u64,
    max_games: u64,
    tranche_games: usize,
    minimum_pairs_per_opening: u64,
    initial_coverage_games: u64,
    minimum_packed_board_unique_ratio: f64,
    minimum_final_post_coverage_new_boards_per_game: f64,
    jobs: usize,
    shard_index: Option<u32>,
    shard_index_end: Option<u32>,
    shard_count: Option<u32>,
}

#[derive(Debug, Serialize)]
struct TrajectoryTotals {
    games: u64,
    game_plies: u64,
    game_length_min: u16,
    game_length_max: u16,
    game_length_mean: f64,
    packed_board_occurrences: u64,
    distinct_packed_boards: u64,
    packed_board_unique_ratio: f64,
    black_wins: u64,
    white_wins: u64,
    draws: u64,
    rollout_decisions: u64,
    rollout_legal_moves: u64,
    rollout_candidates_scored: u64,
    rollout_candidates_truncated: u64,
    rollout_near_best_candidates: u64,
    selected_score_gap_mean: f64,
    selected_score_gap_max: i32,
    rollout_searches: u64,
    rollout_search_states: u64,
    rollout_search_qnodes: u64,
    rollout_cpu_seconds: f64,
}

#[derive(Debug, Serialize)]
struct TrajectoryTranche {
    ordinal_start: u64,
    ordinal_end_exclusive: u64,
    dataset_counts: BTreeMap<String, u64>,
    games: u64,
    packed_board_occurrences: u64,
    new_packed_boards: u64,
    new_packed_boards_per_game: f64,
    cumulative_distinct_packed_boards: u64,
    post_initial_coverage: bool,
}

#[derive(Debug, Serialize)]
struct TrajectorySummary {
    dataset: String,
    game_index: u32,
    pair_index: u32,
    opening_id: String,
    color: String,
    game_length: u16,
    outcome: String,
    packed_board_occurrences: u64,
    distinct_packed_boards_in_game: u64,
    new_packed_boards: u64,
    selected_score_gap_mean: f64,
    selected_score_gap_max: i32,
    rollout_decisions: u64,
    rollout_legal_moves: u64,
    rollout_candidates_scored: u64,
    rollout_candidates_truncated: u64,
    rollout_cpu_seconds: f64,
    trajectory_sha256: String,
}

#[derive(Debug, Serialize)]
struct PairedSymmetry {
    pairs_expected: u64,
    pairs_compared: u64,
    exact_transformed_move_sequence_pairs: u64,
    mismatched_pairs: u64,
    unpaired_games: u64,
    mismatch_examples: Vec<String>,
}

#[derive(Debug, Serialize)]
struct TrajectoryOpeningCoverage {
    schedule: &'static str,
    expected_train_ids: Vec<String>,
    expected_ood_v2_ids: Vec<String>,
    train_pair_counts: BTreeMap<String, u64>,
    ood_v2_pair_counts: BTreeMap<String, u64>,
    missing_train_ids: Vec<String>,
    missing_ood_v2_ids: Vec<String>,
    train_ids_below_minimum_pairs: Vec<String>,
    ood_v2_ids_below_minimum_pairs: Vec<String>,
    minimum_pairs_per_opening: u64,
    unexpected_ids: Vec<String>,
    complete: bool,
}

#[derive(Debug, Serialize)]
struct TrajectoryAuditDecision {
    passed: bool,
    packed_board_unique_ratio: f64,
    final_tranche_new_packed_boards_per_game: f64,
    final_post_coverage_tranche_new_packed_boards_per_game: Option<f64>,
    failures: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct TrajectoryTask {
    dataset_name: &'static str,
    game_index: u32,
}

#[derive(Debug)]
struct TrajectoryGame {
    task: TrajectoryTask,
    opening: GameOpeningMetadata,
    boards: Vec<[u8; PACKED_SFEN_BYTES]>,
    moves: Vec<Move>,
    score_gaps: Vec<i32>,
    stats: SearchUseStats,
    outcome: GameOutcome,
    trajectory_sha256: String,
    calibration_roots: Vec<CalibrationRoot>,
}

#[derive(Debug, Clone)]
struct CalibrationRoot {
    board: Board,
    root_ply: u16,
    side_to_move: Color,
    outcome: GameOutcome,
}

#[derive(Debug, Serialize)]
pub struct LabelCalibrationReport {
    schema: &'static str,
    source: TrajectorySourceIdentity,
    policy: TrajectoryPolicyIdentity,
    calibration: LabelCalibrationIdentity,
    trajectories: Vec<CalibrationTrajectorySummary>,
    budgets: Vec<CalibrationBudgetReport>,
    decision: LabelCalibrationDecision,
}

#[derive(Debug, Serialize)]
struct LabelCalibrationIdentity {
    games: u64,
    suite_ids: Vec<String>,
    train_suite_ids: Vec<String>,
    ood_v2_suite_ids: Vec<String>,
    candidate_root_schedule: String,
    candidate_root_count: u64,
    candidate_roots_sha256: String,
    paired_root_mismatches: u64,
    opening_ids_with_matched_roots: Vec<String>,
    opening_ids_without_matched_roots: Vec<String>,
    trajectory_hashes_sha256: String,
    label_position_policy: String,
    label_retry_policy: String,
    max_label_attempts_per_game: Option<u16>,
    max_depth: u8,
    max_rejection_rate_delta: f64,
    rejection_rate_delta_is_gate: bool,
    max_overall_attempts_per_accepted_root: f64,
    max_split_attempts_per_accepted_root: f64,
}

#[derive(Debug, Serialize)]
struct CalibrationTrajectorySummary {
    dataset: String,
    opening_id: String,
    pair_index: u32,
    base_trajectory_sha256: String,
    swapped_trajectory_sha256: String,
    candidate_roots: u64,
}

#[derive(Debug, Serialize)]
struct CalibrationBudgetReport {
    nodes: u64,
    train: CalibrationSplitReport,
    ood_v2: CalibrationSplitReport,
    attempts_per_accepted_root: f64,
    passed: bool,
}

#[derive(Debug, Serialize)]
struct CalibrationSplitReport {
    candidate_roots: u64,
    accepted_roots: u64,
    exhausted_games: u64,
    attempts_per_accepted_root: f64,
    accepted_bad_labels: u64,
    incomplete_labels: u64,
    incomplete_rate: f64,
    terminal_labels: u64,
    mate_labels: u64,
    rejected_labels: u64,
    alpha_beta_nodes: u64,
    qsearch_nodes: u64,
    accounted_nodes: u64,
    requested_node_budget: u64,
    node_accounting_exact: bool,
    node_accounting_errors: u64,
    incomplete_by_side: BinaryCalibrationCounts,
    rejected_by_side: BinaryCalibrationCounts,
    incomplete_by_outcome: OutcomeCalibrationCounts,
    rejected_by_outcome: OutcomeCalibrationCounts,
    side_rejection_rate_delta: f64,
    outcome_rejection_rate_delta: f64,
    passed: bool,
}

#[derive(Debug, Default, Serialize)]
struct BinaryCalibrationCounts {
    black: u64,
    white: u64,
}

#[derive(Debug, Default, Serialize)]
struct OutcomeCalibrationCounts {
    win: u64,
    loss: u64,
    draw: u64,
}

#[derive(Debug, Serialize)]
struct LabelCalibrationDecision {
    passed: bool,
    selected_budget_nodes: Option<u64>,
    status: String,
    failures: Vec<String>,
}

pub fn audit_trajectories(
    loaded: &LoadedGenerationConfig,
    options: TrajectoryAuditOptions,
) -> Result<TrajectoryAuditReport> {
    loaded.ruleset_requires_matching_engine()?;
    ensure!(
        loaded
            .config
            .data
            .self_play_move_policy
            .is_searched_stochastic(),
        "trajectory-audit requires data.self_play_move_policy=searched-stochastic-rollout-v1; historical uniform-rollout-v1 is readable but not a Phase 8D production policy"
    );
    ensure!(
        loaded.config.data.opening_random_plies == 0,
        "trajectory-audit requires data.opening_random_plies=0"
    );

    let opening_sfen = loaded.opening_sfen()?;
    let opening_source = OpeningSource::from_config(loaded, &opening_sfen)?;
    let opening_split = opening_source.split_openings(
        loaded.config.data.split_policy,
        loaded.config.data.split_seed,
        loaded.config.data.train_games,
        loaded.config.data.validation_games,
        loaded.config.data.validation_opening_ids.as_deref(),
        loaded.config.data.validation_opening_schedule,
        loaded.config.data.validation_opening_pairs_per_id,
    )?;
    let teacher = Teacher::from_evaluator(loaded, &loaded.trajectory_evaluator().evaluator)?;
    let selector = ShardSelector::new(
        options.shard_index,
        options.shard_index_end,
        options.shard_count,
    )?;
    let tasks = trajectory_tasks(loaded, &opening_split, selector)?;
    ensure!(
        tasks.len() <= 4096,
        "trajectory-audit selected {} games, above the hard limit of 4096; use shard selectors or reduce data.train_games/data.validation_games",
        tasks.len()
    );
    ensure!(!tasks.is_empty(), "trajectory-audit selected no games");
    let jobs = resolve_jobs(options.jobs.unwrap_or(loaded.config.data.jobs))?
        .min(tasks.len())
        .max(1);
    let trajectories = collect_trajectory_games(
        loaded,
        &teacher,
        &opening_source,
        &opening_split,
        &tasks,
        jobs,
        false,
    )?;
    let engine_revision = detect_git_revision(loaded)?;
    build_trajectory_audit_report(
        loaded,
        &teacher,
        &opening_source,
        &opening_split,
        engine_revision,
        trajectories,
        jobs,
        options,
    )
}

fn trajectory_tasks(
    loaded: &LoadedConfig,
    opening_split: &OpeningSplit,
    selector: ShardSelector,
) -> Result<Vec<(TrajectoryTask, String)>> {
    let mut tasks = Vec::new();
    for (dataset_name, game_count) in [
        ("train", loaded.config.data.train_games),
        ("validation", loaded.config.data.validation_games),
    ] {
        let opening_ids = opening_split.ids_for(dataset_name)?;
        ensure!(
            !opening_ids.is_empty(),
            "trajectory-audit requires at least one {dataset_name} opening ID"
        );
        for plan in shard_plans(game_count, loaded.config.data.shard_games, selector) {
            for game_index in plan.game_start..plan.game_start + plan.game_count {
                let opening_id =
                    opening_ids[(game_index / 2 % opening_ids.len() as u32) as usize].clone();
                tasks.push((
                    TrajectoryTask {
                        dataset_name,
                        game_index,
                    },
                    opening_id,
                ));
            }
        }
    }
    Ok(tasks)
}

fn collect_trajectory_games(
    loaded: &LoadedConfig,
    teacher: &Teacher,
    opening_source: &OpeningSource,
    opening_split: &OpeningSplit,
    tasks: &[(TrajectoryTask, String)],
    jobs: usize,
    calibration_root_schedule: bool,
) -> Result<Vec<TrajectoryGame>> {
    let queue = Arc::new(Mutex::new(VecDeque::from(tasks.to_vec())));
    let results = Arc::new(Mutex::new(Vec::<(
        TrajectoryTask,
        std::result::Result<TrajectoryGame, String>,
    )>::new()));
    thread::scope(|scope| {
        for _ in 0..jobs.min(tasks.len()).max(1) {
            let queue = Arc::clone(&queue);
            let results = Arc::clone(&results);
            scope.spawn(move || {
                let mut workspace = SearchWorkspace::default();
                loop {
                    let Some((task, opening_id)) =
                        queue.lock().expect("trajectory queue poisoned").pop_front()
                    else {
                        break;
                    };
                    let result = play_label_free_trajectory(
                        loaded,
                        teacher,
                        opening_source,
                        opening_split,
                        task,
                        Some(&opening_id),
                        calibration_root_schedule,
                        &mut workspace,
                    )
                    .map_err(|err| format!("{err:#}"));
                    let failed = result.is_err();
                    results
                        .lock()
                        .expect("trajectory results poisoned")
                        .push((task, result));
                    if failed {
                        // One failure is enough to make the audit invalid, and
                        // stopping this worker avoids producing a misleading
                        // partial report while other workers drain normally.
                        break;
                    }
                }
            });
        }
    });

    let mut results = Arc::try_unwrap(results)
        .map_err(|_| anyhow!("trajectory workers still hold their result queue"))?
        .into_inner()
        .map_err(|_| anyhow!("trajectory result queue was poisoned"))?;
    results.sort_by_key(|(task, _)| (dataset_sort_key(task.dataset_name), task.game_index));
    let mut trajectories = Vec::with_capacity(results.len());
    for (task, result) in results {
        trajectories.push(result.map_err(|err| anyhow!(err)).with_context(|| {
            format!(
                "failed to audit {} trajectory game {}",
                task.dataset_name, task.game_index
            )
        })?);
    }
    ensure!(
        trajectories.len() == tasks.len(),
        "trajectory workers returned {}/{} games",
        trajectories.len(),
        tasks.len()
    );
    Ok(trajectories)
}

fn dataset_sort_key(dataset_name: &str) -> u8 {
    match dataset_name {
        "train" => 0,
        "validation" => 1,
        _ => 2,
    }
}

fn trajectory_audit_sort_key(
    opening_split: &OpeningSplit,
    game: &TrajectoryGame,
) -> (u32, u8, u32, u32) {
    trajectory_task_audit_sort_key(opening_split, game.task)
}

fn trajectory_task_audit_sort_key(
    opening_split: &OpeningSplit,
    task: TrajectoryTask,
) -> (u32, u8, u32, u32) {
    let opening_count = match task.dataset_name {
        "train" => opening_split.train_ids.len(),
        "validation" => opening_split.validation_ids.len(),
        _ => 1,
    }
    .max(1) as u32;
    let pair_index = task.game_index / 2;
    (
        pair_index / opening_count,
        dataset_sort_key(task.dataset_name),
        pair_index % opening_count,
        task.game_index % 2,
    )
}

#[allow(clippy::too_many_arguments)]
fn play_label_free_trajectory(
    loaded: &LoadedConfig,
    teacher: &Teacher,
    opening_source: &OpeningSource,
    opening_split: &OpeningSplit,
    task: TrajectoryTask,
    forced_opening_id: Option<&str>,
    calibration_root_schedule: bool,
    search_workspace: &mut SearchWorkspace,
) -> Result<TrajectoryGame> {
    let seed = game_seed(loaded.config.data.seed, task.dataset_name, task.game_index);
    let pair_seed = game_seed(
        loaded.config.data.seed,
        task.dataset_name,
        task.game_index / 2,
    );
    let selected_opening = match forced_opening_id {
        Some(opening_id) => {
            opening_source.select_for_id(task.dataset_name, opening_id, task.game_index)?
        }
        None => {
            opening_source.select(task.dataset_name, opening_split, pair_seed, task.game_index)?
        }
    };
    let mut board = Board::from_sfen(&selected_opening.sfen)
        .map_err(|err| anyhow!("failed to parse opening SFEN: {err}"))?;
    let sample_origin = if calibration_root_schedule {
        loaded.config.data.sample_start_ply
    } else {
        sampling_origin(
            seed,
            loaded.config.data.sampling_policy,
            loaded.config.data.sample_start_ply,
            loaded.config.data.opening_random_plies,
            loaded.config.data.sample_every_ply,
        )
    };
    let mut boards = Vec::new();
    let mut moves = Vec::new();
    let mut score_gaps = Vec::new();
    let mut calibration_roots = Vec::new();
    let mut attempted_candidate_roots = 0u16;
    let mut played_plies = 0u16;
    let mut stats = SearchUseStats::default();

    while played_plies < loaded.config.data.max_plies {
        if !has_both_kings(&board) {
            break;
        }
        let legal_moves = collect_legal_moves(&board);
        if legal_moves.is_empty() {
            break;
        }
        let calibration_root_limit =
            if calibration_root_schedule && loaded.config.data.label_retry_policy.is_adaptive() {
                loaded
                    .config
                    .data
                    .max_label_attempts_per_game
                    .expect("validated adaptive retry attempt cap")
            } else {
                loaded
                    .config
                    .data
                    .max_candidate_roots_per_game
                    .unwrap_or(loaded.config.data.max_positions_per_game)
            };
        let should_collect_root = played_plies >= sample_origin
            && (played_plies - sample_origin) % loaded.config.data.sample_every_ply == 0
            && attempted_candidate_roots < calibration_root_limit;
        if should_collect_root {
            attempted_candidate_roots += 1;
            if calibration_root_schedule {
                calibration_roots.push(CalibrationRoot {
                    board: board.clone(),
                    root_ply: played_plies,
                    side_to_move: board.side_to_move(),
                    outcome: GameOutcome::Draw,
                });
            }
        }
        boards.push(pack_board_for_training(&board)?);
        let decision = choose_searched_stochastic_rollout_move(
            &board,
            &legal_moves,
            teacher,
            loaded.config.data.rollout_search_depth,
            loaded.config.data.rollout_candidate_limit,
            loaded.config.data.rollout_score_margin,
            loaded.config.data.rollout_temperature,
            loaded.config.data.seed,
            task.dataset_name,
            task.game_index / 2,
            played_plies,
            &loaded.config.data.rollout_rng_version,
            search_workspace,
            &mut stats,
        )?;
        score_gaps.push(decision.best_score.saturating_sub(decision.selected_score));
        moves.push(decision.move_);
        board.play_unchecked(decision.move_);
        played_plies += 1;
    }

    let outcome = determine_game_outcome(&board, played_plies, loaded.config.data.max_plies);
    for root in &mut calibration_roots {
        root.outcome = outcome;
    }
    let trajectory_sha256 = trajectory_hash(task, &selected_opening.metadata, &moves);
    Ok(TrajectoryGame {
        task,
        opening: selected_opening.metadata,
        boards,
        moves,
        score_gaps,
        stats,
        outcome,
        trajectory_sha256,
        calibration_roots,
    })
}

fn determine_game_outcome(board: &Board, played_plies: u16, max_plies: u16) -> GameOutcome {
    if played_plies >= max_plies {
        GameOutcome::Draw
    } else if !board.has(Color::Black, Piece::King) {
        GameOutcome::Winner(Color::White)
    } else if !board.has(Color::White, Piece::King) {
        GameOutcome::Winner(Color::Black)
    } else {
        match board.status() {
            haitaka::GameStatus::Won => GameOutcome::Winner(!board.side_to_move()),
            haitaka::GameStatus::Drawn | haitaka::GameStatus::Ongoing => GameOutcome::Draw,
        }
    }
}

fn trajectory_hash(task: TrajectoryTask, opening: &GameOpeningMetadata, moves: &[Move]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"haitaka-trajectory-v1\0");
    hasher.update(task.dataset_name.as_bytes());
    hasher.update([0]);
    hasher.update(task.game_index.to_le_bytes());
    for value in [&opening.opening_id, &opening.sfen] {
        hasher.update((value.len() as u32).to_le_bytes());
        hasher.update(value.as_bytes());
    }
    hasher.update((moves.len() as u32).to_le_bytes());
    for move_ in moves {
        let text = move_.to_string();
        hasher.update((text.len() as u32).to_le_bytes());
        hasher.update(text.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn trajectory_source_identity(
    loaded: &LoadedConfig,
    teacher: &Teacher,
    opening_source: &OpeningSource,
    opening_split: &OpeningSplit,
    engine_revision: Option<String>,
) -> Result<TrajectorySourceIdentity> {
    Ok(TrajectorySourceIdentity {
        config_path: loaded.path.display().to_string(),
        config_sha256: loaded.hash_hex.clone(),
        ruleset: loaded.config.rules.ruleset.as_str().to_string(),
        rule_id: loaded.effective_rule_id()?,
        teacher_build_mode: teacher_build_mode(loaded, teacher),
        teacher_sha256: teacher.bootstrap_sha256().map(str::to_string),
        engine_revision,
        opening_suite_id: opening_source.suite_id().map(str::to_string),
        opening_suite_sha256: opening_source.suite_sha256().map(str::to_string),
        opening_transformation: opening_source.transformation().to_string(),
        train_opening_ids: opening_split.train_ids.clone(),
        ood_v2_opening_ids: opening_split.validation_ids.clone(),
    })
}

fn trajectory_policy_identity(loaded: &LoadedConfig) -> TrajectoryPolicyIdentity {
    TrajectoryPolicyIdentity {
        self_play_move_policy: loaded
            .config
            .data
            .self_play_move_policy
            .manifest_name()
            .to_string(),
        rollout_search_depth: loaded.config.data.rollout_search_depth,
        rollout_candidate_limit: loaded.config.data.rollout_candidate_limit,
        rollout_score_margin: loaded.config.data.rollout_score_margin,
        rollout_temperature: loaded.config.data.rollout_temperature,
        rollout_rng_version: loaded.config.data.rollout_rng_version.clone(),
        opening_random_plies: loaded.config.data.opening_random_plies,
        stream_key: "dataset+pair-index+ply+rollout-rng-version".to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_trajectory_audit_report(
    loaded: &LoadedConfig,
    teacher: &Teacher,
    opening_source: &OpeningSource,
    opening_split: &OpeningSplit,
    engine_revision: Option<String>,
    mut trajectories: Vec<TrajectoryGame>,
    jobs: usize,
    options: TrajectoryAuditOptions,
) -> Result<TrajectoryAuditReport> {
    let source = trajectory_source_identity(
        loaded,
        teacher,
        opening_source,
        opening_split,
        engine_revision,
    )?;
    let policy = trajectory_policy_identity(loaded);
    trajectories.sort_by_key(|game| trajectory_audit_sort_key(opening_split, game));
    let initial_coverage_games =
        ((opening_split.train_ids.len() + opening_split.validation_ids.len()) * 2) as u64;
    let mut all_boards = BTreeSet::<[u8; PACKED_SFEN_BYTES]>::new();
    let mut total_stats = SearchUseStats::default();
    let mut summaries = Vec::with_capacity(trajectories.len());
    let mut tranches = Vec::new();
    let mut game_plies = 0u64;
    let mut packed_board_occurrences = 0u64;
    let mut score_gap_sum = 0i64;
    let mut score_gap_max = 0i32;
    let mut black_wins = 0u64;
    let mut white_wins = 0u64;
    let mut draws = 0u64;

    for (tranche_index, chunk) in trajectories.chunks(TRAJECTORY_TRANCHE_GAMES).enumerate() {
        let ordinal_start = (tranche_index * TRAJECTORY_TRANCHE_GAMES) as u64;
        let ordinal_end_exclusive = ordinal_start + chunk.len() as u64;
        let mut dataset_counts = BTreeMap::new();
        let mut tranche_occurrences = 0u64;
        let mut tranche_new_boards = 0u64;
        for game in chunk {
            let mut game_boards = BTreeSet::new();
            let mut new_boards = 0u64;
            for board in &game.boards {
                game_boards.insert(*board);
                if all_boards.insert(*board) {
                    new_boards += 1;
                }
            }
            *dataset_counts
                .entry(trajectory_dataset_label(game.task.dataset_name).to_string())
                .or_default() += 1;
            tranche_occurrences += game.boards.len() as u64;
            tranche_new_boards += new_boards;
            game_plies += game.moves.len() as u64;
            packed_board_occurrences += game.boards.len() as u64;
            score_gap_sum += game
                .score_gaps
                .iter()
                .map(|gap| i64::from(*gap))
                .sum::<i64>();
            score_gap_max =
                score_gap_max.max(game.score_gaps.iter().copied().max().unwrap_or_default());
            match game.outcome {
                GameOutcome::Winner(Color::Black) => black_wins += 1,
                GameOutcome::Winner(Color::White) => white_wins += 1,
                GameOutcome::Draw => draws += 1,
            }
            total_stats.add(game.stats.clone());
            let game_gap_sum = game
                .score_gaps
                .iter()
                .map(|gap| i64::from(*gap))
                .sum::<i64>();
            summaries.push(TrajectorySummary {
                dataset: trajectory_dataset_label(game.task.dataset_name).to_string(),
                game_index: game.task.game_index,
                pair_index: game.task.game_index / 2,
                opening_id: game.opening.opening_id.clone(),
                color: game.opening.color.clone(),
                game_length: game.moves.len() as u16,
                outcome: outcome_label(game.outcome).to_string(),
                packed_board_occurrences: game.boards.len() as u64,
                distinct_packed_boards_in_game: game_boards.len() as u64,
                new_packed_boards: new_boards,
                selected_score_gap_mean: if game.score_gaps.is_empty() {
                    0.0
                } else {
                    game_gap_sum as f64 / game.score_gaps.len() as f64
                },
                selected_score_gap_max: game.score_gaps.iter().copied().max().unwrap_or_default(),
                rollout_decisions: game.stats.rollout_decisions,
                rollout_legal_moves: game.stats.rollout_legal_moves,
                rollout_candidates_scored: game.stats.rollout_candidates_scored,
                rollout_candidates_truncated: game.stats.rollout_candidates_truncated,
                rollout_cpu_seconds: game.stats.rollout_search_elapsed_seconds,
                trajectory_sha256: game.trajectory_sha256.clone(),
            });
        }
        tranches.push(TrajectoryTranche {
            ordinal_start,
            ordinal_end_exclusive,
            dataset_counts,
            games: chunk.len() as u64,
            packed_board_occurrences: tranche_occurrences,
            new_packed_boards: tranche_new_boards,
            new_packed_boards_per_game: if chunk.is_empty() {
                0.0
            } else {
                tranche_new_boards as f64 / chunk.len() as f64
            },
            cumulative_distinct_packed_boards: all_boards.len() as u64,
            post_initial_coverage: ordinal_start >= initial_coverage_games,
        });
    }

    let paired_symmetry = audit_paired_symmetry(&trajectories);
    let opening_coverage = audit_trajectory_opening_coverage(opening_split, &trajectories);
    let packed_board_unique_ratio = ratio_f64(all_boards.len() as u64, packed_board_occurrences);
    let final_tranche_new_boards_per_game = tranches
        .last()
        .map(|tranche| tranche.new_packed_boards_per_game)
        .unwrap_or_default();
    let final_post_coverage_tranche_new_boards_per_game = tranches
        .iter()
        .filter(|tranche| tranche.post_initial_coverage)
        .next_back()
        .map(|tranche| tranche.new_packed_boards_per_game);
    let mut failures = Vec::new();
    if packed_board_unique_ratio < TRAJECTORY_MIN_UNIQUE_RATIO {
        failures.push(format!(
            "packed-board uniqueness ratio {:.4} is below {:.2}",
            packed_board_unique_ratio, TRAJECTORY_MIN_UNIQUE_RATIO
        ));
    }
    match final_post_coverage_tranche_new_boards_per_game {
        Some(yield_per_game)
            if yield_per_game >= TRAJECTORY_MIN_FINAL_NEW_BOARDS_PER_GAME => {}
        Some(yield_per_game) => failures.push(format!(
            "final post-coverage tranche new-board yield {:.2}/game is below {:.2}/game",
            yield_per_game, TRAJECTORY_MIN_FINAL_NEW_BOARDS_PER_GAME
        )),
        None => failures.push(format!(
            "trajectory audit has no complete tranche after the initial {initial_coverage_games}-game opening-coverage cycle"
        )),
    }
    if paired_symmetry.mismatched_pairs > 0 {
        failures.push(format!(
            "{} color-swapped pairs did not reproduce the transformed move sequence",
            paired_symmetry.mismatched_pairs
        ));
    }
    if paired_symmetry.unpaired_games > 0 {
        failures.push(format!(
            "{} audited games had no color-swapped partner",
            paired_symmetry.unpaired_games
        ));
    }
    if !opening_coverage.complete {
        failures.push(format!(
            "trajectory opening coverage is incomplete: missing {} train IDs and {} OOD-v2 IDs; {} train IDs and {} OOD-v2 IDs have fewer than {} pairs; unexpected IDs: {}",
            opening_coverage.missing_train_ids.len(),
            opening_coverage.missing_ood_v2_ids.len(),
            opening_coverage.train_ids_below_minimum_pairs.len(),
            opening_coverage.ood_v2_ids_below_minimum_pairs.len(),
            opening_coverage.minimum_pairs_per_opening,
            opening_coverage.unexpected_ids.len()
        ));
    }
    if trajectories.is_empty() {
        failures.push("no trajectories were audited".to_string());
    }

    let games = trajectories.len() as u64;
    let game_length_min = trajectories
        .iter()
        .map(|game| game.moves.len() as u16)
        .min()
        .unwrap_or_default();
    let game_length_max = trajectories
        .iter()
        .map(|game| game.moves.len() as u16)
        .max()
        .unwrap_or_default();
    let totals = TrajectoryTotals {
        games,
        game_plies,
        game_length_min,
        game_length_max,
        game_length_mean: if games == 0 {
            0.0
        } else {
            game_plies as f64 / games as f64
        },
        packed_board_occurrences,
        distinct_packed_boards: all_boards.len() as u64,
        packed_board_unique_ratio,
        black_wins,
        white_wins,
        draws,
        rollout_decisions: total_stats.rollout_decisions,
        rollout_legal_moves: total_stats.rollout_legal_moves,
        rollout_candidates_scored: total_stats.rollout_candidates_scored,
        rollout_candidates_truncated: total_stats.rollout_candidates_truncated,
        rollout_near_best_candidates: total_stats.rollout_near_best_candidates,
        selected_score_gap_mean: if total_stats.rollout_decisions == 0 {
            0.0
        } else {
            score_gap_sum as f64 / total_stats.rollout_decisions as f64
        },
        selected_score_gap_max: score_gap_max,
        rollout_searches: total_stats.rollout_searches,
        rollout_search_states: total_stats.rollout_search_states,
        rollout_search_qnodes: total_stats.rollout_search_qnodes,
        rollout_cpu_seconds: total_stats.rollout_search_elapsed_seconds,
    };
    Ok(TrajectoryAuditReport {
        schema: TRAJECTORY_AUDIT_SCHEMA,
        source,
        policy,
        limits: TrajectoryAuditLimits {
            games_requested: loaded.config.data.train_games + loaded.config.data.validation_games,
            games_audited: games,
            max_games: 4096,
            tranche_games: TRAJECTORY_TRANCHE_GAMES,
            minimum_pairs_per_opening: TRAJECTORY_MIN_PAIRS_PER_OPENING,
            initial_coverage_games,
            minimum_packed_board_unique_ratio: TRAJECTORY_MIN_UNIQUE_RATIO,
            minimum_final_post_coverage_new_boards_per_game:
                TRAJECTORY_MIN_FINAL_NEW_BOARDS_PER_GAME,
            jobs,
            shard_index: options.shard_index,
            shard_index_end: options.shard_index_end,
            shard_count: options.shard_count,
        },
        totals,
        opening_coverage,
        tranches,
        trajectories: summaries,
        paired_symmetry,
        decision: TrajectoryAuditDecision {
            passed: failures.is_empty(),
            packed_board_unique_ratio,
            final_tranche_new_packed_boards_per_game: final_tranche_new_boards_per_game,
            final_post_coverage_tranche_new_packed_boards_per_game:
                final_post_coverage_tranche_new_boards_per_game,
            failures,
        },
    })
}

fn trajectory_dataset_label(dataset_name: &str) -> &str {
    match dataset_name {
        "validation" => "ood-v2",
        _ => dataset_name,
    }
}

fn outcome_label(outcome: GameOutcome) -> &'static str {
    match outcome {
        GameOutcome::Draw => "draw",
        GameOutcome::Winner(Color::Black) => "black-win",
        GameOutcome::Winner(Color::White) => "white-win",
    }
}

fn ratio_f64(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn audit_paired_symmetry(trajectories: &[TrajectoryGame]) -> PairedSymmetry {
    let mut by_pair = BTreeMap::<(u8, u32), Vec<&TrajectoryGame>>::new();
    for game in trajectories {
        by_pair
            .entry((
                dataset_sort_key(game.task.dataset_name),
                game.task.game_index / 2,
            ))
            .or_default()
            .push(game);
    }
    let mut exact = 0u64;
    let mut compared = 0u64;
    let mut unpaired_games = 0u64;
    let mut mismatch_examples = Vec::new();
    for games in by_pair.values() {
        let base = games.iter().find(|game| game.task.game_index % 2 == 0);
        let swapped = games.iter().find(|game| game.task.game_index % 2 == 1);
        match (base, swapped) {
            (Some(base), Some(swapped)) => {
                compared += 1;
                let transformed = base
                    .moves
                    .iter()
                    .copied()
                    .map(transform_move)
                    .collect::<Vec<_>>();
                if transformed == swapped.moves
                    && base.opening.opening_id == swapped.opening.opening_id
                {
                    exact += 1;
                } else if mismatch_examples.len() < 16 {
                    mismatch_examples.push(format!(
                        "{} pair {} ({})",
                        trajectory_dataset_label(base.task.dataset_name),
                        base.task.game_index / 2,
                        base.opening.opening_id
                    ));
                }
            }
            (Some(_), None) | (None, Some(_)) => unpaired_games += 1,
            (None, None) => {}
        }
    }
    let pairs_expected = by_pair.len() as u64;
    PairedSymmetry {
        pairs_expected,
        pairs_compared: compared,
        exact_transformed_move_sequence_pairs: exact,
        mismatched_pairs: compared.saturating_sub(exact),
        unpaired_games,
        mismatch_examples,
    }
}

fn audit_trajectory_opening_coverage(
    opening_split: &OpeningSplit,
    trajectories: &[TrajectoryGame],
) -> TrajectoryOpeningCoverage {
    let expected_train = opening_split
        .train_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let expected_ood = opening_split
        .validation_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut pairs = BTreeMap::<(u8, u32), Vec<&TrajectoryGame>>::new();
    let mut unexpected = BTreeSet::new();
    for game in trajectories {
        let expected = if game.task.dataset_name == "train" {
            &expected_train
        } else {
            &expected_ood
        };
        if !expected.contains(&game.opening.opening_id) {
            unexpected.insert(game.opening.opening_id.clone());
        }
        pairs
            .entry((
                dataset_sort_key(game.task.dataset_name),
                game.task.game_index / 2,
            ))
            .or_default()
            .push(game);
    }

    let mut train_pair_counts = BTreeMap::new();
    let mut ood_pair_counts = BTreeMap::new();
    for games in pairs.values() {
        let base = games.iter().find(|game| game.task.game_index % 2 == 0);
        let swapped = games.iter().find(|game| game.task.game_index % 2 == 1);
        let (Some(base), Some(swapped)) = (base, swapped) else {
            continue;
        };
        if base.opening.opening_id != swapped.opening.opening_id {
            continue;
        }
        let counts = if base.task.dataset_name == "train" {
            &mut train_pair_counts
        } else {
            &mut ood_pair_counts
        };
        *counts.entry(base.opening.opening_id.clone()).or_default() += 1;
    }

    let missing_train_ids = expected_train
        .iter()
        .filter(|id| !train_pair_counts.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    let missing_ood_v2_ids = expected_ood
        .iter()
        .filter(|id| !ood_pair_counts.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    let train_ids_below_minimum_pairs = expected_train
        .iter()
        .filter(|id| {
            train_pair_counts.get(*id).copied().unwrap_or_default()
                < TRAJECTORY_MIN_PAIRS_PER_OPENING
        })
        .cloned()
        .collect::<Vec<_>>();
    let ood_v2_ids_below_minimum_pairs = expected_ood
        .iter()
        .filter(|id| {
            ood_pair_counts.get(*id).copied().unwrap_or_default() < TRAJECTORY_MIN_PAIRS_PER_OPENING
        })
        .cloned()
        .collect::<Vec<_>>();
    let unexpected_ids = unexpected.into_iter().collect::<Vec<_>>();
    let complete = missing_train_ids.is_empty()
        && missing_ood_v2_ids.is_empty()
        && train_ids_below_minimum_pairs.is_empty()
        && ood_v2_ids_below_minimum_pairs.is_empty()
        && unexpected_ids.is_empty()
        && expected_train.len() == 52
        && expected_ood.len() == 12
        && expected_train.is_disjoint(&expected_ood);
    TrajectoryOpeningCoverage {
        schedule: "equal-color-swapped-pairs-per-split-v1",
        expected_train_ids: expected_train.into_iter().collect(),
        expected_ood_v2_ids: expected_ood.into_iter().collect(),
        train_pair_counts,
        ood_v2_pair_counts: ood_pair_counts,
        missing_train_ids,
        missing_ood_v2_ids,
        train_ids_below_minimum_pairs,
        ood_v2_ids_below_minimum_pairs,
        minimum_pairs_per_opening: TRAJECTORY_MIN_PAIRS_PER_OPENING,
        unexpected_ids,
        complete,
    }
}

pub fn calibrate_labels(loaded: &LoadedGenerationConfig) -> Result<LabelCalibrationReport> {
    loaded.ruleset_requires_matching_engine()?;
    ensure!(
        loaded
            .config
            .data
            .self_play_move_policy
            .is_searched_stochastic(),
        "calibrate-labels requires data.self_play_move_policy=searched-stochastic-rollout-v1"
    );
    ensure!(
        loaded.config.data.opening_random_plies == 0,
        "calibrate-labels requires data.opening_random_plies=0"
    );
    let opening_sfen = loaded.opening_sfen()?;
    let opening_source = OpeningSource::from_config(loaded, &opening_sfen)?;
    let opening_split = opening_source.split_openings(
        loaded.config.data.split_policy,
        loaded.config.data.split_seed,
        loaded.config.data.train_games,
        loaded.config.data.validation_games,
        loaded.config.data.validation_opening_ids.as_deref(),
        loaded.config.data.validation_opening_schedule,
        loaded.config.data.validation_opening_pairs_per_id,
    )?;
    let mut suite_ids = opening_source.opening_ids();
    suite_ids.sort();
    ensure!(
        suite_ids.len() == 64,
        "calibrate-labels requires exactly 64 opening IDs (found {})",
        suite_ids.len()
    );
    let train_ids = opening_split
        .train_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let ood_ids = opening_split
        .validation_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    ensure!(
        train_ids.len() + ood_ids.len() == suite_ids.len()
            && train_ids.is_disjoint(&ood_ids)
            && suite_ids
                .iter()
                .all(|id| train_ids.contains(id) || ood_ids.contains(id)),
        "calibrate-labels requires the 64 suite IDs to partition into train and OOD-v2 groups"
    );
    ensure!(
        train_ids.len() == 52 && ood_ids.len() == 12,
        "calibrate-labels requires exactly 52 train IDs and 12 OOD-v2 IDs (found {} and {})",
        train_ids.len(),
        ood_ids.len()
    );
    let calibration_tasks = suite_ids
        .iter()
        .enumerate()
        .flat_map(|(ordinal, opening_id)| {
            let dataset_name = if train_ids.contains(opening_id) {
                "train"
            } else {
                "validation"
            };
            let base_index = (ordinal as u32) * 2;
            [
                (
                    TrajectoryTask {
                        dataset_name,
                        game_index: base_index,
                    },
                    opening_id.clone(),
                ),
                (
                    TrajectoryTask {
                        dataset_name,
                        game_index: base_index + 1,
                    },
                    opening_id.clone(),
                ),
            ]
        })
        .collect::<Vec<_>>();
    let teachers = GenerationTeachers::from_config(loaded)?;
    let jobs = resolve_jobs(loaded.config.data.jobs)?
        .min(calibration_tasks.len())
        .max(1);
    let trajectories = collect_calibration_trajectory_games(
        loaded,
        &teachers.trajectory,
        &opening_source,
        &opening_split,
        &calibration_tasks,
        jobs,
    )?;
    let mut trajectories_by_id = BTreeMap::<String, (&TrajectoryGame, &TrajectoryGame)>::new();
    let mut root_hasher = Sha256::new();
    let mut trajectory_hasher = Sha256::new();
    let mut calibration_summaries = Vec::with_capacity(suite_ids.len());
    let mut candidate_root_count = 0u64;
    let mut paired_root_mismatches = 0u64;
    let mut opening_ids_with_matched_roots = Vec::new();
    let mut opening_ids_without_matched_roots = Vec::new();
    for opening_id in &suite_ids {
        let base = trajectories
            .iter()
            .find(|game| game.opening.opening_id == *opening_id && game.task.game_index % 2 == 0)
            .ok_or_else(|| anyhow!("missing base calibration trajectory for `{opening_id}`"))?;
        let swapped = trajectories
            .iter()
            .find(|game| game.opening.opening_id == *opening_id && game.task.game_index % 2 == 1)
            .ok_or_else(|| anyhow!("missing swapped calibration trajectory for `{opening_id}`"))?;
        trajectories_by_id.insert(opening_id.clone(), (base, swapped));
        let roots_match = calibration_roots_match(base, swapped)?
            && (!loaded.config.data.label_retry_policy.is_adaptive()
                || base.calibration_roots.len()
                    == usize::from(
                        loaded
                            .config
                            .data
                            .max_label_attempts_per_game
                            .expect("validated adaptive retry attempt cap"),
                    ));
        if !roots_match {
            paired_root_mismatches += 1;
        }
        if roots_match {
            opening_ids_with_matched_roots.push(opening_id.clone());
        } else {
            opening_ids_without_matched_roots.push(opening_id.clone());
        }
        candidate_root_count +=
            (base.calibration_roots.len() + swapped.calibration_roots.len()) as u64;
        root_hasher.update(base.task.dataset_name.as_bytes());
        root_hasher.update([0]);
        root_hasher.update(opening_id.as_bytes());
        root_hasher.update((base.calibration_roots.len() as u32).to_le_bytes());
        for root in &base.calibration_roots {
            root_hasher.update(root.root_ply.to_le_bytes());
            let sfen = root.board.to_string();
            root_hasher.update((sfen.len() as u32).to_le_bytes());
            root_hasher.update(sfen.as_bytes());
        }
        for game in [base, swapped] {
            trajectory_hasher.update(game.task.dataset_name.as_bytes());
            trajectory_hasher.update([0]);
            trajectory_hasher.update(opening_id.as_bytes());
            trajectory_hasher.update(game.trajectory_sha256.as_bytes());
        }
        calibration_summaries.push(CalibrationTrajectorySummary {
            dataset: trajectory_dataset_label(base.task.dataset_name).to_string(),
            opening_id: opening_id.clone(),
            pair_index: base.task.game_index / 2,
            base_trajectory_sha256: base.trajectory_sha256.clone(),
            swapped_trajectory_sha256: swapped.trajectory_sha256.clone(),
            candidate_roots: (base.calibration_roots.len() + swapped.calibration_roots.len())
                as u64,
        });
    }
    let candidate_roots_sha256 = format!("{:x}", root_hasher.finalize());
    let trajectory_hashes_sha256 = format!("{:x}", trajectory_hasher.finalize());
    let label_budget = loaded.config.data.label_search_budget()?;
    let max_depth = label_budget.max_depth();
    let mut budget_reports = Vec::with_capacity(CALIBRATION_BUDGETS.len());
    for nodes in CALIBRATION_BUDGETS {
        let (train, ood_v2) = if loaded.config.data.label_retry_policy.is_adaptive() {
            let mut train_pairs = Vec::new();
            let mut ood_pairs = Vec::new();
            for opening_id in &suite_ids {
                let (base, swapped) = trajectories_by_id
                    .get(opening_id)
                    .expect("calibration trajectory map contains every suite ID");
                let target = if train_ids.contains(opening_id) {
                    &mut train_pairs
                } else {
                    &mut ood_pairs
                };
                target.push((
                    base.calibration_roots.as_slice(),
                    swapped.calibration_roots.as_slice(),
                ));
            }
            (
                calibrate_adaptive_label_split(
                    &teachers.label,
                    loaded.config.data.position_policy,
                    max_depth,
                    nodes,
                    &train_pairs,
                )?,
                calibrate_adaptive_label_split(
                    &teachers.label,
                    loaded.config.data.position_policy,
                    max_depth,
                    nodes,
                    &ood_pairs,
                )?,
            )
        } else {
            let mut train_roots = Vec::new();
            let mut ood_roots = Vec::new();
            for opening_id in &suite_ids {
                let (base, swapped) = trajectories_by_id
                    .get(opening_id)
                    .expect("calibration trajectory map contains every suite ID");
                let target = if train_ids.contains(opening_id) {
                    &mut train_roots
                } else {
                    &mut ood_roots
                };
                target.extend(base.calibration_roots.iter());
                target.extend(swapped.calibration_roots.iter());
            }
            (
                calibrate_label_split(
                    &teachers.label,
                    loaded.config.data.position_policy,
                    max_depth,
                    nodes,
                    &train_roots,
                )?,
                calibrate_label_split(
                    &teachers.label,
                    loaded.config.data.position_policy,
                    max_depth,
                    nodes,
                    &ood_roots,
                )?,
            )
        };
        let attempts_per_accepted_root = ratio_f64(
            train.candidate_roots.saturating_add(ood_v2.candidate_roots),
            train.accepted_roots.saturating_add(ood_v2.accepted_roots),
        );
        let passed = train.passed
            && ood_v2.passed
            && (!loaded.config.data.label_retry_policy.is_adaptive()
                || attempts_per_accepted_root <= CALIBRATION_MAX_OVERALL_ATTEMPTS_PER_ACCEPT);
        let retryable_failure = train.incomplete_labels > 0
            || ood_v2.incomplete_labels > 0
            || train.node_accounting_errors > 0
            || ood_v2.node_accounting_errors > 0;
        budget_reports.push(CalibrationBudgetReport {
            nodes,
            attempts_per_accepted_root,
            passed,
            train,
            ood_v2,
        });
        if loaded.config.data.label_retry_policy.is_adaptive() && (passed || !retryable_failure) {
            break;
        }
    }
    let mut failures = Vec::new();
    if !opening_ids_without_matched_roots.is_empty() {
        failures.push(if loaded.config.data.label_retry_policy.is_adaptive() {
            format!(
                "{} of 64 opening IDs did not produce the full matched {}-attempt candidate pool: {}",
                opening_ids_without_matched_roots.len(),
                loaded
                    .config
                    .data
                    .max_label_attempts_per_game
                    .expect("validated adaptive retry attempt cap"),
                opening_ids_without_matched_roots.join(", ")
            )
        } else {
            format!(
                "{} of 64 opening IDs did not produce at least one matched candidate root: {}",
                opening_ids_without_matched_roots.len(),
                opening_ids_without_matched_roots.join(", ")
            )
        });
    }
    if paired_root_mismatches > 0 {
        failures.push(format!(
            "{paired_root_mismatches} color-swapped calibration pairs did not have identical transformed candidate roots"
        ));
    }
    let selected_budget_nodes = if failures.is_empty() {
        budget_reports
            .iter()
            .find(|report| report.passed)
            .map(|report| report.nodes)
    } else {
        None
    };
    if selected_budget_nodes.is_none()
        && opening_ids_without_matched_roots.is_empty()
        && paired_root_mismatches == 0
    {
        failures.push(if loaded.config.data.label_retry_policy.is_adaptive() {
            "adaptive retry did not satisfy accepted-root, exhaustion, node-accounting, and attempts-per-accept gates at an eligible predeclared budget"
                .to_string()
        } else {
            "none of the predeclared 50k/100k/200k node budgets passed both train and OOD-v2 calibration gates; define an adaptive-retry contract before proceeding"
                .to_string()
        });
    }
    let status = if failures.is_empty() {
        format!("passed; selected smallest budget {selected_budget_nodes:?}")
    } else {
        "blocked; no production label budget was selected".to_string()
    };
    let engine_revision = detect_git_revision(loaded)?;
    Ok(LabelCalibrationReport {
        schema: LABEL_CALIBRATION_SCHEMA,
        source: trajectory_source_identity(
            loaded,
            &teachers.trajectory,
            &opening_source,
            &opening_split,
            engine_revision,
        )?,
        policy: trajectory_policy_identity(loaded),
        calibration: LabelCalibrationIdentity {
            games: trajectories.len() as u64,
            suite_ids,
            train_suite_ids: train_ids.into_iter().collect(),
            ood_v2_suite_ids: ood_ids.into_iter().collect(),
            candidate_root_schedule: if loaded.config.data.label_retry_policy.is_adaptive() {
                "pair-coupled-adaptive-retry-v1".to_string()
            } else {
                "fixed-phase-calibration-v1".to_string()
            },
            candidate_root_count,
            candidate_roots_sha256,
            paired_root_mismatches,
            opening_ids_with_matched_roots,
            opening_ids_without_matched_roots,
            trajectory_hashes_sha256,
            label_position_policy: loaded
                .config
                .data
                .position_policy
                .manifest_name()
                .to_string(),
            label_retry_policy: loaded
                .config
                .data
                .label_retry_policy
                .manifest_name()
                .to_string(),
            max_label_attempts_per_game: loaded.config.data.max_label_attempts_per_game,
            max_depth,
            max_rejection_rate_delta: CALIBRATION_MAX_BIAS_DELTA,
            rejection_rate_delta_is_gate: !loaded.config.data.label_retry_policy.is_adaptive(),
            max_overall_attempts_per_accepted_root: CALIBRATION_MAX_OVERALL_ATTEMPTS_PER_ACCEPT,
            max_split_attempts_per_accepted_root: CALIBRATION_MAX_SPLIT_ATTEMPTS_PER_ACCEPT,
        },
        trajectories: calibration_summaries,
        budgets: budget_reports,
        decision: LabelCalibrationDecision {
            passed: failures.is_empty(),
            selected_budget_nodes,
            status,
            failures,
        },
    })
}

fn collect_calibration_trajectory_games(
    loaded: &LoadedConfig,
    teacher: &Teacher,
    opening_source: &OpeningSource,
    opening_split: &OpeningSplit,
    tasks: &[(TrajectoryTask, String)],
    jobs: usize,
) -> Result<Vec<TrajectoryGame>> {
    let queue = Arc::new(Mutex::new(VecDeque::from(tasks.to_vec())));
    let results = Arc::new(Mutex::new(Vec::<(
        TrajectoryTask,
        std::result::Result<TrajectoryGame, String>,
    )>::new()));
    thread::scope(|scope| {
        for _ in 0..jobs.min(tasks.len()).max(1) {
            let queue = Arc::clone(&queue);
            let results = Arc::clone(&results);
            scope.spawn(move || {
                let mut workspace = SearchWorkspace::default();
                loop {
                    let Some((task, opening_id)) = queue
                        .lock()
                        .expect("calibration queue poisoned")
                        .pop_front()
                    else {
                        break;
                    };
                    let result = play_label_free_trajectory(
                        loaded,
                        teacher,
                        opening_source,
                        opening_split,
                        task,
                        Some(&opening_id),
                        true,
                        &mut workspace,
                    )
                    .map_err(|err| format!("{err:#}"));
                    let failed = result.is_err();
                    results
                        .lock()
                        .expect("calibration results poisoned")
                        .push((task, result));
                    if failed {
                        break;
                    }
                }
            });
        }
    });
    let mut results = Arc::try_unwrap(results)
        .map_err(|_| anyhow!("calibration workers still hold their result queue"))?
        .into_inner()
        .map_err(|_| anyhow!("calibration result queue was poisoned"))?;
    results.sort_by_key(|(task, _)| (dataset_sort_key(task.dataset_name), task.game_index));
    let mut trajectories = Vec::with_capacity(results.len());
    for (task, result) in results {
        trajectories.push(result.map_err(|err| anyhow!(err)).with_context(|| {
            format!(
                "failed to generate {} calibration trajectory game {}",
                task.dataset_name, task.game_index
            )
        })?);
    }
    ensure!(
        trajectories.len() == tasks.len(),
        "calibration workers returned {}/{} games",
        trajectories.len(),
        tasks.len()
    );
    Ok(trajectories)
}

fn calibration_roots_match(base: &TrajectoryGame, swapped: &TrajectoryGame) -> Result<bool> {
    calibration_root_slices_match(&base.calibration_roots, &swapped.calibration_roots)
}

fn calibration_root_slices_match(
    base_roots: &[CalibrationRoot],
    swapped_roots: &[CalibrationRoot],
) -> Result<bool> {
    if base_roots.is_empty() || base_roots.len() != swapped_roots.len() {
        return Ok(false);
    }
    for (base_root, swapped_root) in base_roots.iter().zip(swapped_roots) {
        if base_root.root_ply != swapped_root.root_ply
            || color_swap_anhoku_sfen(&base_root.board.to_string())?
                != swapped_root.board.to_string()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Debug)]
struct CalibrationSearchObservation {
    incomplete: bool,
    terminal: bool,
    mate: bool,
    accounting_error: bool,
    alpha_beta_nodes: u64,
    qsearch_nodes: u64,
    accounted_nodes: u64,
}

impl CalibrationSearchObservation {
    fn is_admissible(&self) -> bool {
        !self.incomplete && !self.terminal && !self.mate
    }
}

fn observe_calibration_label(
    teacher: &Teacher,
    position_policy: PositionPolicy,
    max_depth: u8,
    nodes: u64,
    root: &CalibrationRoot,
    workspace: &mut SearchWorkspace,
) -> Result<CalibrationSearchObservation> {
    let summary = teacher.search_label(
        &root.board,
        LabelSearchBudget::Nodes { nodes, max_depth },
        position_policy,
        workspace,
    )?;
    let counter_sum = summary.states.checked_add(summary.qnodes);
    let terminal = !has_both_kings(&root.board)
        || root.board.status() != haitaka::GameStatus::Ongoing
        || summary
            .training_trace
            .as_ref()
            .is_some_and(|trace| trace.terminal || !has_both_kings(&trace.leaf_board));
    Ok(CalibrationSearchObservation {
        incomplete: !node_budget_summary_is_complete(&summary),
        terminal,
        mate: summary
            .best_score
            .is_some_and(|score| score.unsigned_abs() >= SEARCH_MATE_SCORE_THRESHOLD as u32),
        accounting_error: summary.node_limit != Some(nodes)
            || counter_sum != Some(summary.total_nodes)
            || summary.total_nodes > nodes,
        alpha_beta_nodes: summary.states,
        qsearch_nodes: summary.qnodes,
        accounted_nodes: summary.total_nodes,
    })
}

fn calibrate_adaptive_label_split(
    teacher: &Teacher,
    position_policy: PositionPolicy,
    max_depth: u8,
    nodes: u64,
    root_pairs: &[(&[CalibrationRoot], &[CalibrationRoot])],
) -> Result<CalibrationSplitReport> {
    let mut workspace = SearchWorkspace::default();
    let mut candidate_roots = 0u64;
    let mut accepted_roots = 0u64;
    let mut exhausted_games = 0u64;
    let mut incomplete_labels = 0u64;
    let mut terminal_labels = 0u64;
    let mut mate_labels = 0u64;
    let mut alpha_beta_nodes = 0u64;
    let mut qsearch_nodes = 0u64;
    let mut accounted_nodes = 0u64;
    let mut node_accounting_errors = 0u64;
    let mut incomplete_by_side = BinaryCalibrationCounts::default();
    let mut rejected_by_side = BinaryCalibrationCounts::default();
    let mut incomplete_by_outcome = OutcomeCalibrationCounts::default();
    let mut rejected_by_outcome = OutcomeCalibrationCounts::default();
    let mut candidate_by_side = BinaryCalibrationCounts::default();
    let mut candidate_by_outcome = OutcomeCalibrationCounts::default();

    for (base_roots, swapped_roots) in root_pairs {
        let mut pair_accepted = false;
        for (base, swapped) in base_roots.iter().zip(*swapped_roots) {
            let roots = [base, swapped];
            let mut observations = Vec::with_capacity(2);
            for root in roots {
                candidate_roots += 1;
                increment_binary(&mut candidate_by_side, root.side_to_move);
                increment_outcome(
                    &mut candidate_by_outcome,
                    root.outcome.relative_to(root.side_to_move),
                );
                let observation = observe_calibration_label(
                    teacher,
                    position_policy,
                    max_depth,
                    nodes,
                    root,
                    &mut workspace,
                )?;
                alpha_beta_nodes = alpha_beta_nodes.saturating_add(observation.alpha_beta_nodes);
                qsearch_nodes = qsearch_nodes.saturating_add(observation.qsearch_nodes);
                accounted_nodes = accounted_nodes.saturating_add(observation.accounted_nodes);
                node_accounting_errors += u64::from(observation.accounting_error);
                incomplete_labels += u64::from(observation.incomplete);
                terminal_labels += u64::from(observation.terminal);
                mate_labels += u64::from(observation.mate);
                if observation.incomplete {
                    increment_binary(&mut incomplete_by_side, root.side_to_move);
                    increment_outcome(
                        &mut incomplete_by_outcome,
                        root.outcome.relative_to(root.side_to_move),
                    );
                }
                observations.push(observation);
            }
            if observations
                .iter()
                .all(CalibrationSearchObservation::is_admissible)
            {
                accepted_roots += 2;
                pair_accepted = true;
                break;
            }
            for root in roots {
                increment_binary(&mut rejected_by_side, root.side_to_move);
                increment_outcome(
                    &mut rejected_by_outcome,
                    root.outcome.relative_to(root.side_to_move),
                );
            }
        }
        if !pair_accepted {
            exhausted_games += 2;
        }
    }

    let rejected_labels = candidate_roots.saturating_sub(accepted_roots);
    let attempts_per_accepted_root = ratio_f64(candidate_roots, accepted_roots);
    let requested_node_budget = nodes.saturating_mul(candidate_roots);
    let side_rejection_rate_delta = binary_rate_delta(&rejected_by_side, &candidate_by_side);
    let outcome_rejection_rate_delta =
        outcome_rate_delta(&rejected_by_outcome, &candidate_by_outcome);
    let expected_accepted_roots = (root_pairs.len() as u64).saturating_mul(2);
    let passed = accepted_roots == expected_accepted_roots
        && exhausted_games == 0
        && node_accounting_errors == 0
        && attempts_per_accepted_root <= CALIBRATION_MAX_SPLIT_ATTEMPTS_PER_ACCEPT;
    Ok(CalibrationSplitReport {
        candidate_roots,
        accepted_roots,
        exhausted_games,
        attempts_per_accepted_root,
        accepted_bad_labels: 0,
        incomplete_labels,
        incomplete_rate: ratio_f64(incomplete_labels, candidate_roots),
        terminal_labels,
        mate_labels,
        rejected_labels,
        alpha_beta_nodes,
        qsearch_nodes,
        accounted_nodes,
        requested_node_budget,
        node_accounting_exact: node_accounting_errors == 0,
        node_accounting_errors,
        incomplete_by_side,
        rejected_by_side,
        incomplete_by_outcome,
        rejected_by_outcome,
        side_rejection_rate_delta,
        outcome_rejection_rate_delta,
        passed,
    })
}

fn calibrate_label_split(
    teacher: &Teacher,
    position_policy: PositionPolicy,
    max_depth: u8,
    nodes: u64,
    roots: &[&CalibrationRoot],
) -> Result<CalibrationSplitReport> {
    let mut workspace = SearchWorkspace::default();
    let mut incomplete_labels = 0u64;
    let mut terminal_labels = 0u64;
    let mut mate_labels = 0u64;
    let mut alpha_beta_nodes = 0u64;
    let mut qsearch_nodes = 0u64;
    let mut accounted_nodes = 0u64;
    let mut node_accounting_errors = 0u64;
    let mut incomplete_by_side = BinaryCalibrationCounts::default();
    let mut rejected_by_side = BinaryCalibrationCounts::default();
    let mut incomplete_by_outcome = OutcomeCalibrationCounts::default();
    let mut rejected_by_outcome = OutcomeCalibrationCounts::default();
    let mut candidate_by_side = BinaryCalibrationCounts::default();
    let mut candidate_by_outcome = OutcomeCalibrationCounts::default();
    for root in roots {
        increment_binary(&mut candidate_by_side, root.side_to_move);
        increment_outcome(
            &mut candidate_by_outcome,
            root.outcome.relative_to(root.side_to_move),
        );
        let summary = teacher.search_label(
            &root.board,
            LabelSearchBudget::Nodes { nodes, max_depth },
            position_policy,
            &mut workspace,
        )?;
        alpha_beta_nodes = alpha_beta_nodes.saturating_add(summary.states);
        qsearch_nodes = qsearch_nodes.saturating_add(summary.qnodes);
        accounted_nodes = accounted_nodes.saturating_add(summary.total_nodes);
        let counter_sum = summary.states.checked_add(summary.qnodes);
        if summary.node_limit != Some(nodes)
            || counter_sum != Some(summary.total_nodes)
            || summary.total_nodes > nodes
        {
            node_accounting_errors += 1;
        }
        let terminal = summary
            .training_trace
            .as_ref()
            .is_some_and(|trace| trace.terminal || !has_both_kings(&trace.leaf_board));
        let mate = summary
            .best_score
            .is_some_and(|score| score.unsigned_abs() >= SEARCH_MATE_SCORE_THRESHOLD as u32);
        terminal_labels += u64::from(terminal);
        mate_labels += u64::from(mate);
        let incomplete = !node_budget_summary_is_complete(&summary);
        incomplete_labels += u64::from(incomplete);
        if incomplete {
            increment_binary(&mut incomplete_by_side, root.side_to_move);
            increment_outcome(
                &mut incomplete_by_outcome,
                root.outcome.relative_to(root.side_to_move),
            );
        }
        let rejected = incomplete || terminal || mate;
        if rejected {
            increment_binary(&mut rejected_by_side, root.side_to_move);
            increment_outcome(
                &mut rejected_by_outcome,
                root.outcome.relative_to(root.side_to_move),
            );
        }
    }
    let rejected_labels = rejected_by_side
        .black
        .saturating_add(rejected_by_side.white);
    let candidate_count = roots.len() as u64;
    let requested_node_budget = nodes.saturating_mul(candidate_count);
    let side_rejection_rate_delta = binary_rate_delta(&rejected_by_side, &candidate_by_side);
    let outcome_rejection_rate_delta =
        outcome_rate_delta(&rejected_by_outcome, &candidate_by_outcome);
    let passed = candidate_count > 0
        && incomplete_labels.saturating_mul(100)
            <= candidate_count.saturating_mul(CALIBRATION_MAX_INCOMPLETE_PERCENT)
        && terminal_labels == 0
        && mate_labels == 0
        && node_accounting_errors == 0
        && side_rejection_rate_delta <= CALIBRATION_MAX_BIAS_DELTA
        && outcome_rejection_rate_delta <= CALIBRATION_MAX_BIAS_DELTA;
    Ok(CalibrationSplitReport {
        candidate_roots: candidate_count,
        accepted_roots: candidate_count.saturating_sub(rejected_labels),
        exhausted_games: 0,
        attempts_per_accepted_root: ratio_f64(
            candidate_count,
            candidate_count.saturating_sub(rejected_labels),
        ),
        accepted_bad_labels: 0,
        incomplete_labels,
        incomplete_rate: ratio_f64(incomplete_labels, candidate_count),
        terminal_labels,
        mate_labels,
        rejected_labels,
        alpha_beta_nodes,
        qsearch_nodes,
        accounted_nodes,
        requested_node_budget,
        node_accounting_exact: node_accounting_errors == 0,
        node_accounting_errors,
        incomplete_by_side,
        rejected_by_side,
        incomplete_by_outcome,
        rejected_by_outcome,
        side_rejection_rate_delta,
        outcome_rejection_rate_delta,
        passed,
    })
}

fn increment_binary(counts: &mut BinaryCalibrationCounts, side: Color) {
    match side {
        Color::Black => counts.black += 1,
        Color::White => counts.white += 1,
    }
}

fn increment_outcome(counts: &mut OutcomeCalibrationCounts, result: i8) {
    match result {
        1 => counts.win += 1,
        -1 => counts.loss += 1,
        0 => counts.draw += 1,
        _ => {}
    }
}

fn rejection_rate(rejected: u64, candidates: u64) -> f64 {
    ratio_f64(rejected, candidates)
}

fn binary_rate_delta(
    rejected: &BinaryCalibrationCounts,
    candidates: &BinaryCalibrationCounts,
) -> f64 {
    (rejection_rate(rejected.black, candidates.black)
        - rejection_rate(rejected.white, candidates.white))
    .abs()
}

fn outcome_rate_delta(
    rejected: &OutcomeCalibrationCounts,
    candidates: &OutcomeCalibrationCounts,
) -> f64 {
    let rates = [
        (rejected.win, candidates.win),
        (rejected.loss, candidates.loss),
        (rejected.draw, candidates.draw),
    ]
    .into_iter()
    .filter(|(_, candidates)| *candidates > 0)
    .map(|(rejected, candidates)| rejection_rate(rejected, candidates))
    .collect::<Vec<_>>();
    match (
        rates.iter().copied().min_by(f64::total_cmp),
        rates.iter().copied().max_by(f64::total_cmp),
    ) {
        (Some(min), Some(max)) => max - min,
        _ => 0.0,
    }
}

pub fn write_trajectory_audit_report(
    loaded: &LoadedConfig,
    report: &TrajectoryAuditReport,
    output: Option<PathBuf>,
) -> Result<PathBuf> {
    write_json_report(loaded, report, output, "trajectory-audit.json")
}

pub fn write_label_calibration_report(
    loaded: &LoadedConfig,
    report: &LabelCalibrationReport,
    output: Option<PathBuf>,
) -> Result<PathBuf> {
    write_json_report(loaded, report, output, "label-calibration.json")
}

fn write_json_report<T: Serialize>(
    loaded: &LoadedConfig,
    report: &T,
    output: Option<PathBuf>,
    default_name: &str,
) -> Result<PathBuf> {
    let artifacts = loaded.artifact_paths();
    let path = output
        .map(|path| loaded.resolve_path(&path))
        .unwrap_or_else(|| artifacts.artifacts_dir.join(default_name));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut bytes = serde_json::to_vec_pretty(report)?;
    bytes.push(b'\n');
    fs::write(&path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

fn node_budget_summary_is_complete(summary: &TeacherSearchSummary) -> bool {
    summary.best_move.is_some() && summary.best_score.is_some()
}

fn label_node_accounting_is_exact(
    summary: &TeacherSearchSummary,
    budget: LabelSearchBudget,
) -> bool {
    match budget {
        LabelSearchBudget::Depth { .. } => true,
        LabelSearchBudget::Nodes { nodes, .. } => {
            summary.node_limit == Some(nodes)
                && summary.states.checked_add(summary.qnodes) == Some(summary.total_nodes)
                && summary.total_nodes <= nodes
        }
    }
}

fn apply_incomplete_label_policy(
    summary: TeacherSearchSummary,
    budget: LabelSearchBudget,
    policy: IncompleteLabelPolicy,
    root_side: Color,
    root_ply: u16,
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
            *stats
                .rejected_incomplete_root_plies
                .entry(root_ply)
                .or_default() += 1;
            Ok(None)
        }
    }
}

#[cfg(test)]
pub fn generate_data(loaded: &LoadedGenerationConfig) -> Result<DatasetOutput> {
    generate_data_with_options(loaded, GenerateOptions::from_config(loaded))
}

pub fn generate_data_with_options(
    loaded: &LoadedGenerationConfig,
    options: GenerateOptions,
) -> Result<DatasetOutput> {
    ensure!(
        !options.ignore_identity_mismatch,
        "strict R0 generation forbids --ignore-identity-mismatch; import foreign shards through an explicitly historical, non-decisional workflow"
    );
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
        loaded.config.data.validation_opening_ids.as_deref(),
        loaded.config.data.validation_opening_schedule,
        loaded.config.data.validation_opening_pairs_per_id,
    )?;

    let teachers = GenerationTeachers::from_config(loaded)?;
    let artifacts = loaded.artifact_paths();
    artifacts.ensure_dirs()?;

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
        &teachers,
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
        &teachers,
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
    if shard_selector.is_full_run() {
        let artifacts = loaded.artifact_paths();
        ensure_minimum_train_boards(
            loaded,
            &artifacts.train_bin,
            &artifacts.train_manifest,
            train_positions,
        )?;
    }
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
        &teachers,
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

    crate::r0::write_generation_manifest(loaded, &artifacts)?;
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
    teachers: &GenerationTeachers,
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
            let teachers = teachers.clone();
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
                        &teachers,
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
    let mut candidate_identity_hasher = Sha256::new();
    for result in &shard_results {
        candidate_identity_hasher.update(result.manifest.game_start.to_le_bytes());
        candidate_identity_hasher.update(result.manifest.candidate_identity_sha256.as_bytes());
    }
    let candidate_identity_sha256 = format!("{:x}", candidate_identity_hasher.finalize());
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
    let build_mode = generation_teacher_build_mode(loaded, teachers);
    let generation_semantic_identity_sha256 = generation_semantic_identity_sha256(
        loaded,
        dataset_name,
        opening_sfen,
        opening_source,
        opening_split,
        &build_mode,
        generation_teacher_sha256(teachers).as_deref(),
        engine_revision.as_deref(),
    )?;
    let schedule_identity_sha256 =
        schedule_identity_sha256(loaded, dataset_name, game_count, None)?;

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
        validation_opening_schedule: opening_split
            .validation_schedule
            .manifest_name()
            .to_string(),
        validation_opening_pairs_per_id: opening_split.validation_pairs_per_id,
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
        label_retry_policy: loaded
            .config
            .data
            .label_retry_policy
            .manifest_name()
            .to_string(),
        max_label_attempts_per_game: loaded.config.data.max_label_attempts_per_game,
        position_selection_audit_version: POSITION_SELECTION_AUDIT_VERSION.to_string(),
        candidate_positions: search_stats.candidate_positions,
        candidate_roots_per_game: loaded.config.data.max_candidate_roots_per_game,
        candidate_identity_version: CANDIDATE_IDENTITY_VERSION.to_string(),
        candidate_identity_sha256,
        generation_semantic_identity_version: GENERATION_SEMANTIC_IDENTITY_VERSION.to_string(),
        generation_semantic_identity_sha256,
        schedule_identity_version: SCHEDULE_IDENTITY_VERSION.to_string(),
        schedule_identity_sha256,
        minimum_train_boards: loaded.config.data.minimum_train_boards()?,
        minimum_train_positions: loaded.config.data.minimum_train_positions,
        rejected_incomplete_label_positions: search_stats.rejected_incomplete_label_positions,
        rejected_terminal_positions: search_stats.rejected_terminal_positions,
        rejected_mate_score_positions: search_stats.rejected_mate_score_positions,
        rejected_node_accounting_positions: search_stats.rejected_node_accounting_positions,
        label_retry_exhausted_games: search_stats.label_retry_exhausted_games,
        label_retry_attempts_per_accepted_position: ratio_f64(
            search_stats.candidate_positions,
            stored_position_count(&search_stats),
        ),
        rejected_incomplete_root_plies: search_stats.rejected_incomplete_root_plies.clone(),
        rejected_terminal_root_plies: search_stats.rejected_terminal_root_plies.clone(),
        rejected_mate_root_plies: search_stats.rejected_mate_root_plies.clone(),
        rejected_node_accounting_root_plies: search_stats
            .rejected_node_accounting_root_plies
            .clone(),
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
        rollout_candidate_limit: loaded.config.data.rollout_candidate_limit,
        rollout_score_margin: loaded.config.data.rollout_score_margin,
        rollout_temperature: loaded.config.data.rollout_temperature,
        rollout_rng_version: loaded.config.data.rollout_rng_version.clone(),
        label_searches: search_stats.label_searches,
        rollout_searches: search_stats.rollout_searches,
        label_search_states: search_stats.label_search_states,
        label_search_qnodes: search_stats.label_search_qnodes,
        label_search_total_nodes,
        label_nodes_per_search,
        rollout_search_states: search_stats.rollout_search_states,
        rollout_search_qnodes: search_stats.rollout_search_qnodes,
        rollout_decisions: search_stats.rollout_decisions,
        rollout_legal_moves: search_stats.rollout_legal_moves,
        rollout_candidates_scored: search_stats.rollout_candidates_scored,
        rollout_candidates_truncated: search_stats.rollout_candidates_truncated,
        rollout_near_best_candidates: search_stats.rollout_near_best_candidates,
        rollout_selected_score_gap_sum: search_stats.rollout_selected_score_gap_sum,
        rollout_selected_score_gap_max: search_stats.rollout_selected_score_gap_max,
        label_search_cpu_seconds: search_stats.label_search_elapsed_seconds,
        rollout_search_cpu_seconds: search_stats.rollout_search_elapsed_seconds,
        generation_cpu_seconds: search_stats.label_search_elapsed_seconds
            + search_stats.rollout_search_elapsed_seconds,
        bootstrap_nnue: None,
        bootstrap_nnue_sha256: generation_teacher_sha256(teachers),
        engine_revision: engine_revision.clone(),
        config_hash: loaded.hash_hex.clone(),
        seed: loaded.config.data.seed,
        feature_family: loaded.record_format().to_string(),
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
        build_mode,
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
    loaded: &LoadedGenerationConfig,
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
    let teachers = GenerationTeachers::from_config(loaded)?;
    let engine_revision = detect_git_revision(loaded)?;
    let opening_split = opening_source.split_openings(
        loaded.config.data.split_policy,
        loaded.config.data.split_seed,
        loaded.config.data.train_games,
        loaded.config.data.validation_games,
        loaded.config.data.validation_opening_ids.as_deref(),
        loaded.config.data.validation_opening_schedule,
        loaded.config.data.validation_opening_pairs_per_id,
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
        &teachers,
        &engine_revision,
        generated_at_unix_ms,
        ignore_identity_mismatch,
    )?;
    ensure_minimum_train_boards(
        loaded,
        &artifacts.train_bin,
        &artifacts.train_manifest,
        train_positions,
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
        &teachers,
        &engine_revision,
        generated_at_unix_ms,
        ignore_identity_mismatch,
    )?;
    crate::r0::write_generation_manifest(loaded, &artifacts)?;

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

    fn is_full_run(self) -> bool {
        self.count == 1 && self.index == 0 && self.index_end == 0
    }
}

fn ensure_minimum_train_boards(
    loaded: &LoadedConfig,
    train_bin: &Path,
    train_manifest: &Path,
    train_positions: u64,
) -> Result<()> {
    let Some(minimum) = loaded.config.data.minimum_train_boards()? else {
        return Ok(());
    };
    let audit = crate::dataset_audit::audit_dataset(train_bin, train_manifest, None).with_context(
        || {
            format!(
                "failed to audit the training dataset before applying the {minimum}-board minimum"
            )
        },
    )?;
    let distinct_boards = audit.distinct_packed_boards();
    ensure!(
        distinct_boards >= minimum,
        "training dataset contains {distinct_boards} distinct packed boards ({train_positions} accepted records), below the configured minimum of {minimum}; do not start training"
    );
    Ok(())
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct MergeGenerationIdentity {
    version: String,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct GenerationSemanticIdentityMaterial {
    schema: &'static str,
    dataset: String,
    ruleset: Ruleset,
    rule_id: u16,
    opening_sfen: String,
    opening_policy: String,
    opening_suite_id: Option<String>,
    opening_suite_sha256: Option<String>,
    opening_transformation: String,
    split_policy: String,
    split_seed: u64,
    train_opening_ids: Vec<String>,
    validation_opening_ids: Vec<String>,
    validation_opening_schedule: String,
    validation_opening_pairs_per_id: Option<u32>,
    seed: u64,
    max_plies: u16,
    opening_random_plies: u16,
    sample_start_ply: u16,
    sample_every_ply: u16,
    sampling_phase: String,
    sample_after_opening: bool,
    label_search_budget: String,
    label_search_nodes: Option<u64>,
    label_search_max_depth: u8,
    node_counting_version: &'static str,
    position_policy: String,
    training_trace_version: &'static str,
    incomplete_label_policy: String,
    label_retry_policy: String,
    max_positions_per_game: u16,
    max_candidate_roots_per_game: Option<u16>,
    max_label_attempts_per_game: Option<u16>,
    candidate_identity_version: &'static str,
    self_play_move_policy: String,
    rollout_search_depth: u8,
    rollout_candidate_limit: u16,
    rollout_score_margin: i32,
    rollout_temperature: f64,
    rollout_rng_version: String,
    feature_family: String,
    teacher_build_mode: String,
    teacher_sha256: Option<String>,
    engine_revision: Option<String>,
    teacher_move_encoding: &'static str,
    entry_bytes: usize,
}

#[derive(Debug, Serialize)]
struct ScheduleIdentityMaterial {
    schema: &'static str,
    dataset: String,
    train_games: u32,
    validation_games: u32,
    shard_games: u32,
    requested_game_count: u32,
    minimum_train_boards: Option<u64>,
    shard_index: Option<u32>,
    game_start: Option<u32>,
    game_count: Option<u32>,
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

#[allow(clippy::too_many_arguments)]
fn generation_semantic_identity_sha256(
    loaded: &LoadedConfig,
    dataset_name: &str,
    opening_sfen: &str,
    opening_source: &OpeningSource,
    opening_split: &OpeningSplit,
    teacher_build_mode: &str,
    teacher_sha256: Option<&str>,
    engine_revision: Option<&str>,
) -> Result<String> {
    let label_budget = loaded.config.data.label_search_budget()?;
    let material = GenerationSemanticIdentityMaterial {
        schema: GENERATION_SEMANTIC_IDENTITY_VERSION,
        dataset: dataset_name.to_string(),
        ruleset: loaded.config.rules.ruleset,
        rule_id: loaded.effective_rule_id()?,
        opening_sfen: opening_sfen.to_string(),
        opening_policy: opening_source.policy().to_string(),
        opening_suite_id: opening_source.suite_id().map(str::to_string),
        opening_suite_sha256: opening_source.suite_sha256().map(str::to_string),
        opening_transformation: opening_source.transformation().to_string(),
        split_policy: loaded.config.data.split_policy.manifest_name().to_string(),
        split_seed: loaded.config.data.split_seed,
        train_opening_ids: opening_split.train_ids.clone(),
        validation_opening_ids: opening_split.validation_ids.clone(),
        validation_opening_schedule: opening_split
            .validation_schedule
            .manifest_name()
            .to_string(),
        validation_opening_pairs_per_id: opening_split.validation_pairs_per_id,
        seed: loaded.config.data.seed,
        max_plies: loaded.config.data.max_plies,
        opening_random_plies: loaded.config.data.opening_random_plies,
        sample_start_ply: loaded.config.data.sample_start_ply,
        sample_every_ply: loaded.config.data.sample_every_ply,
        sampling_phase: loaded
            .config
            .data
            .sampling_policy
            .manifest_name()
            .to_string(),
        sample_after_opening: loaded.config.data.sampling_policy.samples_after_opening(),
        label_search_budget: label_budget.manifest_name().to_string(),
        label_search_nodes: label_budget.nodes(),
        label_search_max_depth: label_budget.max_depth(),
        node_counting_version: SEARCH_NODE_COUNTING_VERSION,
        position_policy: loaded
            .config
            .data
            .position_policy
            .manifest_name()
            .to_string(),
        training_trace_version: SEARCH_TRAINING_TRACE_VERSION,
        incomplete_label_policy: loaded
            .config
            .data
            .incomplete_label_policy
            .manifest_name()
            .to_string(),
        label_retry_policy: loaded
            .config
            .data
            .label_retry_policy
            .manifest_name()
            .to_string(),
        max_positions_per_game: loaded.config.data.max_positions_per_game,
        max_candidate_roots_per_game: loaded.config.data.max_candidate_roots_per_game,
        max_label_attempts_per_game: loaded.config.data.max_label_attempts_per_game,
        candidate_identity_version: CANDIDATE_IDENTITY_VERSION,
        self_play_move_policy: loaded
            .config
            .data
            .self_play_move_policy
            .manifest_name()
            .to_string(),
        rollout_search_depth: loaded.config.data.rollout_search_depth,
        rollout_candidate_limit: loaded.config.data.rollout_candidate_limit,
        rollout_score_margin: loaded.config.data.rollout_score_margin,
        rollout_temperature: loaded.config.data.rollout_temperature,
        rollout_rng_version: loaded.config.data.rollout_rng_version.clone(),
        feature_family: loaded.record_format().to_string(),
        teacher_build_mode: teacher_build_mode.to_string(),
        teacher_sha256: teacher_sha256.map(str::to_string),
        engine_revision: engine_revision.map(str::to_string),
        teacher_move_encoding: TEACHER_MOVE_ENCODING,
        entry_bytes: ENTRY_BYTES,
    };
    Ok(hash_bytes_hex(&serde_json::to_vec(&material)?))
}

fn schedule_identity_sha256(
    loaded: &LoadedConfig,
    dataset_name: &str,
    requested_game_count: u32,
    plan: Option<ShardPlan>,
) -> Result<String> {
    let material = ScheduleIdentityMaterial {
        schema: SCHEDULE_IDENTITY_VERSION,
        dataset: dataset_name.to_string(),
        train_games: loaded.config.data.train_games,
        validation_games: loaded.config.data.validation_games,
        shard_games: loaded.config.data.shard_games,
        requested_game_count,
        minimum_train_boards: loaded.config.data.minimum_train_boards()?,
        shard_index: plan.map(|plan| plan.shard_index),
        game_start: plan.map(|plan| plan.game_start),
        game_count: plan.map(|plan| plan.game_count),
    };
    Ok(hash_bytes_hex(&serde_json::to_vec(&material)?))
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
    teachers: &GenerationTeachers,
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
            teachers,
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
    let mut search_workspaces = GenerationSearchWorkspaces::default();
    let mut candidate_identity_hasher = Sha256::new();

    for game_index in plan.game_start..plan.game_start + plan.game_count {
        let shard_index = plan.shard_index;
        let error_context =
            format!("failed to generate {dataset_name} game {game_index} in shard {shard_index}");
        let game = generate_game_entries(
            dataset_name,
            loaded,
            teachers,
            &mut search_workspaces,
            opening_source,
            opening_split,
            game_index,
        )
        .context(error_context)?;
        sampled_positions += (game.entries.len() / ENTRY_BYTES) as u64;
        candidate_identity_hasher.update(game_index.to_le_bytes());
        candidate_identity_hasher.update(game.candidate_identity_sha256.as_bytes());
        opening_position_selection
            .entry(game.opening.opening_id.clone())
            .or_default()
            .add(game.stats.position_selection);
        search_stats.add(game.stats);
        games.push(game.opening);
        writer.write_all(&game.entries)?;
    }
    writer.flush()?;

    let label_budget = loaded.config.data.label_search_budget()?;
    let build_mode = generation_teacher_build_mode(loaded, teachers);
    let generation_semantic_identity_sha256 = generation_semantic_identity_sha256(
        loaded,
        dataset_name,
        opening_sfen,
        opening_source,
        opening_split,
        &build_mode,
        generation_teacher_sha256(teachers).as_deref(),
        engine_revision.as_deref(),
    )?;
    let requested_game_count = match dataset_name {
        "train" => loaded.config.data.train_games,
        "validation" => loaded.config.data.validation_games,
        _ => unreachable!("generate_or_reuse_shard validates the dataset name"),
    };
    let schedule_identity_sha256 =
        schedule_identity_sha256(loaded, dataset_name, requested_game_count, Some(plan))?;

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
        validation_opening_schedule: opening_split
            .validation_schedule
            .manifest_name()
            .to_string(),
        validation_opening_pairs_per_id: opening_split.validation_pairs_per_id,
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
        label_retry_policy: loaded
            .config
            .data
            .label_retry_policy
            .manifest_name()
            .to_string(),
        max_label_attempts_per_game: loaded.config.data.max_label_attempts_per_game,
        position_selection_audit_version: POSITION_SELECTION_AUDIT_VERSION.to_string(),
        candidate_positions: search_stats.candidate_positions,
        candidate_roots_per_game: loaded.config.data.max_candidate_roots_per_game,
        candidate_identity_version: CANDIDATE_IDENTITY_VERSION.to_string(),
        candidate_identity_sha256: format!("{:x}", candidate_identity_hasher.finalize()),
        generation_semantic_identity_version: GENERATION_SEMANTIC_IDENTITY_VERSION.to_string(),
        generation_semantic_identity_sha256,
        schedule_identity_version: SCHEDULE_IDENTITY_VERSION.to_string(),
        schedule_identity_sha256,
        minimum_train_boards: loaded.config.data.minimum_train_boards()?,
        minimum_train_positions: loaded.config.data.minimum_train_positions,
        rejected_incomplete_label_positions: search_stats.rejected_incomplete_label_positions,
        rejected_terminal_positions: search_stats.rejected_terminal_positions,
        rejected_mate_score_positions: search_stats.rejected_mate_score_positions,
        rejected_node_accounting_positions: search_stats.rejected_node_accounting_positions,
        label_retry_exhausted_games: search_stats.label_retry_exhausted_games,
        label_retry_attempts_per_accepted_position: ratio_f64(
            search_stats.candidate_positions,
            stored_position_count(&search_stats),
        ),
        rejected_incomplete_root_plies: search_stats.rejected_incomplete_root_plies.clone(),
        rejected_terminal_root_plies: search_stats.rejected_terminal_root_plies.clone(),
        rejected_mate_root_plies: search_stats.rejected_mate_root_plies.clone(),
        rejected_node_accounting_root_plies: search_stats
            .rejected_node_accounting_root_plies
            .clone(),
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
        rollout_candidate_limit: loaded.config.data.rollout_candidate_limit,
        rollout_score_margin: loaded.config.data.rollout_score_margin,
        rollout_temperature: loaded.config.data.rollout_temperature,
        rollout_rng_version: loaded.config.data.rollout_rng_version.clone(),
        label_searches: search_stats.label_searches,
        rollout_searches: search_stats.rollout_searches,
        label_search_states: search_stats.label_search_states,
        label_search_qnodes: search_stats.label_search_qnodes,
        rollout_search_states: search_stats.rollout_search_states,
        rollout_search_qnodes: search_stats.rollout_search_qnodes,
        rollout_decisions: search_stats.rollout_decisions,
        rollout_legal_moves: search_stats.rollout_legal_moves,
        rollout_candidates_scored: search_stats.rollout_candidates_scored,
        rollout_candidates_truncated: search_stats.rollout_candidates_truncated,
        rollout_near_best_candidates: search_stats.rollout_near_best_candidates,
        rollout_selected_score_gap_sum: search_stats.rollout_selected_score_gap_sum,
        rollout_selected_score_gap_max: search_stats.rollout_selected_score_gap_max,
        label_search_cpu_seconds: search_stats.label_search_elapsed_seconds,
        rollout_search_cpu_seconds: search_stats.rollout_search_elapsed_seconds,
        bootstrap_nnue: None,
        bootstrap_nnue_sha256: generation_teacher_sha256(teachers),
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
        feature_family: loaded.record_format().to_string(),
        generated_at_unix_ms,
        build_mode,
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
    teachers: &GenerationTeachers,
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
    let build_mode = generation_teacher_build_mode(loaded, teachers);
    let expected_semantic_identity = generation_semantic_identity_sha256(
        loaded,
        dataset_name,
        opening_sfen,
        opening_source,
        opening_split,
        &build_mode,
        generation_teacher_sha256(teachers).as_deref(),
        engine_revision.as_deref(),
    )?;
    if !shard_manifest_matches(
        loaded,
        dataset_name,
        opening_sfen,
        opening_source,
        opening_split,
        plan,
        &manifest,
        &expected_semantic_identity,
        allow_identity_mismatch,
    )? {
        return Ok(None);
    }
    if !shard_teacher_matches(
        loaded,
        teachers,
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
    expected_semantic_identity: &str,
    ignore_identity: bool,
) -> Result<bool> {
    let label_budget = loaded.config.data.label_search_budget()?;
    let requested_game_count = match dataset_name {
        "train" => loaded.config.data.train_games,
        "validation" => loaded.config.data.validation_games,
        _ => bail!("unknown dataset split `{dataset_name}`"),
    };
    let expected_schedule_identity =
        schedule_identity_sha256(loaded, dataset_name, requested_game_count, Some(plan))?;
    let semantic_identity_matches = manifest.generation_semantic_identity_version
        == GENERATION_SEMANTIC_IDENTITY_VERSION
        && manifest.generation_semantic_identity_sha256 == expected_semantic_identity;
    let legacy_config_identity_matches = manifest.generation_semantic_identity_version.is_empty()
        && manifest.config_hash == loaded.hash_hex;
    let schedule_extension_config_mismatch = manifest.generation_semantic_identity_version
        == GENERATION_SEMANTIC_IDENTITY_VERSION
        && manifest.schedule_identity_version == SCHEDULE_IDENTITY_VERSION
        && manifest.schedule_identity_sha256 != expected_schedule_identity;
    let config_identity_matches = if semantic_identity_matches {
        manifest.config_hash == loaded.hash_hex || schedule_extension_config_mismatch
    } else {
        legacy_config_identity_matches
    };
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
                && manifest.validation_opening_schedule
                    == opening_split.validation_schedule.manifest_name()
                && manifest.validation_opening_pairs_per_id
                    == opening_split.validation_pairs_per_id
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
        && manifest.label_retry_policy == loaded.config.data.label_retry_policy.manifest_name()
        && manifest.max_label_attempts_per_game == loaded.config.data.max_label_attempts_per_game
        && manifest.position_selection_audit_version == POSITION_SELECTION_AUDIT_VERSION
        && manifest.candidate_roots_per_game == loaded.config.data.max_candidate_roots_per_game
        && (manifest.candidate_identity_version.is_empty()
            || manifest.candidate_identity_version == CANDIDATE_IDENTITY_VERSION)
        && (manifest.generation_semantic_identity_version != GENERATION_SEMANTIC_IDENTITY_VERSION
            || (manifest.rollout_candidate_limit == loaded.config.data.rollout_candidate_limit
                && manifest.rollout_score_margin == loaded.config.data.rollout_score_margin
                && manifest.rollout_temperature == loaded.config.data.rollout_temperature
                && manifest.rollout_rng_version == loaded.config.data.rollout_rng_version
                && manifest.feature_family == loaded.record_format()))
        && (manifest.feature_family.is_empty()
            || manifest.feature_family == loaded.record_format())
        && manifest.rollout_search_depth() == loaded.config.data.rollout_search_depth
        && (ignore_identity
            || manifest.self_play_move_policy
                == loaded.config.data.self_play_move_policy.manifest_name())
        && (ignore_identity
            || (manifest.sampling_phase == loaded.config.data.sampling_policy.manifest_name()
                && manifest.sample_after_opening
                    == loaded.config.data.sampling_policy.samples_after_opening()
                && manifest.teacher_move_encoding == TEACHER_MOVE_ENCODING))
        && (ignore_identity || config_identity_matches)
        && manifest.entry_bytes == ENTRY_BYTES
        && manifest.shard_index == plan.shard_index)
}

fn shard_teacher_matches(
    loaded: &LoadedConfig,
    teachers: &GenerationTeachers,
    engine_revision: &Option<String>,
    manifest: &ShardManifest,
    ignore_identity: bool,
) -> bool {
    manifest.bootstrap_nnue.is_none()
        && manifest.bootstrap_nnue_sha256 == generation_teacher_sha256(teachers)
        && (ignore_identity || manifest.engine_revision == *engine_revision)
        && manifest.build_mode == generation_teacher_build_mode(loaded, teachers)
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
    teachers: &GenerationTeachers,
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
        teachers,
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
    teachers: &GenerationTeachers,
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
        let build_mode = generation_teacher_build_mode(loaded, teachers);
        let expected_semantic_identity = generation_semantic_identity_sha256(
            loaded,
            dataset_name,
            opening_sfen,
            opening_source,
            opening_split,
            &build_mode,
            generation_teacher_sha256(teachers).as_deref(),
            engine_revision.as_deref(),
        )?;
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
                    &expected_semantic_identity,
                    false,
                )? && shard_teacher_matches(loaded, teachers, engine_revision, &manifest, false);
            let relaxed =
                shard_manifest_matches(
                    loaded,
                    dataset_name,
                    opening_sfen,
                    opening_source,
                    opening_split,
                    plan,
                    &manifest,
                    &expected_semantic_identity,
                    true,
                )? && shard_teacher_matches(loaded, teachers, engine_revision, &manifest, true);
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
    teachers: &GenerationTeachers,
    search_workspaces: &mut GenerationSearchWorkspaces,
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
    let mut attempted_candidate_roots = 0u16;
    let mut candidate_identity_hasher = Sha256::new();

    while played_plies < loaded.config.data.max_plies {
        if !has_both_kings(&board) {
            break;
        }
        let legal_moves = collect_legal_moves(&board);
        if legal_moves.is_empty() {
            break;
        }

        let candidate_limit_available = if loaded.config.data.label_retry_policy.is_adaptive() {
            samples.len() < usize::from(loaded.config.data.max_positions_per_game)
                && attempted_candidate_roots
                    < loaded
                        .config
                        .data
                        .max_label_attempts_per_game
                        .expect("validated adaptive retry attempt cap")
        } else {
            match loaded.config.data.max_candidate_roots_per_game {
                Some(limit) => attempted_candidate_roots < limit,
                None => samples.len() < usize::from(loaded.config.data.max_positions_per_game),
            }
        };
        let should_sample = played_plies >= sample_origin
            && (played_plies - sample_origin) % loaded.config.data.sample_every_ply == 0
            && candidate_limit_available;
        let needs_rollout_search = played_plies >= loaded.config.data.opening_random_plies
            && (loaded.config.data.self_play_move_policy.is_rollout_policy() || !should_sample);
        let label_summary = if should_sample {
            attempted_candidate_roots += 1;
            update_candidate_identity(
                &mut candidate_identity_hasher,
                game_index,
                played_plies,
                &board,
            );
            let summary = teachers.label.search_label(
                &board,
                label_search_budget,
                loaded.config.data.position_policy,
                &mut search_workspaces.label,
            )?;
            stats.record_label(&summary);
            let summary = apply_incomplete_label_policy(
                summary,
                label_search_budget,
                loaded.config.data.incomplete_label_policy,
                board.side_to_move(),
                played_plies,
                &mut stats,
            )?;
            match summary {
                Some(summary)
                    if loaded.config.data.label_retry_policy.is_adaptive()
                        && !label_node_accounting_is_exact(&summary, label_search_budget) =>
                {
                    let root_side = board.side_to_move();
                    stats.record_candidate(root_side);
                    stats.rejected_node_accounting_positions += 1;
                    stats.position_selection.record_node_accounting(root_side);
                    *stats
                        .rejected_node_accounting_root_plies
                        .entry(played_plies)
                        .or_default() += 1;
                    None
                }
                summary => summary,
            }
        } else {
            None
        };
        let rollout_summary = if needs_rollout_search
            && !loaded
                .config
                .data
                .self_play_move_policy
                .is_searched_stochastic()
        {
            let summary = teachers.trajectory.search_depth(
                &board,
                loaded.config.data.rollout_search_depth,
                &mut search_workspaces.trajectory,
            )?;
            stats.record_rollout(&summary);
            Some(summary)
        } else {
            None
        };
        let rollout_decision = if needs_rollout_search
            && loaded
                .config
                .data
                .self_play_move_policy
                .is_searched_stochastic()
        {
            Some(choose_searched_stochastic_rollout_move(
                &board,
                &legal_moves,
                &teachers.trajectory,
                loaded.config.data.rollout_search_depth,
                loaded.config.data.rollout_candidate_limit,
                loaded.config.data.rollout_score_margin,
                loaded.config.data.rollout_temperature,
                loaded.config.data.seed,
                dataset_name,
                game_index / 2,
                played_plies,
                &loaded.config.data.rollout_rng_version,
                &mut search_workspaces.trajectory,
                &mut stats,
            )?)
        } else {
            None
        };

        if let Some(summary) = label_summary.as_ref() {
            record_pending_sample(
                loaded.config.data.position_policy,
                loaded.config.data.label_retry_policy,
                &board,
                summary,
                played_plies,
                &mut samples,
                &mut stats,
            )?;
        }

        let mv = if played_plies < loaded.config.data.opening_random_plies {
            legal_moves[rng.random_range(0..legal_moves.len())]
        } else if let Some(decision) = rollout_decision {
            decision.move_
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

    if loaded.config.data.label_retry_policy.is_adaptive()
        && label_retry_attempt_cap_exhausted(
            samples.len(),
            loaded.config.data.max_positions_per_game,
            attempted_candidate_roots,
            loaded
                .config
                .data
                .max_label_attempts_per_game
                .expect("validated adaptive retry attempt cap"),
        )
    {
        stats.label_retry_exhausted_games += 1;
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
        candidate_identity_sha256: format!("{:x}", candidate_identity_hasher.finalize()),
    })
}

fn label_retry_attempt_cap_exhausted(
    accepted_positions: usize,
    max_positions_per_game: u16,
    attempted_candidate_roots: u16,
    max_label_attempts_per_game: u16,
) -> bool {
    accepted_positions < usize::from(max_positions_per_game)
        && attempted_candidate_roots >= max_label_attempts_per_game
}

fn update_candidate_identity(hasher: &mut Sha256, game_index: u32, root_ply: u16, board: &Board) {
    let board = board.to_string();
    hasher.update(game_index.to_le_bytes());
    hasher.update(root_ply.to_le_bytes());
    hasher.update((board.len() as u32).to_le_bytes());
    hasher.update(board.as_bytes());
}

fn record_pending_sample(
    position_policy: PositionPolicy,
    label_retry_policy: LabelRetryPolicy,
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
            if label_retry_policy.is_adaptive()
                && (!has_both_kings(root_board)
                    || root_board.status() != haitaka::GameStatus::Ongoing)
            {
                stats.rejected_terminal_positions += 1;
                *stats
                    .rejected_terminal_root_plies
                    .entry(root_ply)
                    .or_default() += 1;
                stats
                    .position_selection
                    .record_terminal(root_side, root_side);
                return Ok(());
            }
            if label_retry_policy.is_adaptive()
                && summary
                    .best_score
                    .is_some_and(|score| score.unsigned_abs() >= SEARCH_MATE_SCORE_THRESHOLD as u32)
            {
                stats.rejected_mate_score_positions += 1;
                *stats.rejected_mate_root_plies.entry(root_ply).or_default() += 1;
                stats
                    .position_selection
                    .record_mate(root_side, Some(root_side));
                return Ok(());
            }
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
                *stats
                    .rejected_terminal_root_plies
                    .entry(root_ply)
                    .or_default() += 1;
                stats
                    .position_selection
                    .record_terminal(root_side, trace.leaf_board.side_to_move());
            }
            _ if summary
                .best_score
                .is_some_and(|score| score.abs() >= SEARCH_MATE_SCORE_THRESHOLD) =>
            {
                stats.rejected_mate_score_positions += 1;
                *stats.rejected_mate_root_plies.entry(root_ply).or_default() += 1;
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
        SelfPlayMovePolicy::SearchedStochasticRolloutV1 => rollout,
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
    teachers: &GenerationTeachers,
    engine_revision: &Option<String>,
    generated_at_unix_ms: u128,
    ignore_identity_mismatch: bool,
) -> Result<u64> {
    let started = Instant::now();
    let build_mode = generation_teacher_build_mode(loaded, teachers);
    let expected_generation_identity = generation_semantic_identity_sha256(
        loaded,
        dataset_name,
        opening_sfen,
        opening_source,
        opening_split,
        &build_mode,
        generation_teacher_sha256(teachers).as_deref(),
        engine_revision.as_deref(),
    )?;
    let mut by_start = BTreeMap::new();
    let mut teacher_identity = None;
    let mut generation_identity = None;
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
                &mut generation_identity,
                &expected_generation_identity,
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
    let mut candidate_identity_hasher = Sha256::new();
    for result in &shard_results {
        candidate_identity_hasher.update(result.manifest.game_start.to_le_bytes());
        candidate_identity_hasher.update(result.manifest.candidate_identity_sha256.as_bytes());
    }
    let candidate_identity_sha256 = format!("{:x}", candidate_identity_hasher.finalize());
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
    let first_manifest = shard_results
        .first()
        .map(|result| &result.manifest)
        .ok_or_else(|| anyhow!("cannot assemble an empty {dataset_name} split"))?;
    let has_generation_identity =
        first_manifest.generation_semantic_identity_version == GENERATION_SEMANTIC_IDENTITY_VERSION;
    let generation_semantic_identity_version =
        first_manifest.generation_semantic_identity_version.clone();
    let generation_semantic_identity_sha256 =
        first_manifest.generation_semantic_identity_sha256.clone();
    let (schedule_identity_version, schedule_identity_sha256) = if has_generation_identity {
        (
            SCHEDULE_IDENTITY_VERSION.to_string(),
            schedule_identity_sha256(loaded, dataset_name, game_count, None)?,
        )
    } else {
        (String::new(), String::new())
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
        validation_opening_schedule: opening_split
            .validation_schedule
            .manifest_name()
            .to_string(),
        validation_opening_pairs_per_id: opening_split.validation_pairs_per_id,
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
        label_retry_policy: loaded
            .config
            .data
            .label_retry_policy
            .manifest_name()
            .to_string(),
        max_label_attempts_per_game: loaded.config.data.max_label_attempts_per_game,
        position_selection_audit_version: POSITION_SELECTION_AUDIT_VERSION.to_string(),
        candidate_positions: search_stats.candidate_positions,
        candidate_roots_per_game: loaded.config.data.max_candidate_roots_per_game,
        candidate_identity_version: CANDIDATE_IDENTITY_VERSION.to_string(),
        candidate_identity_sha256,
        generation_semantic_identity_version,
        generation_semantic_identity_sha256,
        schedule_identity_version,
        schedule_identity_sha256,
        minimum_train_boards: loaded.config.data.minimum_train_boards()?,
        minimum_train_positions: loaded.config.data.minimum_train_positions,
        rejected_incomplete_label_positions: search_stats.rejected_incomplete_label_positions,
        rejected_terminal_positions: search_stats.rejected_terminal_positions,
        rejected_mate_score_positions: search_stats.rejected_mate_score_positions,
        rejected_node_accounting_positions: search_stats.rejected_node_accounting_positions,
        label_retry_exhausted_games: search_stats.label_retry_exhausted_games,
        label_retry_attempts_per_accepted_position: ratio_f64(
            search_stats.candidate_positions,
            stored_position_count(&search_stats),
        ),
        rejected_incomplete_root_plies: search_stats.rejected_incomplete_root_plies.clone(),
        rejected_terminal_root_plies: search_stats.rejected_terminal_root_plies.clone(),
        rejected_mate_root_plies: search_stats.rejected_mate_root_plies.clone(),
        rejected_node_accounting_root_plies: search_stats
            .rejected_node_accounting_root_plies
            .clone(),
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
        rollout_candidate_limit: loaded.config.data.rollout_candidate_limit,
        rollout_score_margin: loaded.config.data.rollout_score_margin,
        rollout_temperature: loaded.config.data.rollout_temperature,
        rollout_rng_version: loaded.config.data.rollout_rng_version.clone(),
        label_searches: search_stats.label_searches,
        rollout_searches: search_stats.rollout_searches,
        label_search_states: search_stats.label_search_states,
        label_search_qnodes: search_stats.label_search_qnodes,
        label_search_total_nodes,
        label_nodes_per_search,
        rollout_search_states: search_stats.rollout_search_states,
        rollout_search_qnodes: search_stats.rollout_search_qnodes,
        rollout_decisions: search_stats.rollout_decisions,
        rollout_legal_moves: search_stats.rollout_legal_moves,
        rollout_candidates_scored: search_stats.rollout_candidates_scored,
        rollout_candidates_truncated: search_stats.rollout_candidates_truncated,
        rollout_near_best_candidates: search_stats.rollout_near_best_candidates,
        rollout_selected_score_gap_sum: search_stats.rollout_selected_score_gap_sum,
        rollout_selected_score_gap_max: search_stats.rollout_selected_score_gap_max,
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
        feature_family: loaded.record_format().to_string(),
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
    generation_identity: &mut Option<MergeGenerationIdentity>,
    expected_generation_identity: &str,
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
    validate_merge_teacher_identity(teacher_identity, manifest, ignore_identity_mismatch)?;
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
            manifest.validation_opening_schedule
                == opening_split.validation_schedule.manifest_name(),
            "validation_opening_schedule does not match",
        )?;
        ensure_merge(
            manifest.validation_opening_pairs_per_id == opening_split.validation_pairs_per_id,
            "validation_opening_pairs_per_id does not match",
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
    validate_merge_generation_identity(
        generation_identity,
        expected_generation_identity,
        manifest,
    )?;
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
        let requested_game_count = match dataset_name {
            "train" => loaded.config.data.train_games,
            "validation" => loaded.config.data.validation_games,
            _ => unreachable!("dataset name was checked above"),
        };
        let expected_schedule_identity = schedule_identity_sha256(
            loaded,
            dataset_name,
            requested_game_count,
            Some(ShardPlan {
                shard_index: manifest.shard_index,
                game_start: manifest.game_start,
                game_count: manifest.game_count,
            }),
        )?;
        let schedule_extension = manifest.generation_semantic_identity_version
            == GENERATION_SEMANTIC_IDENTITY_VERSION
            && manifest.schedule_identity_version == SCHEDULE_IDENTITY_VERSION
            && manifest.schedule_identity_sha256 != expected_schedule_identity;
        ensure_merge(
            manifest.config_hash == loaded.hash_hex || schedule_extension,
            "config_hash does not match. If you're sure to continue merging using mismatching identity, rerun with --ignore-identity-mismatch flag",
        )?;
    }
    ensure_merge(
        manifest.entry_bytes == ENTRY_BYTES,
        "entry_bytes does not match",
    )?;
    Ok(())
}

fn validate_merge_generation_identity(
    expected: &mut Option<MergeGenerationIdentity>,
    expected_sha256: &str,
    manifest: &ShardManifest,
) -> Result<()> {
    let version = manifest.generation_semantic_identity_version.as_str();
    let sha256 = manifest.generation_semantic_identity_sha256.as_str();
    if version.is_empty() {
        ensure_merge(
            sha256.is_empty(),
            "generation semantic identity hash is present without a version",
        )?;
    } else {
        ensure_merge(
            version == GENERATION_SEMANTIC_IDENTITY_VERSION,
            "unsupported generation semantic identity version",
        )?;
        ensure_merge(
            sha256 == expected_sha256,
            "generation semantic identity does not match the configured teacher, opening, seed, rollout, sampling, label budget, feature, or ABI contract",
        )?;
    }

    let current = MergeGenerationIdentity {
        version: version.to_string(),
        sha256: sha256.to_string(),
    };
    if let Some(expected) = expected.as_ref() {
        ensure_merge(
            current == *expected,
            "generation semantic identity does not match across shards",
        )?;
    } else {
        *expected = Some(current);
    }
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

fn teacher_build_mode(loaded: &LoadedConfig, teacher: &Teacher) -> String {
    format!("{}+teacher:{}", loaded.runtime_mode(), teacher.describe())
}

fn generation_teacher_build_mode(loaded: &LoadedConfig, teachers: &GenerationTeachers) -> String {
    format!(
        "{}+trajectory:{}+label:{}",
        loaded.runtime_mode(),
        teachers.trajectory.describe(),
        teachers.label.describe()
    )
}

fn generation_teacher_sha256(teachers: &GenerationTeachers) -> Option<String> {
    let trajectory = teachers
        .trajectory
        .bootstrap_sha256()
        .unwrap_or("handcrafted");
    let label = teachers.label.bootstrap_sha256().unwrap_or("handcrafted");
    if trajectory == "handcrafted" && label == "handcrafted" {
        return None;
    }
    Some(hash_bytes_hex(
        format!("trajectory={trajectory}\nlabel={label}\n").as_bytes(),
    ))
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

pub(crate) fn pack_board_for_training(board: &Board) -> Result<[u8; PACKED_SFEN_BYTES]> {
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

/// Decodes the exact 64-byte trainer ABI back into a runtime board.
///
/// The ABI intentionally coalesces promoted gold-like pieces to Gold, matching
/// both the C++ loader and the runtime NNUE `piece_slot` geometry.
pub(crate) fn unpack_board_from_training(packed: &[u8; PACKED_SFEN_BYTES]) -> Result<Board> {
    let mut reader = TrainingBitReader::new(packed);
    let trainer_side = if reader.read_one_bit()? {
        TrainerColor::Black
    } else {
        TrainerColor::White
    };
    let white_king = reader.read_n_bits(7)? as usize;
    let black_king = reader.read_n_bits(7)? as usize;
    ensure!(
        white_king < 81 && black_king < 81 && white_king != black_king,
        "invalid packed king squares"
    );

    let mut trainer_board = [None; 81];
    trainer_board[white_king] = Some(TrainerPiece {
        color: TrainerColor::White,
        piece_type: 9,
    });
    trainer_board[black_king] = Some(TrainerPiece {
        color: TrainerColor::Black,
        piece_type: 9,
    });
    for rank in (0..9).rev() {
        for file in 0..9 {
            let square = rank * 9 + file;
            if square == white_king || square == black_king {
                continue;
            }
            trainer_board[square] = reader.read_board_piece()?;
        }
    }

    let mut hands = [[0u8; 10]; 2];
    for color in &mut hands {
        for count in color {
            *count = reader.read_n_bits(5)? as u8;
        }
    }
    for _ in 0..4 {
        ensure!(!reader.read_one_bit()?, "packed castling bit must be zero");
    }
    ensure!(
        !reader.read_one_bit()?,
        "packed en-passant bit must be zero"
    );
    ensure!(
        reader.read_n_bits(6)? == 0,
        "packed rule50 low bits must be zero"
    );
    let fullmove_low = reader.read_n_bits(8)?;
    let fullmove_high = reader.read_n_bits(8)?;
    ensure!(
        !reader.read_one_bit()?,
        "packed rule50 high bit must be zero"
    );
    let fullmove = (fullmove_high << 8) | fullmove_low;
    ensure!(fullmove > 0, "packed move number must be positive");

    let mut runtime_board = [None; 81];
    for (trainer_square, piece) in trainer_board.into_iter().enumerate() {
        let Some(piece) = piece else { continue };
        let trainer_file = trainer_square % 9;
        let trainer_rank = trainer_square / 9;
        let runtime_file = 8 - trainer_file;
        let runtime_rank = 8 - trainer_rank;
        runtime_board[runtime_rank * 9 + runtime_file] = Some(piece);
    }
    let mut board_text = String::new();
    for rank in 0..9 {
        let mut empty = 0;
        for file in (0..9).rev() {
            match runtime_board[rank * 9 + file] {
                None => empty += 1,
                Some(piece) => {
                    if empty != 0 {
                        board_text.push_str(&empty.to_string());
                        empty = 0;
                    }
                    board_text.push_str(&trainer_piece_sfen(piece)?);
                }
            }
        }
        if empty != 0 {
            board_text.push_str(&empty.to_string());
        }
        if rank != 8 {
            board_text.push('/');
        }
    }
    let side = match trainer_side {
        TrainerColor::White => "b",
        TrainerColor::Black => "w",
    };
    let hand_text = trainer_hands_sfen(&hands)?;
    let sfen = format!("{board_text} {side} {hand_text} {fullmove}");
    Board::from_training_sfen(&sfen)
        .map_err(|err| anyhow!("decoded packed board is invalid ({sfen}): {err}"))
}

fn trainer_piece_sfen(piece: TrainerPiece) -> Result<String> {
    let runtime_piece = match piece.piece_type {
        0 => Piece::Bishop,
        1 => Piece::Rook,
        2 => Piece::Silver,
        3 => Piece::PRook,
        4 => Piece::Pawn,
        5 => Piece::Lance,
        6 => Piece::Knight,
        7 => Piece::Gold,
        8 => Piece::PBishop,
        9 => Piece::King,
        other => bail!("invalid packed piece type {other}"),
    };
    let runtime_color = match piece.color {
        TrainerColor::White => Color::Black,
        TrainerColor::Black => Color::White,
    };
    Ok(runtime_piece.to_str(runtime_color))
}

fn trainer_hands_sfen(hands: &[[u8; 10]; 2]) -> Result<String> {
    let order = [1usize, 0, 7, 2, 6, 5, 4];
    let mut text = String::new();
    for (trainer_color, runtime_color) in [
        (TrainerColor::White, Color::Black),
        (TrainerColor::Black, Color::White),
    ] {
        for piece_type in order {
            let count = hands[trainer_color as usize][piece_type];
            if count == 0 {
                continue;
            }
            let piece = trainer_piece_sfen(TrainerPiece {
                color: trainer_color,
                piece_type,
            })?;
            if count > 1 {
                text.push_str(&count.to_string());
            }
            let expected_color = if piece.chars().last().is_some_and(char::is_uppercase) {
                Color::Black
            } else {
                Color::White
            };
            ensure!(
                expected_color == runtime_color,
                "internal hand color mismatch"
            );
            text.push_str(&piece);
        }
    }
    for invalid_type in [3usize, 8, 9] {
        ensure!(
            hands[0][invalid_type] == 0 && hands[1][invalid_type] == 0,
            "invalid promoted/king hand count"
        );
    }
    Ok(if text.is_empty() {
        "-".to_string()
    } else {
        text
    })
}

struct TrainingBitReader<'a> {
    bytes: &'a [u8; PACKED_SFEN_BYTES],
    bit_cursor: usize,
}

impl<'a> TrainingBitReader<'a> {
    fn new(bytes: &'a [u8; PACKED_SFEN_BYTES]) -> Self {
        Self {
            bytes,
            bit_cursor: 0,
        }
    }

    fn read_one_bit(&mut self) -> Result<bool> {
        ensure!(
            self.bit_cursor < self.bytes.len() * 8,
            "packed board bitstream is overlong"
        );
        let bit = ((self.bytes[self.bit_cursor / 8] >> (self.bit_cursor % 8)) & 1) != 0;
        self.bit_cursor += 1;
        Ok(bit)
    }

    fn read_n_bits(&mut self, bits: usize) -> Result<u32> {
        let mut value = 0u32;
        for shift in 0..bits {
            if self.read_one_bit()? {
                value |= 1 << shift;
            }
        }
        Ok(value)
    }

    fn read_board_piece(&mut self) -> Result<Option<TrainerPiece>> {
        if !self.read_one_bit()? {
            return Ok(None);
        }
        let mut code = 1u32;
        for shift in 1..5 {
            if self.read_one_bit()? {
                code |= 1 << shift;
            }
        }
        ensure!(
            code % 2 == 1 && code <= 19,
            "invalid packed piece code {code}"
        );
        let piece_type = ((code - 1) / 2) as usize;
        let color = if self.read_one_bit()? {
            TrainerColor::Black
        } else {
            TrainerColor::White
        };
        Ok(Some(TrainerPiece { color, piece_type }))
    }
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
    use crate::config::{
        InitialCheckpoint, LoadedGenerationConfig as LoadedConfig, TrainingConfig,
    };
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
    #[cfg(feature = "anhoku")]
    fn r0_generation_is_byte_identical_across_separate_training_initializations() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("generation.toml");
        fs::write(
            &config_path,
            r#"
[rules]
ruleset = "anhoku"
[paths]
output_dir = "run-a"
[data]
train_games = 1
validation_games = 1
max_plies = 2
self_play_move_policy = "uniform-rollout-v1"
opening_random_plies = 0
sample_start_ply = 0
sample_every_ply = 1
max_positions_per_game = 2
seed = 17
jobs = 1
shard_games = 1
resume = false
[generation]
record_format = "haitaka-packed-training-record-v3-72-byte"
[generation.trajectory_evaluator]
search_depth = 1
[generation.trajectory_evaluator.evaluator]
kind = "handcrafted"
[generation.label_evaluator]
search_budget = "depth"
search_depth = 1
target_semantics = "root-backed-up-v1"
score_transform_version = "raw-score-v1"
[generation.label_evaluator.evaluator]
kind = "handcrafted"
"#,
        )
        .unwrap();
        let first_training = TrainingConfig::default();
        let mut second_training = TrainingConfig::default();
        second_training.initial_checkpoint = InitialCheckpoint::QuantizedImportDiagnostic {
            path: PathBuf::from("never-read.nnue"),
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            source_feature_family: "HalfKAv2^+DonorSingleEff".to_string(),
            import_transform_version: "serialize-py-quantized-import-v1".to_string(),
        };

        let orchestrate =
            |training: &TrainingConfig, mut generation: LoadedConfig, output: &str| {
                let _registered_initialization = &training.initial_checkpoint;
                generation.config.paths.output_dir = PathBuf::from(output);
                generate_data_with_options(
                    &generation,
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
                generation.artifact_paths()
            };
        let loaded = LoadedConfig::from_path(&config_path).unwrap();
        let first = orchestrate(&first_training, loaded.clone(), "run-a");
        let second = orchestrate(&second_training, loaded, "run-b");
        assert_eq!(
            fs::read(first.train_bin).unwrap(),
            fs::read(second.train_bin).unwrap()
        );
        assert_eq!(
            fs::read(first.validation_bin).unwrap(),
            fs::read(second.validation_bin).unwrap()
        );
        for name in ["train", "validation"] {
            let left: serde_json::Value = serde_json::from_slice(
                &fs::read(
                    first
                        .datasets_dir
                        .join("shards")
                        .join(name)
                        .join("shard-000000.json"),
                )
                .unwrap(),
            )
            .unwrap();
            let right: serde_json::Value = serde_json::from_slice(
                &fs::read(
                    second
                        .datasets_dir
                        .join("shards")
                        .join(name)
                        .join("shard-000000.json"),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(
                left["generation_semantic_identity_sha256"],
                right["generation_semantic_identity_sha256"]
            );
        }
    }

    #[test]
    fn adaptive_retry_exhaustion_requires_the_attempt_cap() {
        assert!(!label_retry_attempt_cap_exhausted(10, 64, 10, 72));
        assert!(label_retry_attempt_cap_exhausted(10, 64, 72, 72));
        assert!(!label_retry_attempt_cap_exhausted(64, 64, 72, 72));
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
    fn training_board_decoder_round_trips_canonical_bytes() {
        let boards = [
            Board::startpos(),
            Board::from_sfen(
                "lnsgkgsnl/1r5b1/pppp1pppp/4p4/4+P4/9/PPPP1PPPP/1B5R1/LNSGKGSNL b - 3",
            )
            .unwrap(),
        ];
        for board in boards {
            let packed = pack_board_for_training(&board).unwrap();
            let decoded = unpack_board_from_training(&packed).unwrap();
            assert_eq!(pack_board_for_training(&decoded).unwrap(), packed);
        }
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

        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();
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
    fn generate_data_rejects_a_configured_missing_bootstrap_nnue() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("missing-bootstrap.toml");
        let config = deterministic_test_config(active_test_ruleset(), "out").replace(
            "\n[data]",
            "\nbootstrap_nnue = \"missing-bootstrap.nnue\"\n\n[data]",
        );
        fs::write(&config_path, config).unwrap();
        let error = format!("{:#}", LoadedConfig::from_path(&config_path).unwrap_err());
        assert!(!error.is_empty());
        assert!(!temp.path().join("out").exists());
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

        let one = LoadedConfig::from_legacy_test_path(&config_one).unwrap();
        let two = LoadedConfig::from_legacy_test_path(&config_two).unwrap();
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
    fn searched_stochastic_generation_is_deterministic_across_jobs_and_shard_lanes() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("searched-stochastic.toml");
        fs::write(&config_path, adaptive_retry_test_config("out")).unwrap();
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();
        let artifacts = loaded.artifact_paths();
        let options = |jobs, shard_index, shard_count| GenerateOptions {
            jobs: Some(jobs),
            resume: Some(false),
            shard_index,
            shard_index_end: shard_index,
            shard_count,
            ignore_identity_mismatch: false,
        };

        generate_data_with_options(&loaded, options(1, None, None)).unwrap();
        let expected_train = fs::read(&artifacts.train_bin).unwrap();
        let expected_validation = fs::read(&artifacts.validation_bin).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&artifacts.train_manifest).unwrap()).unwrap();
        assert_eq!(
            manifest["label_retry_policy"].as_str(),
            Some("root-position-adaptive-retry-v1")
        );
        assert_eq!(manifest["max_label_attempts_per_game"].as_u64(), Some(4));
        assert_eq!(manifest["label_retry_exhausted_games"].as_u64(), Some(0));
        assert_eq!(
            manifest["label_retry_attempts_per_accepted_position"].as_f64(),
            Some(1.0)
        );
        fs::rename(&artifacts.output_dir, temp.path().join("full-jobs-one")).unwrap();

        generate_data_with_options(&loaded, options(2, None, None)).unwrap();
        assert_eq!(fs::read(&artifacts.train_bin).unwrap(), expected_train);
        assert_eq!(
            fs::read(&artifacts.validation_bin).unwrap(),
            expected_validation
        );
        fs::rename(&artifacts.output_dir, temp.path().join("full-jobs-two")).unwrap();

        let mut lanes = Vec::new();
        for lane in 0..2 {
            generate_data_with_options(&loaded, options(2, Some(lane), Some(2))).unwrap();
            let lane_path = temp.path().join(format!("lane-{lane}"));
            fs::rename(&artifacts.output_dir, &lane_path).unwrap();
            lanes.push(lane_path);
        }
        merge_data(&loaded, &lanes, false).unwrap();
        assert_eq!(fs::read(&artifacts.train_bin).unwrap(), expected_train);
        assert_eq!(
            fs::read(&artifacts.validation_bin).unwrap(),
            expected_validation
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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();
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
        let first = LoadedConfig::from_legacy_test_path(&first_config).unwrap();
        let second = LoadedConfig::from_legacy_test_path(&second_config).unwrap();
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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();
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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();
        generate_data(&loaded).unwrap();

        let mut changed = fs::read_to_string(&suite).unwrap();
        changed.push_str("# identity change\n");
        fs::write(&suite, changed).unwrap();
        let changed_loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();
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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();
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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
    fn strict_generation_rejects_identity_override() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("resume-ignore-identity.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        mutate_first_shard_manifest(&loaded, "train", |manifest| {
            manifest["config_hash"] = serde_json::Value::String("stale-config-hash".to_string());
            manifest["engine_revision"] = serde_json::Value::String("other-revision".to_string());
        });
        let error = generate_data_with_options(
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
        .unwrap_err();
        assert!(format!("{error:#}").contains("forbids --ignore-identity-mismatch"));
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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

        generate_data(&loaded).unwrap();
        let artifacts = loaded.artifact_paths();
        let teachers = GenerationTeachers::from_config(&loaded).unwrap();
        let opening_sfen = loaded.opening_sfen().unwrap();
        let opening_source = OpeningSource::from_config(&loaded, &opening_sfen).unwrap();
        let opening_split = opening_source
            .split_openings(
                loaded.config.data.split_policy,
                loaded.config.data.split_seed,
                loaded.config.data.train_games,
                loaded.config.data.validation_games,
                loaded.config.data.validation_opening_ids.as_deref(),
                loaded.config.data.validation_opening_schedule,
                loaded.config.data.validation_opening_pairs_per_id,
            )
            .unwrap();
        let engine_revision = detect_git_revision(&loaded).unwrap();
        let selector = ShardSelector::new(None, None, None).unwrap();

        let (before, total) = detect_identity_mismatch(
            &loaded,
            &artifacts,
            &teachers,
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
            &teachers,
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
            &teachers,
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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();
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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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

        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();
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

        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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

        let err = format!(
            "{:?}",
            LoadedConfig::from_legacy_test_path(&config_path).unwrap_err()
        );

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();

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
        assert_eq!(manifest["candidate_roots_per_game"].as_u64(), Some(2));
        assert!(candidates <= 2);
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
            total_nodes: 2,
            node_limit: None,
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
            LabelRetryPolicy::None,
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
            LabelRetryPolicy::None,
            &root,
            &traced(123, true),
            10,
            &mut samples,
            &mut stats,
        )
        .unwrap();
        record_pending_sample(
            PositionPolicy::QsearchPvLeaf,
            LabelRetryPolicy::None,
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
    fn adaptive_root_retry_rejects_mate_then_accepts_an_ordinary_label() {
        let root = Board::startpos();
        let best_move = collect_legal_moves(&root)[0].to_string();
        let summary = |score| TeacherSearchSummary {
            best_move: Some(best_move.clone()),
            best_score: Some(score),
            states: 10,
            qnodes: 5,
            total_nodes: 15,
            node_limit: Some(50_000),
            elapsed_seconds: 0.0,
            training_trace: None,
        };
        let mut samples = Vec::new();
        let mut stats = SearchUseStats::default();

        record_pending_sample(
            PositionPolicy::RootPosition,
            LabelRetryPolicy::RootPositionAdaptiveRetryV1,
            &root,
            &summary(SEARCH_MATE_SCORE_THRESHOLD),
            8,
            &mut samples,
            &mut stats,
        )
        .unwrap();
        assert!(samples.is_empty());
        assert_eq!(stats.candidate_positions, 1);
        assert_eq!(stats.rejected_mate_score_positions, 1);

        record_pending_sample(
            PositionPolicy::RootPosition,
            LabelRetryPolicy::RootPositionAdaptiveRetryV1,
            &root,
            &summary(321),
            10,
            &mut samples,
            &mut stats,
        )
        .unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].score, 321);
        assert_eq!(stats.candidate_positions, 2);
        assert_eq!(stats.rejected_mate_score_positions, 1);
    }

    #[test]
    fn incomplete_fixed_node_labels_are_rejected_only_by_explicit_policy() {
        let incomplete = TeacherSearchSummary {
            best_move: None,
            best_score: None,
            states: 1_000,
            qnodes: 4_000,
            total_nodes: 5_000,
            node_limit: Some(5_000),
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
            8,
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
            8,
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
    fn searched_stochastic_test_config(ruleset: &str, output_dir: &str) -> String {
        deterministic_test_config(ruleset, output_dir)
            .replace("validation_games = 2", "validation_games = 4")
            .replace("max_plies = 8", "max_plies = 4")
            .replace(
                "opening_random_plies = 2",
                r#"opening_random_plies = 0
self_play_move_policy = "searched-stochastic-rollout-v1"
rollout_search_depth = 1
rollout_candidate_limit = 4
rollout_score_margin = 80
rollout_temperature = 40.0
rollout_rng_version = "splitmix64-v1""#,
            )
    }

    #[cfg(feature = "anhoku")]
    fn adaptive_retry_test_config(output_dir: &str) -> String {
        searched_stochastic_test_config("anhoku", output_dir)
            .replace(
                "\nsearch_depth = 1\n",
                r#"
label_search_nodes = 5000
label_search_max_depth = 64
incomplete_label_policy = "reject-position"
label_retry_policy = "root-position-adaptive-retry-v1"
max_label_attempts_per_game = 4
"#,
            )
            .replace("max_positions_per_game = 4", "max_positions_per_game = 1")
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
max_candidate_roots_per_game = 2
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

        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();
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

        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();
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

    #[test]
    fn searched_rollout_streams_are_pair_and_ply_specific() {
        let pair_zero = rollout_stream_seed(75, "train", 0, 12, "splitmix64-v1");
        let pair_one = rollout_stream_seed(75, "train", 1, 12, "splitmix64-v1");
        let next_ply = rollout_stream_seed(75, "train", 0, 13, "splitmix64-v1");
        assert_ne!(pair_zero, pair_one);
        assert_ne!(pair_zero, next_ply);

        let move_ = collect_legal_moves(&Board::startpos())[0];
        assert_eq!(transform_move(transform_move(move_)), move_);
        let candidates = [
            ScoredRolloutMove { move_, score: 100 },
            ScoredRolloutMove { move_, score: 50 },
        ];
        assert_eq!(
            weighted_choice_index(&candidates, 40.0, pair_zero),
            weighted_choice_index(&candidates, 40.0, pair_zero)
        );
    }

    #[test]
    fn bounded_rollout_candidates_always_include_the_root_best_move() {
        let mut legal_moves = collect_legal_moves(&Board::startpos());
        legal_moves.sort_by_key(ToString::to_string);
        let root_best_move = *legal_moves.last().unwrap();
        assert!(!legal_moves[..2].contains(&root_best_move));

        let candidates = bounded_rollout_candidates(legal_moves.clone(), root_best_move, 2);
        assert_eq!(candidates.len(), 2);
        assert!(candidates.contains(&root_best_move));

        let mut stats = SearchUseStats::default();
        stats.record_rollout_decision(legal_moves.len() as u64, candidates.len() as u64, 1, 0);
        assert_eq!(stats.rollout_legal_moves, legal_moves.len() as u64);
        assert_eq!(stats.rollout_candidates_scored, 2);
        assert_eq!(
            stats.rollout_candidates_truncated,
            legal_moves.len() as u64 - 2
        );
    }

    #[test]
    fn empty_calibration_root_pairs_do_not_match() {
        assert!(!calibration_root_slices_match(&[], &[]).unwrap());
    }

    #[test]
    #[cfg(feature = "anhoku")]
    fn adaptive_calibration_retries_pairs_and_detects_exhaustion_symmetrically() {
        let observation = |mate| CalibrationSearchObservation {
            incomplete: false,
            terminal: false,
            mate,
            accounting_error: false,
            alpha_beta_nodes: 1,
            qsearch_nodes: 1,
            accounted_nodes: 2,
        };
        let attempts = [
            [observation(true), observation(false)],
            [observation(false), observation(false)],
        ];
        assert_eq!(
            attempts
                .iter()
                .position(|pair| pair.iter().all(CalibrationSearchObservation::is_admissible)),
            Some(1)
        );
        assert!(
            attempts[..1]
                .iter()
                .all(|pair| !pair.iter().all(CalibrationSearchObservation::is_admissible))
        );

        let base_board = Board::startpos();
        let swapped_board =
            Board::from_sfen(&color_swap_anhoku_sfen(&base_board.to_string()).unwrap()).unwrap();
        let base = CalibrationRoot {
            board: base_board,
            root_ply: 8,
            side_to_move: Color::Black,
            outcome: GameOutcome::Draw,
        };
        let swapped = CalibrationRoot {
            board: swapped_board,
            root_ply: 8,
            side_to_move: Color::White,
            outcome: GameOutcome::Draw,
        };
        assert!(calibration_root_slices_match(&[base], &[swapped]).unwrap());
    }

    #[test]
    fn minimum_train_boards_changes_schedule_but_not_generation_semantics() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("identity.toml");
        fs::write(
            &config_path,
            deterministic_test_config(active_test_ruleset(), "out"),
        )
        .unwrap();
        let mut small = LoadedConfig::from_legacy_test_path(&config_path).unwrap();
        small.config.data.minimum_train_boards = Some(262_144);
        let mut large = small.clone();
        large.config.data.minimum_train_boards = Some(1_048_576);
        let opening_sfen = small.opening_sfen().unwrap();
        let opening_source = OpeningSource::from_config(&small, &opening_sfen).unwrap();
        let opening_split = opening_source
            .split_openings(
                small.config.data.split_policy,
                small.config.data.split_seed,
                small.config.data.train_games,
                small.config.data.validation_games,
                small.config.data.validation_opening_ids.as_deref(),
                small.config.data.validation_opening_schedule,
                small.config.data.validation_opening_pairs_per_id,
            )
            .unwrap();
        let semantic = |loaded: &LoadedConfig| {
            generation_semantic_identity_sha256(
                loaded,
                "train",
                &opening_sfen,
                &opening_source,
                &opening_split,
                "handcrafted",
                None,
                Some("test-revision"),
            )
            .unwrap()
        };
        assert_eq!(semantic(&small), semantic(&large));
        assert_ne!(
            schedule_identity_sha256(&small, "train", small.config.data.train_games, None).unwrap(),
            schedule_identity_sha256(&large, "train", large.config.data.train_games, None).unwrap()
        );
    }

    #[test]
    fn minimum_train_boards_extension_reuses_semantically_identical_shards() {
        let temp = tempdir().unwrap();
        let first_config = temp.path().join("minimum-one.toml");
        let second_config = temp.path().join("minimum-two.toml");
        let base = deterministic_test_config(active_test_ruleset(), "out");
        fs::write(
            &first_config,
            base.replace(
                "max_positions_per_game = 4",
                "max_positions_per_game = 4\nminimum_train_boards = 1",
            ),
        )
        .unwrap();
        fs::write(
            &second_config,
            base.replace(
                "max_positions_per_game = 4",
                "max_positions_per_game = 4\nminimum_train_boards = 2",
            ),
        )
        .unwrap();

        let first = LoadedConfig::from_legacy_test_path(&first_config).unwrap();
        generate_data(&first).unwrap();
        let second = LoadedConfig::from_legacy_test_path(&second_config).unwrap();
        generate_data(&second).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(second.artifact_paths().train_manifest).unwrap())
                .unwrap();
        assert_eq!(manifest["generated_shards"].as_u64(), Some(0));
        assert!(manifest["resumed_shards"].as_u64().unwrap() > 0);
        assert_eq!(
            manifest["schedule_identity_version"].as_str(),
            Some(SCHEDULE_IDENTITY_VERSION)
        );
    }

    #[test]
    #[cfg(feature = "anhoku")]
    fn phase8d_trajectory_schedule_covers_every_opening_with_two_pairs() {
        let config_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("haitaka_learn.anhoku-v0.6-phase8d-a.toml");
        let loaded = LoadedConfig::from_legacy_test_path(&config_path).unwrap();
        let opening_sfen = loaded.opening_sfen().unwrap();
        let opening_source = OpeningSource::from_config(&loaded, &opening_sfen).unwrap();
        let opening_split = opening_source
            .split_openings(
                loaded.config.data.split_policy,
                loaded.config.data.split_seed,
                loaded.config.data.train_games,
                loaded.config.data.validation_games,
                loaded.config.data.validation_opening_ids.as_deref(),
                loaded.config.data.validation_opening_schedule,
                loaded.config.data.validation_opening_pairs_per_id,
            )
            .unwrap();
        let mut tasks = trajectory_tasks(
            &loaded,
            &opening_split,
            ShardSelector::new(None, None, None).unwrap(),
        )
        .unwrap();
        assert_eq!(tasks.len(), 256);
        let mut game_counts = BTreeMap::new();
        for (task, opening_id) in &tasks {
            *game_counts
                .entry((task.dataset_name, opening_id.clone()))
                .or_insert(0u64) += 1;
        }
        assert_eq!(game_counts.len(), 64);
        assert!(game_counts.values().all(|games| *games == 4));
        for pair in tasks.chunks_exact(2) {
            assert_eq!(pair[0].0.dataset_name, pair[1].0.dataset_name);
            assert_eq!(pair[0].0.game_index / 2, pair[1].0.game_index / 2);
            assert_eq!(pair[0].1, pair[1].1);
        }
        tasks.sort_by_key(|(task, _)| trajectory_task_audit_sort_key(&opening_split, *task));
        for cycle in tasks.chunks_exact(128) {
            let mut cycle_game_counts = BTreeMap::new();
            for (task, opening_id) in cycle {
                *cycle_game_counts
                    .entry((task.dataset_name, opening_id.clone()))
                    .or_insert(0u64) += 1;
            }
            assert_eq!(cycle_game_counts.len(), 64);
            assert!(cycle_game_counts.values().all(|games| *games == 2));
        }
    }
}

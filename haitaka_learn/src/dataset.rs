use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use haitaka::{Board, Color, Move, Piece, Square};
use haitaka_wasm::{
    NnueModel, SearchEvalMode, SearchSummary, search_board_impl_handcrafted,
    search_board_impl_with_eval_mode,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::Serialize;

use crate::config::{ArtifactPaths, HandicapPreset, LoadedConfig, Ruleset};

const PACKED_SFEN_BYTES: usize = 64;
const ENTRY_BYTES: usize = PACKED_SFEN_BYTES + 8;

#[derive(Debug, Clone)]
pub struct DatasetOutput {
    pub output_dir: PathBuf,
    pub train_positions: u64,
    pub validation_positions: u64,
}

#[derive(Debug, Serialize)]
struct DatasetManifest {
    dataset: String,
    ruleset: Ruleset,
    rule_id: Option<u16>,
    opening_sfen: String,
    game_count: u32,
    sampled_positions: u64,
    search_depth: u8,
    bootstrap_nnue: Option<String>,
    engine_revision: Option<String>,
    config_hash: String,
    generated_at_unix_ms: u128,
    build_mode: String,
}

#[derive(Debug, Clone)]
struct PendingSample {
    board: Board,
    score: i16,
    game_ply: u16,
    side_to_move: Color,
}

#[derive(Debug, Clone, Copy)]
enum GameOutcome {
    Draw,
    Winner(Color),
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
    Nnue(Arc<NnueModel>),
}

impl Teacher {
    fn from_config(loaded: &LoadedConfig) -> Result<Self> {
        if let Some(path) = loaded.bootstrap_nnue() {
            if path.exists() {
                let bytes = fs::read(&path)
                    .with_context(|| format!("failed to read bootstrap NNUE {}", path.display()))?;
                let model = NnueModel::from_bytes(&bytes).map_err(|err| {
                    anyhow!("failed to load bootstrap NNUE {}: {err}", path.display())
                })?;
                return Ok(Self::Nnue(Arc::new(model)));
            }
        }
        Ok(Self::Handcrafted)
    }

    fn describe(&self) -> &'static str {
        match self {
            Self::Handcrafted => "handcrafted",
            Self::Nnue(_) => "nnue",
        }
    }

    fn search(&self, board: &Board, depth: u8) -> Result<SearchSummary> {
        match self {
            Self::Handcrafted => search_board_impl_handcrafted(board, depth)
                .map_err(|err| anyhow!("handcrafted teacher search failed: {err}")),
            Self::Nnue(model) => search_board_impl_with_eval_mode(
                board,
                depth,
                model.clone(),
                SearchEvalMode::Incremental,
            )
            .map_err(|err| anyhow!("NNUE teacher search failed: {err}")),
        }
    }
}

pub fn generate_data(loaded: &LoadedConfig) -> Result<DatasetOutput> {
    loaded.ruleset_requires_matching_engine()?;
    let opening_sfen = loaded.opening_sfen()?;
    let _: Board = Board::from_sfen(&opening_sfen)
        .map_err(|err| anyhow!("invalid opening SFEN in config: {err}"))?;

    let artifacts = loaded.artifact_paths();
    artifacts.ensure_dirs()?;

    let teacher = Teacher::from_config(loaded)?;
    let engine_revision = detect_git_revision(loaded)?;
    let generated_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let mut rng = StdRng::seed_from_u64(loaded.config.data.seed);
    let train_positions = generate_split(
        "train",
        loaded,
        &artifacts,
        &teacher,
        &opening_sfen,
        loaded.config.data.train_games,
        &mut rng,
        &engine_revision,
        generated_at_unix_ms,
    )?;
    let validation_positions = generate_split(
        "validation",
        loaded,
        &artifacts,
        &teacher,
        &opening_sfen,
        loaded.config.data.validation_games,
        &mut rng,
        &engine_revision,
        generated_at_unix_ms,
    )?;

    Ok(DatasetOutput {
        output_dir: artifacts.output_dir,
        train_positions,
        validation_positions,
    })
}

fn generate_split(
    dataset_name: &str,
    loaded: &LoadedConfig,
    artifacts: &ArtifactPaths,
    teacher: &Teacher,
    opening_sfen: &str,
    game_count: u32,
    rng: &mut StdRng,
    engine_revision: &Option<String>,
    generated_at_unix_ms: u128,
) -> Result<u64> {
    let (bin_path, manifest_path) = match dataset_name {
        "train" => (&artifacts.train_bin, &artifacts.train_manifest),
        "validation" => (&artifacts.validation_bin, &artifacts.validation_manifest),
        _ => bail!("unknown dataset split `{dataset_name}`"),
    };

    let mut writer = BufWriter::new(
        File::create(bin_path)
            .with_context(|| format!("failed to create {}", bin_path.display()))?,
    );
    let mut sampled_positions = 0u64;

    for _ in 0..game_count {
        let mut board = Board::from_sfen(opening_sfen)
            .map_err(|err| anyhow!("failed to parse opening SFEN: {err}"))?;
        let mut samples = Vec::new();
        let mut played_plies = 0u16;

        while played_plies < loaded.config.data.max_plies {
            if !has_both_kings(&board) {
                break;
            }
            let legal_moves = collect_legal_moves(&board);
            if legal_moves.is_empty() {
                break;
            }

            let should_sample = played_plies >= loaded.config.data.sample_start_ply
                && (played_plies - loaded.config.data.sample_start_ply)
                    % loaded.config.data.sample_every_ply
                    == 0
                && samples.len() < usize::from(loaded.config.data.max_positions_per_game);
            let needs_teacher =
                should_sample || played_plies >= loaded.config.data.opening_random_plies;
            let teacher_summary = if needs_teacher {
                Some(teacher.search(&board, loaded.config.data.search_depth)?)
            } else {
                None
            };

            if should_sample {
                let summary = teacher_summary
                    .as_ref()
                    .ok_or_else(|| anyhow!("teacher search unexpectedly missing"))?;
                let score = summary
                    .best_score
                    .unwrap_or_else(|| terminal_teacher_score(&board))
                    .clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                samples.push(PendingSample {
                    board: board.clone(),
                    score,
                    game_ply: played_plies,
                    side_to_move: board.side_to_move(),
                });
            }

            let mv = if played_plies < loaded.config.data.opening_random_plies {
                legal_moves[rng.random_range(0..legal_moves.len())]
            } else {
                let best_move = teacher_summary
                    .as_ref()
                    .and_then(|summary| summary.best_move.as_deref())
                    .ok_or_else(|| anyhow!("teacher search did not return a best move"))?;
                let mv: Move = best_move
                    .parse()
                    .map_err(|err| anyhow!("failed to parse teacher move `{best_move}`: {err}"))?;
                if !board.is_legal(mv) {
                    bail!("teacher move `{best_move}` was not legal for position `{board}`");
                }
                mv
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

        for sample in samples {
            let game_result = outcome.relative_to(sample.side_to_move);
            let packed = pack_board_for_training(&sample.board)?;
            write_training_entry(
                &mut writer,
                &packed,
                sample.score,
                0,
                sample.game_ply,
                game_result,
            )?;
            sampled_positions += 1;
        }
    }

    writer.flush()?;

    let manifest = DatasetManifest {
        dataset: dataset_name.to_string(),
        ruleset: loaded.config.rules.ruleset,
        rule_id: effective_rule_id(loaded),
        opening_sfen: opening_sfen.to_string(),
        game_count,
        sampled_positions,
        search_depth: loaded.config.data.search_depth,
        bootstrap_nnue: loaded
            .bootstrap_nnue()
            .map(|path| path.display().to_string()),
        engine_revision: engine_revision.clone(),
        config_hash: loaded.hash_hex.clone(),
        generated_at_unix_ms,
        build_mode: format!("{}+teacher:{}", loaded.runtime_mode(), teacher.describe()),
    };
    fs::write(manifest_path, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;

    Ok(sampled_positions)
}

fn effective_rule_id(loaded: &LoadedConfig) -> Option<u16> {
    loaded
        .config
        .rules
        .rule_id
        .or(match loaded.config.rules.ruleset {
            Ruleset::Standard => Some(0),
            Ruleset::Annan => Some(26),
            Ruleset::Handicap => match loaded.config.rules.handicap {
                Some(HandicapPreset::SixPiece) => Some(6),
                Some(HandicapPreset::FourPiece) => Some(4),
                Some(HandicapPreset::TwoPiece) => Some(2),
                None => None,
            },
        })
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
    fn generate_data_smoke_test_writes_non_empty_shards() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join("haitaka_learn.toml");
        let ruleset = if cfg!(feature = "annan") {
            "annan"
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
    #[cfg(not(feature = "annan"))]
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
    fn finds_workspace_root_from_root_config_path() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir.parent().unwrap();
        let config_path = workspace_root.join("haitaka_learn.toml");

        let detected = find_haitaka_workspace_root(&config_path).unwrap();

        assert_eq!(detected, workspace_root);
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
opening_sfen = "4k4/4R4/9/9/9/9/9/9/4K4 b - 1"

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

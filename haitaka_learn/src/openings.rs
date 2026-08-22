use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use haitaka::{Board, Color, Move, Piece};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{LoadedConfig, OpeningPolicy, SplitPolicy};

pub const ANHOKU_COLOR_SWAP_V1: &str = "anhoku-rotate180-color-swap-v1";
pub const NO_OPENING_TRANSFORMATION: &str = "none";

#[derive(Debug, Clone)]
pub(crate) struct SuiteOpening {
    id: String,
    base_sfen: String,
    swapped_sfen: String,
}

#[derive(Debug, Clone)]
pub enum OpeningSource {
    UniformRandom {
        base_sfen: String,
    },
    Suite {
        suite_id: String,
        sha256: String,
        openings: Vec<SuiteOpening>,
    },
}

#[derive(Debug, Clone)]
pub struct SelectedOpening {
    pub sfen: String,
    pub metadata: GameOpeningMetadata,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct GameOpeningMetadata {
    #[serde(default)]
    pub game_id: String,
    pub game_index: u32,
    pub pair_index: u32,
    pub opening_id: String,
    pub color: String,
    pub sfen: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpeningSplit {
    pub train_ids: Vec<String>,
    pub validation_ids: Vec<String>,
}

impl OpeningSplit {
    pub fn ids_for(&self, dataset: &str) -> Result<&[String]> {
        match dataset {
            "train" => Ok(&self.train_ids),
            "validation" => Ok(&self.validation_ids),
            _ => bail!("unknown dataset split `{dataset}`"),
        }
    }

    pub fn overlap(&self) -> Vec<String> {
        let validation = self.validation_ids.iter().collect::<BTreeSet<_>>();
        self.train_ids
            .iter()
            .filter(|id| validation.contains(id))
            .cloned()
            .collect()
    }
}

impl OpeningSource {
    pub fn from_config(loaded: &LoadedConfig, base_sfen: &str) -> Result<Self> {
        match loaded.config.data.opening_policy {
            OpeningPolicy::UniformRandom => Ok(Self::UniformRandom {
                base_sfen: base_sfen.to_string(),
            }),
            OpeningPolicy::Suite => {
                let path = loaded.opening_suite().ok_or_else(|| {
                    anyhow!("data.opening_suite is required for opening_policy=suite")
                })?;
                let suite_id = loaded.config.data.opening_suite_id.clone().ok_or_else(|| {
                    anyhow!("data.opening_suite_id is required for opening_policy=suite")
                })?;
                let bytes = fs::read(&path)
                    .with_context(|| format!("failed to read opening suite {}", path.display()))?;
                let sha256 = hash_bytes_hex(&bytes);
                let text = std::str::from_utf8(&bytes)
                    .with_context(|| format!("opening suite {} is not UTF-8", path.display()))?;
                let openings = parse_suite(&path, text)?;
                Ok(Self::Suite {
                    suite_id,
                    sha256,
                    openings,
                })
            }
        }
    }

    pub fn policy(&self) -> &'static str {
        match self {
            Self::UniformRandom { .. } => OpeningPolicy::UniformRandom.manifest_name(),
            Self::Suite { .. } => OpeningPolicy::Suite.manifest_name(),
        }
    }

    pub fn suite_id(&self) -> Option<&str> {
        match self {
            Self::UniformRandom { .. } => None,
            Self::Suite { suite_id, .. } => Some(suite_id),
        }
    }

    pub fn suite_sha256(&self) -> Option<&str> {
        match self {
            Self::UniformRandom { .. } => None,
            Self::Suite { sha256, .. } => Some(sha256),
        }
    }

    pub fn transformation(&self) -> &'static str {
        match self {
            Self::UniformRandom { .. } => NO_OPENING_TRANSFORMATION,
            Self::Suite { .. } => ANHOKU_COLOR_SWAP_V1,
        }
    }

    pub fn split_openings(
        &self,
        policy: SplitPolicy,
        split_seed: u64,
        train_games: u32,
        validation_games: u32,
        explicit_validation_ids: Option<&[String]>,
    ) -> Result<OpeningSplit> {
        let all_ids = match self {
            Self::UniformRandom { .. } => vec!["uniform-random".to_string()],
            Self::Suite { openings, .. } => {
                openings.iter().map(|opening| opening.id.clone()).collect()
            }
        };
        if policy == SplitPolicy::IndependentLegacy {
            return Ok(OpeningSplit {
                train_ids: all_ids.clone(),
                validation_ids: all_ids,
            });
        }
        if !matches!(self, Self::Suite { .. }) {
            bail!("opening-group-hash-v1 requires an opening suite");
        }
        if all_ids.len() < 2 {
            bail!("opening-group-hash-v1 requires at least two opening IDs");
        }
        if let Some(explicit_validation_ids) = explicit_validation_ids {
            let mut validation_ids = explicit_validation_ids.to_vec();
            validation_ids.sort();
            validation_ids.dedup();
            if validation_ids.len() != explicit_validation_ids.len() {
                bail!("explicit validation opening IDs must be unique");
            }
            if validation_ids.len() >= all_ids.len() {
                bail!("explicit validation opening IDs must leave at least one training ID");
            }
            let all_id_set = all_ids.iter().collect::<BTreeSet<_>>();
            if let Some(unknown) = validation_ids.iter().find(|id| !all_id_set.contains(id)) {
                bail!("explicit validation opening ID `{unknown}` is not in the suite");
            }
            let validation_set = validation_ids.iter().collect::<BTreeSet<_>>();
            let mut train_ids = all_ids
                .into_iter()
                .filter(|id| !validation_set.contains(id))
                .collect::<Vec<_>>();
            train_ids.sort();
            return Ok(OpeningSplit {
                train_ids,
                validation_ids,
            });
        }
        let total_games = u64::from(train_games) + u64::from(validation_games);
        let mut validation_count = ((all_ids.len() as u64 * u64::from(validation_games)
            + total_games / 2)
            / total_games) as usize;
        let minimum_validation_groups = usize::from(all_ids.len() >= 4) + 1;
        validation_count = validation_count.clamp(minimum_validation_groups, all_ids.len() - 1);
        let mut ranked = all_ids;
        ranked.sort_by_key(|id| (opening_group_key(split_seed, id), id.clone()));
        let mut validation_ids = ranked[..validation_count].to_vec();
        let mut train_ids = ranked[validation_count..].to_vec();
        train_ids.sort();
        validation_ids.sort();
        Ok(OpeningSplit {
            train_ids,
            validation_ids,
        })
    }

    pub fn select(
        &self,
        dataset: &str,
        split: &OpeningSplit,
        pair_seed: u64,
        game_index: u32,
    ) -> Result<SelectedOpening> {
        let pair_index = game_index / 2;
        let game_id = format!("{dataset}-{game_index:010}");
        Ok(match self {
            Self::UniformRandom { base_sfen } => SelectedOpening {
                sfen: base_sfen.clone(),
                metadata: GameOpeningMetadata {
                    game_id,
                    game_index,
                    pair_index,
                    opening_id: "uniform-random".to_string(),
                    color: "unpaired".to_string(),
                    sfen: base_sfen.clone(),
                },
            },
            Self::Suite { openings, .. } => {
                let allowed = split.ids_for(dataset)?;
                let index = (pair_seed % allowed.len() as u64) as usize;
                let opening_id = &allowed[index];
                let opening = openings
                    .iter()
                    .find(|opening| &opening.id == opening_id)
                    .expect("split IDs originate from this suite");
                let swapped = game_index % 2 == 1;
                let sfen = if swapped {
                    opening.swapped_sfen.clone()
                } else {
                    opening.base_sfen.clone()
                };
                SelectedOpening {
                    sfen: sfen.clone(),
                    metadata: GameOpeningMetadata {
                        game_id,
                        game_index,
                        pair_index,
                        opening_id: opening.id.clone(),
                        color: if swapped { "swapped" } else { "base" }.to_string(),
                        sfen,
                    },
                }
            }
        })
    }
}

fn opening_group_key(seed: u64, id: &str) -> u64 {
    let mut hash = Sha256::new();
    hash.update(seed.to_le_bytes());
    hash.update(id.as_bytes());
    let digest = hash.finalize();
    u64::from_le_bytes(digest[..8].try_into().expect("SHA-256 prefix is 8 bytes"))
}

pub fn validate_configured_suite(loaded: &LoadedConfig) -> Result<(String, usize, String)> {
    let base_sfen = loaded.opening_sfen()?;
    let source = OpeningSource::from_config(loaded, &base_sfen)?;
    match source {
        OpeningSource::Suite {
            suite_id,
            sha256,
            openings,
        } => Ok((suite_id, openings.len(), sha256)),
        OpeningSource::UniformRandom { .. } => {
            bail!("validate-openings requires data.opening_policy=suite")
        }
    }
}

fn parse_suite(path: &Path, text: &str) -> Result<Vec<SuiteOpening>> {
    let mut openings = Vec::new();
    let mut ids = BTreeSet::new();
    let mut positions = BTreeSet::new();
    for (line_index, raw_line) in text.lines().enumerate() {
        let line = raw_line
            .split_once('#')
            .map_or(raw_line, |(value, _)| value)
            .trim();
        if line.is_empty() {
            continue;
        }
        let (id, raw_sfen) = line.split_once('\t').ok_or_else(|| {
            anyhow!(
                "opening suite {} line {} must be `<opening-id><TAB><SFEN>`",
                path.display(),
                line_index + 1
            )
        })?;
        let id = id.trim();
        let raw_sfen = raw_sfen.trim();
        if id.is_empty() {
            bail!(
                "opening suite {} line {} has an empty ID",
                path.display(),
                line_index + 1
            );
        }
        if !ids.insert(id.to_string()) {
            bail!(
                "opening suite {} has duplicate opening ID `{id}`",
                path.display()
            );
        }
        let board = validate_position(path, line_index + 1, raw_sfen)?;
        let base_sfen = board.to_string();
        if !positions.insert(base_sfen.clone()) {
            bail!(
                "opening suite {} has duplicate position at line {}",
                path.display(),
                line_index + 1
            );
        }
        let swapped_sfen = color_swap_anhoku_sfen(&base_sfen)?;
        let swapped = validate_position(path, line_index + 1, &swapped_sfen)?;
        if swapped.side_to_move() == board.side_to_move() {
            bail!("opening `{id}` color swap did not change side to move");
        }
        if color_swap_anhoku_sfen(&swapped.to_string())? != base_sfen {
            bail!("opening `{id}` color transformation is not reversible");
        }
        openings.push(SuiteOpening {
            id: id.to_string(),
            base_sfen,
            swapped_sfen: swapped.to_string(),
        });
    }
    if openings.is_empty() {
        bail!("opening suite {} contains no positions", path.display());
    }
    Ok(openings)
}

fn validate_position(path: &Path, line: usize, sfen: &str) -> Result<Board> {
    let board = Board::from_sfen(sfen).map_err(|err| {
        anyhow!(
            "failed to parse opening SFEN in {} line {}: {err}",
            path.display(),
            line
        )
    })?;
    if !board.has(Color::Black, Piece::King) || !board.has(Color::White, Piece::King) {
        bail!(
            "opening suite {} line {} must contain both kings",
            path.display(),
            line
        );
    }
    let mut has_legal_move = false;
    board.generate_moves(|moves| {
        has_legal_move = moves.into_iter().any(|mv: Move| board.is_legal(mv));
        has_legal_move
    });
    if !has_legal_move {
        bail!(
            "opening suite {} line {} has no legal move",
            path.display(),
            line
        );
    }
    Ok(board)
}

pub fn color_swap_anhoku_sfen(sfen: &str) -> Result<String> {
    let fields = sfen.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 4 {
        bail!("SFEN must contain board, side, hand, and move number");
    }
    let ranks = fields[0].split('/').collect::<Vec<_>>();
    if ranks.len() != 9 {
        bail!("SFEN board must contain 9 ranks");
    }
    let mut expanded = Vec::with_capacity(9);
    for rank in ranks {
        expanded.push(expand_rank(rank)?);
    }
    let board = expanded
        .into_iter()
        .rev()
        .map(|rank| encode_rank(rank.into_iter().rev().map(swap_piece_case).collect()))
        .collect::<Vec<_>>()
        .join("/");
    let side = match fields[1] {
        "b" => "w",
        "w" => "b",
        other => bail!("invalid SFEN side to move `{other}`"),
    };
    let hand = if fields[2] == "-" {
        "-".to_string()
    } else {
        fields[2]
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphabetic() {
                    swap_ascii_case(ch)
                } else {
                    ch
                }
            })
            .collect()
    };
    let transformed = format!("{board} {side} {hand} {}", fields[3]);
    let board = Board::from_sfen(&transformed)
        .map_err(|err| anyhow!("color-swapped SFEN is invalid: {err}"))?;
    Ok(board.to_string())
}

fn expand_rank(rank: &str) -> Result<Vec<Option<String>>> {
    let mut squares = Vec::with_capacity(9);
    let mut chars = rank.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(empty) = ch.to_digit(10) {
            squares.extend((0..empty).map(|_| None));
        } else if ch == '+' {
            let piece = chars
                .next()
                .ok_or_else(|| anyhow!("dangling `+` in SFEN rank"))?;
            if !piece.is_ascii_alphabetic() {
                bail!("invalid promoted SFEN piece `+{piece}`");
            }
            squares.push(Some(format!("+{piece}")));
        } else if ch.is_ascii_alphabetic() {
            squares.push(Some(ch.to_string()));
        } else {
            bail!("invalid character `{ch}` in SFEN rank");
        }
    }
    if squares.len() != 9 {
        bail!("SFEN rank expands to {} squares, expected 9", squares.len());
    }
    Ok(squares)
}

fn swap_piece_case(piece: Option<String>) -> Option<String> {
    piece.map(|piece| {
        piece
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphabetic() {
                    swap_ascii_case(ch)
                } else {
                    ch
                }
            })
            .collect()
    })
}

fn swap_ascii_case(ch: char) -> char {
    if ch.is_ascii_uppercase() {
        ch.to_ascii_lowercase()
    } else {
        ch.to_ascii_uppercase()
    }
}

fn encode_rank(rank: Vec<Option<String>>) -> String {
    let mut encoded = String::new();
    let mut empty = 0;
    for square in rank {
        match square {
            None => empty += 1,
            Some(piece) => {
                if empty > 0 {
                    encoded.push(char::from_digit(empty, 10).expect("rank has at most 9 squares"));
                    empty = 0;
                }
                encoded.push_str(&piece);
            }
        }
    }
    if empty > 0 {
        encoded.push(char::from_digit(empty, 10).expect("rank has at most 9 squares"));
    }
    encoded
}

fn hash_bytes_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn anhoku_color_swap_is_reversible_and_changes_side() {
        let sfen = "ln2kg1n1/1r1sg1sbl/pppp1ppp1/4p4/9/4P1P1p/PPPP1P1P1/1BGSGS1R1/LN1K3NL b p 17";
        let swapped = color_swap_anhoku_sfen(sfen).unwrap();
        assert!(swapped.contains(" w P 17"));
        assert_eq!(color_swap_anhoku_sfen(&swapped).unwrap(), sfen);
    }

    #[test]
    fn suite_pair_uses_same_id_with_opposite_colors() {
        let source = OpeningSource::Suite {
            suite_id: "fixture-v1".to_string(),
            sha256: "hash".to_string(),
            openings: vec![SuiteOpening {
                id: "fixture-001".to_string(),
                base_sfen: haitaka::SFEN_STARTPOS.to_string(),
                swapped_sfen: color_swap_anhoku_sfen(haitaka::SFEN_STARTPOS).unwrap(),
            }],
        };
        let split = source
            .split_openings(SplitPolicy::IndependentLegacy, 1, 2, 2, None)
            .unwrap();
        let first = source.select("train", &split, 123, 20).unwrap();
        let second = source.select("train", &split, 123, 21).unwrap();
        assert_eq!(first.metadata.opening_id, second.metadata.opening_id);
        assert_eq!(first.metadata.color, "base");
        assert_eq!(second.metadata.color, "swapped");
        assert_ne!(
            Board::from_sfen(&first.sfen).unwrap().side_to_move(),
            Board::from_sfen(&second.sfen).unwrap().side_to_move()
        );
    }

    #[test]
    #[cfg(feature = "anhoku")]
    fn checked_in_suite_is_valid_and_selection_is_deterministic() {
        let config = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("haitaka_learn.anhoku-v0.6.toml");
        let loaded = LoadedConfig::from_path(&config).unwrap();
        let source = OpeningSource::from_config(&loaded, &loaded.opening_sfen().unwrap()).unwrap();
        let split = source
            .split_openings(
                loaded.config.data.split_policy,
                loaded.config.data.split_seed,
                loaded.config.data.train_games,
                loaded.config.data.validation_games,
                loaded.config.data.validation_opening_ids.as_deref(),
            )
            .unwrap();
        let first = (0..40)
            .map(|game| {
                source
                    .select("train", &split, 0x1234_5678 ^ u64::from(game / 2), game)
                    .unwrap()
                    .metadata
            })
            .collect::<Vec<_>>();
        let second = (0..40)
            .map(|game| {
                source
                    .select("train", &split, 0x1234_5678 ^ u64::from(game / 2), game)
                    .unwrap()
                    .metadata
            })
            .collect::<Vec<_>>();
        assert_eq!(first, second);
        assert!(split.overlap().is_empty());
        for pair in first.chunks_exact(2) {
            assert_eq!(pair[0].opening_id, pair[1].opening_id);
            assert_eq!(pair[0].color, "base");
            assert_eq!(pair[1].color, "swapped");
        }
        let (suite_id, count, hash) = validate_configured_suite(&loaded).unwrap();
        assert_eq!(suite_id, "anhoku-v1");
        assert_eq!(count, 12);
        assert_eq!(hash.len(), 64);

        let phase8_config = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("haitaka_learn.anhoku-v0.6-phase8-root.toml");
        let phase8_loaded = LoadedConfig::from_path(&phase8_config).unwrap();
        let phase8_source =
            OpeningSource::from_config(&phase8_loaded, &phase8_loaded.opening_sfen().unwrap())
                .unwrap();
        let phase8_split = phase8_source
            .split_openings(
                phase8_loaded.config.data.split_policy,
                phase8_loaded.config.data.split_seed,
                phase8_loaded.config.data.train_games,
                phase8_loaded.config.data.validation_games,
                phase8_loaded.config.data.validation_opening_ids.as_deref(),
            )
            .unwrap();
        assert_eq!(phase8_split.validation_ids.len(), 12);
        assert_eq!(phase8_split.validation_ids[0], "anhoku-v2-053");
        assert_eq!(phase8_split.validation_ids[11], "anhoku-v2-064");
    }

    #[test]
    fn suite_parser_rejects_duplicate_ids_and_positions() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("duplicate.tsv");
        let start = haitaka::SFEN_STARTPOS;
        let duplicate_id = format!(
            "same\t{start}\nsame\t{}\n",
            color_swap_anhoku_sfen(start).unwrap()
        );
        let error = parse_suite(&path, &duplicate_id).unwrap_err();
        assert!(format!("{error:#}").contains("duplicate opening ID"));

        let duplicate_position = format!("one\t{start}\ntwo\t{start}\n");
        let error = parse_suite(&path, &duplicate_position).unwrap_err();
        assert!(format!("{error:#}").contains("duplicate position"));
    }
}

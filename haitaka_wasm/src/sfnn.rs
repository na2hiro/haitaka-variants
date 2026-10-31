//! SFNN micro/mini evaluator with exact scalar and SIMD integer kernels;
//! independently implemented for Haitaka (MIT), not a port of YaneuraOu source.
//! See docs/tuyouraou11-sfnn.md for the pinned reference and verification procedure.
use crate::{nnue_kernels::AffineKernel, sfnn_kernels};
use haitaka::{Board, Color, Move, Piece, Square};
use std::fmt;

pub const MICRO_ARCH: &str = "SFNN_ka2_2048_8_64_progress8";
pub const MINI_ARCH: &str = "SFNN_ka2_4096_8_64_progress8";
const FEATURES: usize = 1791;
const BONA_NONKING: usize = 1548;
const PROGRESS_COUNT: usize = 81 * BONA_NONKING;
const THRESHOLDS: [i64; 7] = [-127527, -71999, -33477, 0, 33477, 71999, 127527];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SfnnError(pub String);
impl fmt::Display for SfnnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for SfnnError {}
fn invalid(detail: impl Into<String>) -> SfnnError {
    SfnnError(detail.into())
}

/// All values here are from a full refresh, before any search correction.
#[derive(Debug, PartialEq, Eq)]
pub struct SfnnEvaluation {
    pub progress_sums_q16: [i64; 2],
    pub selected_sum_q16: i64,
    pub bucket: usize,
    pub raw_output: i32,
    /// Side-to-move score, divided by FV_SCALE=24 and clipped like the reference.
    pub eval_cp: i32,
}

#[derive(Debug)]
struct Affine {
    bias: Vec<i32>,
    weights: Vec<i8>,
    stride: usize,
    kernel: AffineKernel,
}
impl Affine {
    fn read(r: &mut Reader<'_>, inputs: usize, outputs: usize) -> Result<Self, SfnnError> {
        let bias = (0..outputs)
            .map(|_| r.u32().map(|v| v as i32))
            .collect::<Result<_, _>>()?;
        let stride = inputs.div_ceil(32) * 32;
        let weights = r.take(stride * outputs)?.iter().map(|&v| v as i8).collect();
        Ok(Self {
            bias,
            weights,
            stride,
            kernel: sfnn_kernels::affine_kernel(),
        })
    }
    fn forward<const N: usize>(&self, inputs: &[u8]) -> [i32; N] {
        assert_eq!(self.bias.len(), N);
        let mut output = [0; N];
        self.kernel
            .forward_into(inputs, &self.weights, &self.bias, self.stride, &mut output);
        output
    }
}
#[derive(Debug)]
struct Stack {
    first: Affine,
    second: Affine,
    output: Affine,
}

/// Per-position values only: no weight arrays are cloned at a search node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SfnnPositionState {
    accumulators: [Vec<i16>; 2],
    progress_sums_q16: [i64; 2],
}

/// Immutable shared weights for full-refresh and incremental integer evaluation.
#[derive(Debug)]
pub struct SfnnModel {
    biases: Vec<i16>,
    weights: Vec<i16>,
    progress: Vec<i32>,
    stacks: Vec<Stack>,
}
impl SfnnModel {
    /// Strict micro/mini loader. Error prefixes follow the P0 public contract.
    pub fn from_bytes(nn: &[u8], progress: &[u8], arch: &str) -> Result<Self, SfnnError> {
        let width = match arch {
            MICRO_ARCH => 2048,
            MINI_ARCH => 4096,
            _ => {
                return Err(invalid(
                    "SFNN_UNSUPPORTED_ARCH: expected micro or mini arch",
                ));
            }
        };
        if cfg!(any(
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
        )) {
            return Err(invalid("SFNN_UNSUPPORTED_RULESET: standard build required"));
        }
        let parsed =
            Self::read_nn(nn, width).map_err(|e| invalid(format!("SFNN_INVALID_NN: {e}")))?;
        let progress =
            read_progress(progress).map_err(|e| invalid(format!("SFNN_INVALID_PROGRESS: {e}")))?;
        Ok(Self {
            biases: parsed.0,
            weights: parsed.1,
            stacks: parsed.2,
            progress,
        })
    }
    fn read_nn(bytes: &[u8], width: usize) -> Result<(Vec<i16>, Vec<i16>, Vec<Stack>), SfnnError> {
        let mut r = Reader { bytes, pos: 0 };
        r.expect_u32(0x7af32f16)?;
        r.expect_u32(0x3c203b32)?;
        let len = r.u32()? as usize;
        let description = format!(
            "ModelType=SFNNWithoutPsqt;Features=K-A2(Friend)[1791->{width}x2],Network=SFNN-{width}{{LayerStack=8}}"
        );
        if r.take(len)? != description.as_bytes() {
            return Err(invalid("architecture description mismatch"));
        }
        r.expect_u32(0x5f134ab8)?;
        let biases = r.leb_i16(width)?;
        let weights = r.leb_i16(width * FEATURES)?;
        let mut stacks = Vec::with_capacity(8);
        for _ in 0..8 {
            r.expect_u32(0x6333718a)?;
            stacks.push(Stack {
                first: Affine::read(&mut r, width, 8)?,
                second: Affine::read(&mut r, 16, 64)?,
                output: Affine::read(&mut r, 64, 1)?,
            });
        }
        if r.pos != bytes.len() {
            return Err(invalid("trailing NN data"));
        }
        // Match the transformer's int16 scaling, including wraparound semantics.
        Ok((
            biases.into_iter().map(|v| v.wrapping_mul(2)).collect(),
            weights.into_iter().map(|v| v.wrapping_mul(2)).collect(),
            stacks,
        ))
    }
    /// Panics if either king is missing or more than 38 nonking pieces exist.
    pub fn build_position_state_full(&self, board: &Board) -> SfnnPositionState {
        let pieces = bona_pieces(board);
        let mut state = SfnnPositionState {
            accumulators: [self.biases.clone(), self.biases.clone()],
            progress_sums_q16: [0; 2],
        };
        for p in 0..2 {
            let friendly = orient(
                board.king(if p == 0 { Color::Black } else { Color::White }),
                p,
            );
            let enemy = orient(
                board.king(if p == 0 { Color::White } else { Color::Black }),
                p,
            );
            state.progress_sums_q16[p] = pieces[p]
                .iter()
                .map(|&i| i64::from(self.progress[friendly * BONA_NONKING + i]))
                .sum();
            for index in pieces[p].iter().copied().chain([
                BONA_NONKING + friendly,
                BONA_NONKING + enemy,
                1629 + friendly,
                1629 + 81 + enemy,
            ]) {
                self.update_row(&mut state.accumulators[p], index, true);
            }
        }
        state
    }

    fn update_row(&self, accumulator: &mut [i16], index: usize, add: bool) {
        let width = self.biases.len();
        sfnn_kernels::update(
            accumulator,
            &self.weights[index * width..(index + 1) * width],
            add,
        );
    }

    /// Requires a state built by this model for parent and a legal move to child.
    /// K+A2 is not king-relative: even king moves update just the changed rows.
    pub fn apply_move(
        &self,
        parent: &Board,
        child: &Board,
        state: &SfnnPositionState,
        mv: Move,
    ) -> SfnnPositionState {
        let mut next = state.clone();
        let mover = parent.side_to_move();
        let king_moved = match mv {
            Move::BoardMove { from, .. } => {
                parent.colored_piece_on(from).unwrap().piece == Piece::King
            }
            _ => false,
        };
        let refresh_pieces = king_moved.then(|| bona_pieces(child));
        for p in 0..2 {
            let mut removed = Vec::with_capacity(4);
            let mut added = Vec::with_capacity(4);
            match mv {
                Move::Drop { piece, to } => {
                    removed.push(hand_index(
                        piece,
                        mover,
                        p,
                        usize::from(parent.num_in_hand(mover, piece)) - 1,
                    ));
                    added.push(board_index(piece, mover, to, p));
                }
                Move::BoardMove { from, to, .. } => {
                    let piece = parent.colored_piece_on(from).unwrap().piece;
                    removed.push(board_index(piece, mover, from, p));
                    added.push(board_index(
                        child.colored_piece_on(to).unwrap().piece,
                        mover,
                        to,
                        p,
                    ));
                    if piece == Piece::King {
                        let plane = 1629 + usize::from(color_index(mover) != p) * 81;
                        removed.push(plane + orient(from, p));
                        added.push(plane + orient(to, p));
                    }
                    if let Some(captured) = parent.colored_piece_on(to) {
                        assert_ne!(
                            captured.piece,
                            Piece::King,
                            "terminal king capture has no SFNN state"
                        );
                        removed.push(board_index(captured.piece, captured.color, to, p));
                        let hand_piece = captured.piece.unpromote();
                        added.push(hand_index(
                            hand_piece,
                            mover,
                            p,
                            usize::from(parent.num_in_hand(mover, hand_piece)),
                        ));
                    }
                }
            }
            for &i in &removed {
                self.update_row(&mut next.accumulators[p], i, false);
            }
            for &i in &added {
                self.update_row(&mut next.accumulators[p], i, true);
            }
            let color = if p == 0 { Color::Black } else { Color::White };
            let king = orient(child.king(color), p);
            if parent.king(color) != child.king(color) {
                next.progress_sums_q16[p] = refresh_pieces.as_ref().unwrap()[p]
                    .iter()
                    .map(|&i| i64::from(self.progress[king * BONA_NONKING + i]))
                    .sum();
            } else {
                for &i in &removed {
                    if i < BONA_NONKING {
                        next.progress_sums_q16[p] -=
                            i64::from(self.progress[king * BONA_NONKING + i]);
                    }
                }
                for &i in &added {
                    if i < BONA_NONKING {
                        next.progress_sums_q16[p] +=
                            i64::from(self.progress[king * BONA_NONKING + i]);
                    }
                }
            }
        }
        next
    }

    pub fn evaluate_full_refresh(&self, board: &Board) -> SfnnEvaluation {
        self.evaluate_from_state(board, &self.build_position_state_full(board))
    }

    pub fn evaluate_from_state(&self, board: &Board, state: &SfnnPositionState) -> SfnnEvaluation {
        let sums = state.progress_sums_q16;
        let sum = sums[0] + sums[1];
        let bucket = progress_bucket(sum);
        let width = self.biases.len();
        let mut buffer = [0u8; 4096];
        let transformed = &mut buffer[..width];
        let stm = color_index(board.side_to_move());
        for (output_side, p) in [stm, 1 - stm].into_iter().enumerate() {
            sfnn_kernels::transform(
                &state.accumulators[p],
                &mut transformed[output_side * (width / 2)..(output_side + 1) * (width / 2)],
            );
        }
        let stack = &self.stacks[bucket];
        let first = stack.first.forward::<8>(transformed);
        let mut activated = [0; 16];
        for j in 0..8 {
            activated[j] = ((i64::from(first[j]) * i64::from(first[j]) >> 19).min(127)) as u8;
            activated[8 + j] = clip(first[j]);
        }
        let second = stack.second.forward::<64>(&activated).map(clip);
        let raw_output = stack.output.forward::<1>(&second)[0];
        SfnnEvaluation {
            progress_sums_q16: sums,
            selected_sum_q16: sum,
            bucket,
            raw_output,
            eval_cp: (raw_output / 24).clamp(-31753, 31753),
        }
    }
}
fn color_index(color: Color) -> usize {
    if color == Color::Black { 0 } else { 1 }
}
fn clip(v: i32) -> u8 {
    (v >> 6).clamp(0, 127) as u8
}
pub fn progress_bucket(sum: i64) -> usize {
    THRESHOLDS.partition_point(|&v| v <= sum)
}

/// File-major squares are identical to the reference. White rotates 180 degrees.
fn bona_pieces(board: &Board) -> [Vec<usize>; 2] {
    let mut lists: [Vec<usize>; 2] = std::array::from_fn(|_| Vec::with_capacity(38));
    for square in Square::ALL {
        if let Some(pc) = board.colored_piece_on(square) {
            if pc.piece != Piece::King {
                for p in 0..2 {
                    lists[p].push(board_index(pc.piece, pc.color, square, p));
                }
            }
        }
    }
    for piece in [
        Piece::Pawn,
        Piece::Lance,
        Piece::Knight,
        Piece::Silver,
        Piece::Gold,
        Piece::Bishop,
        Piece::Rook,
    ] {
        for color in Color::ALL {
            for ordinal in 0..usize::from(board.num_in_hand(color, piece)) {
                for p in 0..2 {
                    lists[p].push(hand_index(piece, color, p, ordinal));
                }
            }
        }
    }
    // Reference EvalList preserves the 38 physical nonking slots in handicaps.
    for list in &mut lists {
        assert!(list.len() <= 38, "SFNN requires at most 38 nonking pieces");
        list.resize(38, 0);
    }
    lists
}
fn orient(square: Square, p: usize) -> usize {
    if p == 0 {
        square as usize
    } else {
        80 - square as usize
    }
}
fn board_index(piece: Piece, color: Color, square: Square, p: usize) -> usize {
    let sq = orient(square, p);
    let slot = match piece {
        Piece::Pawn => 0,
        Piece::Lance => 1,
        Piece::Knight => 2,
        Piece::Silver => 3,
        Piece::Gold | Piece::Tokin | Piece::PLance | Piece::PKnight | Piece::PSilver => 4,
        Piece::Bishop => 5,
        Piece::PBishop => 6,
        Piece::Rook => 7,
        Piece::PRook => 8,
        Piece::King => return BONA_NONKING + sq,
    };
    90 + (slot * 2 + usize::from(color_index(color) != p)) * 81 + sq
}
fn hand_index(piece: Piece, color: Color, p: usize, ordinal: usize) -> usize {
    let (friendly, enemy) = match piece {
        Piece::Pawn => (1, 20),
        Piece::Lance => (39, 44),
        Piece::Knight => (49, 54),
        Piece::Silver => (59, 64),
        Piece::Gold => (69, 74),
        Piece::Bishop => (79, 82),
        Piece::Rook => (85, 88),
        _ => panic!("invalid hand piece"),
    };
    if color_index(color) == p {
        friendly + ordinal
    } else {
        enemy + ordinal
    }
}
fn read_progress(bytes: &[u8]) -> Result<Vec<i32>, SfnnError> {
    if bytes.len() != PROGRESS_COUNT * 8 {
        return Err(invalid("expected 1003104 bytes"));
    }
    bytes
        .chunks_exact(8)
        .map(|b| {
            let value = f64::from_le_bytes(b.try_into().unwrap());
            if !value.is_finite() {
                return Err(invalid("non-finite weight"));
            }
            // The pinned reference saturates finite Q16 weights to int32.
            Ok((value * 65536.0)
                .round()
                .clamp(i32::MIN as f64, i32::MAX as f64) as i32)
        })
        .collect()
}
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], SfnnError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| invalid("length overflow"))?;
        let b = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| invalid("truncated data"))?;
        self.pos = end;
        Ok(b)
    }
    fn u32(&mut self) -> Result<u32, SfnnError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn expect_u32(&mut self, value: u32) -> Result<(), SfnnError> {
        let actual = self.u32()?;
        if actual != value {
            return Err(invalid(format!(
                "expected hash/version {value:08x}, got {actual:08x}"
            )));
        }
        Ok(())
    }
    fn leb_i16(&mut self, count: usize) -> Result<Vec<i16>, SfnnError> {
        if self.take(17)? != b"COMPRESSED_LEB128" {
            return Err(invalid("missing LEB128 marker"));
        }
        let len = self.u32()? as usize;
        let payload = self.take(len)?;
        let mut r = Reader {
            bytes: payload,
            pos: 0,
        };
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            let mut v = 0i32;
            let mut shift = 0;
            loop {
                let b = r.take(1)?[0];
                v |= i32::from(b & 127) << shift;
                shift += 7;
                if b & 128 == 0 {
                    if b & 64 != 0 {
                        v |= !0 << shift;
                    }
                    values.push(i16::try_from(v).map_err(|_| invalid("LEB128 int16 overflow"))?);
                    break;
                }
                if shift >= 21 {
                    return Err(invalid("unterminated LEB128 integer"));
                }
            }
        }
        if r.pos != payload.len() {
            return Err(invalid("trailing LEB128 payload"));
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_boundaries_and_rounding() {
        for (i, &v) in THRESHOLDS.iter().enumerate() {
            assert_eq!(progress_bucket(v - 1), i);
            assert_eq!(progress_bucket(v), i + 1);
            assert_eq!(progress_bucket(v + 1), i + 1);
        }
        let mut bytes = vec![0; PROGRESS_COUNT * 8];
        bytes[..8].copy_from_slice(&(0.5f64 / 65536.0).to_le_bytes());
        bytes[8..16].copy_from_slice(&(-0.5f64 / 65536.0).to_le_bytes());
        assert_eq!(&read_progress(&bytes).unwrap()[..2], &[1, -1]);
        bytes[..8].copy_from_slice(&f64::MAX.to_le_bytes());
        assert_eq!(read_progress(&bytes).unwrap()[0], i32::MAX);
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            bytes[..8].copy_from_slice(&v.to_le_bytes());
            assert!(read_progress(&bytes).is_err());
        }
        assert!(read_progress(&bytes[..bytes.len() - 1]).is_err());
        bytes.push(0);
        assert!(read_progress(&bytes).is_err());
    }
    #[test]
    fn rejects_arch_and_build_before_parsing() {
        assert!(
            SfnnModel::from_bytes(&[], &[], "mini")
                .err()
                .unwrap()
                .0
                .starts_with("SFNN_UNSUPPORTED_ARCH:")
        );
        for arch in [MICRO_ARCH, MINI_ARCH] {
            let err = SfnnModel::from_bytes(&[], &[], arch).err().unwrap();
            if cfg!(any(
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
            )) {
                assert!(err.0.starts_with("SFNN_UNSUPPORTED_RULESET:"));
            } else {
                assert!(err.0.starts_with("SFNN_INVALID_NN:"));
            }
        }
    }
    #[test]
    fn leb128_checks() {
        fn payload(b: &[u8]) -> Vec<u8> {
            let mut bytes = b"COMPRESSED_LEB128".to_vec();
            bytes.extend_from_slice(&(b.len() as u32).to_le_bytes());
            bytes.extend_from_slice(b);
            bytes
        }
        let bytes = payload(&[0, 0x7f, 0x80, 1]);
        assert_eq!(
            Reader {
                bytes: &bytes,
                pos: 0
            }
            .leb_i16(3)
            .unwrap(),
            [0, -1, 128]
        );
        for b in [
            &[0x80][..],
            &[0xff, 0xff, 0xff, 0x7f],
            &[0, 0],
            &[0x80, 0x80, 4],
        ] {
            let bytes = payload(b);
            assert!(
                Reader {
                    bytes: &bytes,
                    pos: 0
                }
                .leb_i16(1)
                .is_err()
            );
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod search_range_test {
    use super::*;
    #[test]
    #[ignore = "requires micro; explicit P2 score-range verification"]
    fn sfnn_score_stays_outside_search_mate_range() {
        let dir = std::path::PathBuf::from(
            std::env::var_os("HAITAKA_SFNN_MODEL_DIR").expect("model directory required"),
        );
        let mut model = SfnnModel::from_bytes(
            &std::fs::read(dir.join("nn.bin")).unwrap(),
            &std::fs::read(dir.join("progress.bin")).unwrap(),
            MICRO_ARCH,
        )
        .unwrap();
        for stack in &mut model.stacks {
            stack.output.weights.fill(0);
            stack.output.bias[0] = i32::MAX;
        }
        assert_eq!(
            model.evaluate_full_refresh(&Board::startpos()).eval_cp,
            31753
        );
        let model = std::sync::Arc::new(model);
        let summary = crate::search_impl_sfnn(
            &Board::startpos().to_string(),
            1,
            model,
            crate::SearchEvalMode::Incremental,
        )
        .unwrap();
        assert_eq!(
            summary.root_result.last_completed_iteration_value,
            Some(-crate::MATE_TT_THRESHOLD + 1)
        );
    }
}

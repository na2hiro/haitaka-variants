//! Scalar SFNN micro evaluator. File-format and arithmetic interoperability only;
//! independently implemented for Haitaka (MIT), not a port of YaneuraOu source.
//! See docs/tuyouraou11-sfnn.md for the pinned reference and verification procedure.
use haitaka::{Board, Color, Piece, Square};
use std::fmt;

pub const MICRO_ARCH: &str = "SFNN_ka2_2048_8_64_progress8";
const WIDTH: usize = 2048;
const FEATURES: usize = 1791;
const BONA_NONKING: usize = 1548;
const PROGRESS_COUNT: usize = 81 * BONA_NONKING;
const THRESHOLDS: [i64; 7] = [-127527, -71999, -33477, 0, 33477, 71999, 127527];
const DESCRIPTION: &str =
    "ModelType=SFNNWithoutPsqt;Features=K-A2(Friend)[1791->2048x2],Network=SFNN-2048{LayerStack=8}";

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

struct Affine {
    bias: Vec<i32>,
    weights: Vec<i8>,
    stride: usize,
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
        })
    }
    fn forward(&self, inputs: &[u8]) -> Vec<i32> {
        self.bias
            .iter()
            .enumerate()
            .map(|(row, &bias)| {
                inputs.iter().enumerate().fold(bias, |sum, (col, &v)| {
                    sum.wrapping_add(
                        i32::from(v) * i32::from(self.weights[row * self.stride + col]),
                    )
                })
            })
            .collect()
    }
}
struct Stack {
    first: Affine,
    second: Affine,
    output: Affine,
}

/// Immutable weights; P1 deliberately does not install them in the search runtime.
pub struct SfnnModel {
    biases: Vec<i16>,
    weights: Vec<i16>,
    progress: Vec<i32>,
    stacks: Vec<Stack>,
}
impl SfnnModel {
    /// Strict micro-only loader. Error prefixes follow the P0 public contract.
    pub fn from_bytes(nn: &[u8], progress: &[u8], arch: &str) -> Result<Self, SfnnError> {
        if arch != MICRO_ARCH {
            return Err(invalid("SFNN_UNSUPPORTED_ARCH: expected micro arch"));
        }
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
        let parsed = Self::read_nn(nn).map_err(|e| invalid(format!("SFNN_INVALID_NN: {e}")))?;
        let progress =
            read_progress(progress).map_err(|e| invalid(format!("SFNN_INVALID_PROGRESS: {e}")))?;
        Ok(Self {
            biases: parsed.0,
            weights: parsed.1,
            stacks: parsed.2,
            progress,
        })
    }
    fn read_nn(bytes: &[u8]) -> Result<(Vec<i16>, Vec<i16>, Vec<Stack>), SfnnError> {
        let mut r = Reader { bytes, pos: 0 };
        r.expect_u32(0x7af32f16)?;
        r.expect_u32(0x3c203b32)?;
        let len = r.u32()? as usize;
        if r.take(len)? != DESCRIPTION.as_bytes() {
            return Err(invalid("micro architecture description mismatch"));
        }
        r.expect_u32(0x5f134ab8)?;
        let biases = r.leb_i16(WIDTH)?;
        let weights = r.leb_i16(WIDTH * FEATURES)?;
        let mut stacks = Vec::with_capacity(8);
        for _ in 0..8 {
            r.expect_u32(0x6333718a)?;
            stacks.push(Stack {
                first: Affine::read(&mut r, WIDTH, 8)?,
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
    pub fn evaluate_full_refresh(&self, board: &Board) -> SfnnEvaluation {
        let pieces = bona_pieces(board);
        let kings = [
            board.king(Color::Black) as usize,
            80 - board.king(Color::White) as usize,
        ];
        let sums = std::array::from_fn(|p| {
            pieces[p]
                .iter()
                .map(|&i| i64::from(self.progress[kings[p] * BONA_NONKING + i]))
                .sum::<i64>()
        });
        let sum = sums[0] + sums[1];
        let bucket = progress_bucket(sum);
        let mut transformed = vec![0u8; WIDTH];
        let stm = color_index(board.side_to_move());
        for (output_side, p) in [stm, 1 - stm].into_iter().enumerate() {
            let mut acc = self.biases.clone();
            let friendly = if p == 0 {
                board.king(Color::Black) as usize
            } else {
                80 - board.king(Color::White) as usize
            };
            let enemy = if p == 0 {
                board.king(Color::White) as usize
            } else {
                80 - board.king(Color::Black) as usize
            };
            // A2 includes both kings folded into one plane; K distinguishes colors.
            for index in pieces[p].iter().copied().chain([
                BONA_NONKING + friendly,
                BONA_NONKING + enemy,
                1629 + friendly,
                1629 + 81 + enemy,
            ]) {
                for (a, &w) in acc
                    .iter_mut()
                    .zip(&self.weights[index * WIDTH..(index + 1) * WIDTH])
                {
                    *a = a.wrapping_add(w);
                }
            }
            for j in 0..WIDTH / 2 {
                let a = i32::from(acc[j].clamp(0, 254));
                let b = i32::from(acc[j + WIDTH / 2].clamp(0, 254));
                transformed[output_side * (WIDTH / 2) + j] = ((a * b) >> 9) as u8;
            }
        }
        let stack = &self.stacks[bucket];
        let first = stack.first.forward(&transformed);
        let mut activated = vec![0; 16];
        for j in 0..8 {
            activated[j] = ((i64::from(first[j]) * i64::from(first[j]) >> 19).min(127)) as u8;
            activated[8 + j] = clip(first[j]);
        }
        let second: Vec<_> = stack
            .second
            .forward(&activated)
            .into_iter()
            .map(clip)
            .collect();
        let raw_output = stack.output.forward(&second)[0];
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
        let Some(pc) = board.colored_piece_on(square) else {
            continue;
        };
        let slot = match pc.piece {
            Piece::Pawn => 0,
            Piece::Lance => 1,
            Piece::Knight => 2,
            Piece::Silver => 3,
            Piece::Gold | Piece::Tokin | Piece::PLance | Piece::PKnight | Piece::PSilver => 4,
            Piece::Bishop => 5,
            Piece::PBishop => 6,
            Piece::Rook => 7,
            Piece::PRook => 8,
            Piece::King => continue,
        };
        for p in 0..2 {
            let color = usize::from(color_index(pc.color) != p);
            let sq = if p == 0 {
                square as usize
            } else {
                80 - square as usize
            };
            lists[p].push(90 + (slot * 2 + color) * 81 + sq);
        }
    }
    for (piece, friendly, enemy) in [
        (Piece::Pawn, 1, 20),
        (Piece::Lance, 39, 44),
        (Piece::Knight, 49, 54),
        (Piece::Silver, 59, 64),
        (Piece::Gold, 69, 74),
        (Piece::Bishop, 79, 82),
        (Piece::Rook, 85, 88),
    ] {
        for color in Color::ALL {
            for ordinal in 0..usize::from(board.num_in_hand(color, piece)) {
                for p in 0..2 {
                    lists[p].push(if color_index(color) == p {
                        friendly + ordinal
                    } else {
                        enemy + ordinal
                    });
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
        let err = SfnnModel::from_bytes(&[], &[], MICRO_ARCH).err().unwrap();
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

//! Inference for the P-type MNUE v5 network shipped with Minase v2.6.0.
//! Format and feature numbering are documented by Minase at tag v2.6.0.
use super::{Cell, ChuGame, ChuPosition, Outcome, PathMove, SearchGame};
use sha2::{Digest, Sha256};

const HEADER: usize = 192;
const SQUARES: usize = 144;
const PIECE_STATES: usize = 47;
const BOARD_FEATURES: usize = 2 * PIECE_STATES * SQUARES;
const FEATURES: usize = BOARD_FEATURES + SQUARES;
const WIDTH: usize = 128;
const ARCHITECTURE: &[u8] = b"features=P;ft=128;linear=tapered2;head=direct-screlu";

// Minase piece states in the order of CHU_PIECES. Promotions that acquire the
// same movement share Minase's unpromotable state, irrespective of origin.
const STATES: [usize; super::CHU_PIECES.len()] = [
    29, 31, 43, 44, 35, 36, 11, 37, 38, 20, 39, 42, 40, 32, 41, 45, 46, 30, 33, 34, 10, 17, 22, 5,
    7, 8, 9, 27, 28, 21, 4, 6, 23, 26, 20, 10, 12, 25, 24,
];

pub struct MinaseNnue {
    linear: Box<[[i16; 2]]>,
    weights: Box<[[i16; WIDTH]]>,
    bias: [i32; WIDTH],
    output: [[i16; WIDTH]; 2],
    output_bias: i32,
    scale: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Accumulator {
    hidden: [[i32; WIDTH]; 2],
    linear: [[i32; 2]; 2],
    pieces: usize,
    lion_square: Option<usize>,
}

impl MinaseNnue {
    pub const FORMAT: &'static str = "minase-mnue-v5";

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let invalid = |message| format!("NNUE_INVALID_MINASE: {message}");
        if !bytes.starts_with(b"MNUE\x05\0\0\0") || bytes.len() < HEADER {
            return Err(invalid("expected a Minase MNUE v5 file"));
        }
        let arch_end = 8 + ARCHITECTURE.len();
        if &bytes[8..arch_end] != ARCHITECTURE
            || bytes[arch_end..104].iter().any(|&byte| byte != 0)
            || u32::from_le_bytes(bytes[104..108].try_into().unwrap()) as usize != WIDTH
        {
            return Err(invalid(
                "expected the Minase v2.6 P-type 128 direct-SCReLU architecture",
            ));
        }
        let cp_per_logit = f32::from_le_bytes(bytes[108..112].try_into().unwrap());
        if !(0.0 < cp_per_logit && cp_per_logit <= 100_000.0)
            || bytes[176..HEADER].iter().any(|&byte| byte != 0)
        {
            return Err(invalid("invalid scale or reserved header bytes"));
        }
        // The model's move-ordering piece values are metadata for Minase's
        // search. Haitaka keeps its own move ordering.
        let linear_end = HEADER + FEATURES * 4;
        let weights_start = linear_end + PIECE_STATES * 4;
        let bias_start = weights_start + FEATURES * WIDTH * 2;
        let output_start = bias_start + WIDTH * 4;
        let output_bias_start = output_start + WIDTH * 4;
        if bytes.len() != output_bias_start + 4 {
            return Err(invalid("incorrect weight file length"));
        }
        if Sha256::digest(&bytes[HEADER..])[..] != bytes[144..176] {
            return Err(invalid("weight checksum mismatch"));
        }
        let linear = bytes[HEADER..linear_end]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| {
                [
                    i16::from_le_bytes([b[0], b[1]]),
                    i16::from_le_bytes([b[2], b[3]]),
                ]
            })
            .collect();
        let weights = bytes[weights_start..bias_start]
            .as_chunks::<{ WIDTH * 2 }>()
            .0
            .iter()
            .map(|b| read_i16_row(b))
            .collect::<Box<[_]>>();
        let bias = std::array::from_fn(|i| {
            i32::from_le_bytes(bytes[bias_start + i * 4..][..4].try_into().unwrap())
        });
        let output = std::array::from_fn(|side| {
            read_i16_row(&bytes[output_start + side * WIDTH * 2..][..WIDTH * 2])
        });
        let output_bias = i32::from_le_bytes(bytes[output_bias_start..].try_into().unwrap());
        if weights.iter().flatten().any(|v| v.unsigned_abs() > 1016)
            || bias.iter().any(|v| v.unsigned_abs() > 1016)
            || output.iter().flatten().any(|v| v.unsigned_abs() > 8192)
            || output_bias.unsigned_abs() > 4_161_536
        {
            return Err(invalid("quantized parameter out of range"));
        }
        Ok(Self {
            linear,
            weights,
            bias,
            output,
            output_bias,
            scale: (f64::from(cp_per_logit) * 65_536.0 / (127.0 * 4096.0)).round_ties_even() as i64,
        })
    }

    fn add_feature(&self, acc: &mut Accumulator, side: usize, feature: usize, sign: i32) {
        for (sum, weight) in acc.linear[side].iter_mut().zip(self.linear[feature]) {
            *sum += sign * i32::from(weight);
        }
        for (sum, &weight) in acc.hidden[side].iter_mut().zip(&self.weights[feature]) {
            *sum += sign * i32::from(weight);
        }
    }

    fn add_piece(&self, acc: &mut Accumulator, square: usize, cell: Cell, sign: i32) {
        let state = STATES[cell.piece.0 as usize];
        for side in 0..2 {
            let enemy = usize::from(cell.color as usize != side);
            let feature = (enemy * PIECE_STATES + state) * SQUARES + relative_square(square, side);
            self.add_feature(acc, side, feature, sign);
        }
    }

    fn set_lion_square(&self, acc: &mut Accumulator, square: Option<usize>) {
        if acc.lion_square == square {
            return;
        }
        for (target, sign) in [(acc.lion_square, -1), (square, 1)] {
            if let Some(target) = target {
                for side in 0..2 {
                    self.add_feature(
                        acc,
                        side,
                        BOARD_FEATURES + relative_square(target, side),
                        sign,
                    );
                }
            }
        }
        acc.lion_square = square;
    }

    fn refresh(&self, pos: &ChuPosition, lion_square: Option<usize>) -> Accumulator {
        let mut acc = Accumulator {
            hidden: [self.bias; 2],
            linear: [[0; 2]; 2],
            pieces: 0,
            lion_square: None,
        };
        for (square, cell) in pos.cells.iter().enumerate() {
            if let Some(cell) = cell {
                self.add_piece(&mut acc, square, *cell, 1);
                acc.pieces += 1;
            }
        }
        self.set_lion_square(&mut acc, lion_square);
        acc
    }

    fn after_move(
        &self,
        mut acc: Accumulator,
        before: &ChuPosition,
        after: &ChuPosition,
        mv: PathMove,
    ) -> Accumulator {
        let mut squares = [0; 3];
        for (i, at) in std::iter::once(mv.from)
            .chain(mv.path.iter().copied())
            .enumerate()
        {
            let square = ChuPosition::index(at).unwrap();
            if squares[..i].contains(&square) {
                continue;
            }
            squares[i] = square;
            if before.cells[square] == after.cells[square] {
                continue;
            }
            if let Some(cell) = before.cells[square] {
                self.add_piece(&mut acc, square, cell, -1);
                acc.pieces -= 1;
            }
            if let Some(cell) = after.cells[square] {
                self.add_piece(&mut acc, square, cell, 1);
                acc.pieces += 1;
            }
        }
        self.set_lion_square(&mut acc, lion_square(after, Some(mv)));
        acc
    }

    fn evaluate(&self, acc: &Accumulator, side: usize) -> i32 {
        let phase = acc.pieces.saturating_sub(2).min(90) as i64;
        let [opening, ending] = acc.linear[side].map(i64::from);
        let linear = (phase * opening + (90 - phase) * ending) / 720;
        let mut output = self.output_bias;
        for (perspective, weights) in [side, side ^ 1].into_iter().zip(&self.output) {
            for (&value, &weight) in acc.hidden[perspective].iter().zip(weights) {
                let value = value.clamp(0, 127);
                output += ((value * value) >> 7) * i32::from(weight);
            }
        }
        let nonlinear = (i64::from(output) * self.scale + 32_768) >> 16;
        (linear + nonlinear).clamp(-28_999, 28_999) as i32
    }
}

fn read_i16_row(bytes: &[u8]) -> [i16; WIDTH] {
    std::array::from_fn(|i| i16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]))
}

fn relative_square(square: usize, side: usize) -> usize {
    // Haitaka indexes from 1a in file-major order. Minase indexes features
    // from 12l in rank-major order for Black, reversing only ranks for White.
    let file = 11 - square / 12;
    let rank = square % 12;
    let rank = if side == 0 { 11 - rank } else { rank };
    rank * 12 + file
}

fn lion_square(position: &ChuPosition, last_move: Option<PathMove>) -> Option<usize> {
    // JSON v2 stores a flag. The capture square is available from a completed
    // path when the request includes that move in its history.
    if position.state.senjishi {
        let target = *last_move?.path.last()?;
        ChuPosition::index(target).ok()
    } else {
        None
    }
}

pub(super) struct NnueGame<'a> {
    game: ChuGame,
    model: &'a MinaseNnue,
    stack: Vec<Accumulator>,
}

impl<'a> NnueGame<'a> {
    pub(super) fn new(game: ChuGame, model: &'a MinaseNnue) -> Self {
        let pos = game.current();
        let root = model.refresh(pos, lion_square(pos, game.played.last().copied()));
        Self {
            game,
            model,
            stack: vec![root],
        }
    }
}

impl SearchGame for NnueGame<'_> {
    type Move = PathMove;
    const SCHEMA: u8 = ChuGame::SCHEMA;

    fn moves(&self, limit: usize, stop: &mut dyn FnMut() -> bool) -> Option<Vec<PathMove>> {
        SearchGame::moves(&self.game, limit, stop)
    }

    fn outcome(&self, stop: &mut dyn FnMut() -> bool) -> Option<Outcome> {
        SearchGame::outcome(&self.game, stop)
    }

    fn side(&self) -> u8 {
        self.game.side()
    }

    fn evaluate(&self) -> i32 {
        self.model
            .evaluate(self.stack.last().unwrap(), self.side() as usize)
    }

    fn push(&mut self, mv: PathMove) {
        let parent = self.game.history.len() - 1;
        self.game.push(mv);
        self.stack.push(self.model.after_move(
            self.stack.last().unwrap().clone(),
            &self.game.history[parent],
            self.game.current(),
            mv,
        ));
    }

    fn pop(&mut self) {
        self.game.pop();
        self.stack.pop();
    }

    fn key(&self) -> u64 {
        self.game.key()
    }

    fn capture_value(&self, mv: PathMove) -> i32 {
        self.game.capture_value(mv)
    }

    fn promotion(mv: PathMove) -> bool {
        ChuGame::promotion(mv)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extended::chu::{
        start_position_json,
        tests::{game, kings, one, two},
    };

    #[test]
    #[ignore = "requires Minase v2.6.0 nnue.bin via HAITAKA_MINASE_NNUE"]
    fn minase_nnue_scores_and_incremental_updates_match() {
        let path = std::env::var_os("HAITAKA_MINASE_NNUE").expect("set path to Minase nnue.bin");
        let mut bytes = std::fs::read(path).unwrap();
        let model = MinaseNnue::from_bytes(&bytes).unwrap();
        bytes[HEADER] ^= 1;
        assert!(matches!(MinaseNnue::from_bytes(&bytes), Err(e) if e.contains("checksum")));
        assert!(matches!(MinaseNnue::from_bytes(&bytes[..HEADER]), Err(e) if e.contains("length")));

        let start = ChuGame::parse(&start_position_json()).unwrap();
        let start = NnueGame::new(start, &model);
        assert_eq!(start.evaluate(), 28);

        // Expected scores come from Minase v2.6.0's USI eval command.
        let lion = game(
            &kings(&[([5, 5], "bl", 0), ([6, 5], "aa", 1), ([7, 5], "ad", 1)]),
            0,
            false,
        );
        let mut lion = NnueGame::new(lion, &model);
        check(&mut lion, two([5, 5], [6, 5], [7, 5]), -4493);
        check(&mut lion, two([5, 5], [6, 5], [5, 5]), -3845);
        assert_eq!(lion.evaluate(), 3784);

        let promotion = game(&kings(&[([5, 5], "ad", 0)]), 0, false);
        let mut promotion = NnueGame::new(promotion, &model);
        check(&mut promotion, one([5, 5], [5, 4], true), -1253);

        let capture = game(&kings(&[([5, 5], "ag", 0), ([5, 3], "bl", 1)]), 0, false);
        let mut capture = NnueGame::new(capture, &model);
        let take = one([5, 5], [5, 3], false);
        check(&mut capture, take, -1986);
        capture.push(take);
        check(&mut capture, one([12, 1], [11, 1], false), 2017);
    }

    fn check(game: &mut NnueGame<'_>, mv: PathMove, expected: i32) {
        assert!(game.game.moves().contains(&mv));
        let before = game.stack.last().unwrap().clone();
        game.push(mv);
        assert_eq!(game.evaluate(), expected);
        let pos = game.game.current();
        let refreshed = game.model.refresh(pos, lion_square(pos, Some(mv)));
        assert_eq!(game.stack.last().unwrap(), &refreshed);
        game.pop();
        assert_eq!(game.stack.last().unwrap(), &before);
    }
}

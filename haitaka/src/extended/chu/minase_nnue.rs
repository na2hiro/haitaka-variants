//! Inference and shared incremental features for Minase Chu shogi networks.
use super::{Cell, ChuGame, ChuPosition, Outcome, PathMove, SearchGame, lion};
use sha2::{Digest, Sha256};

const HEADER: usize = 192;
const SQUARES: usize = 144;
const PIECE_STATES: usize = 47;
const BOARD_FEATURES: usize = 2 * PIECE_STATES * SQUARES;
const FEATURES: usize = BOARD_FEATURES + SQUARES;
#[path = "minase_layer_stack.rs"]
mod layer_stack;
use layer_stack::LayerStack;

const WIDTH: usize = 128;
const ARCHITECTURE: &[u8] = b"features=P;ft=128;linear=tapered2;head=direct-screlu";

// Minase piece states in the order of CHU_PIECES. Promotions that acquire the
// same movement share Minase's unpromotable state, irrespective of origin.
const STATES: [usize; super::CHU_PIECES.len()] = [
    29, 31, 43, 44, 35, 36, 11, 37, 38, 20, 39, 42, 40, 32, 41, 45, 46, 30, 33, 34, 10, 17, 22, 5,
    7, 8, 9, 27, 28, 21, 4, 6, 23, 26, 20, 10, 12, 25, 24,
];

pub struct MinaseNnue(Network);

enum Network {
    V2(V2),
    V3(Box<LayerStack>),
}

impl MinaseNnue {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.starts_with(b"MNUE") {
            V2::from_bytes(bytes).map(|model| Self(Network::V2(model)))
        } else {
            LayerStack::from_bytes(bytes).map(|model| Self(Network::V3(Box::new(model))))
        }
    }

    pub fn format(&self) -> &'static str {
        match self.0 {
            Network::V2(_) => "minase-mnue-v5",
            Network::V3(_) => "minase-layer-stack",
        }
    }

    pub(super) fn search(
        &self,
        game: ChuGame,
        depth: u8,
        timeout_ms: u32,
        nodes: u32,
    ) -> Result<String, String> {
        match &self.0 {
            Network::V2(model) => super::search::search_json(
                &mut NnueGame::new(game, model),
                depth,
                timeout_ms,
                nodes,
            ),
            Network::V3(model) => super::search::search_json(
                &mut NnueGame::new(game, model.as_ref()),
                depth,
                timeout_ms,
                nodes,
            ),
        }
    }
}

struct V2 {
    linear: Box<[[i16; 2]]>,
    weights: Box<[[i16; WIDTH]]>,
    bias: [i32; WIDTH],
    output: [[i16; WIDTH]; 2],
    output_bias: i32,
    scale: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Accumulator<const N: usize> {
    hidden: [[i32; N]; 2],
    linear: [[i32; 2]; 2],
    pieces: usize,
    lion_square: Option<usize>,
}

impl V2 {
    fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
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
}

trait Model<const N: usize> {
    fn bias(&self) -> [i32; N];
    fn row(&self, feature: usize) -> &[i16; N];
    fn linear(&self, feature: usize) -> [i32; 2];
    fn evaluate(&self, acc: &Accumulator<N>, side: usize) -> i32;

    fn add_feature(&self, acc: &mut Accumulator<N>, side: usize, feature: usize, sign: i32) {
        for (sum, weight) in acc.linear[side].iter_mut().zip(self.linear(feature)) {
            *sum += sign * weight;
        }
        for (sum, &weight) in acc.hidden[side].iter_mut().zip(self.row(feature)) {
            *sum += sign * i32::from(weight);
        }
    }

    fn add_piece(&self, acc: &mut Accumulator<N>, square: usize, cell: Cell, sign: i32) {
        let state = STATES[cell.piece.0 as usize];
        for side in 0..2 {
            let enemy = usize::from(cell.color as usize != side);
            let feature = (enemy * PIECE_STATES + state) * SQUARES + relative_square(square, side);
            self.add_feature(acc, side, feature, sign);
        }
    }

    fn set_lion_square(&self, acc: &mut Accumulator<N>, square: Option<usize>) {
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

    fn refresh(&self, pos: &ChuPosition, lion_square: Option<usize>) -> Accumulator<N> {
        let mut acc = Accumulator {
            hidden: [self.bias(); 2],
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
        mut acc: Accumulator<N>,
        before: &ChuPosition,
        after: &ChuPosition,
        mv: PathMove,
    ) -> Accumulator<N> {
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
        self.set_lion_square(&mut acc, lion_capture_square(before, mv));
        acc
    }
}

impl Model<WIDTH> for V2 {
    fn bias(&self) -> [i32; WIDTH] {
        self.bias
    }

    fn row(&self, feature: usize) -> &[i16; WIDTH] {
        &self.weights[feature]
    }

    fn linear(&self, feature: usize) -> [i32; 2] {
        self.linear[feature].map(i32::from)
    }

    fn evaluate(&self, acc: &Accumulator<WIDTH>, side: usize) -> i32 {
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

fn lion_capture_square(before: &ChuPosition, mv: PathMove) -> Option<usize> {
    let mover = before.cells[ChuPosition::index(mv.from).unwrap()].unwrap();
    if lion(mover) {
        return None;
    }
    // Minase keeps the last lion captured anywhere in the completed move.
    // The Shogitter rule flag describes only the final hop.
    mv.path
        .iter()
        .rev()
        .map(|&at| ChuPosition::index(at).unwrap())
        .find(|&square| {
            before.cells[square].is_some_and(|cell| cell.color != mover.color && lion(cell))
        })
}

struct NnueGame<'a, M, const N: usize> {
    game: ChuGame,
    model: &'a M,
    stack: Vec<Accumulator<N>>,
}

impl<'a, M: Model<N>, const N: usize> NnueGame<'a, M, N> {
    fn new(game: ChuGame, model: &'a M) -> Self {
        let capture = game
            .played
            .last()
            .zip(game.history.iter().rev().nth(1))
            .and_then(|(&mv, before)| lion_capture_square(before, mv));
        let root = model.refresh(game.current(), capture);
        Self {
            game,
            model,
            stack: vec![root],
        }
    }
}

impl<M: Model<N>, const N: usize> SearchGame for NnueGame<'_, M, N> {
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
        let network = MinaseNnue::from_bytes(&bytes).unwrap();
        assert_eq!(network.format(), "minase-mnue-v5");
        let Network::V2(model) = network.0 else {
            unreachable!()
        };
        bytes[HEADER] ^= 1;
        assert!(matches!(MinaseNnue::from_bytes(&bytes), Err(e) if e.contains("checksum")));
        assert!(matches!(MinaseNnue::from_bytes(&bytes[..HEADER]), Err(e) if e.contains("length")));

        check_positions(&model, [28, 3784, -4493, -3845, -1253, -1986, 2017]);
    }

    #[test]
    #[ignore = "requires the public v3.0.1 A model via HAITAKA_MINASE_V3_NNUE"]
    fn minase_v3_matches_upstream_and_incremental_updates() {
        let path = std::env::var_os("HAITAKA_MINASE_V3_NNUE").expect("set path to the v3 network");
        let mut bytes = std::fs::read(path).unwrap();
        let network = MinaseNnue::from_bytes(&bytes).unwrap();
        assert_eq!(network.format(), "minase-layer-stack");
        let Network::V3(model) = network.0 else {
            unreachable!()
        };
        // Scores from upstream v3.0.1's Network::evaluate.
        check_positions(model.as_ref(), [35, 3021, -3777, -3120, -664, -859, 970]);
        let mut falcon = NnueGame::new(
            game(
                &kings(&[([5, 5], "cz", 0), ([5, 4], "bl", 1), ([5, 3], "aa", 1)]),
                0,
                false,
            ),
            model.as_ref(),
        );
        // Minase's extended SFEN records the lion square 5d in both cases.
        check(&mut falcon, two([5, 5], [5, 4], [5, 5]), -1755);
        let take_twice = two([5, 5], [5, 4], [5, 3]);
        check(&mut falcon, take_twice, -2038);
        falcon.push(take_twice);
        let restored = ChuGame::parse(&falcon.game.to_json()).unwrap();
        assert_eq!(NnueGame::new(restored, model.as_ref()).evaluate(), -2038);

        assert!(MinaseNnue::from_bytes(&bytes[..bytes.len() - 1]).is_err());
        bytes.push(0);
        assert!(MinaseNnue::from_bytes(&bytes).is_err());
        bytes.pop();
        bytes[4] ^= 1;
        assert!(matches!(MinaseNnue::from_bytes(&bytes), Err(e) if e.contains("hash mismatch")));
    }

    fn check_positions<M: Model<N>, const N: usize>(model: &M, scores: [i32; 7]) {
        let [
            initial,
            lion,
            double_capture,
            igui,
            promote,
            senjishi,
            clear_senjishi,
        ] = scores;
        let start = ChuGame::parse(&start_position_json()).unwrap();
        assert_eq!(NnueGame::new(start, model).evaluate(), initial);

        let lion_game = game(
            &kings(&[([5, 5], "bl", 0), ([6, 5], "aa", 1), ([7, 5], "ad", 1)]),
            0,
            false,
        );
        let mut lion_game = NnueGame::new(lion_game, model);
        check(&mut lion_game, two([5, 5], [6, 5], [7, 5]), double_capture);
        check(&mut lion_game, two([5, 5], [6, 5], [5, 5]), igui);
        assert_eq!(lion_game.evaluate(), lion);

        let promotion = game(&kings(&[([5, 5], "ad", 0)]), 0, false);
        let mut promotion = NnueGame::new(promotion, model);
        check(&mut promotion, one([5, 5], [5, 4], true), promote);

        let capture = game(&kings(&[([5, 5], "ag", 0), ([5, 3], "bl", 1)]), 0, false);
        let mut capture = NnueGame::new(capture, model);
        let take = one([5, 5], [5, 3], false);
        check(&mut capture, take, senjishi);
        capture.push(take);
        check(&mut capture, one([12, 1], [11, 1], false), clear_senjishi);
    }

    fn check<M: Model<N>, const N: usize>(
        game: &mut NnueGame<'_, M, N>,
        mv: PathMove,
        expected: i32,
    ) {
        assert!(game.game.moves().contains(&mv));
        let before = game.stack.last().unwrap().clone();
        game.push(mv);
        assert_eq!(game.evaluate(), expected);
        let refreshed = NnueGame::new(game.game.clone(), game.model);
        assert_eq!(game.stack.last().unwrap(), refreshed.stack.last().unwrap());
        game.pop();
        assert_eq!(game.stack.last().unwrap(), &before);
    }
}

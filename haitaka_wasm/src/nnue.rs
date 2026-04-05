use std::fmt;

use haitaka::{Board, Color, Move, Piece, Square};

const VERSION: u32 = 0x7AF32F20;
const FEATURE_SET_HASH: u32 = 0x5f234cb8;
const TRANSFORMED_FEATURE_DIMENSIONS: usize = 512;
const FEATURE_TRANSFORMER_OUTPUT_DIMENSIONS: usize = TRANSFORMED_FEATURE_DIMENSIONS * 2;
const PSQT_BUCKETS: usize = 8;
const LAYER_STACKS: usize = 8;
const HIDDEN_LAYER_1_DIMENSIONS: usize = 16;
const HIDDEN_LAYER_2_DIMENSIONS: usize = 32;
const OUTPUT_LAYER_DIMENSIONS: usize = 1;
const OUTPUT_SCALE: i32 = 16;
const WEIGHT_SCALE_BITS: i32 = 6;
const SQUARES: usize = 81;
const POCKETS: usize = 18;
const MAX_ACTIVE_FEATURES: usize = 128;
const MAX_DELTA_FEATURES: usize = 4;
const MAX_PIECES: usize = 40;
const PIECE_TYPE_COUNT: usize = 10;
const NON_DROP_PIECE_INDICES: usize = (2 * PIECE_TYPE_COUNT - 1) * SQUARES;
const PIECE_INDICES: usize = NON_DROP_PIECE_INDICES + 2 * (PIECE_TYPE_COUNT - 1) * POCKETS;
const NNUE_DIMENSIONS: usize = SQUARES * PIECE_INDICES;
const PADDED_HIDDEN_LAYER_1_INPUT_DIMENSIONS: usize = FEATURE_TRANSFORMER_OUTPUT_DIMENSIONS;
const PADDED_HIDDEN_LAYER_2_INPUT_DIMENSIONS: usize = 32;
const PADDED_OUTPUT_INPUT_DIMENSIONS: usize = 32;
const HAND_PIECES: [Piece; Piece::HAND_NUM] = [
    Piece::Pawn,
    Piece::Lance,
    Piece::Knight,
    Piece::Silver,
    Piece::Bishop,
    Piece::Rook,
    Piece::Gold,
];

const fn input_slice_hash(output_dimensions: u32, offset: u32) -> u32 {
    0xEC42E90D ^ output_dimensions ^ (offset << 10)
}

const fn affine_hash(previous_hash: u32, output_dimensions: u32) -> u32 {
    let mut hash = 0xCC03DAE4u32;
    hash = hash.wrapping_add(output_dimensions);
    hash ^= previous_hash >> 1;
    hash ^= previous_hash << 31;
    hash
}

const fn clipped_relu_hash(previous_hash: u32) -> u32 {
    0x538D24C7u32.wrapping_add(previous_hash)
}

const FEATURE_TRANSFORMER_HASH: u32 =
    FEATURE_SET_HASH ^ FEATURE_TRANSFORMER_OUTPUT_DIMENSIONS as u32;
const INPUT_LAYER_HASH: u32 = input_slice_hash(FEATURE_TRANSFORMER_OUTPUT_DIMENSIONS as u32, 0);
const HIDDEN_LAYER_1_AFFINE_HASH: u32 =
    affine_hash(INPUT_LAYER_HASH, HIDDEN_LAYER_1_DIMENSIONS as u32);
const HIDDEN_LAYER_1_HASH: u32 = clipped_relu_hash(HIDDEN_LAYER_1_AFFINE_HASH);
const HIDDEN_LAYER_2_AFFINE_HASH: u32 =
    affine_hash(HIDDEN_LAYER_1_HASH, HIDDEN_LAYER_2_DIMENSIONS as u32);
const HIDDEN_LAYER_2_HASH: u32 = clipped_relu_hash(HIDDEN_LAYER_2_AFFINE_HASH);
const OUTPUT_LAYER_HASH: u32 = affine_hash(HIDDEN_LAYER_2_HASH, OUTPUT_LAYER_DIMENSIONS as u32);
const NETWORK_HASH: u32 = FEATURE_TRANSFORMER_HASH ^ OUTPUT_LAYER_HASH;

#[derive(Debug, Clone)]
pub struct NnueModel {
    description: String,
    transformer: FeatureTransformer,
    buckets: Vec<BucketNetwork>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerspectiveAccumulator {
    king_square: Square,
    sums: [i16; TRANSFORMED_FEATURE_DIMENSIONS],
    psqt: [i32; PSQT_BUCKETS],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NnuePositionState {
    perspectives: [PerspectiveAccumulator; Color::NUM],
}

impl NnuePositionState {
    fn perspective(&self, color: Color) -> &PerspectiveAccumulator {
        &self.perspectives[perspective_index(color)]
    }

    fn perspective_mut(&mut self, color: Color) -> &mut PerspectiveAccumulator {
        &mut self.perspectives[perspective_index(color)]
    }
}

impl NnueModel {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NnueError> {
        let mut reader = ByteReader::new(bytes);
        let version = reader.read_u32()?;
        if version != VERSION {
            return Err(NnueError::new(format!(
                "unsupported NNUE version: expected 0x{VERSION:08x}, got 0x{version:08x}"
            )));
        }

        let hash = reader.read_u32()?;
        if hash != NETWORK_HASH {
            return Err(NnueError::new(format!(
                "unexpected NNUE hash: expected 0x{NETWORK_HASH:08x}, got 0x{hash:08x}"
            )));
        }

        let description_length = reader.read_u32()? as usize;
        let description = reader.read_string(description_length)?;

        reader.read_section_header(FEATURE_TRANSFORMER_HASH)?;
        let transformer = FeatureTransformer::read(&mut reader)?;

        let mut buckets = Vec::with_capacity(LAYER_STACKS);
        for _ in 0..LAYER_STACKS {
            reader.read_section_header(OUTPUT_LAYER_HASH)?;
            buckets.push(BucketNetwork::read(&mut reader)?);
        }

        reader.ensure_finished()?;

        Ok(Self {
            description,
            transformer,
            buckets,
        })
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn evaluate(&self, board: &Board) -> i32 {
        self.evaluate_full_refresh(board)
    }

    pub fn build_position_state_full(&self, board: &Board) -> NnuePositionState {
        NnuePositionState {
            perspectives: [
                self.transformer
                    .build_perspective_accumulator(board, Color::Black),
                self.transformer
                    .build_perspective_accumulator(board, Color::White),
            ],
        }
    }

    pub fn evaluate_from_state(&self, board: &Board, state: &NnuePositionState) -> i32 {
        let bucket = bucket_for_board(board);
        let us = board.side_to_move();
        let them = !us;
        let our_accumulator = state.perspective(us);
        let their_accumulator = state.perspective(them);

        let mut transformed = [0u8; FEATURE_TRANSFORMER_OUTPUT_DIMENSIONS];
        fill_transformed_features(
            &mut transformed[..TRANSFORMED_FEATURE_DIMENSIONS],
            &our_accumulator.sums,
        );
        fill_transformed_features(
            &mut transformed[TRANSFORMED_FEATURE_DIMENSIONS..],
            &their_accumulator.sums,
        );

        let bucket_network = &self.buckets[bucket];
        let mut hidden1 = [0i32; HIDDEN_LAYER_1_DIMENSIONS];
        bucket_network.hidden1.forward_into(&transformed, &mut hidden1);
        let hidden1_relu = clipped_relu_array(hidden1);
        let mut hidden2 = [0i32; HIDDEN_LAYER_2_DIMENSIONS];
        bucket_network.hidden2.forward_into(&hidden1_relu, &mut hidden2);
        let hidden2_relu = clipped_relu_array(hidden2);
        let output = bucket_network.output.forward_single(&hidden2_relu);
        let psqt = (our_accumulator.psqt[bucket] - their_accumulator.psqt[bucket]) / 2;

        (psqt + output) / OUTPUT_SCALE
    }

    pub fn evaluate_full_refresh(&self, board: &Board) -> i32 {
        let state = self.build_position_state_full(board);
        self.evaluate_from_state(board, &state)
    }

    pub fn apply_move(
        &self,
        parent_board: &Board,
        child_board: &Board,
        parent_state: &NnuePositionState,
        mv: Move,
    ) -> NnuePositionState {
        debug_assert_eq!(child_board.side_to_move(), !parent_board.side_to_move());

        let mut child_state = *parent_state;
        for &perspective in &[Color::Black, Color::White] {
            let new_king_square = child_board.king(perspective);
            let parent_accumulator = parent_state.perspective(perspective);

            if new_king_square != parent_accumulator.king_square {
                *child_state.perspective_mut(perspective) = self
                    .transformer
                    .build_perspective_accumulator(child_board, perspective);
            } else {
                let delta = build_feature_delta(parent_board, perspective, parent_accumulator, mv);
                let perspective_accumulator = child_state.perspective_mut(perspective);
                self.transformer
                    .apply_delta(perspective_accumulator, &delta);
                perspective_accumulator.king_square = new_king_square;
            }
        }

        child_state
    }
}

#[derive(Debug, Clone)]
struct FeatureTransformer {
    biases: Vec<i16>,
    weights: Vec<i16>,
    psqt_weights: Vec<i32>,
}

impl FeatureTransformer {
    fn read(reader: &mut ByteReader<'_>) -> Result<Self, NnueError> {
        Ok(Self {
            biases: reader.read_i16_vec(TRANSFORMED_FEATURE_DIMENSIONS)?,
            weights: reader.read_i16_vec(TRANSFORMED_FEATURE_DIMENSIONS * NNUE_DIMENSIONS)?,
            psqt_weights: reader.read_i32_vec(PSQT_BUCKETS * NNUE_DIMENSIONS)?,
        })
    }

    fn build_perspective_accumulator(
        &self,
        board: &Board,
        perspective: Color,
    ) -> PerspectiveAccumulator {
        let king_square = board.king(perspective);
        let mut sums = [0i16; TRANSFORMED_FEATURE_DIMENSIONS];
        sums.copy_from_slice(&self.biases);
        let mut psqt = [0i32; PSQT_BUCKETS];
        let features = active_features(board, perspective, king_square);

        for &index in features.iter() {
            self.add_feature_to_arrays(&mut sums, &mut psqt, index);
        }

        PerspectiveAccumulator {
            king_square,
            sums,
            psqt,
        }
    }

    fn apply_delta(&self, accumulator: &mut PerspectiveAccumulator, delta: &FeatureDelta) {
        for &index in delta.removed() {
            self.remove_feature_from_arrays(&mut accumulator.sums, &mut accumulator.psqt, index);
        }

        for &index in delta.added() {
            self.add_feature_to_arrays(&mut accumulator.sums, &mut accumulator.psqt, index);
        }
    }

    fn add_feature_to_arrays(
        &self,
        sums: &mut [i16; TRANSFORMED_FEATURE_DIMENSIONS],
        psqt: &mut [i32; PSQT_BUCKETS],
        index: usize,
    ) {
        let weight_offset = index * TRANSFORMED_FEATURE_DIMENSIONS;
        for (sum, &weight) in sums
            .iter_mut()
            .zip(&self.weights[weight_offset..weight_offset + TRANSFORMED_FEATURE_DIMENSIONS])
        {
            *sum = add_i16(*sum, weight);
        }

        let psqt_offset = index * PSQT_BUCKETS;
        for (dst, &weight) in psqt
            .iter_mut()
            .zip(&self.psqt_weights[psqt_offset..psqt_offset + PSQT_BUCKETS])
        {
            *dst += weight;
        }
    }

    fn remove_feature_from_arrays(
        &self,
        sums: &mut [i16; TRANSFORMED_FEATURE_DIMENSIONS],
        psqt: &mut [i32; PSQT_BUCKETS],
        index: usize,
    ) {
        let weight_offset = index * TRANSFORMED_FEATURE_DIMENSIONS;
        for (sum, &weight) in sums
            .iter_mut()
            .zip(&self.weights[weight_offset..weight_offset + TRANSFORMED_FEATURE_DIMENSIONS])
        {
            *sum = sub_i16(*sum, weight);
        }

        let psqt_offset = index * PSQT_BUCKETS;
        for (dst, &weight) in psqt
            .iter_mut()
            .zip(&self.psqt_weights[psqt_offset..psqt_offset + PSQT_BUCKETS])
        {
            *dst -= weight;
        }
    }
}

#[derive(Debug, Clone)]
struct BucketNetwork {
    hidden1: AffineLayer,
    hidden2: AffineLayer,
    output: AffineLayer,
}

impl BucketNetwork {
    fn read(reader: &mut ByteReader<'_>) -> Result<Self, NnueError> {
        Ok(Self {
            hidden1: AffineLayer::read(
                reader,
                HIDDEN_LAYER_1_DIMENSIONS,
                PADDED_HIDDEN_LAYER_1_INPUT_DIMENSIONS,
            )?,
            hidden2: AffineLayer::read(
                reader,
                HIDDEN_LAYER_2_DIMENSIONS,
                PADDED_HIDDEN_LAYER_2_INPUT_DIMENSIONS,
            )?,
            output: AffineLayer::read(
                reader,
                OUTPUT_LAYER_DIMENSIONS,
                PADDED_OUTPUT_INPUT_DIMENSIONS,
            )?,
        })
    }
}

#[derive(Debug, Clone)]
struct AffineLayer {
    output_dimensions: usize,
    padded_input_dimensions: usize,
    biases: Vec<i32>,
    weights: Vec<i8>,
}

impl AffineLayer {
    fn read(
        reader: &mut ByteReader<'_>,
        output_dimensions: usize,
        padded_input_dimensions: usize,
    ) -> Result<Self, NnueError> {
        Ok(Self {
            output_dimensions,
            padded_input_dimensions,
            biases: reader.read_i32_vec(output_dimensions)?,
            weights: reader.read_i8_vec(output_dimensions * padded_input_dimensions)?,
        })
    }

    fn forward_into(&self, input: &[u8], output: &mut [i32]) {
        debug_assert_eq!(output.len(), self.output_dimensions);
        for (row, out) in output.iter_mut().enumerate() {
            let offset = row * self.padded_input_dimensions;
            let mut sum = self.biases[row];
            for (value, &weight) in input
                .iter()
                .zip(&self.weights[offset..offset + input.len()])
            {
                sum += i32::from(weight) * i32::from(*value);
            }
            *out = sum;
        }
    }

    fn forward_single(&self, input: &[u8]) -> i32 {
        debug_assert_eq!(self.output_dimensions, 1);
        let mut sum = self.biases[0];
        for (value, &weight) in input.iter().zip(&self.weights[..input.len()]) {
            sum += i32::from(weight) * i32::from(*value);
        }
        sum
    }
}

#[derive(Debug, Clone, Copy)]
struct FeatureDelta {
    removed: [usize; MAX_DELTA_FEATURES],
    removed_len: usize,
    added: [usize; MAX_DELTA_FEATURES],
    added_len: usize,
}

impl FeatureDelta {
    fn push_removed(&mut self, index: usize) {
        debug_assert!(self.removed_len < MAX_DELTA_FEATURES);
        self.removed[self.removed_len] = index;
        self.removed_len += 1;
    }

    fn push_added(&mut self, index: usize) {
        debug_assert!(self.added_len < MAX_DELTA_FEATURES);
        self.added[self.added_len] = index;
        self.added_len += 1;
    }

    fn removed(&self) -> &[usize] {
        &self.removed[..self.removed_len]
    }

    fn added(&self) -> &[usize] {
        &self.added[..self.added_len]
    }
}

#[derive(Debug, Clone, Copy)]
struct ActiveFeatures {
    indices: [usize; MAX_ACTIVE_FEATURES],
    len: usize,
}

impl ActiveFeatures {
    fn push(&mut self, index: usize) {
        debug_assert!(self.len < MAX_ACTIVE_FEATURES);
        self.indices[self.len] = index;
        self.len += 1;
    }

    fn iter(&self) -> impl Iterator<Item = &usize> {
        self.indices[..self.len].iter()
    }
}

impl Default for ActiveFeatures {
    fn default() -> Self {
        Self {
            indices: [0; MAX_ACTIVE_FEATURES],
            len: 0,
        }
    }
}

#[derive(Debug, Clone)]
struct ByteReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> ByteReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn read_section_header(&mut self, expected: u32) -> Result<(), NnueError> {
        let header = self.read_u32()?;
        if header != expected {
            return Err(NnueError::new(format!(
                "unexpected section hash: expected 0x{expected:08x}, got 0x{header:08x}"
            )));
        }
        Ok(())
    }

    fn read_string(&mut self, len: usize) -> Result<String, NnueError> {
        let bytes = self.read_bytes(len)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }

    fn read_u32(&mut self) -> Result<u32, NnueError> {
        let bytes = self.read_bytes(4)?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn read_i16_vec(&mut self, count: usize) -> Result<Vec<i16>, NnueError> {
        let bytes = self.read_bytes(count * 2)?;
        let mut values = Vec::with_capacity(count);
        for chunk in bytes.chunks_exact(2) {
            values.push(i16::from_le_bytes([chunk[0], chunk[1]]));
        }
        Ok(values)
    }

    fn read_i32_vec(&mut self, count: usize) -> Result<Vec<i32>, NnueError> {
        let bytes = self.read_bytes(count * 4)?;
        let mut values = Vec::with_capacity(count);
        for chunk in bytes.chunks_exact(4) {
            values.push(i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
        Ok(values)
    }

    fn read_i8_vec(&mut self, count: usize) -> Result<Vec<i8>, NnueError> {
        let bytes = self.read_bytes(count)?;
        Ok(bytes.iter().map(|&byte| byte as i8).collect())
    }

    fn ensure_finished(&self) -> Result<(), NnueError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(NnueError::new(format!(
                "trailing bytes after NNUE payload: {}",
                self.bytes.len() - self.offset
            )))
        }
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], NnueError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| NnueError::new("NNUE read overflow"))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| NnueError::new("unexpected end of NNUE file"))?;
        self.offset = end;
        Ok(bytes)
    }
}

#[derive(Debug, Clone)]
pub struct NnueError {
    message: String,
}

impl NnueError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for NnueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for NnueError {}

fn add_i16(lhs: i16, rhs: i16) -> i16 {
    let value = i32::from(lhs) + i32::from(rhs);
    debug_assert!((i16::MIN as i32..=i16::MAX as i32).contains(&value));
    value as i16
}

fn sub_i16(lhs: i16, rhs: i16) -> i16 {
    let value = i32::from(lhs) - i32::from(rhs);
    debug_assert!((i16::MIN as i32..=i16::MAX as i32).contains(&value));
    value as i16
}

fn fill_transformed_features(
    output: &mut [u8],
    accumulator: &[i16; TRANSFORMED_FEATURE_DIMENSIONS],
) {
    for (dst, &value) in output.iter_mut().zip(accumulator) {
        *dst = clamp_transformed(value);
    }
}

fn clamp_transformed(value: i16) -> u8 {
    let shifted = i32::from(value) >> WEIGHT_SCALE_BITS;
    shifted.clamp(0, 127) as u8
}

fn clipped_relu_array<const DIMENSIONS: usize>(input: [i32; DIMENSIONS]) -> [u8; DIMENSIONS] {
    let mut output = [0; DIMENSIONS];
    for (dst, value) in output.iter_mut().zip(input) {
        *dst = (value >> WEIGHT_SCALE_BITS).clamp(0, 127) as u8;
    }
    output
}

fn bucket_for_board(board: &Board) -> usize {
    let piece_count = usize::try_from(board.occupied().len()).unwrap_or(0).max(1);
    (((piece_count - 1) * PSQT_BUCKETS) / MAX_PIECES).min(PSQT_BUCKETS - 1)
}

fn build_feature_delta(
    parent_board: &Board,
    perspective: Color,
    accumulator: &PerspectiveAccumulator,
    mv: Move,
) -> FeatureDelta {
    let mover = parent_board.side_to_move();
    let king_offset = king_offset(accumulator.king_square, perspective);
    let mut delta = FeatureDelta {
        removed: [0; MAX_DELTA_FEATURES],
        removed_len: 0,
        added: [0; MAX_DELTA_FEATURES],
        added_len: 0,
    };

    match mv {
        Move::Drop { piece, to } => {
            let old_hand_count = usize::from(parent_board.num_in_hand(mover, piece));
            debug_assert!(old_hand_count > 0);
            delta.push_removed(hand_feature_index(
                perspective,
                king_offset,
                mover,
                piece,
                old_hand_count - 1,
            ));
            delta.push_added(board_feature_index(
                perspective,
                king_offset,
                mover,
                piece,
                to,
            ));
        }
        Move::BoardMove {
            from,
            to,
            promotion,
        } => {
            let moving_piece = parent_board
                .piece_on(from)
                .expect("moving piece should exist on source square");
            let final_piece = if promotion {
                moving_piece.promote()
            } else {
                moving_piece
            };

            delta.push_removed(board_feature_index(
                perspective,
                king_offset,
                mover,
                moving_piece,
                from,
            ));

            if let Some(captured_piece) = parent_board.piece_on(to) {
                delta.push_removed(board_feature_index(
                    perspective,
                    king_offset,
                    !mover,
                    captured_piece,
                    to,
                ));
                if captured_piece != Piece::King {
                    let hand_piece = captured_piece.unpromote();
                    let old_hand_count = usize::from(parent_board.num_in_hand(mover, hand_piece));
                    delta.push_added(hand_feature_index(
                        perspective,
                        king_offset,
                        mover,
                        hand_piece,
                        old_hand_count,
                    ));
                }
            }

            delta.push_added(board_feature_index(
                perspective,
                king_offset,
                mover,
                final_piece,
                to,
            ));
        }
    }

    delta
}

fn active_features(board: &Board, perspective: Color, king_square: Square) -> ActiveFeatures {
    let king_offset = king_offset(king_square, perspective);
    let mut features = ActiveFeatures::default();

    for &square in &Square::ALL {
        let Some(colored_piece) = board.colored_piece_on(square) else {
            continue;
        };

        features.push(board_feature_index(
            perspective,
            king_offset,
            colored_piece.color,
            colored_piece.piece,
            square,
        ));
    }

    for &color in &[Color::Black, Color::White] {
        for &piece in &HAND_PIECES {
            let count = usize::from(board.num_in_hand(color, piece));
            for copy_index in 0..count {
                features.push(hand_feature_index(
                    perspective,
                    king_offset,
                    color,
                    piece,
                    copy_index,
                ));
            }
        }
    }

    features
}

fn king_offset(king_square: Square, perspective: Color) -> usize {
    orient_square(king_square, perspective) * PIECE_INDICES
}

fn board_feature_index(
    perspective: Color,
    king_offset: usize,
    color: Color,
    piece: Piece,
    square: Square,
) -> usize {
    king_offset + piece_square_index(perspective, color, piece) + orient_square(square, perspective)
}

fn hand_feature_index(
    perspective: Color,
    king_offset: usize,
    color: Color,
    piece: Piece,
    copy_index: usize,
) -> usize {
    king_offset + piece_hand_index(perspective, color, piece) + copy_index
}

fn orient_square(square: Square, perspective: Color) -> usize {
    let compact = variant_square(square);
    if perspective == Color::Black {
        compact
    } else {
        let file = compact % 9;
        let rank = compact / 9;
        file + (8 - rank) * 9
    }
}

fn variant_square(square: Square) -> usize {
    let file = 8usize - square.file() as usize;
    let rank = 8usize - square.rank() as usize;
    file + rank * 9
}

fn piece_square_index(perspective: Color, color: Color, piece: Piece) -> usize {
    let slot = piece_slot(piece);
    let color_offset = if piece == Piece::King {
        0
    } else if color == perspective {
        0
    } else {
        1
    };

    (2 * slot + color_offset) * SQUARES
}

fn piece_hand_index(perspective: Color, color: Color, piece: Piece) -> usize {
    let slot = piece_slot(piece.unpromote());
    debug_assert!(slot < PIECE_TYPE_COUNT - 1);
    let color_offset = if color == perspective { 0 } else { 1 };
    NON_DROP_PIECE_INDICES + (2 * slot + color_offset) * POCKETS
}

fn perspective_index(color: Color) -> usize {
    match color {
        Color::Black => 0,
        Color::White => 1,
    }
}

fn piece_slot(piece: Piece) -> usize {
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

#[cfg(test)]
mod tests {
    use super::*;
    use rand::prelude::IndexedRandom;
    use rand::rng;
    use std::path::PathBuf;

    fn load_test_nnue() -> Option<NnueModel> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shogi-878ca61334a7.nnue");
        let bytes = std::fs::read(path).ok()?;
        NnueModel::from_bytes(&bytes).ok()
    }

    fn collect_legal_moves(board: &Board) -> Vec<Move> {
        let mut moves = Vec::new();
        board.generate_moves(|piece_moves| {
            moves.extend(piece_moves);
            false
        });
        moves
    }

    fn assert_single_move_matches_full(sfen: &str, mv: &str) {
        let Some(model) = load_test_nnue() else {
            return;
        };

        let board = Board::from_sfen(sfen).unwrap();
        let mv: Move = mv.parse().unwrap();
        assert!(board.is_legal(mv), "move {mv} should be legal for {sfen}");

        let parent_state = model.build_position_state_full(&board);
        let mut child = board.clone();
        child.play_unchecked(mv);

        let incremental = model.apply_move(&board, &child, &parent_state, mv);
        let full_refresh = model.build_position_state_full(&child);

        assert_eq!(incremental, full_refresh);
        assert_eq!(
            model.evaluate_from_state(&child, &incremental),
            model.evaluate_from_state(&child, &full_refresh)
        );
        assert_eq!(
            model.evaluate_from_state(&child, &incremental),
            model.evaluate_full_refresh(&child)
        );
    }

    fn deterministic_rollout_matches_full(mut board: Board, plies: usize) {
        let Some(model) = load_test_nnue() else {
            return;
        };

        let mut state = model.build_position_state_full(&board);
        for _ in 0..plies {
            let mut moves = collect_legal_moves(&board);
            if moves.is_empty() {
                break;
            }
            moves.sort_unstable_by_key(|mv| mv.to_string());
            let mv = moves[0];
            let mut child = board.clone();
            child.play_unchecked(mv);

            let incremental = model.apply_move(&board, &child, &state, mv);
            let full_refresh = model.build_position_state_full(&child);
            assert_eq!(incremental, full_refresh);
            assert_eq!(
                model.evaluate_from_state(&child, &incremental),
                model.evaluate_full_refresh(&child)
            );

            board = child;
            state = incremental;
        }
    }

    #[test]
    fn computes_expected_piece_constants() {
        assert_eq!(PIECE_INDICES, 1_863);
        assert_eq!(NNUE_DIMENSIONS, 150_903);
        assert_eq!(NETWORK_HASH, 0x3c103e72);
    }

    #[test]
    fn maps_shogi_squares_to_variant_indices() {
        assert_eq!(variant_square(Square::A9), 72);
        assert_eq!(variant_square(Square::I1), 8);
        assert_eq!(variant_square(Square::A1), 80);
        assert_eq!(variant_square(Square::I9), 0);
    }

    #[test]
    fn orients_white_perspective_by_rank_flip() {
        assert_eq!(orient_square(Square::A9, Color::Black), 72);
        assert_eq!(orient_square(Square::A9, Color::White), 0);
        assert_eq!(orient_square(Square::A1, Color::Black), 80);
        assert_eq!(orient_square(Square::A1, Color::White), 8);
    }

    #[test]
    fn incremental_matches_full_for_quiet_move() {
        assert_single_move_matches_full(haitaka::SFEN_STARTPOS, "7g7f");
    }

    #[test]
    fn incremental_matches_full_for_capture() {
        assert_single_move_matches_full("9/9/k8/9/4Rr3/9/9/9/4K4 b - 1", "5e4e");
    }

    #[test]
    fn incremental_matches_full_for_promotion() {
        assert_single_move_matches_full("k8/4P4/9/9/9/9/9/9/4K4 b - 1", "5b5a+");
    }

    #[test]
    fn incremental_matches_full_for_drop() {
        assert_single_move_matches_full("4k4/9/9/9/9/9/9/9/4K4 b P 1", "P*5h");
    }

    #[test]
    fn incremental_matches_full_for_king_move() {
        assert_single_move_matches_full("4k4/9/9/9/9/9/9/9/4K4 b - 1", "5i5h");
    }

    #[test]
    fn deterministic_sequence_matches_full_from_start_position() {
        deterministic_rollout_matches_full(Board::startpos(), 24);
    }

    #[test]
    fn deterministic_sequence_matches_full_from_handicap_position() {
        deterministic_rollout_matches_full(Board::from_sfen(haitaka::SFEN_6PIECE_HANDICAP).unwrap(), 16);
    }

    #[test]
    fn random_rollouts_match_full_refresh() {
        let Some(model) = load_test_nnue() else {
            return;
        };

        let mut rng = rng();
        for _ in 0..16 {
            let mut board = Board::startpos();
            let mut state = model.build_position_state_full(&board);

            for _ in 0..32 {
                let moves = collect_legal_moves(&board);
                let Some(&mv) = moves.choose(&mut rng) else {
                    break;
                };

                let mut child = board.clone();
                child.play_unchecked(mv);
                let incremental = model.apply_move(&board, &child, &state, mv);
                let full_refresh = model.build_position_state_full(&child);
                assert_eq!(incremental, full_refresh);
                assert_eq!(
                    model.evaluate_from_state(&child, &incremental),
                    model.evaluate_full_refresh(&child)
                );

                board = child;
                state = incremental;
            }
        }
    }
}

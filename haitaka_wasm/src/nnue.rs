use std::fmt;

use haitaka::{Board, Color, Piece, Square};

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
        let bucket = bucket_for_board(board);
        let us = board.side_to_move();
        let them = !us;
        let our_features = active_features(board, us);
        let their_features = active_features(board, them);

        let (our_accumulator, our_psqt) =
            self.transformer.accumulate(&our_features, bucket);
        let (their_accumulator, their_psqt) =
            self.transformer.accumulate(&their_features, bucket);

        let mut transformed = [0u8; FEATURE_TRANSFORMER_OUTPUT_DIMENSIONS];
        fill_transformed_features(
            &mut transformed[..TRANSFORMED_FEATURE_DIMENSIONS],
            &our_accumulator,
        );
        fill_transformed_features(
            &mut transformed[TRANSFORMED_FEATURE_DIMENSIONS..],
            &their_accumulator,
        );

        let bucket_network = &self.buckets[bucket];
        let mut hidden1 = [0i32; HIDDEN_LAYER_1_DIMENSIONS];
        bucket_network.hidden1.forward_into(&transformed, &mut hidden1);
        let hidden1_relu = clipped_relu_array(hidden1);
        let mut hidden2 = [0i32; HIDDEN_LAYER_2_DIMENSIONS];
        bucket_network.hidden2.forward_into(&hidden1_relu, &mut hidden2);
        let hidden2_relu = clipped_relu_array(hidden2);
        let output = bucket_network.output.forward_single(&hidden2_relu);
        let psqt = (our_psqt - their_psqt) / 2;

        (psqt + output) / OUTPUT_SCALE
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

    fn accumulate(
        &self,
        features: &ActiveFeatures,
        bucket: usize,
    ) -> ([i32; TRANSFORMED_FEATURE_DIMENSIONS], i32) {
        let mut accumulator = [0i32; TRANSFORMED_FEATURE_DIMENSIONS];
        for (dst, &bias) in accumulator.iter_mut().zip(&self.biases) {
            *dst = i32::from(bias);
        }

        let mut psqt = 0;
        for &index in features.iter() {
            let weight_offset = index * TRANSFORMED_FEATURE_DIMENSIONS;
            for (dst, &weight) in accumulator
                .iter_mut()
                .zip(&self.weights[weight_offset..weight_offset + TRANSFORMED_FEATURE_DIMENSIONS])
            {
                *dst += i32::from(weight);
            }
            psqt += self.psqt_weights[index * PSQT_BUCKETS + bucket];
        }

        (accumulator, psqt)
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
            for (value, &weight) in input.iter().zip(&self.weights[offset..offset + input.len()]) {
                sum += i32::from(weight) * i32::from(*value);
            }
            *out = sum;
        }
    }

    fn forward_single(&self, input: &[u8]) -> i32 {
        debug_assert_eq!(self.output_dimensions, 1);
        let offset = 0;
        let mut sum = self.biases[0];
        for (value, &weight) in input.iter().zip(&self.weights[offset..offset + input.len()]) {
            sum += i32::from(weight) * i32::from(*value);
        }
        sum
    }
}

#[derive(Debug, Clone)]
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

fn fill_transformed_features(output: &mut [u8], accumulator: &[i32; TRANSFORMED_FEATURE_DIMENSIONS]) {
    for (dst, &value) in output.iter_mut().zip(accumulator) {
        *dst = clamp_transformed(value);
    }
}

fn clamp_transformed(value: i32) -> u8 {
    let shifted = value >> WEIGHT_SCALE_BITS;
    shifted.clamp(0, 127) as u8
}

fn clipped_relu_array<const DIMENSIONS: usize>(input: [i32; DIMENSIONS]) -> [u8; DIMENSIONS] {
    let mut output = [0; DIMENSIONS];
    for (dst, value) in output.iter_mut().zip(input) {
        *dst = clamp_transformed(value);
    }
    output
}

fn bucket_for_board(board: &Board) -> usize {
    let piece_count = usize::try_from(board.occupied().len()).unwrap_or(0).max(1);
    (((piece_count - 1) * PSQT_BUCKETS) / MAX_PIECES).min(PSQT_BUCKETS - 1)
}

fn active_features(board: &Board, perspective: Color) -> ActiveFeatures {
    let oriented_king = orient_square(board.king(perspective), perspective);
    let king_offset = oriented_king * PIECE_INDICES;
    let mut features = ActiveFeatures::default();

    for &square in &Square::ALL {
        let Some(colored_piece) = board.colored_piece_on(square) else {
            continue;
        };

        let oriented_square = orient_square(square, perspective);
        let piece_square_index = piece_square_index(perspective, colored_piece.color, colored_piece.piece);
        features.push(king_offset + piece_square_index + oriented_square);
    }

    for &color in &[Color::Black, Color::White] {
        for &piece in &HAND_PIECES {
            let count = usize::from(board.num_in_hand(color, piece));
            let hand_index = piece_hand_index(perspective, color, piece);
            for copy_index in 0..count {
                features.push(king_offset + hand_index + copy_index);
            }
        }
    }

    features
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
}

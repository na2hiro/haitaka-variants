//! Tatara LayerStack with the Minase 256x16x32 A/P features and one PSQT bucket.
//! Format reference: https://github.com/stepney141/minase/tree/v3.0.1/crates/minase/src/eval/nnue
use super::{Accumulator, FEATURES, Model};

const WIDTH: usize = 256;
const VERSION: u32 = 0x7af3_2f21;
const FC_HASH: u32 = 0x6333_714e;
const ARCH_SUFFIX: &str = "(Friend)[13680->256x2],PSQT=1,Network=AffineTransform[1<-32](ClippedReLU[32](AffineTransform[32<-30](SqrClippedReLU[30](AffineTransform[16<-512](InputSlice[512(0:512)]))))),fv_scale=";

pub(super) struct LayerStack {
    bias: [i32; WIDTH],
    weights: Box<[[i16; WIDTH]]>,
    psqt: Box<[i32]>,
    l1_bias: [i32; 16],
    l1: Box<[[i8; WIDTH]]>,
    l2_bias: [i32; 32],
    l2: [[i8; 32]; 32],
    output_bias: i32,
    output: [i8; 32],
    scale: i32,
}

impl LayerStack {
    pub(super) fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        Self::decode(bytes).map_err(|e| format!("NNUE_INVALID_MINASE: {e}"))
    }

    fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        let mut file = Reader(bytes);
        if file.u32()? != VERSION {
            return Err("expected Minase Tatara LayerStack or MNUE v5");
        }
        let network_hash = file.u32()?;
        let length = file.u32()? as usize;
        let architecture = std::str::from_utf8(file.take(length)?)
            .map_err(|_| "invalid LayerStack architecture text")?;
        let arch = architecture
            .strip_prefix("Features=")
            .ok_or("missing features")?;
        // v3.0.0 called the same input set P. v3.0.1 changed its name and hash only.
        let (arch, feature_hash) = match arch.as_bytes().first() {
            Some(b'A') => (&arch[1..], transformer_hash(b'A')),
            Some(b'P') => (&arch[1..], transformer_hash(b'P')),
            _ => return Err("expected Minase A or P features"),
        };
        let scale = arch
            .strip_prefix(ARCH_SUFFIX)
            .and_then(|s| {
                s.parse::<i32>()
                    .ok()
                    .filter(|n| *n > 0 && n.to_string() == s)
            })
            .ok_or("unsupported LayerStack architecture or scale")?;
        if network_hash != feature_hash ^ FC_HASH || file.u32()? != 1 || file.u32()? != feature_hash
        {
            return Err("LayerStack network hash, bucket count or transformer hash mismatch");
        }
        let mut biases = file.leb_block(WIDTH)?;
        let mut bias = [0; WIDTH];
        for value in &mut bias {
            *value = i32::from(biases.signed_i16()?);
        }
        biases.finish()?;

        let mut transformer = file.leb_block(FEATURES * WIDTH)?;
        let mut weights = Vec::with_capacity(FEATURES);
        for _ in 0..FEATURES {
            let mut row = [0; WIDTH];
            for value in &mut row {
                *value = transformer.signed_i16()?;
            }
            weights.push(row);
        }
        transformer.finish()?;

        if file.u32()? != 0 {
            return Err("expected a zero PSQT bias");
        }
        let psqt = (0..FEATURES)
            .map(|_| file.i32())
            .collect::<Result<Box<_>, _>>()?;
        if file.u32()? != FC_HASH {
            return Err("LayerStack dense network hash mismatch");
        }
        let l1_bias = file.dense_biases()?;
        let l1 = (0..16)
            .map(|_| file.dense_row())
            .collect::<Result<Box<_>, _>>()?;
        let l2_bias = file.dense_biases()?;
        let mut l2 = [[0; 32]; 32];
        for row in &mut l2 {
            *row = file.dense_row()?;
            if row[30..] != [0, 0] {
                return Err("invalid LayerStack L2 padding");
            }
        }
        let [output_bias] = file.dense_biases()?;
        let output = file.dense_row()?;
        file.finish()?;

        if bias[..WIDTH / 2] != bias[WIDTH / 2..] {
            return Err("LayerStack transformer bias has asymmetric halves");
        }
        for (index, row) in weights.iter().enumerate() {
            let mirror = index / 12 * 12 + 11 - index % 12;
            if row[..WIDTH / 2] != weights[mirror][WIDTH / 2..] || psqt[index] != psqt[mirror] {
                return Err("LayerStack mirror weights do not match");
            }
            if psqt[index].unsigned_abs() > 1 << 22 {
                return Err("LayerStack PSQT weight out of range");
            }
        }
        Ok(Self {
            bias,
            weights: weights.into_boxed_slice(),
            psqt,
            l1_bias,
            l1,
            l2_bias,
            l2,
            output_bias,
            output,
            scale,
        })
    }
}

impl Model<WIDTH> for LayerStack {
    fn bias(&self) -> [i32; WIDTH] {
        self.bias
    }

    fn row(&self, feature: usize) -> &[i16; WIDTH] {
        &self.weights[feature]
    }

    fn linear(&self, feature: usize) -> [i32; 2] {
        [self.psqt[feature], 0]
    }

    fn evaluate(&self, acc: &Accumulator<WIDTH>, side: usize) -> i32 {
        let [us, them] = [side, side ^ 1].map(|player| &acc.hidden[player]);
        let mut input = [0; WIDTH];
        for (half, values) in input
            .as_chunks_mut::<{ WIDTH / 2 }>()
            .0
            .iter_mut()
            .zip([us, them])
        {
            let (left, right) = values.split_at(WIDTH / 2);
            for ((value, &a), &b) in half.iter_mut().zip(left).zip(right) {
                *value = ((a.clamp(0, 127) * b.clamp(0, 127)) >> 7) as u8;
            }
        }

        let mut second = [0; 32];
        for i in 0..15 {
            let activation = affine(self.l1_bias[i], &self.l1[i], &input);
            second[i] = ((i64::from(activation).pow(2) >> 19).min(127)) as u8;
            second[i + 15] = crelu(activation);
        }
        let skip = affine(self.l1_bias[15], &self.l1[15], &input);
        let third = std::array::from_fn(|i| crelu(affine(self.l2_bias[i], &self.l2[i], &second)));
        let dense = affine(self.output_bias, &self.output, &third);
        let psqt = (acc.linear[side][0] - acc.linear[side ^ 1][0]) / 2;
        ((i64::from(dense) + i64::from(skip) + i64::from(psqt)) / i64::from(self.scale))
            .clamp(-28_999, 28_999) as i32
    }
}

fn transformer_hash(feature: u8) -> u32 {
    ((0x811c_9dc5_u32 ^ u32::from(feature)).wrapping_mul(0x0100_0193)) ^ (2 * WIDTH as u32)
}

fn affine<const N: usize>(bias: i32, weights: &[i8; N], input: &[u8; N]) -> i32 {
    bias + weights
        .iter()
        .zip(input)
        .map(|(&w, &x)| i32::from(w) * i32::from(x))
        .sum::<i32>()
}

fn crelu(value: i32) -> u8 {
    (value >> 6).clamp(0, 127) as u8
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], &'static str> {
        let (head, tail) = self
            .0
            .split_at_checked(count)
            .ok_or("truncated LayerStack file")?;
        self.0 = tail;
        Ok(head)
    }

    fn u32(&mut self) -> Result<u32, &'static str> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32, &'static str> {
        Ok(self.u32()? as i32)
    }

    fn leb_block(&mut self, count: usize) -> Result<Self, &'static str> {
        if self.take(17)? != b"COMPRESSED_LEB128" {
            return Err("missing LayerStack LEB128 marker");
        }
        let size = self.u32()? as usize;
        if !(count..=3 * count).contains(&size) {
            return Err("invalid LayerStack LEB128 section size");
        }
        Ok(Self(self.take(size)?))
    }

    fn signed_i16(&mut self) -> Result<i16, &'static str> {
        let mut value = 0_i32;
        for shift in [0, 7, 14] {
            let byte = self.take(1)?[0];
            value |= i32::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                if byte & 0x40 != 0 {
                    value |= -1 << (shift + 7);
                }
                return i16::try_from(value).map_err(|_| "LayerStack LEB128 value out of range");
            }
        }
        Err("LayerStack LEB128 value exceeds three bytes")
    }

    fn dense_biases<const N: usize>(&mut self) -> Result<[i32; N], &'static str> {
        let mut values = [0; N];
        for value in &mut values {
            *value = self.i32()?;
            if value.unsigned_abs() > 1 << 24 {
                return Err("LayerStack dense bias out of range");
            }
        }
        Ok(values)
    }

    fn dense_row<const N: usize>(&mut self) -> Result<[i8; N], &'static str> {
        let bytes: [u8; N] = self.take(N)?.try_into().unwrap();
        Ok(bytes.map(|byte| byte as i8))
    }

    fn finish(&self) -> Result<(), &'static str> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err("trailing bytes in LayerStack section")
        }
    }
}

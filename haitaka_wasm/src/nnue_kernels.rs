//! Integer affine kernels used by NNUE inference.
//!
//! The scalar implementation is the portability path and correctness oracle.
//! Optimized backends are selected once when an affine layer is loaded.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AffineKernel(AffineBackend);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AffineBackend {
    Scalar,
    #[cfg(target_arch = "x86_64")]
    Avx2,
    #[cfg(target_arch = "aarch64")]
    Neon,
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    WasmSimd128,
}

impl AffineKernel {
    pub const fn scalar() -> Self {
        Self(AffineBackend::Scalar)
    }

    pub fn detected() -> Self {
        #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
        {
            Self(AffineBackend::WasmSimd128)
        }

        #[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
        {
            #[cfg(target_arch = "x86_64")]
            if std::arch::is_x86_feature_detected!("avx2") {
                return Self(AffineBackend::Avx2);
            }

            #[cfg(target_arch = "aarch64")]
            if std::arch::is_aarch64_feature_detected!("neon") {
                return Self(AffineBackend::Neon);
            }

            Self::scalar()
        }
    }

    pub const fn name(self) -> &'static str {
        match self.0 {
            AffineBackend::Scalar => "scalar",
            #[cfg(target_arch = "x86_64")]
            AffineBackend::Avx2 => "avx2",
            #[cfg(target_arch = "aarch64")]
            AffineBackend::Neon => "neon",
            #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
            AffineBackend::WasmSimd128 => "wasm-simd128",
        }
    }

    pub fn forward_into(
        self,
        input: &[u8],
        weights: &[i8],
        biases: &[i32],
        padded_input_dimensions: usize,
        output: &mut [i32],
    ) {
        debug_assert_eq!(biases.len(), output.len());
        debug_assert!(input.len() <= padded_input_dimensions);
        debug_assert!(weights.len() >= output.len() * padded_input_dimensions);

        for (row, (bias, out)) in biases.iter().zip(output).enumerate() {
            let offset = row * padded_input_dimensions;
            let row_weights = &weights[offset..offset + input.len()];
            *out = bias.wrapping_add(self.dot(input, row_weights));
        }
    }

    pub fn forward_single(self, input: &[u8], weights: &[i8], bias: i32) -> i32 {
        debug_assert!(weights.len() >= input.len());
        bias.wrapping_add(self.dot(input, &weights[..input.len()]))
    }

    pub fn dot(self, input: &[u8], weights: &[i8]) -> i32 {
        debug_assert_eq!(input.len(), weights.len());
        match self.0 {
            AffineBackend::Scalar => dot_scalar(input, weights),
            #[cfg(target_arch = "x86_64")]
            AffineBackend::Avx2 => {
                // `Avx2` is only returned after runtime feature detection. Tests
                // that force it also guard with the same detection macro.
                unsafe { dot_avx2(input, weights) }
            }
            #[cfg(target_arch = "aarch64")]
            AffineBackend::Neon => {
                // AArch64 feature detection above establishes this precondition.
                unsafe { dot_neon(input, weights) }
            }
            #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
            AffineBackend::WasmSimd128 => {
                // This variant only exists in binaries compiled with simd128.
                unsafe { dot_wasm_simd128(input, weights) }
            }
        }
    }
}

pub fn dot_scalar(input: &[u8], weights: &[i8]) -> i32 {
    input
        .iter()
        .zip(weights)
        .fold(0i32, |sum, (&value, &weight)| {
            sum.wrapping_add(i32::from(value) * i32::from(weight))
        })
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn dot_avx2(input: &[u8], weights: &[i8]) -> i32 {
    use std::arch::x86_64::*;

    unsafe {
        let mut offset = 0;
        let mut sums = _mm256_setzero_si256();
        let ones = _mm256_set1_epi16(1);

        // Widen before multiplying. This is exact for the full u8/i8 domain;
        // unlike maddubs, it cannot saturate an intermediate i16 pair sum.
        while offset + 16 <= input.len() {
            let values = _mm_loadu_si128(input.as_ptr().add(offset).cast::<__m128i>());
            let row_weights = _mm_loadu_si128(weights.as_ptr().add(offset).cast::<__m128i>());
            let values = _mm256_cvtepu8_epi16(values);
            let row_weights = _mm256_cvtepi8_epi16(row_weights);
            let products = _mm256_mullo_epi16(values, row_weights);
            sums = _mm256_add_epi32(sums, _mm256_madd_epi16(products, ones));
            offset += 16;
        }

        let mut lanes = [0i32; 8];
        _mm256_storeu_si256(lanes.as_mut_ptr().cast::<__m256i>(), sums);
        let mut sum = lanes.into_iter().fold(0i32, i32::wrapping_add);
        for index in offset..input.len() {
            sum = sum.wrapping_add(i32::from(input[index]) * i32::from(weights[index]));
        }
        sum
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn dot_neon(input: &[u8], weights: &[i8]) -> i32 {
    use std::arch::aarch64::*;

    unsafe {
        let mut offset = 0;
        let mut sums = vdupq_n_s32(0);
        while offset + 16 <= input.len() {
            let values = vld1q_u8(input.as_ptr().add(offset));
            let row_weights = vld1q_s8(weights.as_ptr().add(offset));

            let values_low = vreinterpretq_s16_u16(vmovl_u8(vget_low_u8(values)));
            let values_high = vreinterpretq_s16_u16(vmovl_high_u8(values));
            let weights_low = vmovl_s8(vget_low_s8(row_weights));
            let weights_high = vmovl_high_s8(row_weights);
            sums = vaddq_s32(sums, vpaddlq_s16(vmulq_s16(values_low, weights_low)));
            sums = vaddq_s32(sums, vpaddlq_s16(vmulq_s16(values_high, weights_high)));
            offset += 16;
        }

        let mut sum = vaddvq_s32(sums);
        for index in offset..input.len() {
            sum = sum.wrapping_add(i32::from(input[index]) * i32::from(weights[index]));
        }
        sum
    }
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn dot_wasm_simd128(input: &[u8], weights: &[i8]) -> i32 {
    use std::arch::wasm32::*;

    unsafe {
        let mut offset = 0;
        let mut sums = i32x4_splat(0);
        while offset + 16 <= input.len() {
            let values = v128_load(input.as_ptr().add(offset).cast::<v128>());
            let row_weights = v128_load(weights.as_ptr().add(offset).cast::<v128>());

            let values_low = u16x8_extend_low_u8x16(values);
            let values_high = u16x8_extend_high_u8x16(values);
            let weights_low = i16x8_extend_low_i8x16(row_weights);
            let weights_high = i16x8_extend_high_i8x16(row_weights);
            sums = i32x4_add(
                sums,
                i32x4_extadd_pairwise_i16x8(i16x8_mul(values_low, weights_low)),
            );
            sums = i32x4_add(
                sums,
                i32x4_extadd_pairwise_i16x8(i16x8_mul(values_high, weights_high)),
            );
            offset += 16;
        }

        let mut sum = i32x4_extract_lane::<0>(sums)
            .wrapping_add(i32x4_extract_lane::<1>(sums))
            .wrapping_add(i32x4_extract_lane::<2>(sums))
            .wrapping_add(i32x4_extract_lane::<3>(sums));
        for index in offset..input.len() {
            sum = sum.wrapping_add(i32::from(input[index]) * i32::from(weights[index]));
        }
        sum
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng, rngs::StdRng};

    fn available_optimized_kernel() -> Option<AffineKernel> {
        let detected = AffineKernel::detected();
        (detected != AffineKernel::scalar()).then_some(detected)
    }

    #[test]
    fn detected_kernel_matches_scalar_for_random_and_boundary_inputs() {
        let Some(kernel) = available_optimized_kernel() else {
            return;
        };
        let mut rng = StdRng::seed_from_u64(0x5eed_600d);

        for &len in &[0, 1, 15, 16, 17, 31, 32, 33, 1_024] {
            for _ in 0..64 {
                let input: Vec<u8> = (0..len).map(|_| rng.random()).collect();
                let weights: Vec<i8> = (0..len).map(|_| rng.random()).collect();
                assert_eq!(
                    kernel.dot(&input, &weights),
                    dot_scalar(&input, &weights),
                    "{} kernel, input length {len}",
                    kernel.name()
                );
            }

            for &(value, weight) in &[(0, -128), (255, -128), (255, 127), (127, 127)] {
                let input = vec![value; len];
                let weights = vec![weight; len];
                assert_eq!(kernel.dot(&input, &weights), dot_scalar(&input, &weights));
            }
        }
    }

    #[test]
    fn affine_rows_and_padding_match_scalar() {
        let Some(kernel) = available_optimized_kernel() else {
            return;
        };
        let mut rng = StdRng::seed_from_u64(0x00a6_614e);

        for &(inputs, padded_inputs, outputs) in &[(1_024, 1_024, 16), (16, 32, 32), (32, 32, 1)] {
            let input: Vec<u8> = (0..inputs).map(|_| rng.random_range(0..=127)).collect();
            let weights: Vec<i8> = (0..padded_inputs * outputs).map(|_| rng.random()).collect();
            let biases: Vec<i32> = (0..outputs)
                .map(|_| rng.random_range(-1_000_000..=1_000_000))
                .collect();
            let mut scalar = vec![0; outputs];
            let mut optimized = vec![0; outputs];
            AffineKernel::scalar().forward_into(
                &input,
                &weights,
                &biases,
                padded_inputs,
                &mut scalar,
            );
            kernel.forward_into(&input, &weights, &biases, padded_inputs, &mut optimized);
            assert_eq!(optimized, scalar, "{} kernel", kernel.name());
        }
    }
}

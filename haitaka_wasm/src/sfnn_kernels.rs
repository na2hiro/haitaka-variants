//! Exact integer SFNN kernels. The portable paths remain the correctness oracle.
use crate::nnue_kernels::AffineKernel;

pub fn affine_kernel() -> AffineKernel {
    if cfg!(feature = "sfnn-force-scalar") {
        AffineKernel::scalar()
    } else {
        AffineKernel::detected()
    }
}

pub fn update_scalar(acc: &mut [i16], weights: &[i16], add: bool) {
    assert_eq!(acc.len(), weights.len());
    for (a, &w) in acc.iter_mut().zip(weights) {
        *a = if add {
            a.wrapping_add(w)
        } else {
            a.wrapping_sub(w)
        };
    }
}

pub fn update(acc: &mut [i16], weights: &[i16], add: bool) {
    assert_eq!(acc.len(), weights.len());
    #[cfg(all(
        target_arch = "wasm32",
        target_feature = "simd128",
        not(feature = "sfnn-force-scalar")
    ))]
    {
        // Each vector fits the checked slices. v128 loads/stores allow unaligned addresses.
        unsafe { update_simd(acc, weights, add) };
    }
    #[cfg(not(all(
        target_arch = "wasm32",
        target_feature = "simd128",
        not(feature = "sfnn-force-scalar")
    )))]
    update_scalar(acc, weights, add);
}

pub fn transform_scalar(acc: &[i16], output: &mut [u8]) {
    assert_eq!(acc.len(), output.len() * 2);
    let half = output.len();
    for (j, out) in output.iter_mut().enumerate() {
        let a = i32::from(acc[j].clamp(0, 254));
        let b = i32::from(acc[j + half].clamp(0, 254));
        *out = ((a * b) >> 9) as u8;
    }
}

pub fn transform(acc: &[i16], output: &mut [u8]) {
    assert_eq!(acc.len(), output.len() * 2);
    #[cfg(all(
        target_arch = "wasm32",
        target_feature = "simd128",
        not(feature = "sfnn-force-scalar")
    ))]
    unsafe {
        transform_simd(acc, output)
    };
    #[cfg(not(all(
        target_arch = "wasm32",
        target_feature = "simd128",
        not(feature = "sfnn-force-scalar")
    )))]
    transform_scalar(acc, output);
}

#[cfg(all(
    target_arch = "wasm32",
    target_feature = "simd128",
    not(feature = "sfnn-force-scalar")
))]
unsafe fn update_simd(acc: &mut [i16], weights: &[i16], add: bool) {
    use std::arch::wasm32::*;
    let mut j = 0;
    unsafe {
        while j + 8 <= acc.len() {
            let a = v128_load(acc.as_ptr().add(j).cast());
            let w = v128_load(weights.as_ptr().add(j).cast());
            let result = if add {
                i16x8_add(a, w)
            } else {
                i16x8_sub(a, w)
            };
            v128_store(acc.as_mut_ptr().add(j).cast(), result);
            j += 8;
        }
    }
    update_scalar(&mut acc[j..], &weights[j..], add);
}

#[cfg(all(
    target_arch = "wasm32",
    target_feature = "simd128",
    not(feature = "sfnn-force-scalar")
))]
unsafe fn transform_simd(acc: &[i16], output: &mut [u8]) {
    use std::arch::wasm32::*;
    let mut j = 0;
    let half = output.len();
    let zero = i16x8_splat(0);
    let ceiling = i16x8_splat(254);
    unsafe {
        while j + 8 <= half {
            let a = i16x8_min(
                i16x8_max(v128_load(acc.as_ptr().add(j).cast()), zero),
                ceiling,
            );
            let b = i16x8_min(
                i16x8_max(v128_load(acc.as_ptr().add(half + j).cast()), zero),
                ceiling,
            );
            // 254² fits u16 but not i16: use the low product bits and a logical shift.
            let product = u16x8_shr(i16x8_mul(a, b), 9);
            let packed = u8x16_narrow_i16x8(product, zero);
            v128_store64_lane::<0>(packed, output.as_mut_ptr().add(j).cast());
            j += 8;
        }
    }
    for k in j..half {
        output[k] =
            ((i32::from(acc[k].clamp(0, 254)) * i32::from(acc[half + k].clamp(0, 254))) >> 9) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn kernels_match_scalar_at_wrap_clamp_and_tail_boundaries() {
        let edge = [i16::MIN, -255, -1, 0, 1, 127, 253, 254, 255, i16::MAX];
        for len in [0, 1, 7, 8, 9, 15, 16, 17, 1024, 2048] {
            let weights: Vec<i16> = (0..len).map(|i| edge[(i * 7 + 3) % edge.len()]).collect();
            for add in [false, true] {
                // Slice by one to exercise unaligned vector accesses.
                let mut actual: Vec<i16> = (0..len + 1).map(|i| edge[i % edge.len()]).collect();
                let mut expected = actual.clone();
                update(&mut actual[1..], &weights, add);
                update_scalar(&mut expected[1..], &weights, add);
                assert_eq!(actual, expected);
            }
            let acc: Vec<i16> = (0..2 * len).map(|i| edge[(i * 3) % edge.len()]).collect();
            let mut actual = vec![0; len];
            let mut expected = vec![0; len];
            transform(&acc, &mut actual);
            transform_scalar(&acc, &mut expected);
            assert_eq!(actual, expected);
        }
        for a in 0..=254 {
            let acc = [a; 16];
            let mut actual = [0; 8];
            transform(&acc, &mut actual);
            assert_eq!(actual, [((i32::from(a) * i32::from(a)) >> 9) as u8; 8]);
        }
    }
}

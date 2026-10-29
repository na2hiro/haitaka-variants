#![cfg(target_arch = "wasm32")]

use haitaka_wasm::nnue_kernels::{AffineKernel, dot_scalar};
use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test]
fn detected_wasm_kernel_matches_scalar() {
    let kernel = AffineKernel::detected();
    #[cfg(target_feature = "simd128")]
    assert_eq!(kernel.name(), "wasm-simd128");
    #[cfg(not(target_feature = "simd128"))]
    assert_eq!(kernel.name(), "scalar");

    for &len in &[0, 1, 15, 16, 17, 31, 32, 33, 1_024] {
        let input: Vec<u8> = (0..len)
            .map(|index| ((index * 37 + 11) & 255) as u8)
            .collect();
        let weights: Vec<i8> = (0..len)
            .map(|index| ((index * 29 + 7) & 255) as u8 as i8)
            .collect();
        assert_eq!(kernel.dot(&input, &weights), dot_scalar(&input, &weights));

        for &(value, weight) in &[(0, -128), (255, -128), (255, 127), (127, 127)] {
            let input = vec![value; len];
            let weights = vec![weight; len];
            assert_eq!(kernel.dot(&input, &weights), dot_scalar(&input, &weights));
        }
    }
}

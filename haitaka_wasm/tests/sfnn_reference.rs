//! Explicit real-model verification: never silently passes without model files.
#![cfg(not(target_arch = "wasm32"))]
use haitaka::Board;
use haitaka_wasm::sfnn::{MICRO_ARCH, SfnnModel};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, str::FromStr};

#[test]
#[ignore = "requires fetched micro model; run scripts/tuyouraou11.py verify"]
fn micro_matches_external_reference_and_rejects_corruption() {
    let dir = PathBuf::from(
        std::env::var_os("HAITAKA_SFNN_MODEL_DIR").expect("HAITAKA_SFNN_MODEL_DIR required"),
    );
    let nn = std::fs::read(dir.join("nn.bin")).expect("nn.bin required");
    let progress = std::fs::read(dir.join("progress.bin")).expect("progress.bin required");
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../plans/tuyouraou11/reference-fixture.json"
    ))
    .unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&nn)),
        fixtures["model"]["nnSha256"].as_str().unwrap()
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(&progress)),
        fixtures["model"]["progressSha256"].as_str().unwrap()
    );
    let model = SfnnModel::from_bytes(&nn, &progress, MICRO_ARCH).unwrap();
    let cases = fixtures["cases"].as_array().unwrap();
    assert!(cases.len() >= 64);
    for case in cases {
        let board = Board::from_str(case["sfen"].as_str().unwrap()).unwrap();
        let actual = model.evaluate_full_refresh(&board);
        let expected = &case["progress"];
        let id = case["id"].as_str().unwrap();
        assert_eq!(
            actual.progress_sums_q16[0].to_string(),
            expected["sumsQ16"]["black"].as_str().unwrap(),
            "{id}: black progress"
        );
        assert_eq!(
            actual.progress_sums_q16[1].to_string(),
            expected["sumsQ16"]["white"].as_str().unwrap(),
            "{id}: white progress"
        );
        assert_eq!(
            actual.selected_sum_q16.to_string(),
            expected["selectedSumQ16"].as_str().unwrap(),
            "{id}: selected progress"
        );
        assert_eq!(
            actual.bucket,
            expected["bucket"].as_u64().unwrap() as usize,
            "{id}: bucket"
        );
        assert_eq!(
            i64::from(actual.eval_cp),
            case["evalCp"].as_i64().unwrap(),
            "{id}: eval; raw={}",
            actual.raw_output
        );
    }
    let reject_nn = |b: &[u8]| {
        let e = SfnnModel::from_bytes(b, &progress, MICRO_ARCH)
            .err()
            .expect("must reject NN");
        assert!(e.0.starts_with("SFNN_INVALID_NN:"), "{e}");
    };
    reject_nn(&nn[..nn.len() - 1]);
    let mut bad = nn.clone();
    bad.push(0);
    reject_nn(&bad);
    for at in [0, 4, 12, 105, 109] {
        let mut bad = nn.clone();
        bad[at] ^= 0x01;
        reject_nn(&bad);
    }
    let reject_progress = |b: &[u8]| {
        let e = SfnnModel::from_bytes(&nn, b, MICRO_ARCH)
            .err()
            .expect("must reject progress");
        assert!(e.0.starts_with("SFNN_INVALID_PROGRESS:"), "{e}");
    };
    reject_progress(&[]);
    reject_progress(&progress[..progress.len() - 1]);
    let mut bad = progress.clone();
    bad.push(0);
    reject_progress(&bad);
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut bad = progress.clone();
        bad[..8].copy_from_slice(&v.to_le_bytes());
        reject_progress(&bad);
    }
    for arch in ["", "SFNN_ka2_4096_8_64_progress8", "HalfKAv2^"] {
        assert!(
            SfnnModel::from_bytes(&nn, &progress, arch)
                .err()
                .unwrap()
                .0
                .starts_with("SFNN_UNSUPPORTED_ARCH:")
        );
    }
    println!(
        "matched {} external reference positions exactly; real-file corruption checks passed",
        cases.len()
    );
}

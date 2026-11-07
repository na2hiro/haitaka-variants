#![cfg(feature = "chushogi")]
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};
fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_haitaka_cli"))
}
fn json(action: &[&str], input: &str) -> serde_json::Value {
    let mut c = cli()
        .arg("json")
        .args(action)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let r = c.wait_with_output().unwrap();
    assert!(r.status.success());
    serde_json::from_slice(&r.stdout).unwrap()
}
#[test]
fn json_cli_shares_core_and_rejects_usi() {
    let start = json(&["startpos"], "");
    let input = start.to_string();
    assert_eq!(json(&["validate"], &input), start);
    assert_eq!(json(&["perft", "--depth", "2"], &input)["nodes"], 1296);
    let result = json(
        &[
            "search",
            "--depth",
            "8",
            "--timeout-ms",
            "10000",
            "--nodes",
            "1000",
        ],
        &input,
    );
    assert!(result["nodeLimitReached"].as_bool().unwrap());
    assert!(
        json(&["moves"], &input)
            .as_array()
            .unwrap()
            .contains(&result["bestMove"])
    );
    let usi = cli().arg("usi").output().unwrap();
    assert!(!usi.status.success());
    assert!(String::from_utf8_lossy(&usi.stderr).contains("SFEN/USI"));
}
#[test]
fn package_declares_json_only_and_rejects_models_and_wrong_ids() {
    let tmp = std::env::temp_dir().join(format!("haitaka-chu-package-{}", std::process::id()));
    fs::create_dir_all(&tmp).unwrap();
    let out = tmp.join("chu.tgz");
    let status = cli()
        .args(["package", "--allow-missing-wasm", "--output"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    let tar = Command::new("tar")
        .arg("-xOzf")
        .arg(&out)
        .arg("./shogitter-engine.json")
        .output()
        .unwrap();
    assert!(tar.status.success());
    let m: serde_json::Value = serde_json::from_slice(&tar.stdout).unwrap();
    assert_eq!(
        m["capabilities"]["protocols"],
        serde_json::json!(["haitaka-json-v2"])
    );
    assert!(
        !m["capabilities"]["commands"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("dfpn"))
    );
    let p = &m["profiles"][0];
    assert!(p["nnue"].is_null());
    assert_eq!(p["rules"][0]["ruleId"], 48);
    assert_eq!(p["rules"][0]["positionFormat"], "haitaka-json-v2");
    assert_eq!(p["rules"][0]["moveFormat"], "haitaka-json-v2");
    assert!(
        haitaka::extended::compiled::CompiledGame::parse(
            p["rules"][0]["startpos"].as_str().unwrap()
        )
        .is_ok()
    );
    for extra in [["--rule-id", "43"], ["--nnue", "missing.nnue"]] {
        let r = cli()
            .args(["package", "--allow-missing-wasm", "--output"])
            .arg(tmp.join("bad.tgz"))
            .args(extra)
            .output()
            .unwrap();
        assert!(!r.status.success());
        assert!(!tmp.join("bad.tgz").exists());
    }
    fs::remove_dir_all(tmp).unwrap();
}

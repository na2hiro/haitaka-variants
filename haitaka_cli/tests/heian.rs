#![cfg(feature = "heian")]
use haitaka::{RuleSpec, SelectedRule};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_haitaka_cli"))
}
fn temp_dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "haitaka-minishogi-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn native_usi_returns_a_legal_move_on_the_compact_board() {
    let mut child = cli()
        .arg("usi")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"usi\nisready\nposition startpos\ngo depth 2\nquit\n")
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("usiok"));
    assert!(output.contains("readyok"));
    let best = output
        .lines()
        .find_map(|line| line.strip_prefix("bestmove "))
        .unwrap();
    assert!(haitaka::Board::startpos().is_legal(best.parse().unwrap()));
}

#[test]
fn package_advertises_selected_heian_rule_and_rejects_models() {
    let temp = temp_dir();
    let pkg = temp.join("pkg");
    fs::create_dir(&pkg).unwrap();
    fs::write(pkg.join("haitaka_wasm.js"), "// metadata fixture").unwrap();
    fs::write(pkg.join("haitaka_wasm_bg.wasm"), b"\0asm\x01\0\0\0").unwrap();
    let output = temp.join("engine.tgz");
    let result = cli()
        .args(["package", "--wasm-dir"])
        .arg(&pkg)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let manifest = Command::new("tar")
        .arg("-xOzf")
        .arg(&output)
        .arg("./shogitter-engine.json")
        .output()
        .unwrap();
    assert!(manifest.status.success());
    let value: serde_json::Value = serde_json::from_slice(&manifest.stdout).unwrap();
    let profile = &value["profiles"][0];
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(profile["id"], format!("{}-default", SelectedRule::NAME));
    assert_eq!(profile["rules"][0]["ruleId"], SelectedRule::RULE_ID);
    assert_eq!(profile["rules"][0]["variant"], SelectedRule::NAME);
    assert_eq!(profile["rules"][0]["startpos"], haitaka::SFEN_STARTPOS);
    assert!(profile["nnue"].is_null());
    assert!(
        !value["capabilities"]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "dfpn")
    );
    let wrong = cli()
        .args(["package", "--ruleset", "standard", "--allow-missing-wasm"])
        .arg("--output")
        .arg(temp.join("wrong.tgz"))
        .output()
        .unwrap();
    assert!(!wrong.status.success());
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("does not match compiled ruleset"));
    let model = cli()
        .args(["package", "--nnue", "missing.nnue", "--allow-missing-wasm"])
        .arg("--output")
        .arg(temp.join("model.tgz"))
        .output()
        .unwrap();
    assert!(!model.status.success());
    assert!(
        String::from_utf8_lossy(&model.stderr)
            .contains(&format!("{} does not support", SelectedRule::NAME))
    );
    fs::remove_dir_all(temp).unwrap();
}

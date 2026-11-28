#![cfg(feature = "chushogi")]
use std::{
    fs,
    io::{BufRead, BufReader, Write},
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
fn json_and_usi_share_the_chu_search() {
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
    let mut subcommand = cli();
    subcommand.arg("usi");
    assert_usi_matches_json(subcommand, &result);
    assert_usi_matches_json(Command::new(env!("CARGO_BIN_EXE_haitaka-usi")), &result);
}

fn assert_usi_matches_json(mut command: Command, result: &serde_json::Value) {
    let mut usi = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = usi.stdin.take().unwrap();
    let mut stdout = BufReader::new(usi.stdout.take().unwrap());
    let mut await_line = |prefix: &str| -> String {
        loop {
            let mut line = String::new();
            assert!(stdout.read_line(&mut line).unwrap() > 0, "missing {prefix}");
            if line.starts_with(prefix) {
                return line.trim().to_owned();
            }
        }
    };
    writeln!(
        stdin,
        "usi\nisready\nposition startpos\ngo depth 8 nodes 1000"
    )
    .unwrap();
    await_line("usiok");
    await_line("readyok");
    let info = await_line("info depth ");
    assert!(
        info.starts_with(&format!("info depth {} ", result["completedDepth"])),
        "{info}"
    );
    assert!(
        info.contains(&format!(" nodes {} ", result["states"])),
        "{info}"
    );
    assert!(
        info.contains(&format!(" score cp {} ", result["score"])),
        "{info}"
    );
    let best = await_line("bestmove ");
    let mv = &result["bestMove"];
    let format_square = |at: &serde_json::Value| {
        let file = at[0].as_u64().unwrap();
        let rank = at[1].as_u64().unwrap() as u8;
        format!("{file}{}", char::from(b'a' + rank - 1))
    };
    let mut expected = format_square(&mv["from"]);
    for at in mv["path"].as_array().unwrap() {
        expected.push_str(&format_square(at));
    }
    if mv["promotion"].as_bool().unwrap() {
        expected.push('+');
    }
    assert_eq!(best, format!("bestmove {expected}"));

    writeln!(stdin, "go infinite\nstop").unwrap();
    let best = await_line("bestmove ");
    assert_ne!(best, "bestmove resign");
    writeln!(stdin, "isready\nquit").unwrap();
    await_line("readyok");
    assert!(usi.wait().unwrap().success());
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

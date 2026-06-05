use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn cli_exe() -> &'static str {
    env!("CARGO_BIN_EXE_haitaka_cli")
}

#[test]
fn usi_subprocess_returns_legal_bestmove() {
    let mut child = Command::new(cli_exe())
        .arg("usi")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn haitaka_cli usi");

    let mut stdin = child.stdin.take().expect("stdin should be piped");
    let stdout = child.stdout.take().expect("stdout should be piped");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    send(&mut stdin, "usi");
    read_until(&rx, "usiok", Duration::from_secs(5));
    send(&mut stdin, "isready");
    read_until(&rx, "readyok", Duration::from_secs(5));
    send(&mut stdin, "position startpos");
    send(&mut stdin, "go depth 1");

    let bestmove = read_prefix(&rx, "bestmove ", Duration::from_secs(10));
    assert_ne!(bestmove, "bestmove resign");
    assert!(
        bestmove.split_whitespace().nth(1).is_some(),
        "bestmove should include a move: {bestmove}"
    );

    let _ = writeln!(stdin, "quit");
    let _ = stdin.flush();
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn self_play_can_use_current_binary_as_both_external_engines() {
    let output = Command::new(cli_exe())
        .args([
            "self-play",
            "--games",
            "2",
            "--threads",
            "1",
            "--a-depth",
            "1",
            "--b-depth",
            "1",
            "--a-engine",
            cli_exe(),
            "--a-engine-arg",
            "usi",
            "--b-engine",
            cli_exe(),
            "--b-engine-arg",
            "usi",
            "--max-plies",
            "4",
        ])
        .output()
        .expect("run external self-play smoke");

    assert!(
        output.status.success(),
        "self-play failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("self-play threads=1"));
    assert!(stdout.contains("games: 2"));
}

#[test]
fn self_play_can_use_current_binary_archive_as_both_engines() {
    let temp = unique_temp_dir("archive-smoke");
    fs::create_dir_all(&temp).expect("create temp dir");
    let archive = temp.join("haitaka-native.tgz");
    let report_dir = temp.join("archive-report");
    let report_json = report_dir.join("self-play-report.json");

    let archive_output = Command::new(cli_exe())
        .args([
            "archive-engine",
            "--output",
            archive.to_str().expect("archive path should be utf-8"),
            "--binary",
            cli_exe(),
            "--profile",
            "debug",
            "--target",
            "test-target",
        ])
        .output()
        .expect("run archive-engine");
    assert!(
        archive_output.status.success(),
        "archive-engine failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&archive_output.stdout),
        String::from_utf8_lossy(&archive_output.stderr)
    );

    let output = Command::new(cli_exe())
        .args([
            "self-play",
            "--games",
            "2",
            "--threads",
            "1",
            "--a-depth",
            "1",
            "--b-depth",
            "1",
            "--a-engine-archive",
            archive.to_str().expect("archive path should be utf-8"),
            "--b-engine-archive",
            archive.to_str().expect("archive path should be utf-8"),
            "--max-plies",
            "4",
            "--report-dir",
            report_dir.to_str().expect("report dir should be utf-8"),
        ])
        .output()
        .expect("run archive external self-play smoke");

    assert!(
        output.status.success(),
        "archive self-play failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("self-play threads=1"));
    assert!(stdout.contains("games: 2"));

    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report_json).expect("read archive report json"))
            .expect("parse archive report json");
    assert_eq!(report["engines"][0]["kind"], "archive-usi");
    assert_eq!(
        report["engines"][0]["archivePath"],
        archive.display().to_string()
    );
    assert_eq!(
        report["engines"][0]["archive"]["schema"],
        "haitaka-engine-archive"
    );
    assert_eq!(
        report["engines"][0]["archive"]["runtime"]["protocol"],
        "usi"
    );
    assert_eq!(
        report["engines"][0]["command"],
        serde_json::Value::String("bin/haitaka_cli".to_string())
    );
    assert_eq!(report["engines"][0]["args"], serde_json::json!(["usi"]));

    let merge_output = run_with_stdin(
        &[
            "self-play",
            "--games",
            "1",
            "--threads",
            "1",
            "--a-depth",
            "1",
            "--b-depth",
            "1",
            "--a-engine-archive",
            archive.to_str().expect("archive path should be utf-8"),
            "--b-engine-archive",
            archive.to_str().expect("archive path should be utf-8"),
            "--max-plies",
            "4",
            "--report-dir",
            report_dir.to_str().expect("report dir should be utf-8"),
        ],
        "2\n",
    );
    assert!(
        merge_output.status.success(),
        "archive merge failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&merge_output.stdout),
        String::from_utf8_lossy(&merge_output.stderr)
    );

    let merged_report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report_json).expect("read merged report json"))
            .expect("parse merged archive report json");
    assert_eq!(merged_report["summary"]["games"], 3);

    fs::remove_dir_all(temp).expect("clean temp dir");
}

#[test]
fn self_play_writes_opening_jsonl_and_report_outputs() {
    let temp = unique_temp_dir("report-smoke");
    fs::create_dir_all(&temp).expect("create temp dir");
    let openings = temp.join("openings.sfen");
    let report_dir = temp.join("report");
    let games_jsonl = report_dir.join("self-play-games.jsonl");
    let report_json = report_dir.join("self-play-report.json");
    fs::write(&openings, format!("{}\n", haitaka::SFEN_STARTPOS)).expect("write openings");

    let args = [
        "self-play",
        "--games",
        "2",
        "--threads",
        "1",
        "--a-depth",
        "1",
        "--b-depth",
        "1",
        "--openings",
        openings.to_str().expect("openings path should be utf-8"),
        "--report-dir",
        report_dir.to_str().expect("report dir should be utf-8"),
        "--max-plies",
        "4",
    ];
    let output = Command::new(cli_exe())
        .args(args)
        .output()
        .expect("run report self-play smoke");

    assert!(
        output.status.success(),
        "self-play failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let games = fs::read_to_string(&games_jsonl).expect("read game jsonl");
    let game_lines = games.lines().collect::<Vec<_>>();
    assert_eq!(game_lines.len(), 2);
    let first_game: serde_json::Value =
        serde_json::from_str(game_lines[0]).expect("parse game json");
    assert_eq!(first_game["schema"], "haitaka-self-play-game");
    assert_eq!(first_game["opening"]["source"], "suite");
    assert_eq!(first_game["opening"]["suiteIndex"], 0);
    assert!(
        first_game["moves"]
            .as_array()
            .expect("moves should be array")
            .len()
            <= 4
    );

    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report_json).expect("read report json"))
            .expect("parse report json");
    assert_eq!(report["schema"], "haitaka-self-play-report");
    assert_eq!(report["summary"]["games"], 2);
    assert_eq!(
        report["command"]["openings"],
        openings.display().to_string()
    );
    assert_eq!(
        report["engines"].as_array().expect("engines array").len(),
        2
    );

    let merge_output = run_with_stdin(
        &[
            "self-play",
            "--games",
            "1",
            "--threads",
            "1",
            "--a-depth",
            "1",
            "--b-depth",
            "1",
            "--openings",
            openings.to_str().expect("openings path should be utf-8"),
            "--report-dir",
            report_dir.to_str().expect("report dir should be utf-8"),
            "--max-plies",
            "4",
        ],
        "2\n",
    );
    assert!(
        merge_output.status.success(),
        "merge self-play failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&merge_output.stdout),
        String::from_utf8_lossy(&merge_output.stderr)
    );

    let merged_games = fs::read_to_string(&games_jsonl).expect("read merged game jsonl");
    let merged_lines = merged_games.lines().collect::<Vec<_>>();
    assert_eq!(merged_lines.len(), 3);
    let appended_game: serde_json::Value =
        serde_json::from_str(merged_lines[2]).expect("parse appended game json");
    assert_eq!(appended_game["gameIndex"], 3);

    let merged_report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report_json).expect("read merged report json"))
            .expect("parse merged report json");
    assert_eq!(merged_report["summary"]["games"], 3);

    fs::remove_dir_all(temp).expect("clean temp dir");
}

#[cfg(unix)]
#[test]
fn self_play_ctrl_c_writes_partial_report() {
    let temp = unique_temp_dir("ctrl-c-report");
    fs::create_dir_all(&temp).expect("create temp dir");
    let report_dir = temp.join("report");
    let report_json = report_dir.join("self-play-report.json");
    let games_jsonl = report_dir.join("self-play-games.jsonl");

    let child = Command::new(cli_exe())
        .args([
            "self-play",
            "--games",
            "100",
            "--threads",
            "1",
            "--a-depth",
            "64",
            "--b-depth",
            "64",
            "--movetime-ms",
            "1000",
            "--max-plies",
            "2",
            "--report-dir",
            report_dir.to_str().expect("report dir should be utf-8"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn interruptible self-play");

    wait_for_file(&games_jsonl, Duration::from_secs(5));
    thread::sleep(Duration::from_millis(100));
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGINT);
    }
    let output = child.wait_with_output().expect("wait for interrupted run");

    assert!(
        !output.status.success(),
        "interrupted self-play should exit non-zero\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(report_json.is_file(), "partial report should be written");
    assert!(games_jsonl.is_file(), "game log should be written");

    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report_json).expect("read partial report"))
            .expect("parse partial report");
    assert_eq!(report["schema"], "haitaka-self-play-report");
    assert!(
        report["summary"]["games"]
            .as_u64()
            .expect("games should be u64")
            < 100,
        "partial report should not claim all games completed"
    );
    assert!(
        report["summary"]["warnings"]
            .as_array()
            .expect("warnings should be array")
            .iter()
            .any(|warning| warning.as_str().unwrap_or_default().contains("interrupted")),
        "partial report should include interrupt warning"
    );

    fs::remove_dir_all(temp).expect("clean temp dir");
}

fn send(stdin: &mut impl Write, command: &str) {
    writeln!(stdin, "{command}").expect("write command");
    stdin.flush().expect("flush command");
}

fn read_until(rx: &mpsc::Receiver<String>, expected: &str, timeout: Duration) {
    let line = read_prefix(rx, expected, timeout);
    assert_eq!(line, expected);
}

fn read_prefix(rx: &mpsc::Receiver<String>, prefix: &str, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        let now = Instant::now();
        assert!(now < deadline, "timed out waiting for {prefix}");
        let line = rx
            .recv_timeout(deadline - now)
            .expect("engine stdout should stay open");
        if line.starts_with(prefix) {
            return line;
        }
    }
}

fn run_with_stdin(args: &[&str], input: &str) -> std::process::Output {
    let mut child = Command::new(cli_exe())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn command");
    {
        let stdin = child.stdin.as_mut().expect("stdin should be piped");
        stdin.write_all(input.as_bytes()).expect("write stdin");
        stdin.flush().expect("flush stdin");
    }
    child.wait_with_output().expect("wait for command")
}

fn wait_for_file(path: &std::path::Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.is_file() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {}", path.display());
}

fn unique_temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("haitaka-cli-{name}-{}-{nonce}", std::process::id()))
}

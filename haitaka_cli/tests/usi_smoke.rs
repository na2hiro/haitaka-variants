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
            "--b-engine",
            cli_exe(),
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

fn unique_temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("haitaka-cli-{name}-{}-{nonce}", std::process::id()))
}

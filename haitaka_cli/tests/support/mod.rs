use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(10);

pub struct UsiProcess {
    child: Child,
    pub stdin: Option<ChildStdin>,
    pub lines: mpsc::Receiver<String>,
}

impl UsiProcess {
    pub fn spawn(mut command: Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            lines,
        }
    }

    pub fn send(&mut self, command: &str) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{command}").unwrap();
        stdin.flush().unwrap();
    }

    pub fn read_through(&self, prefix: &str) -> Vec<String> {
        let deadline = Instant::now() + TIMEOUT;
        let mut lines = Vec::new();
        loop {
            let line = self
                .lines
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|err| panic!("waiting for {prefix:?}: {err}, received {lines:?}"));
            let found = line.starts_with(prefix);
            lines.push(line);
            if found {
                return lines;
            }
        }
    }

    pub fn wait_for_exit(&mut self) -> Vec<String> {
        let deadline = Instant::now() + TIMEOUT;
        let mut lines = Vec::new();
        loop {
            match self
                .lines
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(line) => lines.push(line),
                Err(RecvTimeoutError::Disconnected) => break,
                Err(error) => panic!("waiting for exit: {error}, received {lines:?}"),
            }
        }
        assert!(self.child.wait().unwrap().success());
        lines
    }
}

impl Drop for UsiProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn assert_legal_bestmove(board: &haitaka::Board, lines: &[String]) {
    let best = lines.last().unwrap().strip_prefix("bestmove ").unwrap();
    let mv = best
        .parse()
        .unwrap_or_else(|err| panic!("invalid bestmove {best}: {err}"));
    assert!(board.is_legal(mv), "illegal bestmove {mv} for {board}");
}

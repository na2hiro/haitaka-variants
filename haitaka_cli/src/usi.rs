use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use haitaka_wasm::native_usi::MAX_DEPTH;

#[cfg(not(feature = "extended-json"))]
use {
    crate::ENGINE_NAME,
    anyhow::{Context, Result, anyhow},
    haitaka_wasm::native_usi::{NativeGo, NativeUsiSession},
    std::fs,
    std::io::{self, BufRead, Write},
    std::sync::atomic::{AtomicBool, Ordering},
    std::sync::{Arc, mpsc},
    std::thread::{self, JoinHandle},
};

#[derive(Debug, Parser)]
#[command(about = "Run Haitaka Variants as a USI engine")]
pub struct UsiArgs {
    /// Diagnostic NNUE score multiplier (1 or 2).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(i32).range(1..=2))]
    pub nnue_multiplier: i32,
    /// Evaluator used by the USI engine.
    #[arg(long = "eval", value_enum, default_value = "handcrafted")]
    pub eval: EngineEvalKind,
    /// NNUE file used when --eval nnue is selected.
    #[arg(long)]
    pub nnue: Option<PathBuf>,
    /// Default maximum depth for time-controlled searches.
    #[arg(long, default_value_t = 64, value_parser = clap::value_parser!(u8).range(1..=i64::from(MAX_DEPTH)))]
    pub movetime_max_depth: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum EngineEvalKind {
    Handcrafted,
    Nnue,
}

#[cfg(all(feature = "extended-json", not(feature = "chushogi")))]
pub fn run(_args: UsiArgs) -> anyhow::Result<()> {
    anyhow::bail!("this ruleset uses the extended JSON protocol; run haitaka_cli json")
}

#[cfg(feature = "chushogi")]
pub fn run(args: UsiArgs) -> anyhow::Result<()> {
    crate::chu_usi::run(args)
}

#[cfg(not(feature = "extended-json"))]
pub fn run(args: UsiArgs) -> anyhow::Result<()> {
    let name = format!(
        "{ENGINE_NAME} {} ({})",
        env!("CARGO_PKG_VERSION"),
        rule_profile()
    );
    let mut session = NativeUsiSession::new(name, args.movetime_max_depth);
    if args.eval == EngineEvalKind::Nnue {
        let path = args
            .nnue
            .context("USI engine uses NNUE but no NNUE path was provided")?;
        let bytes =
            fs::read(&path).with_context(|| format!("failed to read NNUE {}", path.display()))?;
        session
            .load_nnue(&bytes, args.nnue_multiplier)
            .map_err(|err| anyhow::anyhow!("failed to load NNUE {}: {err}", path.display()))?;
    }
    transport(session)
}

pub fn default_ruleset() -> &'static str {
    #[cfg(feature = "extended-json")]
    {
        haitaka::extended::compiled::NAME
    }
    #[cfg(not(feature = "extended-json"))]
    {
        use haitaka::RuleSpec;
        haitaka::SelectedRule::NAME
    }
}

pub fn rule_profile() -> String {
    let mut profile = default_ruleset().to_string();
    for (enabled, name) in [
        (cfg!(feature = "oute"), "oute"),
        (cfg!(feature = "sanmaime"), "sanmaime"),
        (cfg!(feature = "inochinotokin"), "inochinotokin"),
        (cfg!(feature = "shihonkangen"), "shihonkangen"),
        (cfg!(feature = "ningenshogi"), "ningenshogi"),
        (cfg!(feature = "tennouzan"), "tennouzan"),
        (cfg!(feature = "tesushogi"), "tesushogi"),
        (cfg!(feature = "tensuu"), "tensuu"),
        (cfg!(feature = "suika"), "suika"),
        (cfg!(feature = "igyoku"), "igyoku"),
        (cfg!(feature = "hitomasuou"), "hitomasuou"),
        (cfg!(feature = "nakamaware"), "nakamaware"),
        (cfg!(feature = "fushiyo"), "fushiyo"),
        (cfg!(feature = "share"), "share"),
        (cfg!(feature = "zoshoku"), "zoshoku"),
        (cfg!(feature = "babanuki"), "babanuki"),
    ] {
        if enabled {
            profile.push('+');
            profile.push_str(name);
        }
    }
    profile
}

#[cfg(not(feature = "extended-json"))]
enum Event {
    Input(io::Result<String>),
    Eof,
    Info(u64, String),
    Finished(u64),
}

#[cfg(not(feature = "extended-json"))]
struct Running {
    id: u64,
    infinite: bool,
    stop: Arc<AtomicBool>,
    worker: JoinHandle<(NativeUsiSession, Vec<String>)>,
}

#[cfg(not(feature = "extended-json"))]
impl Running {
    fn start(
        id: u64,
        mut session: NativeUsiSession,
        request: NativeGo,
        events: mpsc::Sender<Event>,
    ) -> Self {
        let infinite = request.infinite;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let output = session.search(request, &flag, |line| {
                let _ = events.send(Event::Info(id, line));
            });
            let _ = events.send(Event::Finished(id));
            (session, output)
        });
        Self {
            id,
            infinite,
            stop,
            worker,
        }
    }

    fn finish(self, cancel: bool) -> Result<(NativeUsiSession, Vec<String>)> {
        if cancel {
            self.stop.store(true, Ordering::Relaxed);
        }
        self.worker
            .join()
            .map_err(|_| anyhow!("USI search worker panicked"))
    }
}

#[cfg(not(feature = "extended-json"))]
fn transport(session: NativeUsiSession) -> Result<()> {
    let (send, events) = mpsc::channel();
    let input = send.clone();
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            if input.send(Event::Input(line)).is_err() {
                return;
            }
        }
        let _ = input.send(Event::Eof);
    });

    let mut stdout = io::stdout().lock();
    let mut idle = Some(session);
    let mut running: Option<Running> = None;
    let mut search_id = 0;
    for event in events {
        match event {
            Event::Info(id, line) => {
                if running.as_ref().is_some_and(|search| search.id == id) {
                    write_lines(&mut stdout, [line])?;
                }
            }
            Event::Finished(id) => {
                if running
                    .as_ref()
                    .is_some_and(|search| search.id == id && !search.infinite)
                {
                    let (session, output) = running.take().unwrap().finish(false)?;
                    idle = Some(session);
                    write_lines(&mut stdout, output)?;
                }
            }
            Event::Eof => break,
            Event::Input(line) => {
                let line = line?;
                let words: Vec<_> = line.split_whitespace().collect();
                let Some(&command) = words.first() else {
                    continue;
                };
                match command {
                    "quit" => break,
                    "stop" => {
                        if let Some(search) = running.take() {
                            let (session, output) = search.finish(true)?;
                            idle = Some(session);
                            write_lines(&mut stdout, output)?;
                        }
                        continue;
                    }
                    "isready" if running.is_some() => {
                        write_lines(&mut stdout, ["readyok"])?;
                        continue;
                    }
                    "go" | "position" | "setoption" | "usinewgame" | "gameover" => {
                        if let Some(search) = running.take() {
                            idle = Some(search.finish(true)?.0);
                        }
                    }
                    _ if running.is_some() => {
                        write_lines(
                            &mut stdout,
                            ["info string command unavailable during search"],
                        )?;
                        continue;
                    }
                    _ => {}
                }
                let line = words.join(" ");
                let session = idle.as_mut().expect("search returned its session");
                if command != "go" {
                    write_lines(&mut stdout, session.handle_line(&line))?;
                } else if words.get(1) == Some(&"mate") {
                    write_lines(&mut stdout, ["checkmate notimplemented"])?;
                } else {
                    match session.prepare_search(&line) {
                        Ok(request) => {
                            search_id += 1;
                            running = Some(Running::start(
                                search_id,
                                idle.take().unwrap(),
                                request,
                                send.clone(),
                            ));
                        }
                        Err(err) => write_lines(
                            &mut stdout,
                            [
                                format!("info string invalid go command: {err}"),
                                "bestmove resign".into(),
                            ],
                        )?,
                    }
                }
            }
        }
    }
    if let Some(search) = running {
        search.finish(true)?;
    }
    Ok(())
}

#[cfg(not(feature = "extended-json"))]
fn write_lines(
    output: &mut impl Write,
    lines: impl IntoIterator<Item = impl AsRef<str>>,
) -> io::Result<()> {
    for line in lines {
        writeln!(output, "{}", line.as_ref())?;
    }
    output.flush()
}

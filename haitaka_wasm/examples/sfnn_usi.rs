//! Native USI harness for P4: same UsiSession/search, optional micro evaluator.
use haitaka_wasm::{
    UsiSession,
    sfnn::{MICRO_ARCH, MINI_ARCH},
};
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut engine = UsiSession::new("Haitaka P2 (handcrafted)", 64);
    if let Some(flag) = args.next() {
        if flag != "--model-dir" {
            return Err("expected --model-dir PATH".into());
        }
        let dir = PathBuf::from(args.next().ok_or("model directory required")?);
        let arch = match args.next().as_deref() {
            None => MICRO_ARCH,
            Some("--mini") => MINI_ARCH,
            _ => return Err("unexpected argument".into()),
        };
        if args.next().is_some() {
            return Err("unexpected argument".into());
        }
        engine = UsiSession::new("Haitaka P2 (TuyouraOu11 micro)", 64);
        engine.load_sfnn(
            &std::fs::read(dir.join("nn.bin"))?,
            &std::fs::read(dir.join("progress.bin"))?,
            arch,
        )?;
    }
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim() == "quit" {
            break;
        }
        for output in engine.handle_line(&line) {
            println!("{output}");
        }
        io::stdout().flush()?;
    }
    Ok(())
}

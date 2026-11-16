//! Diagnostic endpoints share ordinary rules/search; only SFNN input is changed.
use haitaka_wasm::{
    SearchEvalMode, UsiSession,
    sfnn::{MICRO_ARCH, MINI_ARCH, SfnnModel},
};
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut session = UsiSession::new("Haitaka no-hand diagnostic", 64);
    if let Some(index) = args.iter().position(|s| s == "--model-dir") {
        let dir = PathBuf::from(args.get(index + 1).ok_or("model directory required")?);
        let arch = if args.iter().any(|s| s == "--mini") {
            MINI_ARCH
        } else {
            MICRO_ARCH
        };
        let mut model = SfnnModel::from_bytes(
            &std::fs::read(dir.join("nn.bin"))?,
            &std::fs::read(dir.join("progress.bin"))?,
            arch,
        )?;
        if args.iter().any(|s| s == "--zero-missing-slot") {
            model.diagnostic_zero_missing_slot();
        }
        if args.iter().any(|s| s == "--ghost-hands") {
            if args.iter().any(|s| s == "--full-refresh") {
                return Err("ghost hands require history-carrying incremental mode".into());
            }
            model.diagnostic_enable_ghost_hands()?;
        }
        let mode = if args.iter().any(|s| s == "--full-refresh") {
            SearchEvalMode::FullRefresh
        } else {
            SearchEvalMode::Incremental
        };
        session.diagnostic_set_sfnn_model(model, mode);
    }
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim() == "quit" {
            break;
        }
        for output in session.handle_line(&line) {
            println!("{output}");
        }
        io::stdout().flush()?;
    }
    Ok(())
}

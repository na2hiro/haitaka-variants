use anyhow::{Result, anyhow};
use clap::{Parser, ValueEnum};
use haitaka::extended::*;
use std::io::{self, BufRead, Read};
#[derive(Debug, Clone, ValueEnum)]
enum Action {
    Startpos,
    Validate,
    Inspect,
    Moves,
    Apply,
    Perft,
    Search,
    Oracle,
}
#[derive(Debug, Parser)]
pub struct JsonArgs {
    #[arg(value_enum)]
    action: Action,
    #[arg(long, default_value_t = 3)]
    depth: u8,
    #[arg(long, default_value_t = 1000)]
    timeout_ms: u32,
    /// Zero means unlimited; applies across all iterations and quiescence.
    #[arg(long, default_value_t = 0)]
    nodes: u32,
    #[arg(long)]
    r#move: Option<String>,
}
pub fn run(args: JsonArgs) -> Result<()> {
    if matches!(args.action, Action::Startpos) {
        println!("{}", start_position_json());
        return Ok(());
    }
    if matches!(args.action, Action::Oracle) {
        // JSON-lines verifier: useful for reference engines, no subprocess per position.
        for line in io::stdin().lock().lines() {
            let input = line?;
            let game = ShoGame::parse(&input).map_err(|e| anyhow!(e))?;
            let mut transitions = vec![];
            for mv in game.moves() {
                let mut next = game.clone();
                next.play(mv).map_err(|e| anyhow!(e))?;
                transitions.push(serde_json::json!({"move":mv,"position":next.current().to_wire(),"outcome":next.outcome(),"hash":format!("{:016x}",next.current().hash())}));
            }
            println!(
                "{}",
                serde_json::json!({"position":game.current().to_wire(),"moves":game.moves(),"outcome":game.outcome(),"hash":format!("{:016x}",game.current().hash()),"transitions":transitions})
            );
        }
        return Ok(());
    }
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let result = match args.action {
        Action::Validate => validate_position_json(&input),
        Action::Inspect => inspect_json(&input),
        Action::Moves => legal_moves_json(&input),
        Action::Apply => apply_move_json(
            &input,
            args.r#move
                .as_deref()
                .ok_or_else(|| anyhow!("apply requires --move JSON"))?,
        ),
        Action::Perft => perft_json(&input, args.depth),
        Action::Search => search_json(&input, args.depth, args.timeout_ms, args.nodes),
        _ => unreachable!(),
    }
    .map_err(|e| anyhow!(e))?;
    println!("{result}");
    Ok(())
}

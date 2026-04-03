mod config;
mod dataset;
mod trainer;
mod verify;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::LoadedConfig;

#[derive(Debug, Parser)]
#[command(name = "haitaka_learn")]
#[command(about = "NNUE data generation, training, export, and verification for Haitaka")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    GenerateData {
        #[arg(long)]
        config: PathBuf,
    },
    Train {
        #[arg(long)]
        config: PathBuf,
    },
    Export {
        #[arg(long)]
        config: PathBuf,
    },
    Verify {
        #[arg(long)]
        config: PathBuf,
    },
    Pipeline {
        #[arg(long)]
        config: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::GenerateData { config } => {
            let loaded = LoadedConfig::from_path(&config)?;
            let output = dataset::generate_data(&loaded)?;
            println!(
                "generated {} training and {} validation samples into {}",
                output.train_positions,
                output.validation_positions,
                output.output_dir.display()
            );
        }
        Command::Train { config } => {
            let loaded = LoadedConfig::from_path(&config)?;
            let checkpoint = trainer::train(&loaded)?;
            println!("training finished: {}", checkpoint.display());
        }
        Command::Export { config } => {
            let loaded = LoadedConfig::from_path(&config)?;
            let exported = trainer::export(&loaded, None)?;
            println!("exported NNUE: {}", exported.display());
        }
        Command::Verify { config } => {
            let loaded = LoadedConfig::from_path(&config)?;
            let report = verify::verify(&loaded)?;
            println!(
                "verified {} position(s); report written to {}",
                report.positions.len(),
                report.report_path.display()
            );
        }
        Command::Pipeline { config } => {
            let loaded = LoadedConfig::from_path(&config)?;
            let data = dataset::generate_data(&loaded)?;
            println!(
                "generated {} training and {} validation samples",
                data.train_positions, data.validation_positions
            );
            let checkpoint = trainer::train(&loaded)?;
            println!("training finished: {}", checkpoint.display());
            let exported = trainer::export(&loaded, Some(checkpoint.clone()))?;
            println!("exported NNUE: {}", exported.display());
            let report = verify::verify(&loaded)?;
            println!(
                "verified {} position(s); report written to {}",
                report.positions.len(),
                report.report_path.display()
            );
        }
    }

    Ok(())
}

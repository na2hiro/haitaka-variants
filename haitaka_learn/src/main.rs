mod config;
mod dataset;
mod dataset_audit;
mod selection;
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
    AuditData {
        #[arg(long)]
        bin: PathBuf,
        #[arg(long)]
        manifest: PathBuf,
        /// Optional for legacy manifests that did not embed seed and feature identity.
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    GenerateData {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        jobs: Option<u32>,
        #[arg(long)]
        no_resume: bool,
        #[arg(long)]
        shard_index: Option<u32>,
        #[arg(long)]
        shard_index_end: Option<u32>,
        #[arg(long)]
        shard_count: Option<u32>,
        #[arg(long)]
        ignore_identity_mismatch: bool,
    },
    MergeData {
        #[arg(long)]
        config: PathBuf,
        #[arg(long, required = true)]
        input: Vec<PathBuf>,
        #[arg(long)]
        ignore_identity_mismatch: bool,
    },
    Train {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        no_resume: bool,
    },
    TrainSelect {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        self_play_bin: PathBuf,
        #[arg(long)]
        no_resume: bool,
        #[arg(long)]
        selection_max_games: Option<u32>,
        #[arg(long)]
        ranking_budget: Option<u32>,
        #[arg(long)]
        storage_saver: bool,
    },
    RankExisting {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        self_play_bin: PathBuf,
        #[arg(long)]
        ranking_budget: Option<u32>,
        #[arg(long)]
        output: PathBuf,
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
        #[arg(long)]
        no_resume: bool,
    },
}

fn resume_override(no_resume: bool) -> Option<bool> {
    no_resume.then_some(false)
}

fn generate_options(no_resume: bool) -> dataset::GenerateOptions {
    dataset::GenerateOptions {
        jobs: None,
        resume: resume_override(no_resume),
        shard_index: None,
        shard_index_end: None,
        shard_count: None,
        ignore_identity_mismatch: false,
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::AuditData {
            bin,
            manifest,
            config,
            output,
        } => {
            let report = dataset_audit::audit_dataset(&bin, &manifest, config.as_deref())?;
            if let Some(path) = dataset_audit::write_report(&report, output.as_deref())? {
                println!("dataset audit written to {}", path.display());
            }
        }
        Command::GenerateData {
            config,
            jobs,
            no_resume,
            shard_index,
            shard_index_end,
            shard_count,
            ignore_identity_mismatch,
        } => {
            let loaded = LoadedConfig::from_path(&config)?;
            let output = dataset::generate_data_with_options(
                &loaded,
                dataset::GenerateOptions {
                    jobs,
                    resume: resume_override(no_resume),
                    shard_index,
                    shard_index_end,
                    shard_count,
                    ignore_identity_mismatch,
                },
            )?;
            println!(
                "generated {} training and {} validation samples into {}",
                output.train_positions,
                output.validation_positions,
                output.output_dir.display()
            );
        }
        Command::MergeData {
            config,
            input,
            ignore_identity_mismatch,
        } => {
            let loaded = LoadedConfig::from_path(&config)?;
            let output = dataset::merge_data(&loaded, &input, ignore_identity_mismatch)?;
            println!(
                "merged {} training and {} validation samples into {}",
                output.train_positions,
                output.validation_positions,
                output.output_dir.display()
            );
        }
        Command::Train { config, no_resume } => {
            let loaded = LoadedConfig::from_path(&config)?;
            let checkpoint = trainer::train(&loaded, resume_override(no_resume))?;
            println!("training finished: {}", checkpoint.display());
        }
        Command::TrainSelect {
            config,
            self_play_bin,
            no_resume,
            selection_max_games,
            ranking_budget,
            storage_saver,
        } => {
            let loaded = LoadedConfig::from_path(&config)?;
            let selected = selection::train_select(
                &loaded,
                selection::TrainSelectOptions {
                    self_play_bin,
                    resume_override: resume_override(no_resume),
                    selection_max_games,
                    ranking_budget,
                    storage_saver: storage_saver.then_some(true),
                },
            )?;
            println!("training selection finished: {}", selected.display());
        }
        Command::RankExisting {
            config,
            self_play_bin,
            ranking_budget,
            output,
        } => {
            let loaded = LoadedConfig::from_path(&config)?;
            let selected = selection::rank_existing(
                &loaded,
                selection::RankExistingOptions {
                    self_play_bin,
                    ranking_budget,
                    output,
                },
            )?;
            println!("existing candidates ranked: {}", selected.display());
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
        Command::Pipeline { config, no_resume } => {
            let loaded = LoadedConfig::from_path(&config)?;
            let data = dataset::generate_data_with_options(&loaded, generate_options(no_resume))?;
            println!(
                "generated {} training and {} validation samples",
                data.train_positions, data.validation_positions
            );
            let checkpoint = trainer::train(&loaded, resume_override(no_resume))?;
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

#[cfg(test)]
mod tests {
    use super::{generate_options, resume_override};
    use crate::dataset::GenerateOptions;

    #[test]
    fn resume_override_is_none_without_cli_flag() {
        assert_eq!(resume_override(false), None);
    }

    #[test]
    fn resume_override_disables_resume_when_flag_is_set() {
        assert_eq!(resume_override(true), Some(false));
    }

    #[test]
    fn pipeline_generate_options_preserve_config_resume_without_cli_flag() {
        assert_eq!(
            generate_options(false),
            GenerateOptions {
                jobs: None,
                resume: None,
                shard_index: None,
                shard_index_end: None,
                shard_count: None,
                ignore_identity_mismatch: false,
            }
        );
    }

    #[test]
    fn pipeline_generate_options_disable_resume_when_flag_is_set() {
        assert_eq!(
            generate_options(true),
            GenerateOptions {
                jobs: None,
                resume: Some(false),
                shard_index: None,
                shard_index_end: None,
                shard_count: None,
                ignore_identity_mismatch: false,
            }
        );
    }
}

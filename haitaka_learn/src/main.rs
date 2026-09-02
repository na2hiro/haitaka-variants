mod config;
mod dataset;
mod dataset_audit;
mod openings;
mod phase11a;
mod phase11c;
mod r0;
mod r1a;
mod r1b;
mod r1c;
mod r1d1;
mod r1d2;
mod r1d3;
mod r1evidence;
mod selection;
mod trainer;
mod verify;

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, ensure};
use clap::{Parser, Subcommand};
use config::{LoadedConfig, LoadedGenerationConfig, LoadedTrainingConfig};

#[derive(Debug, Parser)]
#[command(name = "haitaka_learn")]
#[command(about = "NNUE data generation, training, export, and verification for Haitaka")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)] // Phase11cAudit intentionally exposes every frozen input.
enum Command {
    /// Freeze the committed, rebuild-complete source and external-trainer identity used by R1.
    R1SourceIdentity {
        #[arg(
            long,
            default_value = "r0/anhoku-reboot/r1-source-identity-policy.json"
        )]
        policy: PathBuf,
        #[arg(long, default_value = "../engine/variant-nnue-pytorch")]
        external_trainer: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1d3/source-identity.json")]
        output: PathBuf,
    },
    /// Expand an Anhoku DonorSingleEff network into the functionally identical
    /// DonorReceiverPairV2 initialization without training.
    MigrateDonorReceiverPairV2 {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Run the complete Phase 11-A migration, equivalence, tactical, size, and
    /// fixed-position inference gate without training or strength games.
    Phase11aGate {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output_nnue: PathBuf,
        #[arg(long)]
        tactical_suite: PathBuf,
        #[arg(long)]
        report: PathBuf,
    },
    /// Run the frozen Phase 11 tactical and fixed-position latency vetoes on
    /// trained V1 and V2 networks without modifying either artifact.
    Phase11bTacticalGate {
        #[arg(long)]
        v1: PathBuf,
        #[arg(long)]
        v2: PathBuf,
        #[arg(long)]
        tactical_suite: PathBuf,
        #[arg(long)]
        report: PathBuf,
    },
    /// Run the frozen CPU-only Phase 11-C learnability, quantization, replay,
    /// and collapsed-network audit. This command never trains or plays games.
    Phase11cAudit {
        #[arg(long)]
        trainer_config: PathBuf,
        #[arg(long)]
        trainer_checkout: PathBuf,
        #[arg(long)]
        python: PathBuf,
        #[arg(long)]
        helper: PathBuf,
        #[arg(long)]
        reviewed_patch: PathBuf,
        #[arg(long)]
        applied_diff: PathBuf,
        #[arg(long)]
        v1_checkpoint: PathBuf,
        #[arg(long)]
        v2_checkpoint: PathBuf,
        #[arg(long)]
        v1_nnue: PathBuf,
        #[arg(long)]
        v2_nnue: PathBuf,
        #[arg(long)]
        train: PathBuf,
        #[arg(long)]
        ood: PathBuf,
        #[arg(long)]
        tactical_suite: PathBuf,
        #[arg(long)]
        batch_1024_games: PathBuf,
        #[arg(long)]
        batch_1024_report: PathBuf,
        #[arg(long)]
        batch_3072_games: PathBuf,
        #[arg(long)]
        batch_3072_report: PathBuf,
        #[arg(long)]
        results_archive: PathBuf,
        #[arg(long)]
        closeout_archive: PathBuf,
        #[arg(long)]
        output_dir: PathBuf,
    },
    /// Compile the trainer overlay and verify Python/C++/runtime V2 cardinality,
    /// hash, and index anchors without starting training.
    VerifyDonorReceiverPairV2Trainer {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Validate the frozen R0 registry, historical evidence, artifact hashes,
    /// and production execution contract without generating data.
    R0Gate {
        #[arg(long, default_value = "r0/anhoku-reboot")]
        bundle: PathBuf,
    },
    /// Build and execute the complete deterministic R1-A board, feature, sign,
    /// C++ loader, and incremental-accumulator oracle gate.
    R1aGate {
        #[arg(long, default_value = "haitaka_learn.anhoku-reboot-r1a.training.toml")]
        config: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1a")]
        output_dir: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1d3/source-identity.json")]
        source_identity: PathBuf,
    },
    /// Generate the deterministic full-precision sentinel and run the complete
    /// Python-integer/export/Rust full-refresh/incremental R1-B parity gate.
    R1bGate {
        #[arg(long, default_value = "out/anhoku-reboot-r1a")]
        r1a_dir: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1b")]
        output_dir: PathBuf,
        #[arg(long, default_value = "r0/anhoku-reboot/r1b-quantization-limits.json")]
        limits: PathBuf,
        #[arg(
            long,
            default_value = "../engine/variant-nnue-pytorch/.venv/bin/python"
        )]
        python: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1d3/source-identity.json")]
        source_identity: PathBuf,
    },
    /// Train the frozen CPU-only exactly-representable and 8,192-position
    /// deployment learnability oracles, then verify serialized Rust parity.
    R1cGate {
        #[arg(long, default_value = "out/anhoku-reboot-r1a")]
        r1a_dir: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1b")]
        r1b_dir: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1c")]
        output_dir: PathBuf,
        #[arg(
            long,
            default_value = "r0/anhoku-reboot/r1c-learnability-contract.json"
        )]
        contract: PathBuf,
        #[arg(long, default_value = "r0/anhoku-reboot/r1b-quantization-limits.json")]
        limits: PathBuf,
        #[arg(
            long,
            default_value = "../engine/variant-nnue-pytorch/.venv/bin/python"
        )]
        python: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1d3/source-identity.json")]
        source_identity: PathBuf,
    },
    /// Run the frozen deterministic interrupted-root-result and combined-node
    /// accounting gate. This command never plays games or changes adjudication.
    R1d1Gate {
        #[arg(long, default_value = "out/anhoku-reboot-r1a")]
        r1a_dir: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1b")]
        r1b_dir: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1c")]
        r1c_dir: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1d1")]
        output_dir: PathBuf,
        #[arg(long, default_value = "r0/anhoku-reboot/r1d1-search-contract.json")]
        contract: PathBuf,
        #[arg(
            long,
            default_value = "r0/anhoku-reboot/r1d1-forced-interruption-fixtures.json"
        )]
        fixtures: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1d3/source-identity.json")]
        source_identity: PathBuf,
    },
    /// Run the frozen history, repetition, TT, qsearch, DFPN, and adjudication gate.
    /// This command never plays match-equivalence games.
    R1d2Gate {
        #[arg(long, default_value = "out/anhoku-reboot-r1d1")]
        r1d1_dir: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1d2")]
        output_dir: PathBuf,
        #[arg(long, default_value = "r0/anhoku-reboot/r1d2-history-contract.json")]
        contract: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1d3/source-identity.json")]
        source_identity: PathBuf,
    },
    /// Validate the frozen production-WASM null, timing, complete-pair, order-
    /// reversal, and native-equivalence qualification and write the R1 closeout.
    R1d3Gate {
        #[arg(long, default_value = "out/anhoku-reboot-r1a")]
        r1a_dir: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1b")]
        r1b_dir: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1c")]
        r1c_dir: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1d1")]
        r1d1_dir: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1d2")]
        r1d2_dir: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1d3")]
        output_dir: PathBuf,
        #[arg(long, default_value = "r0/anhoku-reboot/r1d3-match-contract.json")]
        contract: PathBuf,
        #[arg(long, default_value = "r0/anhoku-reboot/r1d3-openings.tsv")]
        openings: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1d3/browser-trace.json")]
        browser_trace: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1d3/pkg/haitaka_wasm.js")]
        wasm_js: PathBuf,
        #[arg(
            long,
            default_value = "out/anhoku-reboot-r1d3/pkg/haitaka_wasm_bg.wasm"
        )]
        wasm: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1b/zero.nnue")]
        model: PathBuf,
        #[arg(long, default_value = "out/anhoku-reboot-r1d3/source-identity.json")]
        source_identity: PathBuf,
    },
    ValidateOpenings {
        #[arg(long)]
        config: PathBuf,
    },
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
    TrajectoryAudit {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        jobs: Option<u32>,
        #[arg(long)]
        shard_index: Option<u32>,
        #[arg(long)]
        shard_index_end: Option<u32>,
        #[arg(long)]
        shard_count: Option<u32>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    CalibrateLabels {
        #[arg(long)]
        config: PathBuf,
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
    },
    Train {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        no_resume: bool,
    },
    /// Convert and validate the configured NNUE bootstrap without starting
    /// training. This is useful as a final remote GPU-host preflight.
    PrepareBootstrap {
        #[arg(long)]
        config: PathBuf,
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
    ExportCheckpoint {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        checkpoint: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    EvaluateCheckpoint {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        checkpoint: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Verify {
        #[arg(long)]
        config: PathBuf,
    },
    Pipeline {
        #[arg(long)]
        generation_config: PathBuf,
        #[arg(long)]
        training_config: PathBuf,
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
        Command::R1SourceIdentity {
            policy,
            external_trainer,
            output,
        } => {
            let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .to_path_buf();
            r1evidence::write_source_identity(
                &workspace_root,
                &external_trainer,
                &policy,
                &output,
            )?;
            println!("R1 source identity written to {}", output.display());
        }
        Command::R0Gate { bundle } => {
            let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("haitaka_learn is a workspace member")
                .to_path_buf();
            r0::validate_bundle(&bundle, &workspace_root)?;
            println!("R0 gate passed: {}", bundle.display());
        }
        Command::R1aGate {
            config,
            output_dir,
            source_identity,
        } => {
            let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("haitaka_learn is a workspace member")
                .to_path_buf();
            let report = r1a::run(&config, &output_dir, &source_identity, &workspace_root)?;
            println!(
                "R1-A gate written to {}: {} ({} positions, {} transitions)",
                output_dir.join("r1a-gate-report.json").display(),
                if report.passed { "PASS" } else { "FAIL" },
                report.corpus_positions,
                report.transitions_checked,
            );
            ensure!(report.passed, "R1-A gate failed");
        }
        Command::R1bGate {
            r1a_dir,
            output_dir,
            limits,
            python,
            source_identity,
        } => {
            let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("haitaka_learn is a workspace member")
                .to_path_buf();
            let report = r1b::run(
                &r1a_dir,
                &output_dir,
                &limits,
                &python,
                &source_identity,
                &workspace_root,
            )?;
            println!(
                "R1-B gate written to {}: {}",
                output_dir.join("r1b-gate-report.json").display(),
                if report.passed { "PASS" } else { "FAIL" },
            );
            ensure!(report.passed, "R1-B gate failed");
        }
        Command::R1cGate {
            r1a_dir,
            r1b_dir,
            output_dir,
            contract,
            limits,
            python,
            source_identity,
        } => {
            let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("haitaka_learn is a workspace member")
                .to_path_buf();
            let report = r1c::run(r1c::RunArgs {
                r1a_dir: &r1a_dir,
                r1b_dir: &r1b_dir,
                output_dir: &output_dir,
                contract_path: &contract,
                limits_path: &limits,
                python: &python,
                source_identity_path: &source_identity,
                workspace_root: &workspace_root,
            })?;
            println!(
                "R1-C gate written to {}: {}",
                output_dir.join("r1c-gate-report.json").display(),
                if report.passed { "PASS" } else { "FAIL" },
            );
            ensure!(report.passed, "R1-C gate failed");
        }
        Command::R1d1Gate {
            r1a_dir,
            r1b_dir,
            r1c_dir,
            output_dir,
            contract,
            fixtures,
            source_identity,
        } => {
            let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("haitaka_learn is a workspace member")
                .to_path_buf();
            let report = r1d1::run(r1d1::RunArgs {
                r1a_dir: &r1a_dir,
                r1b_dir: &r1b_dir,
                r1c_dir: &r1c_dir,
                output_dir: &output_dir,
                contract_path: &contract,
                fixtures_path: &fixtures,
                source_identity_path: &source_identity,
                workspace_root: &workspace_root,
            })?;
            println!(
                "R1-D1 gate written to {}: {}",
                output_dir.join("r1d1-gate-report.json").display(),
                if report.passed { "PASS" } else { "FAIL" },
            );
            ensure!(report.passed, "R1-D1 gate failed");
        }
        Command::R1d2Gate {
            r1d1_dir,
            output_dir,
            contract,
            source_identity,
        } => {
            let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("haitaka_learn is a workspace member")
                .to_path_buf();
            let report = r1d2::run(r1d2::RunArgs {
                r1d1_dir: &r1d1_dir,
                output_dir: &output_dir,
                contract_path: &contract,
                source_identity_path: &source_identity,
                workspace_root: &workspace_root,
            })?;
            println!(
                "R1-D2 gate written to {}: {}",
                output_dir.join("r1d2-gate-report.json").display(),
                if report.passed { "PASS" } else { "FAIL" },
            );
            ensure!(report.passed, "R1-D2 gate failed");
        }
        Command::R1d3Gate {
            r1a_dir,
            r1b_dir,
            r1c_dir,
            r1d1_dir,
            r1d2_dir,
            output_dir,
            contract,
            openings,
            browser_trace,
            wasm_js,
            wasm,
            model,
            source_identity,
        } => {
            let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("haitaka_learn is a workspace member")
                .to_path_buf();
            let report = r1d3::run(r1d3::RunArgs {
                r1a_dir: &r1a_dir,
                r1b_dir: &r1b_dir,
                r1c_dir: &r1c_dir,
                r1d1_dir: &r1d1_dir,
                r1d2_dir: &r1d2_dir,
                output_dir: &output_dir,
                contract_path: &contract,
                openings_path: &openings,
                browser_trace_path: &browser_trace,
                wasm_js_path: &wasm_js,
                wasm_path: &wasm,
                model_path: &model,
                source_identity_path: &source_identity,
                workspace_root: &workspace_root,
            })?;
            println!(
                "R1-D3 gate written to {}: {}",
                output_dir.join("r1d3-gate-report.json").display(),
                if report.passed { "PASS" } else { "FAIL" },
            );
            ensure!(report.passed, "R1-D3 gate failed");
        }
        Command::MigrateDonorReceiverPairV2 { input, output } => {
            let source =
                fs::read(&input).with_context(|| format!("failed to read {}", input.display()))?;
            let migrated = haitaka_wasm::migrate_donor_single_to_receiver_pair_v2(&source)
                .map_err(|err| anyhow!("failed to migrate {}: {err}", input.display()))?;
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::write(&output, &migrated)
                .with_context(|| format!("failed to write {}", output.display()))?;
            let stats = haitaka_wasm::donor_receiver_pair_v2_stats();
            println!(
                "migrated {} -> {} ({} -> {} bytes; {} -> {} real features)",
                input.display(),
                output.display(),
                source.len(),
                migrated.len(),
                stats.v1_real_features,
                stats.v2_real_features,
            );
        }
        Command::Phase11aGate {
            input,
            output_nnue,
            tactical_suite,
            report,
        } => {
            let result = phase11a::run(&input, &output_nnue, &tactical_suite, &report)?;
            let go = result.phase11b_go();
            println!(
                "Phase 11-A gate written to {}: {}",
                report.display(),
                if go { "GO" } else { "NO-GO" }
            );
            ensure!(go, "Phase 11-A gate failed; do not start Phase 11-B");
        }
        Command::Phase11bTacticalGate {
            v1,
            v2,
            tactical_suite,
            report,
        } => {
            let result = phase11a::run_trained_gate(&v1, &v2, &tactical_suite, &report)?;
            let passed = result.passed();
            println!(
                "Phase 11-B tactical/latency gate written to {}: {}",
                report.display(),
                if passed { "PASS" } else { "FAIL" }
            );
            ensure!(passed, "Phase 11-B tactical/latency gate failed");
        }
        Command::Phase11cAudit {
            trainer_config,
            trainer_checkout,
            python,
            helper,
            reviewed_patch,
            applied_diff,
            v1_checkpoint,
            v2_checkpoint,
            v1_nnue,
            v2_nnue,
            train,
            ood,
            tactical_suite,
            batch_1024_games,
            batch_1024_report,
            batch_3072_games,
            batch_3072_report,
            results_archive,
            closeout_archive,
            output_dir,
        } => {
            let result = phase11c::run(phase11c::Phase11cArgs {
                trainer_config,
                trainer_checkout,
                python,
                helper,
                reviewed_patch,
                applied_diff,
                v1_checkpoint,
                v2_checkpoint,
                v1_nnue,
                v2_nnue,
                train,
                ood,
                tactical_suite,
                batch_1024_games,
                batch_1024_report,
                batch_3072_games,
                batch_3072_report,
                results_archive,
                closeout_archive,
                output_dir,
            })?;
            println!(
                "Phase 11-C audit written to {}: {}",
                result.report_path.display(),
                result.classification
            );
        }
        Command::VerifyDonorReceiverPairV2Trainer { config, output } => {
            let loaded = LoadedConfig::from_path(&config)?;
            loaded.ruleset_requires_matching_engine()?;
            let report = trainer::verify_receiver_pair_v2_trainer_parity(&loaded, &output)?;
            println!(
                "DonorReceiverPairV2 trainer parity written to {}: {}",
                output.display(),
                if report.passed { "PASS" } else { "FAIL" }
            );
            ensure!(report.passed, "DonorReceiverPairV2 trainer parity failed");
        }
        Command::ValidateOpenings { config } => {
            let loaded = LoadedGenerationConfig::from_path(&config)?;
            loaded.ruleset_requires_matching_engine()?;
            let (suite_id, positions, sha256) = openings::validate_configured_suite(&loaded)?;
            println!(
                "validated opening suite {suite_id}: {positions} position(s), sha256={sha256}"
            );
        }
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
        Command::TrajectoryAudit {
            config,
            jobs,
            shard_index,
            shard_index_end,
            shard_count,
            output,
        } => {
            let loaded = LoadedGenerationConfig::from_path(&config)?;
            let report = dataset::audit_trajectories(
                &loaded,
                dataset::TrajectoryAuditOptions {
                    jobs,
                    shard_index,
                    shard_index_end,
                    shard_count,
                },
            )?;
            let path = dataset::write_trajectory_audit_report(&loaded, &report, output)?;
            println!("trajectory audit written to {}", path.display());
        }
        Command::CalibrateLabels { config, output } => {
            let loaded = LoadedGenerationConfig::from_path(&config)?;
            let report = dataset::calibrate_labels(&loaded)?;
            let path = dataset::write_label_calibration_report(&loaded, &report, output)?;
            println!("label calibration written to {}", path.display());
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
            let loaded = LoadedGenerationConfig::from_path(&config)?;
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
        Command::MergeData { config, input } => {
            let loaded = LoadedGenerationConfig::from_path(&config)?;
            let output = dataset::merge_data(&loaded, &input, false)?;
            println!(
                "merged {} training and {} validation samples into {}",
                output.train_positions,
                output.validation_positions,
                output.output_dir.display()
            );
        }
        Command::Train { config, no_resume } => {
            let loaded = LoadedTrainingConfig::from_path(&config)?;
            let checkpoint = trainer::train(&loaded, resume_override(no_resume))?;
            println!("training finished: {}", checkpoint.display());
        }
        Command::PrepareBootstrap { config } => {
            let loaded = LoadedTrainingConfig::from_path(&config)?;
            let bootstrap = trainer::prepare_bootstrap(&loaded)?;
            println!("prepared bootstrap checkpoint: {}", bootstrap.display());
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
            let loaded = LoadedTrainingConfig::from_path(&config)?;
            let exported = trainer::export(&loaded, None)?;
            println!("exported NNUE: {}", exported.display());
        }
        Command::ExportCheckpoint {
            config,
            checkpoint,
            output,
        } => {
            let loaded = LoadedTrainingConfig::from_path(&config)?;
            let trainer_checkout = loaded.trainer_checkout()?;
            let _guard = trainer::PreparedTrainer::new(&loaded, &trainer_checkout)?;
            trainer::export_checkpoint_to(&loaded, &trainer_checkout, &checkpoint, &output)?;
            println!("exported NNUE: {}", output.display());
        }
        Command::EvaluateCheckpoint {
            config,
            checkpoint,
            output,
        } => {
            let loaded = LoadedConfig::from_path(&config)?;
            let report = trainer::evaluate_checkpoint(&loaded, checkpoint, output)?;
            println!("offline ID/OOD evaluation written to {}", report.display());
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
        Command::Pipeline {
            generation_config,
            training_config,
            no_resume,
        } => {
            let generation = LoadedGenerationConfig::from_path(&generation_config)?;
            let training = LoadedTrainingConfig::from_path(&training_config)?;
            let data =
                dataset::generate_data_with_options(&generation, generate_options(no_resume))?;
            println!(
                "generated {} training and {} validation samples",
                data.train_positions, data.validation_positions
            );
            let checkpoint = trainer::train(&training, resume_override(no_resume))?;
            println!("training finished: {}", checkpoint.display());
            let exported = trainer::export(&training, Some(checkpoint.clone()))?;
            println!("exported NNUE: {}", exported.display());
            let report = verify::verify(&training)?;
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

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::LoadedTrainingConfig;
use crate::trainer::{PINNED_TRAINER_REVISION, PreparedTrainer};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Contract {
    schema: String,
    device: String,
    gpu_allowed_in_cpu_gate: bool,
    batch_size: u32,
    maximum_steps: u32,
    interrupt_step: u32,
    seed: u64,
    gpu_numeric_tolerance: f64,
    max_final_to_initial_loss_ratio: f64,
    max_validation_teacher_loss: f64,
    trainer_revision: String,
    r1_closeout_sha256: String,
    corpus_bin_sha256: String,
    corpus_jsonl_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactIdentity {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolchainIdentity {
    rustc: String,
    cxx: String,
    cmake: String,
    python: String,
    torch: String,
    cuda: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct R2PreGpuReport {
    schema: &'static str,
    schema_version: u32,
    phase: &'static str,
    device: &'static str,
    gpu_used: bool,
    trainer_revision: &'static str,
    trainer_local_diff_identity_sha256: String,
    toolchain: ToolchainIdentity,
    artifacts: BTreeMap<String, ArtifactIdentity>,
    gates: BTreeMap<String, bool>,
    pub(crate) cpu_gates_passed: bool,
    pub(crate) gpu_authorized: bool,
    pub(crate) passed: bool,
    next_command: String,
}

pub(crate) struct RunArgs<'a> {
    pub config_path: &'a Path,
    pub contract_path: &'a Path,
    pub r1_closeout_path: &'a Path,
    pub corpus_bin: &'a Path,
    pub corpus_jsonl: &'a Path,
    pub output_dir: &'a Path,
    pub python: &'a Path,
    pub workspace_root: &'a Path,
}

pub(crate) fn run(args: RunArgs<'_>) -> Result<R2PreGpuReport> {
    let contract: Contract = read_json(args.contract_path)?;
    ensure!(contract.schema == "haitaka-r2-trainer-contract-v1");
    ensure!(contract.device == "cpu" && !contract.gpu_allowed_in_cpu_gate);
    ensure!(contract.trainer_revision == PINNED_TRAINER_REVISION);
    ensure!(contract.maximum_steps >= 16);
    ensure!(contract.interrupt_step > 0 && contract.interrupt_step < contract.maximum_steps);
    ensure!(
        args.python.is_file(),
        "R2 Python is missing: {}",
        args.python.display()
    );
    validate_sha256(
        args.r1_closeout_path,
        &contract.r1_closeout_sha256,
        "R1 closeout",
    )?;
    validate_sha256(
        args.corpus_bin,
        &contract.corpus_bin_sha256,
        "R2 packed corpus",
    )?;
    validate_sha256(
        args.corpus_jsonl,
        &contract.corpus_jsonl_sha256,
        "R2 corpus JSONL",
    )?;
    let closeout: serde_json::Value = read_json(args.r1_closeout_path)?;
    ensure!(
        closeout["allReportsPassing"] == true
            && closeout["sourceIdentityExact"] == true
            && closeout["r2Authorized"] == true,
        "R2 requires a passing, provenance-exact R1 closeout that explicitly authorizes R2"
    );

    if args.output_dir.exists() && fs::read_dir(args.output_dir)?.next().is_some() {
        bail!(
            "R2 gate refuses non-empty output directory {}; use a new directory",
            args.output_dir.display()
        );
    }
    fs::create_dir_all(args.output_dir)?;
    let inputs_dir = args.output_dir.join("vast-bundle");
    fs::create_dir(&inputs_dir)?;

    let loaded = LoadedTrainingConfig::from_path(args.config_path)?;
    ensure!(loaded.config.training.random_fen_skipping == 0);
    ensure!(loaded.config.training.num_workers == 1);
    let trainer_checkout = loaded.trainer_checkout()?;
    let prepared = PreparedTrainer::new(&loaded, &trainer_checkout)?;
    let built_loader = find_loader_library(prepared.path())?;
    let loader = inputs_dir.join(
        built_loader
            .file_name()
            .context("built loader has no filename")?,
    );
    fs::copy(&built_loader, &loader)?;
    for name in [
        "variant.py",
        "variant.h",
        "feature_set.py",
        "features.py",
        "donor_features.py",
        "training_data_loader.cpp",
        "serialize.py",
        "requirements.txt",
        "requirements-CUDA128.txt",
    ] {
        let source = prepared.path().join(name);
        if source.is_file() {
            fs::copy(&source, inputs_dir.join(name))?;
        }
    }

    let helper = args
        .workspace_root
        .join("scripts/r2-deterministic-trainer.py");
    let cpu_dir = args.output_dir.join("cpu");
    let status = Command::new(args.python)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .arg(&helper)
        .args(["--mode", "cpu"])
        .arg("--loader")
        .arg(&loader)
        .arg("--corpus-bin")
        .arg(args.corpus_bin)
        .arg("--corpus-jsonl")
        .arg(args.corpus_jsonl)
        .arg("--output-dir")
        .arg(&cpu_dir)
        .arg("--batch-size")
        .arg(contract.batch_size.to_string())
        .arg("--maximum-steps")
        .arg(contract.maximum_steps.to_string())
        .arg("--interrupt-step")
        .arg(contract.interrupt_step.to_string())
        .arg("--seed")
        .arg(contract.seed.to_string())
        .arg("--gpu-tolerance")
        .arg(contract.gpu_numeric_tolerance.to_string())
        .arg("--max-final-to-initial-loss-ratio")
        .arg(contract.max_final_to_initial_loss_ratio.to_string())
        .arg("--max-validation-teacher-loss")
        .arg(contract.max_validation_teacher_loss.to_string())
        .current_dir(args.workspace_root)
        .status()
        .context("failed to run deterministic R2 CPU trainer")?;
    ensure!(status.success(), "R2 deterministic CPU trainer failed");
    let cpu_report_path = cpu_dir.join("r2-cpu-gate-report.json");
    let cpu_report: serde_json::Value = read_json(&cpu_report_path)?;
    ensure!(
        cpu_report["schema"] == "haitaka-anhoku-r2-cpu-gate-v1"
            && cpu_report["passedCpu"] == true
            && cpu_report["gpuUsed"] == false
            && cpu_report["passed"] == false,
        "R2 CPU helper report has an invalid phase boundary"
    );
    fs::copy(args.corpus_bin, inputs_dir.join("parity-corpus.bin"))?;
    fs::copy(args.corpus_jsonl, inputs_dir.join("parity-corpus.jsonl"))?;
    fs::copy(&helper, inputs_dir.join("r2-deterministic-trainer.py"))?;
    fs::copy(&cpu_report_path, inputs_dir.join("r2-cpu-gate-report.json"))?;
    fs::copy(
        args.contract_path,
        inputs_dir.join("r2-trainer-contract.json"),
    )?;
    fs::copy(
        args.workspace_root
            .join("r0/anhoku-reboot/r2-gpu-requirements.txt"),
        inputs_dir.join("r2-gpu-requirements.txt"),
    )?;
    let cpu_lock = Command::new(args.python)
        .args(["-m", "pip", "freeze", "--all"])
        .output()
        .context("failed to capture the resolved CPU Python dependency lock")?;
    ensure!(cpu_lock.status.success(), "CPU pip freeze failed");
    fs::write(inputs_dir.join("r2-cpu-pip-freeze.txt"), cpu_lock.stdout)?;
    let gpu_script = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nexport CUBLAS_WORKSPACE_CONFIG=:4096:8\npython3 -c \"import numpy, torch; assert numpy.__version__ == '2.2.6'; assert torch.__version__.split('+')[0] == '2.8.0'; assert torch.version.cuda == '12.8'; assert torch.cuda.is_available()\"\npython3 r2-deterministic-trainer.py --mode gpu --loader \"$PWD/{}\" --corpus-bin parity-corpus.bin --corpus-jsonl parity-corpus.jsonl --cpu-reference r2-cpu-gate-report.json --output-dir gpu --batch-size {} --maximum-steps {} --interrupt-step {} --seed {} --gpu-tolerance {} --max-final-to-initial-loss-ratio {} --max-validation-teacher-loss {}\n",
        loader.file_name().unwrap().to_string_lossy(),
        contract.batch_size,
        contract.maximum_steps,
        contract.interrupt_step,
        contract.seed,
        contract.gpu_numeric_tolerance,
        contract.max_final_to_initial_loss_ratio,
        contract.max_validation_teacher_loss,
    );
    fs::write(inputs_dir.join("run-r2-gpu.sh"), gpu_script)?;
    let bundle_archive = args.output_dir.join("anhoku-r2-vast-bundle.tgz");
    let status = Command::new("tar")
        .args(["-czf"])
        .arg(&bundle_archive)
        .arg("-C")
        .arg(args.output_dir)
        .arg("vast-bundle")
        .status()?;
    ensure!(status.success(), "failed to create R2 Vast.ai bundle");

    let toolchain = ToolchainIdentity {
        rustc: command_output("rustc", &["--version"]),
        cxx: command_output("c++", &["--version"]),
        cmake: command_output(&loaded.config.paths.cmake, &["--version"]),
        python: command_output(args.python.to_string_lossy().as_ref(), &["--version"]),
        torch: python_value(args.python, "import torch; print(torch.__version__)")?,
        cuda: python_optional_value(args.python, "import torch; print(torch.version.cuda or '')")?,
    };

    let overlay_sources = [
        args.workspace_root
            .join("trainer-patches/variant-nnue-pytorch-phase7.1.patch"),
        args.workspace_root.join("trainer-patches/evaluate.py"),
        args.workspace_root
            .join("haitaka_learn/trainer_overlay/feature_set.py"),
        args.workspace_root
            .join("haitaka_learn/trainer_overlay/features.py"),
        args.workspace_root
            .join("haitaka_learn/trainer_overlay/donor_features.py"),
        args.workspace_root
            .join("haitaka_learn/trainer_overlay/training_data_loader.cpp"),
    ];
    let mut overlay_hasher = Sha256::new();
    for source in &overlay_sources {
        overlay_hasher.update(fs::read(source)?);
    }
    let trainer_local_diff_identity_sha256 = format!("{:x}", overlay_hasher.finalize());

    let mut artifacts = BTreeMap::new();
    for (name, path) in [
        ("contract", args.contract_path.to_path_buf()),
        ("r1Closeout", args.r1_closeout_path.to_path_buf()),
        ("config", args.config_path.to_path_buf()),
        ("corpusBin", args.corpus_bin.to_path_buf()),
        ("corpusJsonl", args.corpus_jsonl.to_path_buf()),
        ("trainerHelper", helper),
        ("gateExecutable", std::env::current_exe()?),
        (
            "gateSource",
            args.workspace_root.join("haitaka_learn/src/r2.rs"),
        ),
        (
            "trainerIntegrationSource",
            args.workspace_root.join("haitaka_learn/src/trainer.rs"),
        ),
        ("loaderSharedLibrary", loader.clone()),
        ("serializer", inputs_dir.join("serialize.py")),
        ("variantPy", inputs_dir.join("variant.py")),
        ("variantHeader", inputs_dir.join("variant.h")),
        ("upstreamRequirements", inputs_dir.join("requirements.txt")),
        (
            "upstreamCudaRequirements",
            inputs_dir.join("requirements-CUDA128.txt"),
        ),
        (
            "resolvedCpuPythonLock",
            inputs_dir.join("r2-cpu-pip-freeze.txt"),
        ),
        (
            "gpuPythonRequirements",
            inputs_dir.join("r2-gpu-requirements.txt"),
        ),
        ("cpuGateReport", cpu_report_path.clone()),
        (
            "bundleCpuGateReport",
            inputs_dir.join("r2-cpu-gate-report.json"),
        ),
        ("bundleCorpusBin", inputs_dir.join("parity-corpus.bin")),
        ("bundleCorpusJsonl", inputs_dir.join("parity-corpus.jsonl")),
        ("bundleGpuRunner", inputs_dir.join("run-r2-gpu.sh")),
        ("vastBundleArchive", bundle_archive.clone()),
    ] {
        artifacts.insert(name.to_string(), artifact_identity(&path)?);
    }
    for (index, path) in overlay_sources.iter().enumerate() {
        artifacts.insert(format!("overlaySource{index}"), artifact_identity(path)?);
    }

    let gates = BTreeMap::from([
        ("r1CloseoutPassingAndHashBound".to_string(), true),
        ("pinnedImmutableTrainerContext".to_string(), true),
        ("dependencyAndToolchainIdentityRecorded".to_string(), true),
        ("randomSkippingZero".to_string(), true),
        ("singleThreadDeterministicLoader".to_string(), true),
        ("sameSeedCpuReproduction".to_string(), true),
        ("explicitFullStateResume".to_string(), true),
        ("finitePhysicalValidation".to_string(), true),
        ("sourceGroupSplit".to_string(), true),
        ("productionLoaderLearnability".to_string(), true),
        ("gpuCrossHardwareGatePending".to_string(), false),
    ]);
    let cpu_gates_passed = gates
        .iter()
        .filter(|(name, _)| name.as_str() != "gpuCrossHardwareGatePending")
        .all(|(_, value)| *value);
    let next_command = format!(
        "tar -xzf {} && cd vast-bundle && bash run-r2-gpu.sh",
        bundle_archive.display(),
    );
    let report = R2PreGpuReport {
        schema: "haitaka-anhoku-r2-pre-gpu-gate",
        schema_version: 1,
        phase: "R2",
        device: "cpu",
        gpu_used: false,
        trainer_revision: PINNED_TRAINER_REVISION,
        trainer_local_diff_identity_sha256,
        toolchain,
        artifacts,
        gates,
        cpu_gates_passed,
        gpu_authorized: cpu_gates_passed,
        passed: false,
        next_command,
    };
    let report_path = args.output_dir.join("r2-pre-gpu-gate-report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    Ok(report)
}

fn find_loader_library(root: &Path) -> Result<PathBuf> {
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if name.contains("training_data_loader")
            && matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("so" | "dylib" | "dll")
            )
        {
            return Ok(path);
        }
    }
    bail!(
        "compiled trainer loader is missing under {}",
        root.display()
    )
}

fn artifact_identity(path: &Path) -> Result<ArtifactIdentity> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(ArtifactIdentity {
        path: path.display().to_string(),
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    })
}

fn validate_sha256(path: &Path, expected: &str, label: &str) -> Result<()> {
    let actual = artifact_identity(path)?.sha256;
    ensure!(
        actual == expected,
        "{label} sha256 is {actual}, expected {expected}"
    );
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path)?).with_context(|| format!("parse {}", path.display()))
}

fn command_output(program: &str, args: &[&str]) -> String {
    let Ok(output) = Command::new(program).args(args).output() else {
        return "unavailable".to_string();
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    stdout
        .lines()
        .chain(stderr.lines())
        .next()
        .unwrap_or("unavailable")
        .to_string()
}

fn python_value(python: &Path, source: &str) -> Result<String> {
    let output = Command::new(python).args(["-c", source]).output()?;
    ensure!(output.status.success(), "Python provenance query failed");
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn python_optional_value(python: &Path, source: &str) -> Result<Option<String>> {
    Ok(match python_value(python, source)?.trim() {
        "" => None,
        value => Some(value.to_string()),
    })
}

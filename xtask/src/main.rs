use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

type Result<T> = std::result::Result<T, String>;

#[derive(Debug)]
struct PackageArgs {
    ruleset: String,
    rule_id: Option<u32>,
    output: Option<PathBuf>,
    wasm_dir: PathBuf,
    nnue: Option<PathBuf>,
    features: Option<String>,
    skip_wasm_build: bool,
    allow_missing_wasm: bool,
}

#[derive(Debug)]
struct GenerateDataArgs {
    config: PathBuf,
    extra_args: Vec<OsString>,
}

#[derive(Debug)]
struct TrainArgs {
    config: PathBuf,
    extra_args: Vec<OsString>,
}

impl Default for PackageArgs {
    fn default() -> Self {
        Self {
            ruleset: "standard".to_string(),
            rule_id: None,
            output: None,
            wasm_dir: PathBuf::from("haitaka_wasm/pkg"),
            nnue: None,
            features: None,
            skip_wasm_build: false,
            allow_missing_wasm: false,
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let Some(command) = args.next() else {
        print_usage();
        return Err("missing command".to_string());
    };

    match command.to_string_lossy().as_ref() {
        "package" => {
            let raw_args: Vec<OsString> = args.collect();
            if raw_args.iter().any(is_help_arg) {
                print_package_usage();
                Ok(())
            } else {
                package(parse_package_args(raw_args)?)
            }
        }
        "generate" | "generate-data" => {
            let raw_args: Vec<OsString> = args.collect();
            if raw_args.iter().any(is_help_arg) {
                print_generate_data_usage();
                Ok(())
            } else {
                generate_data(parse_generate_data_args(raw_args)?)
            }
        }
        "train" => {
            let raw_args: Vec<OsString> = args.collect();
            if raw_args.iter().any(is_help_arg) {
                print_train_usage();
                Ok(())
            } else {
                train(parse_train_args(raw_args)?)
            }
        }
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(())
        }
        other => Err(format!("unknown command: {other}")),
    }
}

fn parse_package_args(raw_args: Vec<OsString>) -> Result<PackageArgs> {
    let mut args = PackageArgs::default();
    let mut iter = raw_args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.to_string_lossy().as_ref() {
            "--ruleset" => args.ruleset = required_value(&mut iter, "--ruleset")?,
            "--rule-id" => {
                let value = required_value(&mut iter, "--rule-id")?;
                args.rule_id = Some(
                    value
                        .parse()
                        .map_err(|_| format!("--rule-id must be an integer, got {value:?}"))?,
                );
            }
            "--output" => args.output = Some(PathBuf::from(required_value(&mut iter, "--output")?)),
            "--wasm-dir" => args.wasm_dir = PathBuf::from(required_value(&mut iter, "--wasm-dir")?),
            "--nnue" => args.nnue = Some(PathBuf::from(required_value(&mut iter, "--nnue")?)),
            "--features" => args.features = Some(required_value(&mut iter, "--features")?),
            "--skip-wasm-build" => args.skip_wasm_build = true,
            "--allow-missing-wasm" => args.allow_missing_wasm = true,
            "-h" | "--help" => {
                print_package_usage();
                return Err("help requested".to_string());
            }
            other => return Err(format!("unknown package option: {other}")),
        }
    }
    Ok(args)
}

fn parse_generate_data_args(raw_args: Vec<OsString>) -> Result<GenerateDataArgs> {
    let (config, extra_args) = parse_config_first_args(raw_args, "generate-data")?;
    Ok(GenerateDataArgs {
        config,
        extra_args: normalize_generate_data_args(extra_args)?,
    })
}

fn parse_train_args(raw_args: Vec<OsString>) -> Result<TrainArgs> {
    let (config, extra_args) = parse_config_first_args(raw_args, "train")?;
    Ok(TrainArgs { config, extra_args })
}

fn parse_config_first_args(
    raw_args: Vec<OsString>,
    command_name: &str,
) -> Result<(PathBuf, Vec<OsString>)> {
    let mut iter = raw_args.into_iter();
    let Some(first) = iter.next() else {
        return Err("missing config path".to_string());
    };

    let config = if first == "--config" {
        PathBuf::from(required_value(&mut iter, "--config")?)
    } else if first.to_string_lossy().starts_with('-') {
        return Err(format!(
            "{command_name} expects the config path as the first argument"
        ));
    } else {
        PathBuf::from(first)
    };

    Ok((config, iter.collect()))
}

fn normalize_generate_data_args(raw_args: Vec<OsString>) -> Result<Vec<OsString>> {
    let uses_shard_index = raw_args.iter().any(|arg| {
        let arg = arg.to_string_lossy();
        arg == "--shard-index"
            || arg.starts_with("--shard-index=")
            || arg == "--shard-index-end"
            || arg.starts_with("--shard-index-end=")
    });
    let uses_shard_count = raw_args.iter().any(|arg| {
        let arg = arg.to_string_lossy();
        arg == "--shard-count" || arg.starts_with("--shard-count=")
    });

    let mut normalized = Vec::with_capacity(raw_args.len() + 2);
    let mut iter = raw_args.into_iter();
    let mut saw_shard = false;
    while let Some(arg) = iter.next() {
        let arg_text = arg.to_string_lossy();
        if arg_text == "--shard" {
            if saw_shard {
                return Err("--shard may only be specified once".to_string());
            }
            if uses_shard_index || uses_shard_count {
                return Err(
                    "--shard cannot be combined with --shard-index, --shard-index-end, or --shard-count".to_string(),
                );
            }
            let value = required_value(&mut iter, "--shard")?;
            let (shard_index, shard_index_end, shard_count) = parse_shard_spec(&value)?;
            normalized.extend(shard_args(shard_index, shard_index_end, shard_count));
            saw_shard = true;
        } else if let Some(value) = arg_text.strip_prefix("--shard=") {
            if saw_shard {
                return Err("--shard may only be specified once".to_string());
            }
            if uses_shard_index || uses_shard_count {
                return Err(
                    "--shard cannot be combined with --shard-index, --shard-index-end, or --shard-count".to_string(),
                );
            }
            let (shard_index, shard_index_end, shard_count) = parse_shard_spec(value)?;
            normalized.extend(shard_args(shard_index, shard_index_end, shard_count));
            saw_shard = true;
        } else {
            normalized.push(arg);
        }
    }
    Ok(normalized)
}

fn parse_shard_spec(value: &str) -> Result<(u32, u32, u32)> {
    let Some((shard_range, shard_count)) = value.split_once('/') else {
        return Err(format!(
            "--shard must use N/M or N-P/M format, got {value:?}"
        ));
    };
    let (shard_number, shard_number_end) = if let Some((start, end)) = shard_range.split_once('-') {
        (start, Some(end))
    } else {
        (shard_range, None)
    };
    let shard_number = shard_number
        .parse::<u32>()
        .map_err(|_| format!("--shard numerator must be an integer, got {shard_number:?}"))?;
    let shard_number_end = match shard_number_end {
        Some(value) => Some(
            value
                .parse::<u32>()
                .map_err(|_| format!("--shard range end must be an integer, got {value:?}"))?,
        ),
        None => None,
    };
    let shard_count = shard_count
        .parse::<u32>()
        .map_err(|_| format!("--shard denominator must be an integer, got {shard_count:?}"))?;
    if shard_count == 0 {
        return Err("--shard denominator must be greater than 0".to_string());
    }
    let shard_number_end = shard_number_end.unwrap_or(shard_number);
    if shard_number == 0 || shard_number > shard_count {
        return Err(format!(
            "--shard numerator must be between 1 and {shard_count}, got {shard_number}"
        ));
    }
    if shard_number_end == 0 || shard_number_end > shard_count {
        return Err(format!(
            "--shard range end must be between 1 and {shard_count}, got {shard_number_end}"
        ));
    }
    if shard_number_end < shard_number {
        return Err("--shard range end must be greater than or equal to its start".to_string());
    }
    Ok((shard_number - 1, shard_number_end - 1, shard_count))
}

fn shard_args(shard_index: u32, shard_index_end: u32, shard_count: u32) -> Vec<OsString> {
    vec![
        OsString::from("--shard-index"),
        OsString::from(shard_index.to_string()),
        OsString::from("--shard-index-end"),
        OsString::from(shard_index_end.to_string()),
        OsString::from("--shard-count"),
        OsString::from(shard_count.to_string()),
    ]
}

fn is_help_arg(arg: &OsString) -> bool {
    matches!(arg.to_string_lossy().as_ref(), "-h" | "--help")
}

fn required_value(iter: &mut impl Iterator<Item = OsString>, flag: &str) -> Result<String> {
    iter.next()
        .ok_or_else(|| format!("{flag} requires a value"))
        .map(|value| value.to_string_lossy().into_owned())
}

fn package(args: PackageArgs) -> Result<()> {
    let rule_id = args
        .rule_id
        .unwrap_or_else(|| default_rule_id(&args.ruleset));
    let output = args.output.unwrap_or_else(|| {
        if args.ruleset == "standard" {
            PathBuf::from("target/haitaka-variants.tgz")
        } else {
            PathBuf::from(format!("target/haitaka-variants-{}.tgz", args.ruleset))
        }
    });
    let features = args
        .features
        .or_else(|| inferred_package_feature(&args.ruleset).map(str::to_string));

    if !args.skip_wasm_build && !args.allow_missing_wasm {
        run_command(
            "wasm-pack",
            wasm_pack_args(features.as_deref()),
            "build wasm-bindgen package",
        )?;
    }

    run_command(
        "cargo",
        haitaka_cli_package_args(
            &args.ruleset,
            rule_id,
            &output,
            &args.wasm_dir,
            args.nnue.as_ref(),
            features.as_deref(),
            args.allow_missing_wasm,
        ),
        "create Shogitter engine package",
    )
}

fn generate_data(args: GenerateDataArgs) -> Result<()> {
    let ruleset = ruleset_from_config(&args.config)?;
    let features = required_learn_feature_for_ruleset(&ruleset)?;
    run_command(
        "cargo",
        haitaka_learn_generate_data_args(&args.config, features, &args.extra_args),
        "generate haitaka_learn data",
    )
}

fn train(args: TrainArgs) -> Result<()> {
    let ruleset = ruleset_from_config(&args.config)?;
    let features = required_learn_feature_for_ruleset(&ruleset)?;
    run_command(
        "cargo",
        haitaka_cli_build_args(features),
        "build haitaka_cli for self-play selection",
    )?;
    run_command(
        "cargo",
        haitaka_learn_train_select_args(&args.config, features, &args.extra_args),
        "train and select strongest NNUE checkpoint",
    )
}

fn ruleset_from_config(config: &PathBuf) -> Result<String> {
    let raw_toml = fs::read_to_string(config)
        .map_err(|err| format!("failed to read config {}: {err}", config.display()))?;
    ruleset_from_toml(&raw_toml)
}

fn ruleset_from_toml(raw_toml: &str) -> Result<String> {
    let value = raw_toml
        .parse::<toml::Value>()
        .map_err(|err| format!("failed to parse haitaka_learn TOML: {err}"))?;
    let ruleset = value
        .get("rules")
        .and_then(|rules| rules.get("ruleset"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| "config must set [rules].ruleset".to_string())?;
    Ok(ruleset.to_string())
}

fn required_learn_feature_for_ruleset(ruleset: &str) -> Result<Option<&'static str>> {
    match ruleset {
        "standard" | "handicap" => Ok(None),
        "annan" => Ok(Some("annan")),
        "anhoku" => Ok(Some("anhoku")),
        "antouzai" => Ok(Some("antouzai")),
        "taimen" => Ok(Some("taimen")),
        "haimen" => Ok(Some("haimen")),
        "neko" => Ok(Some("neko")),
        "nekoneko" => Ok(Some("nekoneko")),
        "yokoneko" => Ok(Some("yokoneko")),
        "yokonekoneko" => Ok(Some("yokonekoneko")),
        "tenkyo" => Ok(Some("tenkyo")),
        "tenjiku" => Ok(Some("tenjiku")),
        "anki" => Ok(Some("anki")),
        _ => Err(format!("unsupported rules.ruleset={ruleset:?}")),
    }
}

fn default_rule_id(ruleset: &str) -> u32 {
    match ruleset {
        "annan" => 26,
        "anhoku" => 55,
        "antouzai" => 95,
        "taimen" => 72,
        "haimen" => 74,
        "neko" => 130,
        "nekoneko" => 131,
        "yokoneko" => 132,
        "yokonekoneko" => 133,
        "tenkyo" => 151,
        "tenjiku" => 56,
        "anki" => 94,
        _ => 0,
    }
}

fn inferred_package_feature(ruleset: &str) -> Option<&'static str> {
    match ruleset {
        "annan" => Some("annan"),
        "anhoku" => Some("anhoku"),
        "antouzai" => Some("antouzai"),
        "taimen" => Some("taimen"),
        "haimen" => Some("haimen"),
        "neko" => Some("neko"),
        "nekoneko" => Some("nekoneko"),
        "yokoneko" => Some("yokoneko"),
        "yokonekoneko" => Some("yokonekoneko"),
        "tenkyo" => Some("tenkyo"),
        "tenjiku" => Some("tenjiku"),
        "anki" => Some("anki"),
        _ => None,
    }
}

fn wasm_pack_args(features: Option<&str>) -> Vec<OsString> {
    let mut args = os_args([
        "build",
        "haitaka_wasm",
        "--target",
        "web",
        "--out-dir",
        "pkg",
        "--release",
    ]);
    if let Some(features) = features {
        args.push("--features".into());
        args.push(features.into());
    }
    args
}

fn haitaka_cli_package_args(
    ruleset: &str,
    rule_id: u32,
    output: &PathBuf,
    wasm_dir: &PathBuf,
    nnue: Option<&PathBuf>,
    features: Option<&str>,
    allow_missing_wasm: bool,
) -> Vec<OsString> {
    let mut args = os_args(["run", "-p", "haitaka_cli", "--release"]);
    if let Some(features) = features {
        args.push("--features".into());
        args.push(features.into());
    }
    args.extend(os_args([
        "--",
        "package",
        "--wasm-dir",
        &wasm_dir.to_string_lossy(),
        "--ruleset",
        ruleset,
        "--rule-id",
        &rule_id.to_string(),
        "--output",
        &output.to_string_lossy(),
    ]));
    if let Some(nnue) = nnue {
        args.push("--nnue".into());
        args.push(nnue.into());
    }
    if allow_missing_wasm {
        args.push("--allow-missing-wasm".into());
    }
    args
}

fn haitaka_learn_generate_data_args(
    config: &PathBuf,
    features: Option<&str>,
    extra_args: &[OsString],
) -> Vec<OsString> {
    let mut args = os_args(["run", "-p", "haitaka_learn", "--release"]);
    if let Some(features) = features {
        args.push("--features".into());
        args.push(features.into());
    }
    args.extend(os_args(["--", "generate-data", "--config"]));
    args.push(config.as_os_str().to_os_string());
    args.extend(extra_args.iter().cloned());
    args
}

fn haitaka_cli_build_args(features: Option<&str>) -> Vec<OsString> {
    let mut args = os_args(["build", "-p", "haitaka_cli", "--release"]);
    if let Some(features) = features {
        args.push("--features".into());
        args.push(features.into());
    }
    args
}

fn haitaka_learn_train_select_args(
    config: &PathBuf,
    features: Option<&str>,
    extra_args: &[OsString],
) -> Vec<OsString> {
    let mut args = os_args(["run", "-p", "haitaka_learn", "--release"]);
    if let Some(features) = features {
        args.push("--features".into());
        args.push(features.into());
    }
    args.extend(os_args(["--", "train-select", "--config"]));
    args.push(config.as_os_str().to_os_string());
    args.push("--self-play-bin".into());
    args.push(self_play_bin_path().into());
    args.extend(extra_args.iter().cloned());
    args
}

fn self_play_bin_path() -> PathBuf {
    PathBuf::from(format!(
        "target/release/haitaka_cli{}",
        std::env::consts::EXE_SUFFIX
    ))
}

fn os_args<'a>(args: impl IntoIterator<Item = &'a str>) -> Vec<OsString> {
    args.into_iter().map(OsString::from).collect()
}

fn run_command(program: &str, args: Vec<OsString>, action: &str) -> Result<()> {
    println!("==> {action}");
    let mut child = Command::new(program)
        .args(args)
        .spawn()
        .map_err(|err| format!("failed to run {program}: {err}"))?;
    let _sigint_guard = ParentSigintGuard::ignore_while_waiting()?;
    let status = child
        .wait()
        .map_err(|err| format!("failed to wait for {program}: {err}"))?;
    if !status.success() {
        return Err(format!("{program} failed with status {status}"));
    }
    Ok(())
}

struct ParentSigintGuard {
    #[cfg(unix)]
    previous_handler: libc::sighandler_t,
}

impl ParentSigintGuard {
    fn ignore_while_waiting() -> Result<Self> {
        ignore_parent_sigint()
    }
}

#[cfg(unix)]
fn ignore_parent_sigint() -> Result<ParentSigintGuard> {
    let previous_handler = unsafe {
        let previous = libc::signal(libc::SIGINT, libc::SIG_IGN);
        if previous == libc::SIG_ERR {
            return Err("failed to install parent SIGINT ignore handler".to_string());
        }
        previous
    };
    Ok(ParentSigintGuard { previous_handler })
}

#[cfg(not(unix))]
fn ignore_parent_sigint() -> Result<ParentSigintGuard> {
    Ok(ParentSigintGuard {})
}

#[cfg(unix)]
impl Drop for ParentSigintGuard {
    fn drop(&mut self) {
        unsafe {
            libc::signal(libc::SIGINT, self.previous_handler);
        }
    }
}

#[cfg(not(unix))]
impl Drop for ParentSigintGuard {
    fn drop(&mut self) {}
}

fn print_usage() {
    eprintln!("Usage: cargo xtask package [options]");
    eprintln!("       cargo generate <config.toml> [generate-data options]");
    eprintln!("       cargo train <config.toml> [train-select options]");
    eprintln!("       cargo pack");
    eprintln!("       cargo pack-annan");
    eprintln!("       cargo run -p xtask -- package [options]");
}

fn print_generate_data_usage() {
    eprintln!("Usage: cargo generate <config.toml> [generate-data options]");
    eprintln!("Options:");
    eprintln!("  Reads [rules].ruleset from the TOML config, runs haitaka_learn with");
    eprintln!("  --release, and adds the matching --features flag when required.");
    eprintln!("  --shard <N/M> runs lane N of M using 1-indexed lane numbers.");
    eprintln!("  --shard <N-P/M> runs an inclusive lane range, e.g. --shard 3-5/8.");
    eprintln!("  Additional options are passed to haitaka_learn generate-data.");
}

fn print_train_usage() {
    eprintln!("Usage: cargo train <config.toml> [train-select options]");
    eprintln!("Options:");
    eprintln!("  Reads [rules].ruleset from the TOML config, builds haitaka_cli with");
    eprintln!("  matching --features when required, then runs haitaka_learn train-select.");
    eprintln!("  Useful options: --no-resume, --selection-max-games <N>, --storage-saver.");
}

fn print_package_usage() {
    eprintln!("Usage: cargo xtask package [options]");
    eprintln!("       cargo run -p xtask -- package [options]");
    eprintln!("Options:");
    eprintln!("  --ruleset <name>          Package ruleset, default standard");
    eprintln!("  --rule-id <id>            Shogitter rule id, default 0 or 26 for annan");
    eprintln!("  --output <path>           Output .tgz path");
    eprintln!("  --wasm-dir <path>         wasm-pack output directory, default haitaka_wasm/pkg");
    eprintln!("  --nnue <path>             Optional NNUE file to include");
    eprintln!("  --features <features>     Cargo features for wasm and package builds");
    eprintln!("  --skip-wasm-build         Reuse existing wasm-pack output");
    eprintln!("  --allow-missing-wasm      Metadata-only package, not Shogitter-loadable");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_feature_inference_only_uses_real_variant_features() {
        assert_eq!(inferred_package_feature("standard"), None);
        assert_eq!(inferred_package_feature("handicap"), None);
        assert_eq!(inferred_package_feature("custom"), None);
        assert_eq!(inferred_package_feature("annan"), Some("annan"));
        assert_eq!(inferred_package_feature("tenkyo"), Some("tenkyo"));
        assert_eq!(inferred_package_feature("anki"), Some("anki"));
    }

    #[test]
    fn handicap_package_args_do_not_pass_cargo_features() {
        let args = haitaka_cli_package_args(
            "handicap",
            6,
            &PathBuf::from("target/haitaka-variants-handicap.tgz"),
            &PathBuf::from("haitaka_wasm/pkg"),
            None,
            inferred_package_feature("handicap"),
            true,
        );

        assert!(!args.iter().any(|arg| arg == "--features"));
    }

    #[test]
    fn generate_data_infers_variant_feature_from_config_ruleset() {
        let ruleset = ruleset_from_toml("[rules]\nruleset = \"anhoku\"\n").unwrap();
        let args = haitaka_learn_generate_data_args(
            &PathBuf::from("haitaka_learn.anhoku-v0.5.1.toml"),
            required_learn_feature_for_ruleset(&ruleset).unwrap(),
            &normalize_generate_data_args(vec![
                OsString::from("--jobs"),
                OsString::from("0"),
                OsString::from("--shard"),
                OsString::from("1/4"),
            ])
            .unwrap(),
        );
        let args = args_as_strings(&args);

        assert!(args.iter().any(|arg| arg == "--release"));
        assert!(has_adjacent_args(&args, "--features", "anhoku"));
        assert!(has_adjacent_args(
            &args,
            "--config",
            "haitaka_learn.anhoku-v0.5.1.toml"
        ));
        assert!(has_adjacent_args(&args, "--jobs", "0"));
        assert!(has_adjacent_args(&args, "--shard-index", "0"));
        assert!(has_adjacent_args(&args, "--shard-index-end", "0"));
        assert!(has_adjacent_args(&args, "--shard-count", "4"));
    }

    #[test]
    fn generate_data_omits_features_for_standard_config() {
        let ruleset = ruleset_from_toml("[rules]\nruleset = \"standard\"\n").unwrap();
        let args = haitaka_learn_generate_data_args(
            &PathBuf::from("haitaka_learn.toml"),
            required_learn_feature_for_ruleset(&ruleset).unwrap(),
            &[],
        );

        assert!(!args.iter().any(|arg| arg == "--features"));
    }

    #[test]
    fn train_select_infers_variant_feature_and_forwards_options() {
        let ruleset = ruleset_from_toml("[rules]\nruleset = \"annan\"\n").unwrap();
        let args = haitaka_learn_train_select_args(
            &PathBuf::from("haitaka_learn.annan.toml"),
            required_learn_feature_for_ruleset(&ruleset).unwrap(),
            &[
                OsString::from("--selection-max-games"),
                OsString::from("128"),
                OsString::from("--storage-saver"),
            ],
        );
        let args = args_as_strings(&args);

        assert!(has_adjacent_args(&args, "--features", "annan"));
        assert!(args.iter().any(|arg| arg == "train-select"));
        assert!(has_adjacent_args(
            &args,
            "--config",
            "haitaka_learn.annan.toml"
        ));
        assert!(args.iter().any(|arg| arg == "--self-play-bin"));
        assert!(has_adjacent_args(&args, "--selection-max-games", "128"));
        assert!(args.iter().any(|arg| arg == "--storage-saver"));
    }

    #[test]
    fn train_select_omits_features_for_standard_config() {
        let ruleset = ruleset_from_toml("[rules]\nruleset = \"standard\"\n").unwrap();
        let args = haitaka_learn_train_select_args(
            &PathBuf::from("haitaka_learn.toml"),
            required_learn_feature_for_ruleset(&ruleset).unwrap(),
            &[],
        );

        assert!(!args.iter().any(|arg| arg == "--features"));
    }

    #[test]
    fn shard_shorthand_is_one_indexed() {
        let args = normalize_generate_data_args(vec![
            OsString::from("--shard=4/4"),
            OsString::from("--ignore-identity-mismatch"),
        ])
        .unwrap();
        let args = args_as_strings(&args);

        assert!(has_adjacent_args(&args, "--shard-index", "3"));
        assert!(has_adjacent_args(&args, "--shard-index-end", "3"));
        assert!(has_adjacent_args(&args, "--shard-count", "4"));
        assert!(args.iter().any(|arg| arg == "--ignore-identity-mismatch"));
    }

    #[test]
    fn shard_shorthand_accepts_inclusive_ranges() {
        let args = normalize_generate_data_args(vec![OsString::from("--shard=3-5/8")]).unwrap();
        let args = args_as_strings(&args);

        assert!(has_adjacent_args(&args, "--shard-index", "2"));
        assert!(has_adjacent_args(&args, "--shard-index-end", "4"));
        assert!(has_adjacent_args(&args, "--shard-count", "8"));
    }

    #[test]
    fn shard_shorthand_rejects_out_of_range_values() {
        assert!(parse_shard_spec("0/4").is_err());
        assert!(parse_shard_spec("5/4").is_err());
        assert!(parse_shard_spec("1/0").is_err());
        assert!(parse_shard_spec("1:4").is_err());
        assert!(parse_shard_spec("5-3/8").is_err());
        assert!(parse_shard_spec("3-9/8").is_err());
    }

    #[test]
    fn shard_shorthand_rejects_explicit_shard_flags() {
        let result = normalize_generate_data_args(vec![
            OsString::from("--shard"),
            OsString::from("1/4"),
            OsString::from("--shard-index-end"),
            OsString::from("0"),
        ]);

        assert!(result.is_err());
    }

    fn args_as_strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn has_adjacent_args(args: &[String], first: &str, second: &str) -> bool {
        args.windows(2)
            .any(|window| window[0] == first && window[1] == second)
    }
}

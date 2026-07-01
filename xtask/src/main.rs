use std::env;
use std::ffi::OsString;
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
        "package" => package(parse_package_args(args.collect())?),
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

fn os_args<'a>(args: impl IntoIterator<Item = &'a str>) -> Vec<OsString> {
    args.into_iter().map(OsString::from).collect()
}

fn run_command(program: &str, args: Vec<OsString>, action: &str) -> Result<()> {
    println!("==> {action}");
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|err| format!("failed to run {program}: {err}"))?;
    if !status.success() {
        return Err(format!("{program} failed with status {status}"));
    }
    Ok(())
}

fn print_usage() {
    eprintln!("Usage: cargo run -p xtask -- package [options]");
    eprintln!("       cargo pack");
    eprintln!("       cargo pack-annan");
}

fn print_package_usage() {
    eprintln!("Usage: cargo run -p xtask -- package [options]");
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
}

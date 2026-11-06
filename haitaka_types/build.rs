use std::{env, fs, path::PathBuf};
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let rules = [
        "MINISHOGI",
        "JUDKINS",
        "GOROGORO",
        "GOROGORO_PLUS",
        "MINISHOGI_KINGIN",
        "HEIAN_8X8",
        "HEIAN_9X8",
        "HEIAN_9X9",
    ];
    let selected: Vec<_> = rules
        .iter()
        .filter(|rule| env::var_os(format!("CARGO_FEATURE_{rule}")).is_some())
        .collect();
    assert!(
        selected.len() <= 1,
        "board rule features are mutually exclusive"
    );
    assert!(
        env::var_os("CARGO_FEATURE_VARIANT_BOARD").is_none() || selected.len() == 1,
        "variant-board requires exactly one geometry/rule feature"
    );
    let (width, height) = match selected.first().map(|rule| **rule) {
        Some("JUDKINS") => (6, 6),
        Some("HEIAN_8X8") => (8, 8),
        Some("HEIAN_9X8") => (9, 8),
        Some("HEIAN_9X9") => (9, 9),
        Some("GOROGORO" | "GOROGORO_PLUS") => (5, 6),
        Some(_) => (5, 5),
        None => (9, 9),
    };
    assert!(
        env::var_os("CARGO_FEATURE_COMPACT_BOARD").is_none() || selected.len() == 1,
        "compact-board requires a rule feature"
    );
    assert!(
        env::var_os("CARGO_FEATURE_SCALAR64").is_none()
            || (selected.len() == 1 && width * height <= 64),
        "scalar64 requires a <=64-square rule"
    );
    let names = [
        "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine",
    ];
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    for (name, variants) in [
        (
            "File",
            names[..width]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        ),
        (
            "Rank",
            (0..height)
                .map(|i| ((b'A' + i as u8) as char).to_string())
                .collect(),
        ),
        (
            "Square",
            (1..=width)
                .flat_map(|f| {
                    (0..height).map(move |r| format!("{}{}", (b'A' + r as u8) as char, f))
                })
                .collect(),
        ),
    ] {
        let mut source = format!(
            "crate::helpers::simple_enum! {{ #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)] pub enum {name} {{ {} }} }}",
            variants.join(",")
        );
        if name == "File" || name == "Rank" {
            let values = variants
                .iter()
                .enumerate()
                .map(|(i, variant)| {
                    let ch = if name == "File" {
                        b'1' + i as u8
                    } else {
                        b'a' + i as u8
                    } as char;
                    format!("{variant} = '{ch}'")
                })
                .collect::<Vec<_>>()
                .join(",");
            source += &format!(
                "\ncrate::helpers::enum_char_conv! {{ {name}, {name}ParseError {{ {values} }} }}"
            );
        }
        fs::write(out.join(format!("{}.rs", name.to_lowercase())), source).unwrap();
    }
}

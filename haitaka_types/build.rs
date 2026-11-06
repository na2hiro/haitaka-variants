use std::{env, fs, path::PathBuf};
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let rules = [
        "MINISHOGI",
        "JUDKINS",
        "GOROGORO",
        "GOROGORO_PLUS",
        "MINISHOGI_KINGIN",
    ];
    let selected: Vec<_> = rules
        .iter()
        .filter(|rule| env::var_os(format!("CARGO_FEATURE_{rule}")).is_some())
        .collect();
    assert!(
        selected.len() <= 1,
        "small-shogi rule features are mutually exclusive"
    );
    assert!(
        env::var_os("CARGO_FEATURE_SMALL_BOARD").is_none() || selected.len() == 1,
        "small-board requires exactly one small-shogi rule feature"
    );
    let (width, height) = match selected.first().map(|rule| **rule) {
        Some("JUDKINS") => (6, 6),
        Some("GOROGORO" | "GOROGORO_PLUS") => (5, 6),
        Some(_) => (5, 5),
        None => (9, 9),
    };
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

use std::{env, fs, path::PathBuf};
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let mini = env::var_os("CARGO_FEATURE_MINISHOGI").is_some();
    let (width, height) = if mini { (5, 5) } else { (9, 9) };
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
        let source = format!(
            "crate::helpers::simple_enum! {{ #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)] pub enum {name} {{ {} }} }}",
            variants.join(",")
        );
        fs::write(out.join(format!("{}.rs", name.to_lowercase())), source).unwrap();
    }
}

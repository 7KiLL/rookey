// Bakes every locales/<id>.json into the binary, so a new language is one file and nothing else.

use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=locales");
    let mut ids: Vec<String> = fs::read_dir("locales")
        .expect("the locales directory")
        .filter_map(|e| e.ok()?.file_name().to_str()?.strip_suffix(".json").map(str::to_string))
        .collect();
    ids.sort();
    // English first: every other language falls back to it
    ids.sort_by_key(|id| id != "en");
    let dir = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("locales");
    let rows: String =
        ids.iter().map(|id| format!("    ({id:?}, include_str!({:?})),\n", dir.join(format!("{id}.json")))).collect();
    let out = Path::new(&env::var("OUT_DIR").unwrap()).join("locales.rs");
    fs::write(out, format!("pub static LOCALES: &[(&str, &str)] = &[\n{rows}];\n")).unwrap();
}

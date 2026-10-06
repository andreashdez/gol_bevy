//! Embeds every `assets/patterns/*.rle` file as a built-in pattern, so adding
//! one to the palette needs no code change and the binary needs no assets
//! folder at runtime.
use std::{env, fs, path::Path};

const PATTERN_DIR: &str = "assets/patterns";

fn main() {
    // A directory makes cargo rerun this script when files are added or removed.
    println!("cargo:rerun-if-changed={PATTERN_DIR}");

    let mut stems: Vec<String> = fs::read_dir(PATTERN_DIR)
        .unwrap_or_else(|err| panic!("cannot read {PATTERN_DIR}: {err}"))
        .map(|entry| entry.expect("readable pattern directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rle"))
        .map(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_else(|| panic!("{} needs a UTF-8 file name", path.display()))
                .to_owned()
        })
        .collect();
    // Sorted so the palette order does not depend on the file system.
    stems.sort();

    let mut list = String::from("&[\n");
    for stem in &stems {
        let file = format!("/{PATTERN_DIR}/{stem}.rle");
        list.push_str(&format!(
            "    ({stem:?}, include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {file:?}))),\n"
        ));
    }
    list.push_str("]\n");

    let out_dir = env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    fs::write(Path::new(&out_dir).join("builtin_patterns.rs"), list)
        .expect("write builtin pattern list");
}

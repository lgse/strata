//! Generates the icon catalogs (K3, `docs/work/phase-1-leaves/K3-icon-catalogs.md`) from the
//! SVG sources in `icons/` (`icons/README.md`) into `$OUT_DIR/catalog.rs`.
//!
//! This file reads and writes; `src/icons/lucide/generate.rs` holds the rules (what a source
//! may hold, how it's coloured) and the crate's tests check them. A source the rules refuse
//! stops the build with its file's name.

use std::path::{Path, PathBuf};
use std::{env, fs};

#[path = "src/icons/lucide/generate.rs"]
mod generate;

use generate::Kind;

/// The source folders, and which colours each holds.
const FOLDERS: [(&str, Kind); 4] = [
    ("lucide", Kind::Lucide),
    ("lucide-extra", Kind::Lucide),
    ("own", Kind::Lucide),
    ("catppuccin", Kind::Catppuccin),
];

fn main() {
    let root =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let icons = root.join("icons");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icons/lucide/generate.rs");
    println!("cargo:rerun-if-changed=icons");

    let mut sources = Vec::new();
    for (folder, kind) in FOLDERS {
        for path in svgs(&icons.join(folder)) {
            let shown = format!(
                "icons/{folder}/{}",
                path.file_name()
                    .map_or(String::new(), |n| n.to_string_lossy().into_owned())
            );
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_else(|| panic!("{shown}: the name isn't UTF-8"));
            let svg = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{shown}: {e}"));
            let source =
                generate::icon(name, &svg, kind).unwrap_or_else(|why| panic!("{shown}: {why}"));
            sources.push(source);
        }
    }
    let catalog = generate::catalog(sources).unwrap_or_else(|why| panic!("icons: {why}"));

    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("catalog.rs");
    fs::write(&out, catalog).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
}

/// The `*.svg` files in `dir`, sorted by name.
fn svgs(dir: &Path) -> Vec<PathBuf> {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let mut paths: Vec<PathBuf> = entries
        .map(|e| {
            e.unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
                .path()
        })
        .filter(|p| p.extension().is_some_and(|x| x == "svg"))
        .collect();
    paths.sort();
    paths
}

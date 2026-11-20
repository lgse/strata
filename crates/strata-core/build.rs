// SPDX-License-Identifier: MIT

use std::{env, fmt::Write as _, fs, path::Path};

/// The catalogs live at the repository root with the rest of the application data.
const LOCALES: &str = "../../data/locales";

fn main() {
    println!("cargo::rerun-if-changed={LOCALES}");
    write_translation_catalogs();

    println!("cargo::rerun-if-env-changed=STRATA_TRUSTED_COMMAND_PATH");
    validate_trusted_command_path();
}

/// Extra host helper directories are embedded verbatim, so a relative or empty
/// entry would be an unsearchable mistake; reject it where the packager sees it.
fn validate_trusted_command_path() {
    let Some(value) = env::var_os("STRATA_TRUSTED_COMMAND_PATH") else {
        return;
    };
    let value = value
        .to_str()
        .expect("STRATA_TRUSTED_COMMAND_PATH must be valid UTF-8");
    for directory in value.split(':') {
        assert!(
            Path::new(directory).is_absolute(),
            "STRATA_TRUSTED_COMMAND_PATH entry {directory:?} must be an absolute directory"
        );
    }
}

/// Embeds the catalogs as static tables. rust-i18n's own `i18n!` loader emits one
/// statement per message, and that unoptimized frame overflows 2 MiB thread stacks.
fn write_translation_catalogs() {
    let catalogs = rust_i18n_support::load_locales(LOCALES, |_| false);
    // load_locales silently returns nothing when the catalog directory is absent.
    assert!(
        catalogs.contains_key("en"),
        "translation catalogs are missing from {LOCALES}"
    );
    let mut source = String::from("pub(crate) static CATALOGS: &[(&str, &[(&str, &str)])] = &[\n");
    for (locale, messages) in &catalogs {
        writeln!(source, "({locale:?}, &[").expect("format catalog");
        for (key, text) in messages {
            writeln!(source, "({key:?}, {text:?}),").expect("format catalog");
        }
        source.push_str("]),\n");
    }
    source.push_str("];\n");
    let output = Path::new(&env::var_os("OUT_DIR").expect("OUT_DIR")).join("catalogs.rs");
    fs::write(output, source).expect("write translation catalogs");
}

// SPDX-License-Identifier: MIT

//! The demo's command line: its own flags, then paths.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

pub const USAGE: &str =
    "usage: shell [--theme ID] [--overrides PATH] [--native] [--all-buttons] [PATH…]

  --theme ID        a bundled theme (default: tokyo-night)
  --overrides PATH  the overrides file to watch (default: $XDG_CONFIG_HOME/strata/overrides.toml);
                    read only, never written
  --native          keep the window manager's frame
  --all-buttons     show Minimize and Maximize too (a fresh Strata profile shows only Close)";

/// What a launch asked for.
#[derive(Debug, Default)]
pub struct Options {
    pub theme: Option<String>,
    pub overrides: Option<PathBuf>,
    pub native: bool,
    pub all_buttons: bool,
    pub help: bool,
    pub paths: Vec<OsString>,
}

impl Options {
    /// The arguments after the program's name. `--` ends the flags.
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Options, String> {
        let mut options = Options::default();
        let mut args = args.into_iter();
        let mut flags = true;
        while let Some(arg) = args.next() {
            let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
            match arg.to_str() {
                Some("--") if flags => flags = false,
                Some("--theme") if flags => {
                    options.theme = Some(value("--theme")?.to_string_lossy().into_owned());
                }
                Some("--overrides") if flags => {
                    options.overrides = Some(value("--overrides")?.into())
                }
                Some("--native") if flags => options.native = true,
                Some("--all-buttons") if flags => options.all_buttons = true,
                Some("--help" | "-h") if flags => options.help = true,
                Some(flag) if flags && flag.starts_with('-') && flag != "-" => {
                    return Err(format!("unknown option {flag}"));
                }
                _ => options.paths.push(arg),
            }
        }
        Ok(options)
    }

    pub fn overrides_path(&self) -> Option<PathBuf> {
        if let Some(path) = &self.overrides {
            return Some(path.clone());
        }
        let absolute = |v: OsString| Some(PathBuf::from(v)).filter(|p| p.is_absolute());
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .and_then(absolute)
            .or_else(|| {
                std::env::var_os("HOME")
                    .and_then(absolute)
                    .map(|home| home.join(".config"))
            })?;
        Some(config.join("strata").join("overrides.toml"))
    }
}

/// What a launch hands the running shell.
pub struct LaunchArgs {
    pub paths: Vec<OsString>,
    pub startup_only: Vec<&'static str>,
}

pub fn launch(argv: &[OsString]) -> LaunchArgs {
    let rest = argv.iter().skip(1).cloned();
    match Options::parse(rest) {
        Ok(o) => {
            let flags = [
                (o.theme.is_some(), "--theme"),
                (o.overrides.is_some(), "--overrides"),
                (o.native, "--native"),
                (o.all_buttons, "--all-buttons"),
            ];
            LaunchArgs {
                paths: o.paths,
                startup_only: flags.into_iter().filter(|f| f.0).map(|f| f.1).collect(),
            }
        }
        Err(_) => LaunchArgs {
            paths: argv
                .iter()
                .skip(1)
                .filter(|a| !a.to_string_lossy().starts_with('-'))
                .cloned()
                .collect(),
            startup_only: Vec::new(),
        },
    }
}

pub fn is_uri(arg: &OsStr) -> bool {
    let Some(arg) = arg.to_str() else {
        return false;
    };
    let Some((scheme, _)) = arg.split_once(':') else {
        return false;
    };
    scheme.len() >= 2
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// What a window shows for `path`: as given, lossily, so a name that isn't UTF-8 still opens.
pub fn shown(path: &OsStr) -> String {
    path.to_string_lossy().into_owned()
}

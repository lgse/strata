// SPDX-License-Identifier: MIT

//! Paths the sandbox binds and runs. Launch-time `STRATA_SANDBOX_*` variables
//! override values embedded at build time, which override the FHS defaults.

use std::{
    ffi::OsString,
    path::{Component, Path},
    sync::OnceLock,
};

#[cfg(test)]
mod tests;

const PATH_VARIABLE: &str = "STRATA_SANDBOX_PATH";
const ROOT_VARIABLE: &str = "STRATA_SANDBOX_ROOT";
const PRLIMIT_VARIABLE: &str = "STRATA_SANDBOX_PRLIMIT";
const GDK_PIXBUF_MODULE_FILE_VARIABLE: &str = "STRATA_SANDBOX_GDK_PIXBUF_MODULE_FILE";

/// A launch-time root is less trusted than a packager's build-time root: it
/// must stay inside an admin-managed runtime tree so `/` or user data can never
/// be exposed to untrusted decoders.
const ALLOWED_RUNTIME_ROOTS: &[&str] = &["/usr", "/nix/store", "/gnu/store"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Runtime {
    pub(crate) path: String,
    pub(crate) root: String,
    pub(crate) prlimit: String,
    pub(crate) gdk_pixbuf_module_file: Option<String>,
}

impl Runtime {
    fn embedded() -> Self {
        Self {
            path: option_env!("STRATA_SANDBOX_PATH")
                .unwrap_or("/usr/bin")
                .to_owned(),
            root: option_env!("STRATA_SANDBOX_ROOT")
                .unwrap_or("/usr")
                .to_owned(),
            prlimit: option_env!("STRATA_SANDBOX_PRLIMIT")
                .unwrap_or("/usr/bin/prlimit")
                .to_owned(),
            gdk_pixbuf_module_file: option_env!("STRATA_SANDBOX_GDK_PIXBUF_MODULE_FILE")
                .map(str::to_owned),
        }
    }

    pub(crate) fn tool_directories(&self) -> impl Iterator<Item = &Path> {
        self.path
            .split(':')
            .filter(|directory| !directory.is_empty())
            .map(Path::new)
    }
}

pub(crate) fn current() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| resolve(|name| std::env::var_os(name)))
}

/// Applies launch-time overrides on top of the embedded configuration. A value
/// that fails validation is ignored with a warning so the sandbox keeps the
/// packager's configuration rather than binding an unsafe path.
fn resolve(lookup: impl Fn(&str) -> Option<OsString>) -> Runtime {
    let mut runtime = Runtime::embedded();
    if let Some(path) = override_value(&lookup, PATH_VARIABLE, is_search_path) {
        runtime.path = path;
    }
    if let Some(root) = override_value(&lookup, ROOT_VARIABLE, is_allowed_root) {
        runtime.root = root;
    }
    if let Some(prlimit) = override_value(&lookup, PRLIMIT_VARIABLE, is_plain_absolute_path) {
        runtime.prlimit = prlimit;
    }
    if let Some(loaders) = override_value(
        &lookup,
        GDK_PIXBUF_MODULE_FILE_VARIABLE,
        is_plain_absolute_path,
    ) {
        runtime.gdk_pixbuf_module_file = Some(loaders);
    }
    runtime
}

fn override_value(
    lookup: &impl Fn(&str) -> Option<OsString>,
    variable: &str,
    accepts: fn(&str) -> bool,
) -> Option<String> {
    let value = lookup(variable)?;
    match value.into_string() {
        Ok(value) if accepts(&value) => Some(value),
        Ok(value) => {
            tracing::warn!(
                variable,
                value,
                "ignoring an invalid launch-time sandbox override"
            );
            None
        }
        Err(_) => {
            tracing::warn!(
                variable,
                "ignoring a launch-time sandbox override that is not valid UTF-8"
            );
            None
        }
    }
}

fn is_search_path(value: &str) -> bool {
    !value.is_empty() && value.split(':').all(is_plain_absolute_path)
}

fn is_allowed_root(value: &str) -> bool {
    is_plain_absolute_path(value)
        && ALLOWED_RUNTIME_ROOTS
            .iter()
            .any(|allowed| Path::new(value).starts_with(allowed))
}

/// Absolute, with no `..` component that could escape the root prefix check.
fn is_plain_absolute_path(value: &str) -> bool {
    let path = Path::new(value);
    path.is_absolute()
        && !value.contains('\0')
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
}

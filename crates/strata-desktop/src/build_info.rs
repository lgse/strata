// SPDX-License-Identifier: MIT

use crate::services::{BuildKind, Version};

pub const VERSION: &str = env!("STRATA_APP_VERSION");
pub const COMMIT: &str = env!("STRATA_BUILD_COMMIT");
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
pub const AUTHOR: &str = env!("CARGO_PKG_AUTHORS");

pub const RELEASE_TAG: &str = env!("STRATA_RELEASE_TAG");

pub const BUILD_KIND: &str = env!("STRATA_BUILD_KIND");

pub fn build_kind() -> BuildKind {
    match BUILD_KIND {
        "nightly" => BuildKind::Nightly,
        "alpha" => BuildKind::Alpha,
        "beta" => BuildKind::Beta,
        "rc" => BuildKind::Rc,
        _ => BuildKind::Stable,
    }
}

pub fn updater_enabled() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    if UPDATER_DISABLED_FOR_TEST.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    !cfg!(strata_updater_disabled)
}

#[cfg(any(test, feature = "test-support"))]
static UPDATER_DISABLED_FOR_TEST: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(any(test, feature = "test-support"))]
pub fn disable_updater_for_test() {
    UPDATER_DISABLED_FOR_TEST.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn runtime_integrations_enabled() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    if RUNTIME_INTEGRATIONS_DISABLED_FOR_TEST.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    !cfg!(strata_integrations_package)
}

#[cfg(any(test, feature = "test-support"))]
static RUNTIME_INTEGRATIONS_DISABLED_FOR_TEST: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(any(test, feature = "test-support"))]
pub fn disable_runtime_integrations_for_test() {
    RUNTIME_INTEGRATIONS_DISABLED_FOR_TEST.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn installed_version() -> Version {
    Version::parse(RELEASE_TAG)
        .or_else(|| Version::parse(VERSION))
        .unwrap_or_else(|| {
            Version::parse("0.0.0").expect(
                "\"0.0.0\" is exactly the bare MAJOR.MINOR.PATCH form Version::parse accepts",
            )
        })
}

pub fn displayed_version() -> String {
    let version = installed_version().to_string();
    if cfg!(debug_assertions) {
        format!("{version}+debug")
    } else {
        version
    }
}

#[cfg(test)]
mod tests;

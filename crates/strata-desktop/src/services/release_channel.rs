// SPDX-License-Identifier: MIT

use std::{cmp::Ordering, fmt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Preview,
    Nightly,
}

impl Channel {
    /// The persisted/config-file representation of this channel.
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Preview => "preview",
            Channel::Nightly => "nightly",
        }
    }

    pub fn parse(value: &str) -> Channel {
        match value {
            "preview" => Channel::Preview,
            "nightly" => Channel::Nightly,
            _ => Channel::Stable,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BuildKind {
    Stable,
    Nightly,
    Alpha,
    Beta,
    Rc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Ordinal {
    primary: u64,
    suffix: u64,
}

#[derive(Clone, Debug)]
pub struct Prerelease {
    kind: BuildKind,
    ordinal: Ordinal,
}

#[derive(Clone, Debug)]
pub struct Version {
    core: (u64, u64, u64),
    prerelease: Option<Prerelease>,
}

fn parse_strict_u64(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn parse_core(value: &str) -> Option<(u64, u64, u64)> {
    let mut parts = value.split('.');
    let major = parse_strict_u64(parts.next()?)?;
    let minor = parse_strict_u64(parts.next()?)?;
    let patch = parse_strict_u64(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn parse_prerelease(suffix: &str) -> Option<Prerelease> {
    let mut parts = suffix.split('.');
    match parts.next()? {
        kind @ ("alpha" | "beta" | "rc") => {
            let n = parse_strict_u64(parts.next()?)?;
            if n == 0 || parts.next().is_some() {
                return None;
            }
            let kind = match kind {
                "alpha" => BuildKind::Alpha,
                "beta" => BuildKind::Beta,
                _ => BuildKind::Rc,
            };
            Some(Prerelease {
                kind,
                ordinal: Ordinal {
                    primary: n,
                    suffix: 0,
                },
            })
        }
        "nightly" => {
            let date_str = parts.next()?;
            if date_str.len() != 8 {
                return None;
            }
            let date = parse_strict_u64(date_str)?;
            let suffix_n = match parts.next() {
                Some(n_str) => parse_strict_u64(n_str)?,
                None => 0,
            };
            if parts.next().is_some() {
                return None;
            }
            Some(Prerelease {
                kind: BuildKind::Nightly,
                ordinal: Ordinal {
                    primary: date,
                    suffix: suffix_n,
                },
            })
        }
        _ => None,
    }
}

impl Version {
    pub fn parse(tag: &str) -> Option<Version> {
        let rest = tag.strip_prefix('v').unwrap_or(tag);
        if rest.is_empty() {
            return None;
        }
        let (core_str, prerelease_str) = match rest.split_once('-') {
            Some((core, suffix)) => (core, Some(suffix)),
            None => (rest, None),
        };
        let core = parse_core(core_str)?;
        let prerelease = match prerelease_str {
            Some(suffix) => Some(parse_prerelease(suffix)?),
            None => None,
        };
        Some(Version { core, prerelease })
    }

    pub fn build_kind(&self) -> BuildKind {
        self.prerelease
            .as_ref()
            .map_or(BuildKind::Stable, |prerelease| prerelease.kind)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (major, minor, patch) = self.core;
        write!(f, "{major}.{minor}.{patch}")?;
        match &self.prerelease {
            Some(prerelease) => match prerelease.kind {
                BuildKind::Alpha => write!(f, "-alpha.{}", prerelease.ordinal.primary),
                BuildKind::Beta => write!(f, "-beta.{}", prerelease.ordinal.primary),
                BuildKind::Rc => write!(f, "-rc.{}", prerelease.ordinal.primary),
                BuildKind::Nightly => {
                    let date = prerelease.ordinal.primary;
                    let suffix_n = prerelease.ordinal.suffix;
                    if suffix_n == 0 {
                        write!(f, "-nightly.{date}")
                    } else {
                        write!(f, "-nightly.{date}.{suffix_n}")
                    }
                }
                BuildKind::Stable => Ok(()),
            },
            None => Ok(()),
        }
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.kind.cmp(&b.kind).then_with(|| a.ordinal.cmp(&b.ordinal)),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

#[derive(Clone, Debug)]
pub struct ReleaseSummary {
    pub tag: String,
    pub version: Version,
    pub draft: bool,
    pub prerelease: bool,
    pub download_url: Option<String>,
    pub published_at: Option<String>,
    pub notes: String,
}

pub fn is_eligible(channel: Channel, release: &ReleaseSummary) -> bool {
    if release.draft || release.download_url.is_none() {
        return false;
    }
    match channel {
        Channel::Stable => release.version.prerelease.is_none() && !release.prerelease,
        Channel::Preview => release.version.build_kind() != BuildKind::Nightly,
        Channel::Nightly => true,
    }
}

pub fn best_update<'a>(
    channel: Channel,
    installed: &Version,
    releases: &'a [ReleaseSummary],
) -> Option<&'a ReleaseSummary> {
    let candidate = releases
        .iter()
        .filter(|release| is_eligible(channel, release))
        .max_by(|a, b| a.version.cmp(&b.version))?;
    (&candidate.version > installed).then_some(candidate)
}

pub fn installable_releases(channel: Channel, releases: &[ReleaseSummary]) -> Vec<&ReleaseSummary> {
    let mut eligible: Vec<_> = releases
        .iter()
        .filter(|release| is_eligible(channel, release))
        .collect();
    eligible.sort_by(|a, b| b.version.cmp(&a.version));
    eligible.dedup_by(|a, b| a.version == b.version);
    eligible
}

pub fn rollback_target(releases: &[ReleaseSummary]) -> Option<&ReleaseSummary> {
    // `is_eligible(Stable, ..)` already asserts `version.prerelease.is_none()`
    // (alongside GitHub's own `prerelease` flag), so no separate check is
    // needed here.
    releases
        .iter()
        .filter(|release| is_eligible(Channel::Stable, release))
        .max_by(|a, b| a.version.cmp(&b.version))
}

#[cfg(test)]
mod tests;

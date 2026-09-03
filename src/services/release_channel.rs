// SPDX-License-Identifier: GPL-3.0-or-later

//! Pure channel and version types for the self-updater.
//!
//! This module is the single place in the codebase that interprets release
//! tags and reasons about their precedence. It performs no I/O and knows
//! nothing about GitHub's API shapes -- callers hand it plain strings and
//! get back structured, comparable values.

use std::{cmp::Ordering, fmt};

/// The user's persisted update-channel preference.
///
/// This is deliberately distinct from [`BuildKind`], which describes what a
/// given release *is* rather than which stability level the user requested.
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

    /// Parses a persisted channel value, falling back to [`Channel::Stable`]
    /// for anything unrecognised.
    ///
    /// This must fail closed: a corrupted or hand-edited config value must
    /// never silently opt a user into prereleases.
    pub fn parse(value: &str) -> Channel {
        match value {
            "preview" => Channel::Preview,
            "nightly" => Channel::Nightly,
            _ => Channel::Stable,
        }
    }
}

/// What a given release build IS, independent of the user's channel
/// preference.
///
/// Kept separate from [`Channel`] because each prerelease stage retains a
/// distinct user-facing label even when several are accepted by one channel.
///
/// Declaration order pins precedence for prereleases sharing a core version
/// (see [`Version`]'s `Ord` impl): `Nightly < Alpha < Beta < Rc`. A final
/// release always outranks every prerelease of its core version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BuildKind {
    Stable,
    Nightly,
    Alpha,
    Beta,
    Rc,
}

impl BuildKind {
    /// The user-facing label shown in the UI.
    pub fn label(self) -> &'static str {
        match self {
            BuildKind::Stable => "Stable",
            BuildKind::Nightly => "Nightly",
            BuildKind::Alpha => "Alpha",
            BuildKind::Beta => "Beta",
            BuildKind::Rc => "Release candidate",
        }
    }
}

/// A comparable ordinal for a prerelease build.
///
/// For an RC, `primary` is the candidate number and `suffix` is always
/// zero. For a nightly, `primary` is the `YYYYMMDD` date and `suffix` is
/// the optional `.N` disambiguator (zero when absent).
///
/// These are kept as two separate fields, ordered lexicographically,
/// rather than packed into a single integer. Packing (e.g. `date * 1000 +
/// n`) would let an unbounded `.N` spill into the date component -- the
/// grammar places no bound on `N` -- silently corrupting both the ordering
/// and the round-tripped `Display` output. Two fields cannot collide this
/// way.
///
/// Comparing an RC's `primary` against a nightly's `primary` would be
/// comparing a small integer against a date, which is meaningless; see
/// [`Version`]'s `Ord` impl, which only ever compares two `Ordinal`s after
/// confirming both sides share the same [`BuildKind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Ordinal {
    primary: u64,
    suffix: u64,
}

/// A parsed prerelease suffix: its kind, an orderable ordinal, and the
/// original text as published.
#[derive(Clone, Debug)]
pub struct Prerelease {
    kind: BuildKind,
    ordinal: Ordinal,
}

/// A semver-correct, comparable representation of a release tag.
///
/// [`Version::parse`] is the *only* place release tags are interpreted
/// anywhere in the codebase. It accepts an optional leading `v` and exactly
/// three grammar forms:
///
/// ```text
/// v?MAJOR.MINOR.PATCH
/// v?MAJOR.MINOR.PATCH-alpha.N
/// v?MAJOR.MINOR.PATCH-beta.N
/// v?MAJOR.MINOR.PATCH-rc.N
/// v?MAJOR.MINOR.PATCH-nightly.YYYYMMDD[.N]
/// ```
///
/// Anything else returns `None` -- there is no silent zero-fill fallback.
#[derive(Clone, Debug)]
pub struct Version {
    core: (u64, u64, u64),
    prerelease: Option<Prerelease>,
}

/// Parses a string as a strictly non-negative, non-signed decimal `u64`.
///
/// `str::parse` alone is not strict enough here: it accepts a leading `+`,
/// which would let a malformed tag segment slip through.
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
    /// Parses a release tag per the grammar documented on [`Version`].
    /// Returns `None` for anything that does not match exactly.
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

    /// The [`BuildKind`] this version identifies as: [`BuildKind::Stable`]
    /// for a version with no prerelease suffix, or the parsed prerelease's
    /// kind otherwise.
    ///
    /// Used by `update_check` to label a release's build kind for display,
    /// without exposing `Prerelease`'s otherwise-private fields.
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
    /// Per semver §11: compare core triples first; for equal cores, a
    /// prerelease is always less than a final release; for two prereleases
    /// on an equal core, compare `kind` first and *only then* `ordinal`.
    ///
    /// `kind` must be compared first: an RC's ordinal (a small candidate
    /// number) and a nightly's ordinal (a date) are different quantities
    /// and are never meaningfully comparable. Short-circuiting on `kind`
    /// via `then_with` guarantees `ordinal.cmp` only ever runs when both
    /// sides share the same [`BuildKind`].
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

/// A single release, normalised from GitHub's API shape into the fields
/// this module's eligibility rules need.
///
/// Deliberately free of `serde` derives: converting GitHub's JSON into this
/// type is Task 4's job, kept strictly on its own side of the boundary this
/// module enforces. This module never learns what GitHub's JSON looks like.
#[derive(Clone, Debug)]
pub struct ReleaseSummary {
    pub tag: String,
    pub version: Version,
    pub draft: bool,
    /// GitHub's own `prerelease` flag on the release, distinct from
    /// whether `version` itself parsed with a prerelease suffix. Stable
    /// eligibility requires both signals to agree; see [`is_eligible`].
    pub prerelease: bool,
    /// `None` when no published asset matches the running architecture --
    /// an update the user cannot install must never be offered.
    pub download_url: Option<String>,
    pub published_at: Option<String>,
    pub notes: String,
}

/// Whether `release` may be offered to a user on `channel`.
///
/// A draft is never eligible, on any channel: it is not a published
/// release. Nor is a release with no installable asset for this
/// architecture, since offering it would leave the user stuck.
///
/// On [`Channel::Stable`], both the parsed version and GitHub's own
/// `prerelease` flag must agree that the release is final. This
/// redundancy is a stated security requirement of issue #61: a release
/// mislabelled on either signal alone must still be caught by the other.
///
/// [`Channel::Preview`] accepts stable, alpha, beta, and RC releases but not
/// nightlies. [`Channel::Nightly`] accepts every recognised build kind. Drafts
/// and assetless releases are rejected on every channel.
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

/// The newest eligible release strictly newer than `installed`, or `None`
/// if there isn't one.
///
/// Filters `releases` by [`is_eligible`] for `channel`, then returns the
/// maximum by [`Version`] ordering -- but only when that maximum is
/// strictly greater than `installed`. This path must never offer a
/// downgrade; use [`rollback_target`] for that instead.
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

/// The newest final release available, ignoring the installed version
/// entirely.
///
/// Ignoring the installed version is precisely what makes a downgrade
/// possible -- a user on a prerelease must be able to roll back to a
/// stable release older than what they currently have installed. This is
/// why `rollback_target` cannot reuse [`best_update`], which deliberately
/// refuses to go backwards.
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

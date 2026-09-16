// SPDX-License-Identifier: MIT

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use super::update_install::{
    UpdateMethod, aur_repository_version, omarchy_repository_version, package_repository_version,
};
use super::{
    DocumentBlock, parse_markdown,
    release_channel::{BuildKind, Channel, ReleaseSummary, Version, best_update, rollback_target},
};

const API_ROOT: &str = "https://api.github.com/repos/lgse/strata/releases";
const COMMITS_ROOT: &str = "https://api.github.com/repos/lgse/strata/commits";
const RELEASES_URL: &str = "https://github.com/lgse/strata/releases";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Minimum interval between automatic checks against the same channel.
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// How many of the most recent releases (final and prerelease) the preview
/// feed enumerates. Only ever used to find a release *newer* than the
/// installed one, so a bounded page is safe: an update this page cannot
/// reach is older than one it can.
const PREVIEW_PAGE_SIZE: u32 = 30;

/// Everything the update/rollback dialogs need to identify and describe a
/// release before installing it: what build it is, its exact tag and
/// display version, where it was published, and its rendered notes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseMetadata {
    /// The full, prerelease-bearing version for display, e.g. `0.5.0-rc.1`.
    pub version: String,
    pub url: String,
    pub notes: String,
    pub note_blocks: Vec<DocumentBlock>,
    pub kind: BuildKind,
    /// The exact tag as published on GitHub, e.g. `v0.5.0-rc.1`.
    pub tag: String,
    pub published_at: Option<String>,
    pub commit: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateCheck {
    UpToDate,
    Available {
        release: ReleaseMetadata,
        download_url: String,
    },
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReleaseNotes {
    Found(ReleaseMetadata),
    Unavailable { url: String },
    Failed { message: String, url: String },
}

#[derive(Deserialize)]
struct ReleaseResponse {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    published_at: Option<String>,
}

/// The single field this code needs from GitHub's commit representation.
#[derive(Deserialize)]
struct CommitResponse {
    sha: String,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

/// The asset naming convention published by `.github/workflows/release.yml`.
fn archive_name(version: &str) -> String {
    format!(
        "strata-{version}-{}-unknown-linux-gnu.tar.gz",
        std::env::consts::ARCH
    )
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build()
        .into()
}

fn request_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, ureq::Error> {
    agent()
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "strata-file-manager")
        .call()
        .and_then(|mut response| response.body_mut().read_json::<T>())
}

/// Converts a GitHub API release into [`release_channel`]'s pure
/// representation. This is the only place `update_check` interprets
/// `ReleaseResponse`'s wire shape.
///
/// Returns `None` when `tag_name` does not match the tag grammar --
/// [`Version::parse`] is the sole authority on that, so a malformed release
/// is dropped here rather than reaching any eligibility check.
///
/// `download_url` is `None` when no asset matches [`archive_name`] for this
/// architecture; `is_eligible` treats that the same as a draft, since an
/// update the user cannot install must never be offered.
fn to_release_summary(release: &ReleaseResponse) -> Option<ReleaseSummary> {
    let version = Version::parse(&release.tag_name)?;
    let archive = archive_name(&version.to_string());
    let download_url = release
        .assets
        .iter()
        .find(|asset| asset.name == archive)
        .map(|asset| asset.browser_download_url.clone());
    Some(ReleaseSummary {
        tag: release.tag_name.clone(),
        version,
        draft: release.draft,
        prerelease: release.prerelease,
        download_url,
        published_at: release.published_at.clone(),
        notes: release.body.clone().unwrap_or_default(),
    })
}

/// Renders a [`ReleaseSummary`] into the display-ready shape the settings
/// dialogs consume.
fn release_metadata(release: &ReleaseSummary) -> ReleaseMetadata {
    ReleaseMetadata {
        version: release.version.to_string(),
        url: release_page_url(&release.tag),
        notes: release.notes.clone(),
        note_blocks: parse_markdown(&release.notes).blocks,
        kind: release.version.build_kind(),
        tag: release.tag.clone(),
        published_at: release.published_at.clone(),
        // Resolved by `resolve_commit` on the worker thread; a release's own
        // JSON carries no usable commit. See [`fetch_commit`].
        commit: None,
    }
}

/// Fetches the single newest final release from GitHub's `/releases/latest`,
/// which itself never returns a draft or prerelease.
///
/// Kept as its own function, deliberately never enumerating the full release
/// list: this is the strongest form of channel isolation, since prerelease
/// data for a Stable user never even enters the process. Its result is still
/// additionally run through [`is_eligible`] by [`select_update`] -- both
/// checks are required, per issue #61's stated redundancy requirement.
fn fetch_stable(etag: Option<&str>) -> ChannelFetch {
    match request_json_conditional::<ReleaseResponse>(&format!("{API_ROOT}/latest"), etag) {
        Ok(Some((release, etag))) => ChannelFetch::Fetched {
            releases: to_release_summary(&release).into_iter().collect(),
            etag,
        },
        Ok(None) => ChannelFetch::Unchanged,
        Err(ureq::Error::StatusCode(404)) => ChannelFetch::Fetched {
            releases: Vec::new(),
            etag: None,
        },
        Err(error) => ChannelFetch::Failed(error),
    }
}

/// Fetches the most recent releases, final and prerelease alike, for the
/// preview channel.
///
/// Kept as its own function rather than reused for [`Channel::Stable`]: a
/// Stable user's code path must call [`fetch_stable`] instead, never this,
/// so prerelease metadata never reaches that path at all.
fn fetch_preview(etag: Option<&str>) -> ChannelFetch {
    match request_json_conditional::<Vec<ReleaseResponse>>(
        &format!("{API_ROOT}?per_page={PREVIEW_PAGE_SIZE}"),
        etag,
    ) {
        Ok(Some((releases, etag))) => ChannelFetch::Fetched {
            releases: releases.iter().filter_map(to_release_summary).collect(),
            etag,
        },
        Ok(None) => ChannelFetch::Unchanged,
        Err(error) => ChannelFetch::Failed(error),
    }
}

/// Resolves `tag` to the commit SHA it points at.
///
/// A release's own `target_commitish` cannot be used for this: GitHub
/// ignores that value when the tag already exists, which is how
/// `release.yml` publishes every release, so it comes back as the default
/// branch name rather than a SHA. `/commits/{tag}` dereferences the
/// annotated tag and returns the real commit.
///
/// `None` on any failure -- the dialog's identity block falls back to
/// "Unknown", since an offered update must not hinge on a lookup that only
/// feeds one display row.
fn fetch_commit(tag: &str) -> Option<String> {
    request_json::<CommitResponse>(&format!("{COMMITS_ROOT}/{tag}"))
        .ok()
        .map(|commit| commit.sha)
}

/// Fills in the commit the update dialog displays, off the GTK thread.
/// Kept out of [`release_metadata`] so that every selection step stays
/// pure and network-free.
fn resolve_commit(release: &mut ReleaseMetadata) {
    release.commit = fetch_commit(&release.tag);
}

fn cache_dir() -> PathBuf {
    glib::user_cache_dir().join("strata")
}

fn update_check_cache_path() -> PathBuf {
    cache_dir().join("update-check.toml")
}

fn release_notes_cache_path() -> PathBuf {
    cache_dir().join("release-notes.toml")
}

fn read_cache_file<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    toml::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn write_cache_file<T: Serialize>(path: &Path, value: &T) {
    let result = (|| -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let value = toml::to_string_pretty(value).map_err(io::Error::other)?;
        crate::storage::atomic_write(path, value.as_bytes())
    })();
    if let Err(error) = result {
        tracing::warn!(%error, path = %path.display(), "unable to save update-check cache");
    }
}

fn unix_seconds_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[derive(Serialize, Deserialize)]
struct CachedReleaseNotes {
    tag: String,
    notes: String,
    published_at: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct CachedRelease {
    tag: String,
    draft: bool,
    prerelease: bool,
    download_url: Option<String>,
    published_at: Option<String>,
    notes: String,
    #[serde(default)]
    commit: Option<String>,
}

fn to_cached_release(release: &ReleaseSummary) -> CachedRelease {
    CachedRelease {
        tag: release.tag.clone(),
        draft: release.draft,
        prerelease: release.prerelease,
        download_url: release.download_url.clone(),
        published_at: release.published_at.clone(),
        notes: release.notes.clone(),
        commit: None,
    }
}

fn from_cached_release(cached: &CachedRelease) -> Option<ReleaseSummary> {
    Some(ReleaseSummary {
        tag: cached.tag.clone(),
        version: Version::parse(&cached.tag)?,
        draft: cached.draft,
        prerelease: cached.prerelease,
        download_url: cached.download_url.clone(),
        published_at: cached.published_at.clone(),
        notes: cached.notes.clone(),
    })
}

#[derive(Clone, Serialize, Deserialize)]
struct UpdateCheckCache {
    channel: String,
    checked_at: u64,
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    releases: Vec<CachedRelease>,
    #[serde(default)]
    error: Option<String>,
}

fn cache_is_fresh(cache: &UpdateCheckCache, channel: Channel, force: bool, now: u64) -> bool {
    !force
        && cache.channel == channel.as_str()
        && now.saturating_sub(cache.checked_at) < CHECK_INTERVAL.as_secs()
}

/// Returns `None` when the server responds with `304 Not Modified`.
fn request_json_conditional<T: serde::de::DeserializeOwned>(
    url: &str,
    etag: Option<&str>,
) -> Result<Option<(T, Option<String>)>, ureq::Error> {
    let mut request = agent()
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "strata-file-manager");
    if let Some(etag) = etag {
        request = request.header("If-None-Match", etag);
    }
    let mut response = request.call()?;
    if response.status().as_u16() == 304 {
        return Ok(None);
    }
    let etag = response
        .headers()
        .get("etag")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = response.body_mut().read_json::<T>()?;
    Ok(Some((body, etag)))
}

enum ChannelFetch {
    Fetched {
        releases: Vec<ReleaseSummary>,
        etag: Option<String>,
    },
    Unchanged,
    Failed(ureq::Error),
}

/// Builds the `UpdateCheck` for an eligible release, or `UpToDate` in the
/// defensive case where it has no installable asset -- which should be
/// unreachable, since every caller of this function has already filtered
/// through [`is_eligible`], but this avoids ever unwrapping the `Option`.
fn available_check(release: &ReleaseSummary) -> UpdateCheck {
    match &release.download_url {
        Some(download_url) => UpdateCheck::Available {
            release: release_metadata(release),
            download_url: download_url.clone(),
        },
        None => UpdateCheck::UpToDate,
    }
}

/// The pure selection step of [`check_for_updates`], split out so it can be
/// exercised against fixtures with no network involved. Delegates every
/// eligibility and ordering judgement to [`best_update`].
fn select_update(
    channel: Channel,
    installed: &Version,
    releases: &[ReleaseSummary],
) -> UpdateCheck {
    let candidate = if channel == Channel::Stable && installed.build_kind() != BuildKind::Stable {
        // Selecting Stable while running a prerelease is an explicit channel
        // transition, so offer the newest final release even when that is a
        // semantic downgrade. Keeping this in the ordinary update result lets
        // Settings present one channel-aware status/action card instead of a
        // separate, competing rollback flow.
        rollback_target(releases).filter(|release| release.version != *installed)
    } else {
        best_update(channel, installed, releases)
    };
    match candidate {
        Some(release) => available_check(release),
        None => UpdateCheck::UpToDate,
    }
}

fn select_and_resolve(
    channel: Channel,
    installed: &Version,
    releases: &[ReleaseSummary],
) -> UpdateCheck {
    let mut check = select_update(channel, installed, releases);
    if let UpdateCheck::Available { release, .. } = &mut check {
        resolve_commit(release);
    }
    check
}

fn cached_releases(releases: &[ReleaseSummary], check: &UpdateCheck) -> Vec<CachedRelease> {
    let resolved = match check {
        UpdateCheck::Available { release, .. } => Some((&release.tag, &release.commit)),
        UpdateCheck::UpToDate | UpdateCheck::Failed(_) => None,
    };
    releases
        .iter()
        .map(|release| {
            let mut cached = to_cached_release(release);
            if let Some((_, commit)) = resolved.filter(|(tag, _)| **tag == release.tag) {
                cached.commit.clone_from(commit);
            }
            cached
        })
        .collect()
}

fn select_cached_update(
    channel: Channel,
    installed: &Version,
    cached: &[CachedRelease],
) -> UpdateCheck {
    let releases: Vec<_> = cached.iter().filter_map(from_cached_release).collect();
    let mut check = select_update(channel, installed, &releases);
    if let UpdateCheck::Available { release, .. } = &mut check {
        release.commit = cached
            .iter()
            .find(|cached| cached.tag == release.tag)
            .and_then(|cached| cached.commit.clone());
    }
    check
}

fn check_from_cache(
    cache: &UpdateCheckCache,
    channel: Channel,
    installed: &Version,
) -> UpdateCheck {
    match &cache.error {
        Some(error) => UpdateCheck::Failed(error.clone()),
        None => select_cached_update(channel, installed, &cache.releases),
    }
}

fn fetch_update(channel: Channel, installed: &Version, force: bool) -> UpdateCheck {
    let path = update_check_cache_path();
    let cache = read_cache_file::<UpdateCheckCache>(&path);
    let now = unix_seconds_now();

    if let Some(cache) = cache
        .as_ref()
        .filter(|cache| cache_is_fresh(cache, channel, force, now))
    {
        return check_from_cache(cache, channel, installed);
    }

    let same_channel = cache
        .as_ref()
        .filter(|cache| cache.channel == channel.as_str());
    let etag = same_channel.and_then(|cache| cache.etag.clone());
    let outcome = match channel {
        Channel::Stable => fetch_stable(etag.as_deref()),
        Channel::Preview | Channel::Nightly => fetch_preview(etag.as_deref()),
    };

    match outcome {
        ChannelFetch::Fetched { releases, etag } => {
            let check = select_and_resolve(channel, installed, &releases);
            write_cache_file(
                &path,
                &UpdateCheckCache {
                    channel: channel.as_str().to_owned(),
                    checked_at: now,
                    etag,
                    releases: cached_releases(&releases, &check),
                    error: None,
                },
            );
            check
        }
        ChannelFetch::Unchanged => {
            let cached_releases = same_channel
                .map(|cache| cache.releases.clone())
                .unwrap_or_default();
            write_cache_file(
                &path,
                &UpdateCheckCache {
                    channel: channel.as_str().to_owned(),
                    checked_at: now,
                    etag: same_channel.and_then(|cache| cache.etag.clone()),
                    releases: cached_releases.clone(),
                    error: None,
                },
            );
            select_cached_update(channel, installed, &cached_releases)
        }
        ChannelFetch::Failed(error) => {
            let message = request_error_message(&error);
            // Keep prior data for conditional retries, but preserve the failure outcome.
            write_cache_file(
                &path,
                &UpdateCheckCache {
                    channel: channel.as_str().to_owned(),
                    checked_at: now,
                    etag: same_channel.and_then(|cache| cache.etag.clone()),
                    releases: same_channel
                        .map(|cache| cache.releases.clone())
                        .unwrap_or_default(),
                    error: Some(message.clone()),
                },
            );
            UpdateCheck::Failed(message)
        }
    }
}

fn fetch_package_update(
    installed: &Version,
    repository_version: impl FnOnce() -> Result<Version, String>,
) -> UpdateCheck {
    let available = match repository_version() {
        Ok(version) => version,
        Err(error) => return UpdateCheck::Failed(error),
    };
    if available <= *installed {
        return UpdateCheck::UpToDate;
    }

    let tag = format!("v{available}");
    match request_json::<ReleaseResponse>(&format!("{API_ROOT}/tags/{tag}")) {
        Ok(response) => match package_update_from_response(&available, &response) {
            UpdateCheck::Available {
                mut release,
                download_url,
            } => {
                resolve_commit(&mut release);
                UpdateCheck::Available {
                    release,
                    download_url,
                }
            }
            check => check,
        },
        Err(error) => UpdateCheck::Failed(request_error_message(&error)),
    }
}

fn package_update_from_response(available: &Version, response: &ReleaseResponse) -> UpdateCheck {
    // Prerelease packages (strata-rc-bin) legitimately point at prerelease tags.
    let accepts_prerelease = available.build_kind() != BuildKind::Stable;
    match to_release_summary(response).filter(|release| {
        release.version == *available && (!release.prerelease || accepts_prerelease)
    }) {
        Some(summary) => available_check(&summary),
        None => UpdateCheck::Failed(
            "package repository version has no matching stable release".to_owned(),
        ),
    }
}

fn fetch_exact_release(tag: &str) -> ReleaseNotes {
    let url = release_page_url(tag);
    let cached = read_cache_file::<CachedReleaseNotes>(&release_notes_cache_path())
        .filter(|cached| cached.tag == tag)
        .zip(Version::parse(tag));
    if let Some((cached, version)) = cached {
        return ReleaseNotes::Found(ReleaseMetadata {
            version: version.to_string(),
            url,
            note_blocks: parse_markdown(&cached.notes).blocks,
            notes: cached.notes,
            kind: version.build_kind(),
            tag: tag.to_owned(),
            published_at: cached.published_at,
            commit: None,
        });
    }
    match request_json::<ReleaseResponse>(&format!("{API_ROOT}/tags/{tag}")) {
        Ok(release) => match to_release_summary(&release) {
            Some(summary) => {
                write_cache_file(
                    &release_notes_cache_path(),
                    &CachedReleaseNotes {
                        tag: tag.to_owned(),
                        notes: summary.notes.clone(),
                        published_at: summary.published_at.clone(),
                    },
                );
                ReleaseNotes::Found(release_metadata(&summary))
            }
            None => ReleaseNotes::Unavailable { url },
        },
        Err(ureq::Error::StatusCode(404)) => ReleaseNotes::Unavailable { url },
        Err(error) => ReleaseNotes::Failed {
            message: request_error_message(&error),
            url,
        },
    }
}

fn request_error_message(error: &ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(403 | 429) => "GitHub API rate limit reached".to_owned(),
        ureq::Error::StatusCode(code) => format!("GitHub API returned HTTP {code}"),
        _ => format!("Network request failed: {error}"),
    }
}

fn release_page_url(tag: &str) -> String {
    format!("{RELEASES_URL}/tag/{tag}")
}

/// Checks the applicable release source off the GTK thread.
/// `force` bypasses the cache interval for in-place update checks.
pub fn check_for_updates(
    channel: Channel,
    installed: Version,
    update_method: UpdateMethod,
    force: bool,
) -> Receiver<UpdateCheck> {
    let (sender, receiver) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("strata-update-check".into())
        .spawn(move || {
            let result = match update_method {
                UpdateMethod::InPlace => fetch_update(channel, &installed, force),
                UpdateMethod::Aur => fetch_package_update(&installed, aur_repository_version),
                UpdateMethod::Omarchy => {
                    fetch_package_update(&installed, omarchy_repository_version)
                }
                UpdateMethod::Pacman => {
                    fetch_package_update(&installed, package_repository_version)
                }
            };
            let _sent = sender.send(result);
        });
    drop(spawned);
    receiver
}

/// Fetches the release whose tag exactly matches `tag`, e.g.
/// [`crate::build_info::RELEASE_TAG`].
pub fn fetch_release_notes(tag: &'static str) -> Receiver<ReleaseNotes> {
    let (sender, receiver) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("strata-release-notes".into())
        .spawn(move || {
            let _sent = sender.send(fetch_exact_release(tag));
        });
    drop(spawned);
    receiver
}

#[cfg(test)]
mod tests;

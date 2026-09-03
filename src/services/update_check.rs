// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    sync::mpsc::{self, Receiver},
    time::Duration,
};

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::Deserialize;

use super::release_channel::{
    BuildKind, Channel, ReleaseSummary, Version, best_update, rollback_target,
};

const API_ROOT: &str = "https://api.github.com/repos/lgse/strata/releases";
const COMMITS_ROOT: &str = "https://api.github.com/repos/lgse/strata/commits";
const RELEASES_URL: &str = "https://github.com/lgse/strata/releases";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How many of the most recent releases (final and prerelease) the preview
/// feed enumerates. Only ever used to find a release *newer* than the
/// installed one, so a bounded page is safe: an update this page cannot
/// reach is older than one it can.
const PREVIEW_PAGE_SIZE: u32 = 30;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReleaseNoteBlock {
    Heading {
        level: u8,
        markup: String,
    },
    Paragraph(String),
    ListItem {
        marker: String,
        depth: usize,
        markup: String,
    },
    Code(String),
    Rule,
}

/// Everything the update/rollback dialogs need to identify and describe a
/// release before installing it: what build it is, its exact tag and
/// display version, where it was published, and its rendered notes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseMetadata {
    /// The full, prerelease-bearing version for display, e.g. `0.5.0-rc.1`.
    pub version: String,
    pub url: String,
    pub notes: String,
    pub note_blocks: Vec<ReleaseNoteBlock>,
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
        note_blocks: parse_markdown(&release.notes),
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
///
/// `Ok(None)` covers both "no releases have been published yet" and "the
/// tag GitHub returned failed to parse"; neither is a network failure.
fn fetch_stable() -> Result<Option<ReleaseSummary>, ureq::Error> {
    match request_json::<ReleaseResponse>(&format!("{API_ROOT}/latest")) {
        Ok(release) => Ok(to_release_summary(&release)),
        Err(ureq::Error::StatusCode(404)) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Fetches the most recent releases, final and prerelease alike, for the
/// preview channel.
///
/// Kept as its own function rather than reused for [`Channel::Stable`]: a
/// Stable user's code path must call [`fetch_stable`] instead, never this,
/// so prerelease metadata never reaches that path at all.
fn fetch_preview() -> Result<Vec<ReleaseSummary>, ureq::Error> {
    let releases =
        request_json::<Vec<ReleaseResponse>>(&format!("{API_ROOT}?per_page={PREVIEW_PAGE_SIZE}"))?;
    Ok(releases.iter().filter_map(to_release_summary).collect())
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

fn fetch_update(channel: Channel, installed: &Version) -> UpdateCheck {
    let releases = match channel {
        Channel::Stable => fetch_stable().map(|release| release.into_iter().collect::<Vec<_>>()),
        Channel::Preview | Channel::Nightly => fetch_preview(),
    };
    match releases {
        Ok(releases) => {
            let mut check = select_update(channel, installed, &releases);
            if let UpdateCheck::Available { release, .. } = &mut check {
                resolve_commit(release);
            }
            check
        }
        Err(error) => UpdateCheck::Failed(request_error_message(&error)),
    }
}

fn fetch_exact_release(tag: &str) -> ReleaseNotes {
    let url = release_page_url(tag);
    match request_json::<ReleaseResponse>(&format!("{API_ROOT}/tags/{tag}")) {
        Ok(release) => match to_release_summary(&release) {
            Some(summary) => ReleaseNotes::Found(release_metadata(&summary)),
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

#[derive(Debug)]
enum ActiveBlock {
    Heading {
        level: u8,
        markup: String,
    },
    Paragraph(String),
    ListItem {
        marker: String,
        depth: usize,
        markup: String,
    },
    Code(String),
}

impl ActiveBlock {
    fn markup_mut(&mut self) -> &mut String {
        match self {
            Self::Heading { markup, .. }
            | Self::Paragraph(markup)
            | Self::ListItem { markup, .. }
            | Self::Code(markup) => markup,
        }
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn finish_block(active: &mut Option<ActiveBlock>, blocks: &mut Vec<ReleaseNoteBlock>) {
    let Some(active) = active.take() else {
        return;
    };
    let block = match active {
        ActiveBlock::Heading { level, markup } => ReleaseNoteBlock::Heading { level, markup },
        ActiveBlock::Paragraph(markup) => ReleaseNoteBlock::Paragraph(markup),
        ActiveBlock::ListItem {
            marker,
            depth,
            markup,
        } => ReleaseNoteBlock::ListItem {
            marker,
            depth,
            markup,
        },
        ActiveBlock::Code(markup) => ReleaseNoteBlock::Code(markup),
    };
    blocks.push(block);
}

fn append_markup(active: &mut Option<ActiveBlock>, markup: &str) {
    let block = active.get_or_insert_with(|| ActiveBlock::Paragraph(String::new()));
    block.markup_mut().push_str(markup);
}

fn append_escaped(active: &mut Option<ActiveBlock>, text: &str) {
    append_markup(active, &glib::markup_escape_text(text));
}

/// Parses the supported GitHub Markdown subset into safe, balanced blocks while
/// release metadata is processed on a worker thread.
fn parse_markdown(markdown: &str) -> Vec<ReleaseNoteBlock> {
    let mut blocks = Vec::new();
    let mut active = None;
    let mut links = Vec::new();
    let mut lists = Vec::<Option<u64>>::new();
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;

    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                finish_block(&mut active, &mut blocks);
                active = Some(ActiveBlock::Heading {
                    level: heading_level(level),
                    markup: String::new(),
                });
            }
            Event::End(TagEnd::Heading(_)) => finish_block(&mut active, &mut blocks),
            Event::Start(Tag::Paragraph) => {
                if active.is_none() {
                    active = Some(ActiveBlock::Paragraph(String::new()));
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if matches!(active, Some(ActiveBlock::Paragraph(_))) {
                    finish_block(&mut active, &mut blocks);
                }
            }
            Event::Start(Tag::List(start)) => {
                if matches!(active, Some(ActiveBlock::ListItem { .. })) {
                    finish_block(&mut active, &mut blocks);
                }
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                if matches!(active, Some(ActiveBlock::ListItem { .. })) {
                    finish_block(&mut active, &mut blocks);
                }
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                finish_block(&mut active, &mut blocks);
                let marker = match lists.last_mut() {
                    Some(Some(next)) => {
                        let marker = format!("{next}.");
                        *next = next.saturating_add(1);
                        marker
                    }
                    _ => "•".to_owned(),
                };
                active = Some(ActiveBlock::ListItem {
                    marker,
                    depth: lists.len().saturating_sub(1),
                    markup: String::new(),
                });
            }
            Event::End(TagEnd::Item) => {
                if matches!(active, Some(ActiveBlock::ListItem { .. })) {
                    finish_block(&mut active, &mut blocks);
                }
            }
            Event::Start(Tag::Emphasis) => append_markup(&mut active, "<i>"),
            Event::End(TagEnd::Emphasis) => append_markup(&mut active, "</i>"),
            Event::Start(Tag::Strong) => append_markup(&mut active, "<b>"),
            Event::End(TagEnd::Strong) => append_markup(&mut active, "</b>"),
            Event::Start(Tag::Strikethrough) => append_markup(&mut active, "<s>"),
            Event::End(TagEnd::Strikethrough) => append_markup(&mut active, "</s>"),
            Event::Start(Tag::Link { dest_url, .. }) => {
                let destination = dest_url.as_ref();
                let external =
                    destination.starts_with("https://") || destination.starts_with("http://");
                links.push(external);
                if external {
                    append_markup(&mut active, "<a href=\"");
                    append_escaped(&mut active, destination);
                    append_markup(&mut active, "\">");
                } else {
                    append_markup(&mut active, "<u>");
                }
            }
            Event::End(TagEnd::Link) => append_markup(
                &mut active,
                if links.pop().unwrap_or(false) {
                    "</a>"
                } else {
                    "</u>"
                },
            ),
            Event::Start(Tag::Image { .. }) => append_markup(&mut active, "[Image: "),
            Event::End(TagEnd::Image) => append_markup(&mut active, "]"),
            Event::Start(Tag::CodeBlock(_)) => {
                if matches!(active, Some(ActiveBlock::ListItem { .. })) {
                    append_markup(&mut active, "<tt>");
                } else {
                    finish_block(&mut active, &mut blocks);
                    active = Some(ActiveBlock::Code(String::new()));
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if matches!(active, Some(ActiveBlock::Code(_))) {
                    finish_block(&mut active, &mut blocks);
                } else {
                    append_markup(&mut active, "</tt>");
                }
            }
            Event::Code(text) => {
                append_markup(&mut active, "<tt>");
                append_escaped(&mut active, &text);
                append_markup(&mut active, "</tt>");
            }
            Event::Text(text) => append_escaped(&mut active, &text),
            Event::SoftBreak | Event::HardBreak => append_markup(&mut active, "\n"),
            Event::Rule => {
                finish_block(&mut active, &mut blocks);
                blocks.push(ReleaseNoteBlock::Rule);
            }
            Event::Html(text) | Event::InlineHtml(text) => append_escaped(&mut active, &text),
            Event::TaskListMarker(checked) => {
                append_markup(&mut active, if checked { "☑ " } else { "☐ " });
            }
            _ => {}
        }
    }
    finish_block(&mut active, &mut blocks);
    blocks
}

/// Queries the release feed for `channel` off the GTK thread and reports the
/// outcome once. See [`fetch_stable`] and [`fetch_preview`] for why the two
/// feeds are kept as separate functions.
pub fn check_for_updates(channel: Channel, installed: Version) -> Receiver<UpdateCheck> {
    let (sender, receiver) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("strata-update-check".into())
        .spawn(move || {
            let _sent = sender.send(fetch_update(channel, &installed));
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

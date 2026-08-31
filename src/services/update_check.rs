// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    sync::mpsc::{self, Receiver},
    time::Duration,
};

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::Deserialize;

const API_ROOT: &str = "https://api.github.com/repos/lgse/strata/releases";
const RELEASES_URL: &str = "https://github.com/lgse/strata/releases";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseMetadata {
    pub version: String,
    pub url: String,
    pub notes: String,
    pub note_blocks: Vec<ReleaseNoteBlock>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateCheck {
    UpToDate,
    Available {
        release: ReleaseMetadata,
        download_url: Option<String>,
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
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
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

fn request_release(url: &str) -> Result<ReleaseResponse, ureq::Error> {
    agent()
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "strata-file-manager")
        .call()
        .and_then(|mut response| response.body_mut().read_json::<ReleaseResponse>())
}

fn metadata(release: &ReleaseResponse) -> ReleaseMetadata {
    let notes = release.body.clone().unwrap_or_default();
    ReleaseMetadata {
        version: release.tag_name.trim_start_matches('v').to_owned(),
        url: release.html_url.clone(),
        note_blocks: parse_markdown(&notes),
        notes,
    }
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

/// Queries the latest GitHub release off the GTK thread and reports the outcome once.
pub fn check_for_updates(current_version: &'static str) -> Receiver<UpdateCheck> {
    let (sender, receiver) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("strata-update-check".into())
        .spawn(move || {
            let _sent = sender.send(fetch_latest_release(current_version));
        });
    drop(spawned);
    receiver
}

/// Fetches the release whose tag exactly matches the installed package version.
pub fn fetch_release_notes(version: &'static str) -> Receiver<ReleaseNotes> {
    let (sender, receiver) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("strata-release-notes".into())
        .spawn(move || {
            let _sent = sender.send(fetch_exact_release(version));
        });
    drop(spawned);
    receiver
}

fn fetch_latest_release(current_version: &str) -> UpdateCheck {
    match request_release(&format!("{API_ROOT}/latest")) {
        Ok(release) => {
            let release_metadata = metadata(&release);
            if is_newer(&release_metadata.version, current_version) {
                let archive_name = archive_name(&release_metadata.version);
                let download_url = release
                    .assets
                    .iter()
                    .find(|asset| asset.name == archive_name)
                    .map(|asset| asset.browser_download_url.clone());
                UpdateCheck::Available {
                    release: release_metadata,
                    download_url,
                }
            } else {
                UpdateCheck::UpToDate
            }
        }
        Err(error) => UpdateCheck::Failed(request_error_message(&error)),
    }
}

fn fetch_exact_release(version: &str) -> ReleaseNotes {
    let url = release_page_url(version);
    match request_release(&format!("{API_ROOT}/tags/v{version}")) {
        Ok(release) => ReleaseNotes::Found(metadata(&release)),
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

fn release_page_url(version: &str) -> String {
    format!("{RELEASES_URL}/tag/v{version}")
}

fn is_newer(candidate: &str, current: &str) -> bool {
    parse_version(candidate) > parse_version(current)
}

fn parse_version(value: &str) -> (u64, u64, u64) {
    let mut parts = value.split('.').map(|part| part.parse().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

#[cfg(test)]
mod tests;

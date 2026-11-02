// SPDX-License-Identifier: MIT

//! Release-note cards: the current build's notes on the Updates page and the
//! notes of an offered release, rendered from parsed Markdown blocks.

use std::{sync::mpsc::TryRecvError, time::Duration};

use gtk::{glib, prelude::*};

use crate::services::{self, BuildKind, DocumentBlock, ReleaseMetadata, ReleaseNotes};

fn release_notes_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class("release-notes-content");
    label.set_xalign(0.0);
    label.set_yalign(0.0);
    label.set_hexpand(true);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_selectable(true);
    label.set_use_markup(true);
    label
}

fn clear_release_notes(notes: &gtk::Box) {
    while let Some(child) = notes.first_child() {
        notes.remove(&child);
    }
}

pub(super) fn set_release_notes_message(notes: &gtk::Box, message: &str) {
    clear_release_notes(notes);
    let label = release_notes_label();
    label.set_text(message);
    notes.append(&label);
}

pub(super) fn set_release_note_blocks(notes: &gtk::Box, blocks: &[DocumentBlock]) {
    clear_release_notes(notes);
    for block in blocks {
        if let Some(widget) = release_note_block(block) {
            notes.append(&widget);
        }
    }
}

fn release_note_block(block: &DocumentBlock) -> Option<gtk::Widget> {
    let widget: gtk::Widget = match block {
        DocumentBlock::Heading { level, markup } => heading_label(*level, markup).upcast(),
        DocumentBlock::Paragraph(markup) => paragraph_label(markup).upcast(),
        DocumentBlock::ListItem {
            marker,
            depth,
            markup,
        } => list_item_row(marker, *depth, markup).upcast(),
        DocumentBlock::ListChild {
            depth,
            kind,
            markup,
        } => list_child_label(*depth, kind, markup).upcast(),
        DocumentBlock::Code { markup, .. } => code_label(markup).upcast(),
        DocumentBlock::Rule => rule_separator().upcast(),
        DocumentBlock::ListRule { depth } => {
            let separator = rule_separator();
            separator.set_margin_start(nested_indent(*depth));
            separator.upcast()
        }
        DocumentBlock::Quote(_)
        | DocumentBlock::TableRow { .. }
        | DocumentBlock::ListTableRow { .. }
        | DocumentBlock::ContainerBoundary
        | DocumentBlock::Image { .. } => return None,
    };
    Some(widget)
}

fn heading_label(level: u8, markup: &str) -> gtk::Label {
    let label = release_notes_label();
    label.add_css_class("release-notes-heading");
    label.add_css_class(&format!("level-{level}"));
    label.set_markup(markup);
    label
}

fn paragraph_label(markup: &str) -> gtk::Label {
    let label = release_notes_label();
    label.set_markup(markup);
    label
}

fn code_label(markup: &str) -> gtk::Label {
    let label = release_notes_label();
    label.add_css_class("release-notes-code");
    label.set_markup(&format!("<tt>{markup}</tt>"));
    label
}

fn rule_separator() -> gtk::Separator {
    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    separator.add_css_class("release-notes-rule");
    separator
}

fn list_item_row(marker: &str, depth: usize, markup: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_valign(gtk::Align::Start);
    row.set_margin_start(i32::try_from(depth.saturating_mul(18)).unwrap_or(i32::MAX));
    let bullet = gtk::Label::new(Some(marker));
    bullet.add_css_class("release-notes-bullet");
    bullet.set_valign(gtk::Align::Start);
    let copy = release_notes_label();
    copy.set_markup(markup);
    row.append(&bullet);
    row.append(&copy);
    row
}

fn list_child_label(
    depth: usize,
    kind: &services::DocumentListChildKind,
    markup: &str,
) -> gtk::Label {
    let copy = release_notes_label();
    copy.set_margin_start(nested_indent(depth));
    match kind {
        services::DocumentListChildKind::Heading(level) => {
            copy.add_css_class("release-notes-heading");
            copy.add_css_class(&format!("level-{level}"));
            copy.set_markup(markup);
        }
        services::DocumentListChildKind::Code(_) => {
            copy.add_css_class("release-notes-code");
            copy.set_markup(&format!("<tt>{markup}</tt>"));
        }
        services::DocumentListChildKind::Paragraph | services::DocumentListChildKind::Quote => {
            copy.set_markup(markup);
        }
    }
    copy
}

/// The indent of content nested one level below a list item at `depth`.
fn nested_indent(depth: usize) -> i32 {
    i32::try_from(depth.saturating_add(1).saturating_mul(18)).unwrap_or(i32::MAX)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ReleaseNotesKind {
    Current,
    Available,
}

#[derive(Clone)]
pub(super) struct ReleaseNotesCard {
    pub(super) kind: ReleaseNotesKind,
    pub(super) container: gtk::Box,
    pub(super) title: gtk::Label,
    pub(super) summary: gtk::Label,
    pub(super) badge: gtk::Label,
    pub(super) notes: gtk::Box,
    pub(super) fallback: gtk::LinkButton,
}

pub(super) fn release_notes_card(
    kind: ReleaseNotesKind,
    title: &str,
    initial: &str,
) -> ReleaseNotesCard {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 8);
    container.add_css_class("release-notes-card");
    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("release-notes-title");
    title_label.set_xalign(0.0);
    title_label.set_wrap(true);
    title_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    let badge = gtk::Label::new(None);
    badge.add_css_class("prerelease-badge");
    badge.set_xalign(0.0);
    badge.set_halign(gtk::Align::Start);
    badge.set_visible(false);
    let notes = gtk::Box::new(gtk::Orientation::Vertical, 6);
    set_release_notes_message(&notes, initial);
    let fallback = gtk::LinkButton::with_label(
        "https://github.com/lgse/strata/releases",
        &crate::i18n::tr("View on GitHub"),
    );
    fallback.set_has_tooltip(false);
    fallback.add_css_class("release-notes-fallback");
    fallback.set_halign(gtk::Align::Start);
    fallback.set_visible(false);
    let summary = gtk::Label::new(Some(initial));
    summary.set_xalign(0.0);
    summary.set_wrap(true);
    summary.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    summary.add_css_class("settings-option-description");
    container.append(&title_label);
    container.append(&summary);
    container.append(&badge);
    container.append(&notes);
    container.append(&fallback);
    ReleaseNotesCard {
        kind,
        container,
        title: title_label,
        summary,
        badge,
        notes,
        fallback,
    }
}

/// Shows `release`'s notes in `card`, including a visible prerelease badge
/// above the notes whenever `release.kind` is not [`BuildKind::Stable`] --
/// release notes and update surfaces must visibly label prerelease software.
pub(super) fn show_release_notes(card: &ReleaseNotesCard, release: &ReleaseMetadata) {
    card.container.set_visible(true);
    card.title.set_text(&match card.kind {
        ReleaseNotesKind::Current => {
            rust_i18n::t!("What's new in v%{value1}", value1 = release.version)
        }
        ReleaseNotesKind::Available => {
            rust_i18n::t!("Available release · v%{version}", version = release.version)
        }
    });
    let changes = release
        .note_blocks
        .iter()
        .filter(|block| matches!(block, DocumentBlock::ListItem { .. }))
        .count();
    let published = release.published_at.as_deref().map(release_date);
    card.summary.set_text(&match (changes, published) {
        (0, None) => crate::i18n::tr("Release notes"),
        (0, Some(date)) => rust_i18n::t!("Published %{date}", date = date).into_owned(),
        (count, None) => crate::i18n::count("changes", count),
        (count, Some(date)) => rust_i18n::t!(
            "%{changes} · published %{date}",
            changes = crate::i18n::count("changes", count),
            date = date
        )
        .into_owned(),
    });
    if release.kind == BuildKind::Stable {
        card.badge.set_visible(false);
    } else {
        card.badge.set_text(&release.kind.localized_label());
        card.badge.set_visible(true);
    }
    if release.note_blocks.is_empty() {
        set_release_notes_message(
            &card.notes,
            &crate::i18n::tr("No release notes were provided for this release."),
        );
    } else {
        set_release_note_blocks(&card.notes, &release.note_blocks);
    }
    card.fallback.set_uri(&release.url);
    card.fallback.set_visible(true);
}

pub(super) fn load_current_release_notes(card: &ReleaseNotesCard) {
    let receiver = services::fetch_release_notes(crate::build_info::RELEASE_TAG);
    let card = card.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        match receiver.try_recv() {
            Ok(ReleaseNotes::Found(release)) => {
                show_release_notes(&card, &release);
                glib::ControlFlow::Break
            }
            Ok(ReleaseNotes::Unavailable { url }) => {
                card.summary
                    .set_text(&crate::i18n::tr("Release notes unavailable for this build"));
                set_release_notes_message(
                    &card.notes,
                    &crate::i18n::tr(
                        "Release notes are unavailable because this version’s tag was not found.",
                    ),
                );
                card.fallback.set_uri(&url);
                card.fallback.set_visible(true);
                glib::ControlFlow::Break
            }
            Ok(ReleaseNotes::Failed { message, url }) => {
                card.summary
                    .set_text(&crate::i18n::tr("Couldn’t load release notes"));
                set_release_notes_message(
                    &card.notes,
                    &rust_i18n::t!("Couldn’t load release notes: %{message}", message = message),
                );
                card.fallback.set_uri(&url);
                card.fallback.set_visible(true);
                glib::ControlFlow::Break
            }
            Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => {
                card.summary
                    .set_text(&crate::i18n::tr("Couldn’t load release notes"));
                set_release_notes_message(
                    &card.notes,
                    &crate::i18n::tr(
                        "Couldn’t load release notes because the request ended unexpectedly.",
                    ),
                );
                glib::ControlFlow::Break
            }
        }
    });
}

/// Formats a GitHub `published_at` timestamp as a date in the app language,
/// keeping the raw value when it cannot be parsed.
pub(super) fn release_date(published_at: &str) -> String {
    let Ok(date) = glib::DateTime::from_iso8601(published_at, None) else {
        return published_at
            .split('T')
            .next()
            .unwrap_or(published_at)
            .to_owned();
    };
    let date = date.to_local().unwrap_or(date);
    crate::util::localized_date(&date, &crate::i18n::tr("dates.older"))
}

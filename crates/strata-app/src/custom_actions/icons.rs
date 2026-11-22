// SPDX-License-Identifier: MIT

//! Custom-action manifest icon slug mapping (`ACTION_ICON_CHOICES` in `src/ui/actions.rs`).
//!
//! Each bundled slug maps to both its exact Lucide asset key (`look.icon_named`) and a semantic
//! [`IconName`] fallback for [`strata_components::dialog::StrataDialog::icon`] and placeholder
//! icon sets.

use rune::Icon;
use strata_kit::{icons::IconName, look::Look};

const CHOICES: &[(&str, &str, IconName)] = &[
    ("play", "strata-play", IconName::Play),
    ("terminal", "strata-terminal", IconName::Terminal),
    ("image", "strata-image", IconName::Pictures),
    ("file-code", "strata-file-code", IconName::FileCode),
    ("code-xml", "strata-code-xml", IconName::Source),
    ("file-archive", "strata-file-archive", IconName::Archive),
    ("file-text", "strata-file-text", IconName::FileText),
    ("copy", "strata-copy", IconName::Copy),
    ("scissors", "strata-scissors", IconName::Cut),
    ("pencil", "strata-pencil", IconName::Rename),
    ("refresh", "strata-refresh", IconName::Refresh),
    ("download", "strata-download", IconName::Downloads),
    ("scale", "strata-scale", IconName::Play),
    ("bug", "strata-bug", IconName::Warning),
    ("key", "strata-key", IconName::Lock),
    ("lock", "strata-lock", IconName::Lock),
    ("printer", "strata-printer", IconName::Play),
    ("search", "strata-search", IconName::Search),
    ("folder", "strata-folder", IconName::Folder),
    ("check", "strata-check", IconName::Check),
    ("triangle-alert", "strata-triangle-alert", IconName::Warning),
    ("globe", "strata-globe", IconName::Network),
    ("plus", "strata-plus", IconName::Plus),
    ("monitor", "strata-monitor", IconName::Desktop),
];

pub(crate) const ACTION_ICON_SLUGS: &[&str] = &[
    "play",
    "terminal",
    "image",
    "file-code",
    "code-xml",
    "file-archive",
    "file-text",
    "copy",
    "scissors",
    "pencil",
    "refresh",
    "download",
    "scale",
    "bug",
    "key",
    "lock",
    "printer",
    "search",
    "folder",
    "check",
    "triangle-alert",
    "globe",
    "plus",
    "monitor",
];

pub(crate) fn is_known_action_icon(slug: &str) -> bool {
    CHOICES.iter().any(|(known, _, _)| *known == slug)
}

pub(crate) fn action_icon(slug: Option<&str>) -> IconName {
    slug.and_then(|candidate| {
        CHOICES
            .iter()
            .find(|(known, _, _)| *known == candidate)
            .map(|(_, _, icon)| *icon)
    })
    .unwrap_or(IconName::Play)
}

pub(crate) fn action_icon_glyph(look: &Look, slug: Option<&str>) -> &'static Icon {
    let (asset, fallback) = slug
        .and_then(|candidate| {
            CHOICES
                .iter()
                .find(|(known, _, _)| *known == candidate)
                .map(|(_, asset, fallback)| (*asset, *fallback))
        })
        .unwrap_or(("strata-play", IconName::Play));
    look.icon_named(asset)
        .unwrap_or_else(|| look.icon(fallback))
}

// SPDX-License-Identifier: MIT

//! Custom-action manifest icon slug mapping (`ACTION_ICON_CHOICES` in `src/ui/actions.rs`).

use strata_kit::icons::IconName;

const CHOICES: &[(&str, IconName)] = &[
    ("play", IconName::Play),
    ("terminal", IconName::Terminal),
    ("image", IconName::Pictures),
    ("file-code", IconName::FileCode),
    ("code-xml", IconName::Source),
    ("file-archive", IconName::Archive),
    ("file-text", IconName::FileText),
    ("copy", IconName::Copy),
    ("scissors", IconName::Cut),
    ("pencil", IconName::Rename),
    ("refresh", IconName::Refresh),
    ("download", IconName::Downloads),
    ("lock", IconName::Lock),
    ("key", IconName::Lock),
    ("search", IconName::Search),
    ("folder", IconName::Folder),
    ("check", IconName::Check),
    ("triangle-alert", IconName::Warning),
    ("bug", IconName::Warning),
    ("globe", IconName::Network),
    ("plus", IconName::Plus),
    ("monitor", IconName::Desktop),
];

pub(crate) fn action_icon(slug: Option<&str>) -> IconName {
    slug.and_then(|candidate| {
        CHOICES
            .iter()
            .find(|(known, _)| *known == candidate)
            .map(|(_, icon)| *icon)
    })
    .unwrap_or(IconName::Play)
}

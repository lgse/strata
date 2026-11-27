// SPDX-License-Identifier: MIT

use crate::tokens::Token;

#[cfg(doc)]
use crate::look::Look;

pub mod collection;
pub mod columns;
pub mod icons;
pub mod list;
pub mod media;
pub mod scroll;
pub mod sidebar;
pub mod tabs;

#[cfg(test)]
mod tests;

/// Every slot's tokens, held in `Look::slots`.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Slots {
    pub collection: collection::Collection,
    pub columns: columns::Columns,
    pub icons: icons::Icons,
    pub list: list::List,
    pub media: media::Media,
    pub scroll: scroll::Scroll,
    pub sidebar: sidebar::Sidebar,
    pub tabs: tabs::Tabs,
}

pub const TOKENS: &[(&str, &[Token])] = &[
    ("collection", collection::TOKENS),
    ("columns", columns::TOKENS),
    ("icons", icons::TOKENS),
    ("list", list::TOKENS),
    ("media", media::TOKENS),
    ("scroll", scroll::TOKENS),
    ("sidebar", sidebar::TOKENS),
    ("tabs", tabs::TOKENS),
];

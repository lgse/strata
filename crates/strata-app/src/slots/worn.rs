// SPDX-License-Identifier: MIT

//! Saved item colours and icons (`Preferences::folder_colors` and `Preferences::custom_icons`),
//! keyed by [`EntryId`] so [`crate::listing::Column`] lends [`EntryGlyph`] from `&self`.

use std::{collections::HashMap, path::Path, rc::Rc};

use rune::Color;
use strata_components::{
    contract::{EntryGlyph, EntryId},
    controls::color_picker::Srgba,
    sidebar::RowIcon,
};
use strata_core::model::{FolderColorValue, Location};
use strata_desktop::settings::{Preferences, custom_emoji, is_icon_choice};

use super::Ids;

/// A colour of the Customize bar as `rune` draws it.
pub(crate) fn color_of(value: &FolderColorValue) -> Option<Color> {
    Srgba::parse_hex(value.hex()).map(Srgba::to_color)
}

/// An icon choice saved in `Preferences::custom_icons`: a bundled `strata-*` name or a safe
/// emoji.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SavedGlyph {
    Icon(Box<str>),
    Emoji(Box<str>),
}

impl SavedGlyph {
    /// Parses a saved `custom_icons` value when `is_icon_choice` accepts it.
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        if let Some(emoji) = custom_emoji(raw) {
            return Some(Self::Emoji(emoji.into()));
        }
        is_icon_choice(raw).then(|| Self::Icon(raw.into()))
    }

    pub(crate) fn as_entry(&self) -> EntryGlyph<'_> {
        match self {
            Self::Icon(name) => EntryGlyph::Icon(name),
            Self::Emoji(emoji) => EntryGlyph::Emoji(emoji),
        }
    }

    pub(crate) fn to_row_icon(&self) -> RowIcon {
        match self {
            Self::Icon(name) => RowIcon::Catalog(name.to_string()),
            Self::Emoji(emoji) => RowIcon::Emoji(emoji.to_string()),
        }
    }
}

/// What `settings` saves for `path`: its colour and its icon or emoji, if any.
pub(crate) fn path_customization(
    settings: &Preferences,
    path: &Path,
) -> (Option<Color>, Option<SavedGlyph>) {
    let color = settings.item_color(path).as_ref().and_then(color_of);
    let glyph = settings.item_icon(path).and_then(SavedGlyph::parse);
    (color, glyph)
}

#[derive(Default)]
struct WornItem {
    color: Option<Color>,
    glyph: Option<SavedGlyph>,
}

/// Every item customization saved in the window's settings, indexed by [`EntryId`].
#[derive(Default)]
pub(crate) struct Worn {
    version: u64,
    by_id: HashMap<EntryId, WornItem>,
}

impl Worn {
    pub(crate) fn empty() -> Rc<Self> {
        Rc::new(Self::default())
    }

    pub(crate) fn from_preferences(settings: &Preferences, ids: &Ids, version: u64) -> Rc<Self> {
        let mut by_id: HashMap<EntryId, WornItem> = HashMap::new();
        for (path, raw) in &settings.folder_colors {
            let Some(color) = FolderColorValue::parse(raw).and_then(|value| color_of(&value))
            else {
                continue;
            };
            let id = ids.of(&Location::local(Path::new(path)));
            by_id.entry(id).or_default().color = Some(color);
        }
        for (path, raw) in &settings.custom_icons {
            let Some(glyph) = SavedGlyph::parse(raw) else {
                continue;
            };
            let id = ids.of(&Location::local(Path::new(path)));
            by_id.entry(id).or_default().glyph = Some(glyph);
        }
        Rc::new(Self { version, by_id })
    }

    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    pub(crate) fn get(&self, id: EntryId) -> Option<(Option<Color>, Option<EntryGlyph<'_>>)> {
        let item = self.by_id.get(&id)?;
        Some((item.color, item.glyph.as_ref().map(SavedGlyph::as_entry)))
    }
}

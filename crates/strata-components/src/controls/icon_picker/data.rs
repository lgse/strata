//! What a picker offers, as data: choices, their names and keywords, and their categories.

use std::borrow::Cow;
use std::collections::HashMap;

use strata_kit::icons::IconSet;

use super::matching::{self, Words};

/// What a choice is: a bundled icon by its catalog name, or an emoji by its text.
///
/// The host maps it onto what it saves: Strata writes the icon's name (`strata-pin`) or
/// `emoji:` and the emoji. The picker doesn't check lengths or characters; that is Strata's.
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Choice {
    /// An icon, by its K3 catalog name (`Icon::name`, `IconSet::named`).
    Icon(Cow<'static, str>),
    /// An emoji, by its text: a fully-qualified sequence.
    Emoji(Cow<'static, str>),
}

impl Choice {
    /// The icon named `name` in K3's catalogs.
    pub fn icon(name: impl Into<Cow<'static, str>>) -> Self {
        Choice::Icon(name.into())
    }

    /// The emoji `text`.
    pub fn emoji(text: impl Into<Cow<'static, str>>) -> Self {
        Choice::Emoji(text.into())
    }

    /// The icon's name or the emoji's text.
    pub fn text(&self) -> &str {
        match self {
            Choice::Icon(n) | Choice::Emoji(n) => n,
        }
    }

    /// Whether it is an emoji.
    pub fn is_emoji(&self) -> bool {
        matches!(self, Choice::Emoji(_))
    }

    /// The text a cell stores as its entry's icon: the icon's name, or `emoji:` and the emoji.
    pub(super) fn cell_key(&self) -> String {
        match self {
            Choice::Icon(n) => n.to_string(),
            Choice::Emoji(e) => format!("{EMOJI_PREFIX}{e}"),
        }
    }
}

/// How a cell's entry icon marks an emoji.
pub(super) const EMOJI_PREFIX: &str = "emoji:";

/// One choice the picker offers.
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq)]
pub struct PickerItem {
    /// What choosing it sends.
    pub choice: Choice,
    /// What it is called: its cell's accessible name and tooltip, and what a search matches
    /// ("grinning face", "Documents").
    pub name: Cow<'static, str>,
    /// Other words a search matches ("happy", "smile"); may be empty.
    pub keywords: Vec<Cow<'static, str>>,
    /// Its category, an index into the source's categories.
    pub category: usize,
}

impl PickerItem {
    /// `choice`, called `name`, in `category`, with no keywords.
    pub fn new(choice: Choice, name: impl Into<Cow<'static, str>>, category: usize) -> Self {
        Self {
            choice,
            name: name.into(),
            keywords: Vec::new(),
            category,
        }
    }

    /// The same item with `keywords`.
    pub fn keywords(
        mut self,
        keywords: impl IntoIterator<Item = impl Into<Cow<'static, str>>>,
    ) -> Self {
        self.keywords = keywords.into_iter().map(Into::into).collect();
        self
    }
}

/// A category: a tab of the picker.
///
/// Used by: hosts.
#[derive(Clone, Debug, PartialEq)]
pub struct PickerCategory {
    /// A stable id, for the tab's key (`smileys`).
    pub key: Cow<'static, str>,
    /// What the tab is called ("Smileys & Emotion").
    pub name: Cow<'static, str>,
    /// What the tab draws: an icon or an emoji.
    pub glyph: Choice,
}

impl PickerCategory {
    /// A category keyed `key`, called `name`, drawn as `glyph`.
    pub fn new(
        key: impl Into<Cow<'static, str>>,
        name: impl Into<Cow<'static, str>>,
        glyph: Choice,
    ) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            glyph,
        }
    }
}

/// Everything a picker offers: its categories and their items, in order, with the words a
/// search matches worked out once. Built once by the host and shared (`Rc`).
///
/// Used by: hosts.
#[derive(Debug, Default)]
pub struct PickerSource {
    categories: Vec<PickerCategory>,
    items: Vec<PickerItem>,
    /// Each item's search words, by item.
    words: Vec<Words>,
    /// Each item's cell key (its entry icon).
    keys: Vec<String>,
    /// Each category's items, in order.
    members: Vec<Vec<u32>>,
    /// An item by its choice.
    by_choice: HashMap<Choice, u32>,
}

impl PickerSource {
    /// `items` in `categories`. An item whose category is out of range is dropped, and so is a
    /// second item with the same choice.
    pub fn new(categories: Vec<PickerCategory>, items: Vec<PickerItem>) -> Self {
        let mut source = PickerSource {
            members: vec![Vec::new(); categories.len()],
            categories,
            ..PickerSource::default()
        };
        for item in items {
            source.push(item);
        }
        source
    }

    fn push(&mut self, item: PickerItem) {
        if item.category >= self.categories.len() || self.by_choice.contains_key(&item.choice) {
            return;
        }
        let Ok(index) = u32::try_from(self.items.len()) else {
            return;
        };
        self.words.push(matching::words(&item.name, &item.keywords));
        self.keys.push(item.choice.cell_key());
        self.members[item.category].push(index);
        self.by_choice.insert(item.choice.clone(), index);
        self.items.push(item);
    }

    /// Every icon in K3's catalog `set`, by its catalog name, in two categories: "Interface"
    /// (Lucide and this repository's own) and "File types" (Catppuccin's, `strata-ctp-*`).
    /// Each is called by its name less the `strata-`, `strata-kit-` and `ctp-` prefixes, with
    /// spaces for dashes ("folder open").
    pub fn catalog(set: &IconSet) -> Self {
        let categories = vec![
            PickerCategory::new("interface", "Interface", Choice::icon("strata-icons")),
            PickerCategory::new("file-types", "File types", Choice::icon("strata-ctp-rust")),
        ];
        let items = set
            .all
            .iter()
            .map(|icon| {
                let stem = icon.name.strip_prefix("strata-").unwrap_or(icon.name);
                let (category, stem) = match stem.strip_prefix("ctp-") {
                    Some(rest) => (1, rest),
                    None => (0, stem.strip_prefix("kit-").unwrap_or(stem)),
                };
                PickerItem::new(Choice::icon(icon.name), stem.replace('-', " "), category)
            })
            .collect();
        PickerSource::new(categories, items)
    }

    /// This source's categories and items followed by `other`'s.
    pub fn chain(self, other: PickerSource) -> Self {
        let offset = self.categories.len();
        let mut categories = self.categories;
        categories.extend(other.categories);
        let mut items = self.items;
        items.extend(other.items.into_iter().map(|mut i| {
            i.category += offset;
            i
        }));
        PickerSource::new(categories, items)
    }

    /// The categories, in order.
    pub fn categories(&self) -> &[PickerCategory] {
        &self.categories
    }

    /// The items, in order.
    pub fn items(&self) -> &[PickerItem] {
        &self.items
    }

    /// The item offering `choice`.
    pub fn find(&self, choice: &Choice) -> Option<usize> {
        self.by_choice.get(choice).map(|&i| i as usize)
    }

    /// Category `c`'s items, in order.
    pub(super) fn members(&self, c: usize) -> &[u32] {
        self.members.get(c).map_or(&[], Vec::as_slice)
    }

    pub(super) fn words(&self, item: usize) -> &Words {
        &self.words[item]
    }

    pub(super) fn cell_key(&self, item: usize) -> &str {
        &self.keys[item]
    }
}

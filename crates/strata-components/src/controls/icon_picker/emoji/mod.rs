// SPDX-License-Identifier: MIT

use std::sync::OnceLock;

use super::data::{Choice, PickerCategory, PickerItem, PickerSource};

#[cfg(test)]
mod tests;

/// The Emoji version [`unicode`] keeps by default.
pub const DEFAULT_VERSION: f32 = 15.1;

/// One bundled emoji.
#[derive(Debug)]
struct Record {
    glyph: &'static str,
    version: f32,
    name: &'static str,
    keywords: Vec<&'static str>,
    group: usize,
}

/// The groups, in order, with a tab key and the emoji each tab draws.
const GROUPS: [(&str, &str, &str); 9] = [
    ("Smileys & Emotion", "smileys", "😀"),
    ("People & Body", "people", "👋"),
    ("Animals & Nature", "nature", "🐻"),
    ("Food & Drink", "food", "🍔"),
    ("Travel & Places", "travel", "✈\u{fe0f}"),
    ("Activities", "activities", "⚽"),
    ("Objects", "objects", "💡"),
    ("Symbols", "symbols", "❤\u{fe0f}"),
    ("Flags", "flags", "🏁"),
];

fn records() -> &'static [Record] {
    static RECORDS: OnceLock<Vec<Record>> = OnceLock::new();
    RECORDS.get_or_init(|| parse(include_str!("emoji.tsv")))
}

pub(super) fn qualified(glyph: &str) -> Option<&'static str> {
    records()
        .iter()
        .find_map(|r| (r.glyph.strip_suffix('\u{fe0f}') == Some(glyph)).then_some(r.glyph))
}

fn parse(tsv: &'static str) -> Vec<Record> {
    let mut group = None;
    let mut out = Vec::new();
    for line in tsv.lines() {
        // A comment is `# ` and text: the keycap #️⃣ starts with `#` too.
        if line.starts_with("# ") || line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('@') {
            group = GROUPS.iter().position(|(g, _, _)| *g == name);
            continue;
        }
        let Some(group) = group else { continue };
        let mut fields = line.split('\t');
        let (Some(glyph), Some(version), Some(name)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let keywords = fields
            .next()
            .map(|k| k.split('|').filter(|w| !w.is_empty()).collect())
            .unwrap_or_default();
        out.push(Record {
            glyph,
            version: version.parse().unwrap_or(f32::INFINITY),
            name,
            keywords,
            group,
        });
    }
    out
}

pub(super) fn unicode(max_version: f32) -> PickerSource {
    let categories = GROUPS
        .iter()
        .map(|&(name, key, glyph)| PickerCategory::new(key, name, Choice::emoji(glyph)))
        .collect();
    let items = records()
        .iter()
        .filter(|r| r.version <= max_version + 1e-3)
        .map(|r| {
            PickerItem::new(Choice::emoji(r.glyph), r.name, r.group)
                .keywords(r.keywords.iter().copied())
        })
        .collect();
    PickerSource::new(categories, items)
}

impl PickerSource {
    /// The bundled emoji (Unicode 18.0 data, CLDR release-49 English names and keywords) up to Emoji `max_version`.
    pub fn emoji(max_version: f32) -> Self {
        unicode(max_version)
    }
}

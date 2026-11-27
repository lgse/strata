// SPDX-License-Identifier: MIT

use rune::{El, TextStyle, row, text};
use strata_kit::look::Look;

pub const PILL_FILL_ALPHA: f32 = 0.14;
pub const PILL_BORDER_ALPHA: f32 = 0.4;

const PILL_BASE_RADIUS: f32 = 999.0;
/// Strata's pill font weight (`font-weight: bold`) and letter spacing (`letter-spacing: 0.5px`).
const PILL_WEIGHT: f32 = 700.0;
const PILL_TRACKING: f32 = 0.5;

pub fn pill(look: &Look, words: &str) -> El {
    let k = &look.kit;
    let accent = k.c.text_accent;
    let height = look.frame.control;
    let radius = look.round(PILL_BASE_RADIUS).min(height / 2.0);
    let font = TextStyle {
        weight: PILL_WEIGHT,
        tracking: PILL_TRACKING,
        ..k.ty.small_medium
    };
    row()
        .h(height)
        .px(look.space.md)
        .center()
        .radius(radius)
        .fill(k.alpha(accent, PILL_FILL_ALPHA))
        .border(1.0, k.alpha(accent, PILL_BORDER_ALPHA))
        .child(
            text(words.to_owned())
                .font(font)
                .color(accent)
                .a11y_hidden(),
        )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterMark {
    /// A filter of the folder's own items (10xer `f`).
    Filter(String),
    /// A search below the folder (10xer `s`).
    Search(String),
}

impl FilterMark {
    /// The query.
    pub fn query(&self) -> &str {
        match self {
            FilterMark::Filter(q) | FilterMark::Search(q) => q,
        }
    }
}

/// The 10X pill: "10X", named for assistive technology with the experimental caveat.
pub(super) fn tenxer(look: &Look, key: String, words: &str, name: &str, caveat: &str) -> El {
    pill(look, words)
        .key(key)
        .role("status")
        .label(name.to_owned())
        .description(caveat.to_owned())
}

/// The Files on clipboard badge: a status (polite already), so it is heard when it appears.
pub(super) fn clipboard(look: &Look, key: String, words: &str, description: &str) -> El {
    pill(look, words)
        .key(key)
        .role("status")
        .label(words.to_owned())
        .description(description.to_owned())
}

pub(super) fn filter(look: &Look, key: String, words: String, query: &str) -> El {
    let k = &look.kit;
    text(words)
        .key(key)
        .font(k.ty.small)
        .color(k.c.text_base)
        .truncate()
        .max_w(MARK_MAX_CHARS as f32 * k.ty.small.size * CHAR_WIDTH)
        .description(query.to_owned())
}

pub const MARK_MAX_CHARS: usize = 40;
pub const HIT_FOLDER_MAX_CHARS: usize = 60;
pub const HIT_NAME_MIN_CHARS: usize = 32;
const CHAR_WIDTH: f32 = 0.6;

pub(super) fn hit(look: &Look, key: String, path: &str) -> El {
    let k = &look.kit;
    let font = k.ty.small;
    let (folder, name) = match path.rfind('/') {
        Some(i) if i + 1 < path.len() => (&path[..i], &path[i..]),
        _ => ("", path),
    };
    let width = |chars: usize| chars as f32 * font.size * CHAR_WIDTH;
    let mut el = row()
        .key(key)
        .role("label")
        .label(path.to_owned())
        .min_w(0.0)
        .shrink()
        .items_center();
    if !folder.is_empty() {
        el = el.child(
            text(folder.to_owned())
                .font(font)
                .color(k.c.text_muted)
                .truncate()
                .shrink()
                .min_w(0.0)
                .max_w(width(HIT_FOLDER_MAX_CHARS))
                .a11y_hidden(),
        );
    }
    let name_chars = name.chars().count().min(HIT_NAME_MIN_CHARS);
    el.child(
        text(name.to_owned())
            .font(font)
            .color(k.c.text_muted)
            .truncate_middle()
            .min_w(width(name_chars))
            .a11y_hidden(),
    )
}

pub(super) fn range(look: &Look, key: String, words: &str, name: &str) -> El {
    let k = &look.kit;
    text(words.to_owned())
        .key(key)
        .role("status")
        .label(name.to_owned())
        .font(k.ty.small)
        .color(k.c.text_base)
}

//! The footer's marks: the pill the `10X` tag and the clipboard badge share, the `filter:` and
//! `search:` marks, the search hit under the cursor, and the `VISUAL` and `UNSET` range marks.

use rune::{El, row, text};
use strata_kit::look::Look;

/// The pill's fill and border: the accent at these opacities, as Strata's `.tenxer-tag`,
/// `.shortcut-footer-paste` and `.shortcut-footer-chord-pill` draw them (`src/style.css`:
/// `alpha(@strata_accent, 0.14)` and a 1 px `alpha(@strata_accent, 0.4)` border). No Kit or
/// strata-kit token holds them; C18's chord pill uses the same values.
pub const PILL_FILL_ALPHA: f32 = 0.14;
pub const PILL_BORDER_ALPHA: f32 = 0.4;

/// Strata's footer pill around `words`: the accent on a faint accent tint with a fainter accent
/// border, fully rounded (`.tenxer-tag`). The `10X` tag, the clipboard badge and C18's chord
/// pill share it, so showing any of them leaves the footer's height alone. The words are hidden
/// from assistive technology: the caller names the pill.
///
/// Used by: the footer; hosts drawing a pill beside them.
pub fn pill(look: &Look, words: &str) -> El {
    let k = &look.kit;
    let accent = k.c.text_accent;
    row()
        .px(look.space.sm)
        .center()
        .radius(look.frame.control)
        .fill(k.alpha(accent, PILL_FILL_ALPHA))
        .border(1.0, k.alpha(accent, PILL_BORDER_ALPHA))
        .child(
            text(words.to_owned())
                .font(k.ty.small_medium)
                .color(accent)
                .a11y_hidden(),
        )
}

/// A mark the footer shows while a filter or a search is open: `filter: …` or `search: …`.
/// Strata shows it in 10xer mode, which hides the pane's own filter field, and for a subfolder
/// search in either map; which is the host's.
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

/// The `filter:` or `search:` mark, in the footer's text colour, cut short past
/// [`MARK_MAX_CHARS`] characters' width (Strata's 40), with the query as its description.
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

/// The longest a filter mark draws before its ellipsis, in characters (Strata's
/// `set_max_width_chars(40)`), and the longest folder part of a hit (60).
pub const MARK_MAX_CHARS: usize = 40;
pub const HIT_FOLDER_MAX_CHARS: usize = 60;
/// The hit's name keeps at least this many characters before it truncates
/// (`CurrentHit::NAME_MIN_CHARS`).
pub const HIT_NAME_MIN_CHARS: usize = 32;
/// A character's width as a share of the font size, to turn Strata's character widths into
/// points: about a monospace advance (JetBrains Mono's is 0.6 em).
const CHAR_WIDTH: f32 = 0.6;

/// The search hit under the cursor, as a path relative to the searched folder: the folder part
/// gives way to an ellipsis before the name does, which truncates in its middle. Both in
/// `text_muted` (Strata's `.shortcut-footer-chord-hint`). Assistive technology reads the path as
/// a label, not a live region: it changes with every cursor move.
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

/// The `VISUAL` or `UNSET` mark of an active 10xer range, in the footer's text colour, named
/// "Visual select" or "Visual unset".
pub(super) fn range(look: &Look, key: String, words: &str, name: &str) -> El {
    let k = &look.kit;
    text(words.to_owned())
        .key(key)
        .role("status")
        .label(name.to_owned())
        .font(k.ty.small)
        .color(k.c.text_base)
}

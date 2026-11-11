//! The grid: C1e's collection element with C3's grid geometry, in cells sized for a picker.
//!
//! Not C3's `IconsGrid`: its tiles are file tiles (at least `MIN_CARD`, 116 pt, wide, with a
//! caption, a details line, thumbnails, rename and marquee). The picker keeps the parts under
//! them: C1e's virtual lines, C3's [`IconsGeometry`] (2D keys, a short last row as GTK's), C1c's
//! keyboard with a map of its own and C1b's pointer intent in single-click mode.

use std::borrow::Cow;
use std::hash::{Hash, Hasher};

use rune::{Chord, ChordKey, ChordMod, El, TextStyle, col, icon, text};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::data::{Choice, EMOJI_PREFIX};
use crate::collection::element::{Ctx, Flow, Lines, PointerController, PointerInput};
use crate::collection::keyboard::{Command, KeyMap, KeySettings, Letters, Target, Vertical};
use crate::collection::pointer::{ClickCount, ClickModes, PointerIntent, PointerSettings};
use crate::contract::{CollectionGeometry, Direction};
use crate::icons_grid::IconsGeometry;

/// A cell's sizes, from Look tokens, so they follow the text size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Metrics {
    /// An icon's side: `icon_size.lg` (20 at 13 px, Strata's 20 px customize icons).
    pub icon: f32,
    /// An emoji's font size: an icon and `space.xs` (24 at 13 px).
    pub emoji: f32,
    /// The drawn tile: the icon and `space.md` around it, and a point of border (Strata's
    /// 38 px `.customize-icon-button`).
    pub tile: f32,
    /// A cell: the tile and `space.sm` between tiles (Strata's 6 px grid spacing).
    pub cell: f32,
    /// The grid's padding left and right.
    pub pad: f32,
}

impl Metrics {
    pub(super) fn of(look: &Look) -> Self {
        let icon = look.icon_size.lg;
        let tile = icon + 2.0 * look.space.md + 2.0;
        Self {
            icon,
            emoji: icon + look.space.xs,
            tile,
            cell: tile + look.space.sm,
            pad: look.space.xxs,
        }
    }

    /// The width of a grid `columns` cells wide.
    pub(super) fn width(&self, columns: usize) -> f32 {
        columns.max(1) as f32 * self.cell + 2.0 * self.pad
    }

    pub(super) fn key(&self) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        for v in [self.icon, self.emoji, self.tile, self.cell, self.pad] {
            v.to_bits().hash(&mut h);
        }
        h.finish()
    }
}

/// The picker's packing rule: as many cells a line as fit, at most `max`.
#[derive(Clone, Copy, Debug)]
pub(super) struct PickerLines {
    pub m: Metrics,
    pub max: usize,
}

impl PickerLines {
    fn per(&self, width: f32) -> usize {
        let inner = width - 2.0 * self.m.pad;
        if !(inner > 0.0 && self.m.cell > 0.0) {
            return 1;
        }
        // A hair over, so a width made for N cells (`Metrics::width`) holds N.
        ((inner / self.m.cell + 1e-3).floor() as usize).clamp(1, self.max.max(1))
    }
}

impl Lines for PickerLines {
    fn per_line(&self, width: f32) -> usize {
        self.per(width)
    }

    fn inset(&self) -> f32 {
        self.m.pad
    }

    fn geometry(
        &self,
        len: usize,
        row_height: f32,
        width: f32,
        viewport_height: f32,
    ) -> Box<dyn CollectionGeometry> {
        let per = self.per(width);
        let inner = (width - 2.0 * self.m.pad).max(0.0);
        let cell_w = if inner > 0.0 {
            inner / per as f32
        } else {
            self.m.cell
        };
        Box::new(IconsGeometry {
            len,
            per,
            cell_w,
            row_h: row_height,
            pad: self.m.pad,
            viewport_h: viewport_height,
        })
    }
}

/// The grid's keys: arrows, Home, End and the page keys move the cursor (no Shift range: one
/// choice at a time), and Return or Space chooses. Left and Right cross row ends.
pub(super) fn key_settings() -> KeySettings {
    let mut map = KeyMap::new();
    for (k, d) in [
        (ChordKey::Up, Direction::Up),
        (ChordKey::Down, Direction::Down),
        (ChordKey::Left, Direction::Left),
        (ChordKey::Right, Direction::Right),
    ] {
        map.bind(Chord::new(k), Command::Select(Target::Step(d)));
    }
    map.bind(Chord::new(ChordKey::Home), Command::Select(Target::First));
    map.bind(Chord::new(ChordKey::End), Command::Select(Target::Last));
    map.bind_tolerating(
        Chord::new(ChordKey::PageUp),
        &[ChordMod::Super],
        Command::Select(Target::Page(Vertical::Up)),
    );
    map.bind_tolerating(
        Chord::new(ChordKey::PageDown),
        &[ChordMod::Super],
        Command::Select(Target::Page(Vertical::Down)),
    );
    map.bind(Chord::new(ChordKey::Enter), Command::Activate);
    map.bind(Chord::new(ChordKey::Char(' ')), Command::Activate);
    KeySettings {
        map,
        letters: Letters::Pass,
        wrap_rows: true,
        ..KeySettings::default()
    }
}

/// One click chooses, on release; no drags and no rename.
pub(super) fn pointer_settings() -> PointerSettings {
    let mut settings =
        PointerSettings::default().modes(ClickModes::new(ClickCount::One, ClickCount::One));
    settings.drags = false;
    settings.rename = false;
    settings
}

/// A cell's choice, read back from its entry's icon.
pub(super) fn choice_of(cell_key: &str) -> Choice {
    match cell_key.strip_prefix(EMOJI_PREFIX) {
        Some(e) => Choice::emoji(e.to_owned()),
        None => Choice::icon(cell_key.to_owned()),
    }
}

/// `choice` drawn `m.icon` points tall: an icon in the accent, Strata's primary icon colour,
/// or an emoji as text. Hidden from assistive technology: what holds it is named.
pub(super) fn glyph(look: &Look, m: &Metrics, choice: &Choice) -> El {
    let k = &look.kit;
    match choice {
        Choice::Icon(name) => {
            let art = look
                .icon_named(name)
                .unwrap_or_else(|| look.icon(IconName::File));
            icon(art, m.icon).color(k.c.icon_accent).a11y_hidden()
        }
        Choice::Emoji(_) => text(drawn_text(choice).into_owned())
            .font(TextStyle::new(m.emoji, 400.0).lh(m.emoji))
            .color(k.c.text_base)
            .a11y_hidden(),
    }
}

/// The text to draw for `choice`, for display only: an emoji's own text with U+FE0F after it
/// when it holds none, an icon's name as it is. Save and compare the choice itself, never
/// this. Hosts drawing a saved emoji (a button label, a preview) draw it through this.
///
/// Rune 5d421fd sends a grapheme to the colour emoji font first only when it holds U+FE0F
/// (anomalyco/rune#102), so an emoji whose default presentation is already emoji (♈, ⛺, 🆎)
/// draws as the outline of the first symbol font covering it (Noto Sans Symbols here; DejaVu
/// Sans in #102). Probe P1 (C20d): 23 of the 1,898 bundled emoji drew so, and every one in
/// colour with U+FE0F after it.
///
/// Used by: hosts.
pub fn drawn_text(choice: &Choice) -> Cow<'_, str> {
    match choice {
        Choice::Emoji(e) if !e.contains('\u{fe0f}') => Cow::Owned(format!("{e}\u{fe0f}")),
        Choice::Icon(t) | Choice::Emoji(t) => Cow::Borrowed(t),
    }
}

/// A cell: the tile, centred in it, lit by its state. The cursor shows only as a focus ring
/// in keyboard mode (C1e's `cursor_shown`), as GTK's chooser shows its focus: a fill would
/// read as the chosen icon, which is the current choice's mark.
pub(super) struct CellLook {
    pub cursor_shown: bool,
    pub current: bool,
    pub hover: bool,
}

pub(super) fn cell(look: &Look, m: &Metrics, st: CellLook, glyph: El) -> El {
    let k = &look.kit;
    let mut tile = col()
        .size(m.tile, m.tile)
        .center()
        .radius(k.radius.md)
        .child(glyph);
    if st.current {
        // Strata's `.customize-icon-button.active`: a fifth of the accent, an accent border.
        tile = tile.fill(k.alpha(k.c.text_accent, 0.2));
    }
    if st.hover {
        tile = tile.hover(k.c.hover);
    }
    tile = if st.cursor_shown {
        tile.outline(2.0, -2.0, k.c.border_focus)
    } else if st.current {
        tile.outline(1.0, -1.0, k.c.text_accent)
    } else {
        tile
    };
    col().w_full().h_full().center().child(tile)
}

/// C1b's pointer intent with every press read as a plain one: a picker has one choice, so
/// Ctrl and Shift clicks choose as a click does, rather than toggling or extending a
/// selection (probe P4: under C1b alone they selected several cells and chose none).
#[derive(Debug, Default)]
pub(super) struct OneChoice(PointerIntent);

impl PointerController for OneChoice {
    fn pointer(&mut self, input: &PointerInput, cx: &mut Ctx<'_>) -> Flow {
        let mut plain = *input;
        plain.mods = rune::Mods::default();
        self.0.pointer(&plain, cx)
    }

    fn tick(&mut self, cx: &mut Ctx<'_>) -> Flow {
        self.0.tick(cx)
    }

    fn reset(&mut self) {
        self.0.reset();
    }
}

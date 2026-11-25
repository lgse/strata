// SPDX-License-Identifier: MIT

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

/// Emoji text to draw: appends `U+FE0F` only for unqualified text-default emoji from the bundled
/// table; `Emoji_Presentation` emoji, ZWJ sequences, and already-qualified forms borrow as-is.
pub(crate) fn drawn_emoji(emoji: &str) -> &str {
    if !emoji.contains('\u{fe0f}')
        && let Some(qualified) = super::emoji::qualified(emoji)
    {
        return qualified;
    }
    emoji
}

/// The text to draw for `choice`, for display only.
pub fn drawn_text(choice: &Choice) -> Cow<'_, str> {
    match choice {
        Choice::Emoji(e) => Cow::Borrowed(drawn_emoji(e)),
        Choice::Icon(t) => Cow::Borrowed(t),
    }
}

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

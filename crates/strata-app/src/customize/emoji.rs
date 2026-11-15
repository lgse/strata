// SPDX-License-Identifier: MIT

//! "Choose Emoji…": GTK's `EmojiChooser` in `customization/icon_picker.rs`, on C20d's
//! [`IconPicker`].
//!
//! The popover sits over the Customize dialog, opens on Recently used once an emoji has been
//! picked in this window (else Smileys & Emotion), and marks the item's saved emoji as
//! Current. Picking an emoji saves `emoji:<glyph>` at once, remembers it in the recent tab,
//! updates the button to "Emoji  <glyph>", and closes the popover with focus back on the
//! button. Escape closes only the popover; a second Escape closes Customize.
//!
//! C20d's bundled emoji keep their English CLDR names and keywords, as GTK's chooser searches
//! English keywords under a non-English locale; the picker's chrome and its nine category tabs
//! take their words from the thirteen catalogs.

use std::{cell::RefCell, rc::Rc, sync::OnceLock};

use rune::{El, TextStyle, Tx, text};
use strata_components::controls::icon_picker::{
    Choice, DEFAULT_VERSION, IconPicker, IconPickerState, PickerCategory, PickerSource,
    PickerWords, drawn_text, remember,
};
use strata_core::i18n::{count, tr};
use strata_desktop::settings::{EMOJI_PREFIX, custom_emoji};
use strata_kit::look::Look;

use crate::App;

const PICKER: &str = "customize-emoji";

/// Keeps the emoji grid's cell ids (`row-<id>`) clear of `slots::Ids` in the browser views.
const ID_BASE: u64 = 1 << 48;

const PREVIEW_FONT: f32 = 48.0;

pub(super) struct Emoji {
    picker: IconPickerState,
    recent: Vec<Choice>,
    locale: String,
}

impl Emoji {
    pub(super) fn new() -> Self {
        let locale = rust_i18n::locale().to_string();
        let picker = IconPickerState::new(source_for(&locale)).id_base(ID_BASE);
        Self {
            picker,
            recent: Vec::new(),
            locale,
        }
    }

    pub(super) fn is_open(&self) -> bool {
        self.picker.is_open()
    }

    pub(super) fn field_focused(&self) -> bool {
        self.picker.is_open() && self.picker.edit().focused
    }

    pub(super) fn close(&mut self) {
        self.picker.close();
    }

    pub(super) fn sync(&mut self, saved_icon: Option<&str>) {
        self.sync_locale();
        self.sync_current(saved_icon);
    }

    #[cfg(test)]
    pub(super) fn picker_page(&self) -> strata_components::controls::icon_picker::Page {
        self.picker.page()
    }

    #[cfg(test)]
    pub(super) fn current(&self) -> Option<&Choice> {
        self.picker.current()
    }

    fn sync_locale(&mut self) -> bool {
        let now = rust_i18n::locale();
        if self.locale == *now {
            return false;
        }
        let locale = now.to_string();
        self.picker.set_source(source_for(&locale));
        self.locale = locale;
        true
    }

    fn sync_current(&mut self, saved_icon: Option<&str>) {
        let choice = saved_icon
            .and_then(custom_emoji)
            .map(|emoji| choice_for(emoji, self.picker.source()));
        self.picker.set_current(choice.as_ref());
    }
}

fn state(app: &App) -> &IconPickerState {
    &app.customize.emoji.picker
}

fn state_mut(app: &mut App) -> &mut IconPickerState {
    &mut app.customize.emoji.picker
}

fn look(app: &App) -> &Look {
    &app.look
}

fn chose(app: &mut App, choice: &Choice, _: &mut Tx) {
    let saved = format!("{EMOJI_PREFIX}{}", choice.text());
    super::save_icon(app, &saved);
    let emoji = &mut app.customize.emoji;
    remember(&mut emoji.recent, choice.clone());
    emoji.picker.set_recent(&emoji.recent);
    emoji.picker.set_current(Some(choice));
}

fn picker() -> IconPicker<App> {
    IconPicker::new(PICKER, state, state_mut, look)
        .words(words())
        .on_choose(chose)
}

/// Runs the picker's grid and dismissal checks while Customize or the popover is open.
pub(super) fn poll(app: &mut App, tx: &mut Tx) -> bool {
    if !app.customize.open && !app.customize.emoji.is_open() {
        return false;
    }
    let icon = super::saved(app).and_then(|(_, icon)| icon);
    let locale_changed = app.customize.emoji.sync_locale();
    app.customize.emoji.sync_current(icon.as_deref());
    let polled = picker().poll(app, tx);
    locale_changed || polled
}

/// "Choose Emoji…", or "Emoji  🚀" when `saved_icon` is an emoji; its popover pushes onto `over`.
pub(super) fn button(app: &App, saved_icon: Option<&str>, over: &mut Vec<El>) -> El {
    let label = match saved_icon.and_then(custom_emoji) {
        Some(emoji) => {
            let choice = Choice::emoji(emoji.to_owned());
            rust_i18n::t!("Emoji  %{emoji}", emoji = drawn_text(&choice)).into_owned()
        }
        None => tr("Choose Emoji…"),
    };
    let description = tr("Choose any emoji for this item");
    picker().button(app, &app.look, &label, Some(&description), over)
}

/// The item's saved emoji drawn for the dialog's 56 px preview, through [`drawn_text`].
pub(super) fn preview(look: &Look, saved_icon: Option<&str>, height: f32) -> Option<El> {
    let emoji = saved_icon.and_then(custom_emoji)?;
    let choice = Choice::emoji(emoji.to_owned());
    Some(
        text(drawn_text(&choice).into_owned())
            .font(TextStyle::new(PREVIEW_FONT, 400.0).lh(height))
            .color(look.kit.c.text_base)
            .a11y_hidden(),
    )
}

/// Matches a saved emoji to the source's spelling, tolerating a variation-selector difference.
fn choice_for(emoji: &str, source: &PickerSource) -> Choice {
    let exact = Choice::emoji(emoji.to_owned());
    if source.find(&exact).is_some() {
        return exact;
    }
    let trimmed = emoji.trim_end_matches('\u{fe0f}');
    let bare = Choice::emoji(trimmed.to_owned());
    if source.find(&bare).is_some() {
        return bare;
    }
    let qualified = Choice::emoji(format!("{trimmed}\u{fe0f}"));
    if source.find(&qualified).is_some() {
        return qualified;
    }
    exact
}

fn words() -> PickerWords {
    words_for(&rust_i18n::locale())
}

pub(super) fn words_for(locale: &str) -> PickerWords {
    let t = |key: &str| rust_i18n::t!(key, locale = locale).into_owned();
    let mut words = PickerWords::default();
    words.grid = t("Emoji");
    words.field = t("Search");
    words.clear = t("Clear search");
    words.tabs = t("Categories");
    words.recent = t("Recently used");
    words.results = t("Search results");
    words.no_results = t("No results");
    words.count = |n| count("results", n);
    words.current = t("Current");
    words.popover = t("Choose an emoji");
    words
}

fn base_source() -> &'static PickerSource {
    static BASE: OnceLock<PickerSource> = OnceLock::new();
    BASE.get_or_init(|| PickerSource::emoji(DEFAULT_VERSION))
}

thread_local! {
    /// One [`PickerSource`] per UI language, sharing C20d's English emoji items while naming
    /// the nine category tabs from the catalogs.
    static SOURCES: RefCell<Vec<(String, Rc<PickerSource>)>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn source_for(locale: &str) -> Rc<PickerSource> {
    SOURCES.with_borrow_mut(|made| {
        if let Some((_, source)) = made.iter().find(|(tag, _)| tag == locale) {
            return source.clone();
        }
        let base = base_source();
        let categories = base
            .categories()
            .iter()
            .map(|cat| {
                let name = rust_i18n::t!(cat.name.as_ref(), locale = locale).into_owned();
                PickerCategory::new(cat.key.clone(), name, cat.glyph.clone())
            })
            .collect();
        let source = Rc::new(PickerSource::new(categories, base.items().to_vec()));
        made.push((locale.to_owned(), source.clone()));
        source
    })
}

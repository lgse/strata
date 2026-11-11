//! The picker's builder: the field, the grid, the name line and the tabs, inline or in a
//! popover behind a button.

use std::hash::{Hash, Hasher};
use std::rc::Rc;

use rune::{
    Chord, ChordKey, ChordMod, El, Key, KeyEvent, KeyPlatform, LiveRegion, Tx, col, el, row, text,
};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Popover};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::data::Choice;
use super::grid::{self, CellLook, Metrics, PickerLines};
use super::matching::{Hit, keyword_range, name_ranges};
use super::state::{IconPickerState, Page};
use super::words::PickerWords;
use crate::collection::element::{Collection, ContentSlot, Describe, Intent, Roles, RowCx, Slot};
use crate::contract::{EntrySource, MatchedText, Point};
use crate::filter_bar::field::{self, Handlers, MenuFn, Spec};

type ChooseFn<S> = Rc<dyn Fn(&mut S, &Choice, &mut Tx)>;
type CloseFn<S> = Rc<dyn Fn(&mut S, &mut Tx)>;

/// rune-kit's Popover body at its widest: `max_w(320.0)` in rune-kit's `overlay.rs`, which it
/// doesn't export (as C20b's ColorButton reads it).
const POPOVER_MAX_W: f32 = 320.0;

/// What sits between the name and the keyword a search matched.
const KEYWORD_MARK: &str = "\u{b7} ";

/// Escape as Rune's modal dismisses on it: bare or with Shift, no Ctrl, Alt, Cmd or Super.
fn escape_key(e: &KeyEvent) -> bool {
    let m = e.mods;
    e.key == Key::Escape && !(m.ctrl || m.alt || m.cmd || m.logo)
}

/// Ctrl+N (`Some(false)`) or Ctrl+P (`Some(true)`): GTK's chooser steps sections with them.
/// Matched as Rune's layout-aware chords, so they follow the key's label.
fn tab_step(e: &KeyEvent, tx: &Tx) -> Option<bool> {
    let platform = KeyPlatform::current();
    let chord = |c| Chord::new(ChordKey::Char(c)).with(ChordMod::Ctrl);
    if chord('n').matches_tx_on(e, tx, platform) {
        Some(false)
    } else if chord('p').matches_tx_on(e, tx, platform) {
        Some(true)
    } else {
        None
    }
}

/// An icon picker over an [`IconPickerState`]: see the [module docs](super).
///
/// Used by: hosts.
pub struct IconPicker<S> {
    pub(super) key: Rc<str>,
    pub(super) get: fn(&S) -> &IconPickerState,
    pub(super) get_mut: fn(&mut S) -> &mut IconPickerState,
    pub(super) look: fn(&S) -> &Look,
    pub(super) words: Rc<PickerWords>,
    search: bool,
    tooltips: bool,
    columns: usize,
    rows: usize,
    on_choose: Option<ChooseFn<S>>,
    on_close: Option<CloseFn<S>>,
    on_escape: Option<CloseFn<S>>,
    on_menu: Option<MenuFn<S>>,
}

impl<S> Clone for IconPicker<S> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            get: self.get,
            get_mut: self.get_mut,
            look: self.look,
            words: self.words.clone(),
            search: self.search,
            tooltips: self.tooltips,
            columns: self.columns,
            rows: self.rows,
            on_choose: self.on_choose.clone(),
            on_close: self.on_close.clone(),
            on_escape: self.on_escape.clone(),
            on_menu: self.on_menu.clone(),
        }
    }
}

impl<S: 'static> IconPicker<S> {
    /// A picker keyed `key` over the state `get` and `get_mut` return, drawn with the look
    /// `look` returns: searchable, 8 cells a row at most and 6 rows tall.
    pub fn new(
        key: impl Into<Rc<str>>,
        get: fn(&S) -> &IconPickerState,
        get_mut: fn(&mut S) -> &mut IconPickerState,
        look: fn(&S) -> &Look,
    ) -> Self {
        Self {
            key: key.into(),
            get,
            get_mut,
            look,
            words: Rc::new(PickerWords::default()),
            search: true,
            tooltips: false,
            columns: 8,
            rows: 6,
            on_choose: None,
            on_close: None,
            on_escape: None,
            on_menu: None,
        }
    }

    /// The picker's words.
    pub fn words(mut self, words: PickerWords) -> Self {
        self.words = Rc::new(words);
        self
    }

    /// Whether it has a search field and the name line under the grid (on by default).
    /// Strata's 16 customize icons have neither.
    pub fn search(mut self, on: bool) -> Self {
        self.search = on;
        self
    }

    /// Whether each cell shows its name as a tooltip (off by default): Strata's customize
    /// icons do, GTK's emoji don't.
    pub fn tooltips(mut self, on: bool) -> Self {
        self.tooltips = on;
        self
    }

    /// How many cells a row holds at most (8 by default, Strata's): the picker is this wide.
    /// In a popover, as many as fit inside rune-kit's widest popover.
    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = columns.max(1);
        self
    }

    /// How many rows of cells show at once (6 by default); the grid scrolls past them.
    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows.max(1);
        self
    }

    /// Hears a choice: a click or a tap on a cell, Return or Space on the grid's cursor, or
    /// Return in the field (the first result). It fires once per choice. The host saves it,
    /// calls `IconPickerState::set_current`, and keeps its recent list with `remember`. In a
    /// popover, the popover then closes.
    pub fn on_choose(mut self, f: impl Fn(&mut S, &Choice, &mut Tx) + 'static) -> Self {
        self.on_choose = Some(Rc::new(f));
        self
    }

    /// Hears the popover close: after a choice, Escape, or a press outside (reported from
    /// [`IconPicker::poll`], a frame later). Focus goes back to the button either way.
    pub fn on_close(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.on_close = Some(Rc::new(f));
        self
    }

    /// Escape in an inline picker's search field with no text left to clear: what the host
    /// does next (C20c's `on_escape`; Settings closes). Rune's field takes an Escape its owner
    /// leaves (it blurs), so the key never reaches the host's own handlers.
    pub fn on_escape(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.on_escape = Some(Rc::new(f));
        self
    }

    /// A menu press on the search field (right-click, Menu, Shift+F10), at a window point:
    /// the host opens C21's `FieldMenu` on [`IconPicker::field_key`].
    pub fn on_menu(mut self, f: impl Fn(&mut S, Point, &mut Tx) + 'static) -> Self {
        self.on_menu = Some(Rc::new(f));
        self
    }

    /// The search field's key.
    pub fn field_key(&self) -> String {
        format!("{}-field", self.key)
    }

    /// The grid's key: its collection's scope, so its cells are `row-<id>` under it.
    pub fn grid_key(&self) -> String {
        format!("{}-grid", self.key)
    }

    /// The button's key (`IconPicker::button`).
    pub fn button_key(&self) -> String {
        format!("{}-button", self.key)
    }

    /// The popover's key.
    pub fn popover_key(&self) -> String {
        format!("{}-popover", self.key)
    }

    pub(super) fn tabs_key(&self) -> String {
        format!("{}-tabs", self.key)
    }

    /// The key of the tab showing `page`: `<key>-tab-recent`, `<key>-tab-<category key>`.
    pub(super) fn tab_key(&self, s: &S, page: Page) -> String {
        match page {
            Page::Recent => format!("{}-tab-recent", self.key),
            Page::Category(c) => {
                let source = (self.get)(s).source();
                let id = source.categories().get(c).map_or("", |c| &c.key);
                format!("{}-tab-{id}", self.key)
            }
            Page::Results => format!("{}-tab-results", self.key),
        }
    }

    /// What a page is called: its tab's name.
    pub(super) fn page_name(&self, s: &S, page: Page) -> String {
        match page {
            Page::Recent => self.words.recent.clone(),
            Page::Category(c) => (self.get)(s)
                .source()
                .categories()
                .get(c)
                .map_or_else(String::new, |c| c.name.to_string()),
            Page::Results => self.words.results.clone(),
        }
    }

    /// What a page's tab draws.
    pub(super) fn page_glyph(&self, s: &S, page: Page) -> Option<Choice> {
        match page {
            Page::Recent => Some(Choice::icon("strata-clock")),
            Page::Category(c) => (self.get)(s)
                .source()
                .categories()
                .get(c)
                .map(|c| c.glyph.clone()),
            Page::Results => None,
        }
    }

    /// The picker inline.
    pub fn build(&self, s: &S, look: &Look) -> El {
        let m = Metrics::of(look);
        self.body(s, look, &m, self.columns)
    }

    /// A button labelled `label` (Strata's "Choose Emoji…", then "Emoji  🚀"), described by
    /// `description`, keyed [`IconPicker::button_key`]. A press opens the picker in a popover
    /// under it, pushed onto `layer`, with focus in the search field (on the grid without
    /// one). A choice, Escape or a press outside closes it, and focus goes back to the button.
    pub fn button(
        &self,
        s: &S,
        look: &Look,
        label: &str,
        description: Option<&str>,
        layer: &mut Vec<El>,
    ) -> El {
        let k = &look.kit;
        let m = Metrics::of(look);
        let open = (self.get)(s).is_open();
        let get_mut = self.get_mut;
        let closer = self.clone();
        let mut button = Button::new(label.to_owned())
            .variant(ButtonVariant::Neutral)
            .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                if get_mut(s).is_open() {
                    closer.close(s, tx);
                } else {
                    get_mut(s).open();
                }
            })
            .build(k)
            .key(self.button_key());
        if let Some(d) = description {
            button = button.description(d.to_owned());
        }
        let pad = look.space.lg;
        let mut popover = Popover::new(self.popover_key())
            .trigger(button)
            .label(self.words.popover.clone())
            .padding(pad)
            .open(open);
        if open {
            // As many columns as fit inside the popover's widest body and its padding.
            let fit =
                (((POPOVER_MAX_W - 2.0 * pad - 2.0 * m.pad) / m.cell).floor() as usize).max(1);
            let columns = self.columns.min(fit);
            // Rune's modal traps Tab, focuses the first stop (the field) and gives focus back
            // to the button when it goes. A press outside dismisses it (reported by `poll`);
            // Escape never reaches the dismissal, as the picker takes it first and closes.
            let body = self
                .body(s, look, &m, columns)
                .role("none")
                .modal(true, move |s: &mut S| get_mut(s).dismiss());
            popover = popover.child(body);
        }
        row().child(popover.build(k, layer))
    }

    /// Call from the host's poll every frame: reports a popover a press outside closed
    /// (Rune's modal dismissal has no `Tx`), and runs the grid's controllers. True when it
    /// changed the state.
    pub fn poll(&self, s: &mut S, tx: &mut Tx) -> bool {
        let mut changed = false;
        if (self.get_mut)(s).take_dismissed() {
            if let Some(f) = &self.on_close {
                f(s, tx);
            }
            changed = true;
        }
        changed | self.collection(s).tick(s, tx)
    }

    /// Searches for the field's text after the host changed it (C21's `FieldMenu` pasted,
    /// cut or undid): the grid shows its results from the top.
    pub fn edited(&self, s: &mut S, tx: &mut Tx) {
        let scope = self.grid_key();
        (self.get_mut)(s).query_changed(Some((&scope, tx)));
    }

    /// Chooses item `item`: the host hears it, and an open popover closes.
    fn choose(&self, s: &mut S, item: usize, tx: &mut Tx) {
        let Some(choice) = (self.get)(s)
            .source()
            .items()
            .get(item)
            .map(|i| i.choice.clone())
        else {
            return;
        };
        if let Some(f) = &self.on_choose {
            f(s, &choice, tx);
        }
        if (self.get)(s).is_open() {
            self.close(s, tx);
        }
    }

    /// Closes the popover and tells the host.
    fn close(&self, s: &mut S, tx: &mut Tx) {
        (self.get_mut)(s).close();
        if let Some(f) = &self.on_close {
            f(s, tx);
        }
    }

    /// The grid's collection (C1e), as the view builds it: for its `tick` and its geometry.
    pub fn collection(&self, s: &S) -> Collection<S> {
        let (get, get_mut, look_of) = (self.get, self.get_mut, self.look);
        let st = get(s);
        let page = st.page();
        let name = match page {
            Page::Results => self.words.results.clone(),
            _ => self.words.grid.clone(),
        };
        let chooser = self.clone();
        let (slot_get, describe_get, version_get) = (get, get, get);
        let tooltips = self.tooltips;
        let current_words = self.words.current.clone();
        let empty_words = self.words.no_results.clone();
        let empty_key = format!("{}-empty", self.key);
        Collection::new(
            self.grid_key(),
            move |s: &S| {
                let st = get(s);
                (&st.shown as &dyn EntrySource, &st.coll)
            },
            move |s: &mut S| {
                let st = get_mut(s);
                (&st.shown as &dyn EntrySource, &mut st.coll)
            },
            look_of,
            move |s: &S| version_get(s).version(),
        )
        .name(name)
        .roles(Roles::GRID_CELLS)
        .bare_items()
        .read_only()
        .row_height(|_: &S, l: &Look| Metrics::of(l).cell)
        .lines(PickerLines {
            m: Metrics::of(look_of(s)),
            max: self.columns,
        })
        .settings(grid::key_settings())
        .settings(grid::pointer_settings())
        .announce(|_: &S, _| String::new())
        .icon(Slot::pure(|_: &S, rcx: &RowCx<'_>| {
            let m = Metrics::of(rcx.look);
            grid::glyph(rcx.look, &m, &grid::choice_of(rcx.entry.icon))
        }))
        .label(Slot::pure(|_: &S, _: &RowCx<'_>| el().a11y_hidden()))
        .content(ContentSlot::new(
            move |s: &S, rcx: &RowCx<'_>| {
                let current = is_current(slot_get(s), rcx);
                let mut h = std::hash::DefaultHasher::new();
                (current, tooltips, Metrics::of(rcx.look).key()).hash(&mut h);
                h.finish()
            },
            move |s: &S, rcx: &RowCx<'_>, glyph: El, _label: El| {
                let m = Metrics::of(rcx.look);
                let look = CellLook {
                    cursor_shown: rcx.state.cursor_shown,
                    current: is_current(slot_get(s), rcx),
                    hover: rcx.hover_live,
                };
                let cell = grid::cell(rcx.look, &m, look, glyph);
                if tooltips {
                    cell.help(rcx.entry.name.to_owned())
                } else {
                    cell
                }
            },
        ))
        .describe(Describe::new(
            move |s: &S, rcx: &RowCx<'_>| u64::from(is_current(describe_get(s), rcx)),
            move |s: &S, rcx: &RowCx<'_>| {
                is_current(describe_get(s), rcx).then(|| current_words.clone())
            },
        ))
        .empty(move |s: &S| {
            let st = get(s);
            if st.page() != Page::Results || !st.shown.is_empty() {
                return None;
            }
            let k = &look_of(s).kit;
            Some(
                col()
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .center()
                    .child(
                        text(empty_words.clone())
                            .key(empty_key.clone())
                            .font(k.ty.ui)
                            .color(k.c.text_muted),
                    ),
            )
        })
        .on_intent(move |s: &mut S, intent: Intent, tx: &mut Tx| {
            if let Intent::Activate { id, .. } = intent
                && let Some(item) = (chooser.get)(s).shown.item_of(id)
            {
                chooser.choose(s, item, tx);
            }
        })
    }

    fn body(&self, s: &S, look: &Look, m: &Metrics, columns: usize) -> El {
        let st = (self.get)(s);
        let width = m.width(columns);
        let mut picker = self.clone();
        picker.columns = columns;
        let grid = col()
            .w(width)
            .h(self.rows as f32 * m.cell)
            .min_h(0.0)
            .child(picker.collection(s).build(s));
        let tabs = picker.tabs(s, look, m);
        let grid = if tabs.is_some() {
            col()
                .key(format!("{}-panel", self.key))
                .role("tabpanel")
                .label(self.page_name(s, st.page()))
                .child(grid)
        } else {
            grid
        };
        let mut body = col().key(self.key.to_string()).w(width).gap(look.space.sm);
        if self.search {
            body = body.child(self.field(s, look));
        }
        body = body.child(grid);
        if self.search {
            body = body.child(self.name_line(s, look, m));
        }
        if let Some(tabs) = tabs {
            body = body.child(tabs);
        }
        let heard = match st.results() {
            None => String::new(),
            Some(0) => self.words.no_results.clone(),
            Some(n) => (self.words.count)(n),
        };
        body = body.child(
            el().key(format!("{}-status", self.key))
                .role("status")
                .live_region(LiveRegion::Polite)
                .label(heard)
                .absolute(0.0, 0.0)
                .w(0.0)
                .h(0.0),
        );
        let keys = self.clone();
        body.on_key(move |s: &mut S, e: &KeyEvent, tx: &mut Tx| keys.body_key(s, e, tx))
    }

    /// Keys from the grid or the tabs that they didn't take: Escape closes a popover, Ctrl+N
    /// and Ctrl+P step the tabs.
    fn body_key(&self, s: &mut S, e: &KeyEvent, tx: &mut Tx) -> bool {
        if escape_key(e) && (self.get)(s).is_open() {
            self.close(s, tx);
            return true;
        }
        match tab_step(e, tx) {
            Some(back) => self.step_tab(s, back, tx),
            None => false,
        }
    }

    /// The search field: a magnifier, the text, and a clear button while there is text.
    fn field(&self, s: &S, look: &Look) -> El {
        let k = &look.kit;
        let st = (self.get)(s);
        let menu: MenuFn<S> = self
            .on_menu
            .clone()
            .unwrap_or_else(|| Rc::new(|_: &mut S, _, _: &mut Tx| {}));
        let get_mut = self.get_mut;
        let (keys, change) = (self.clone(), self.clone());
        let handlers = Handlers {
            edit: Rc::new(move |s: &mut S| &mut get_mut(s).field),
            key: Rc::new(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
                keys.field_key_event(s, ev, tx)
            }),
            change: Rc::new(move |s: &mut S, tx: &mut Tx| {
                let scope = change.grid_key();
                (change.get_mut)(s).query_changed(Some((&scope, tx)));
            }),
            menu: menu.clone(),
        };
        let spec = Spec {
            key: &self.field_key(),
            label: &self.words.field,
            placeholder: Some(&self.words.field),
            description: None,
            invalid: false,
            font: k.ty.ui,
        };
        let input = field::input(look, &st.field, &spec, &handlers).role("searchbox");
        let mut inner = field::frame_row(look, format!("{}-frame", self.key))
            .child(
                rune::icon(look.icon(IconName::Search), look.icon_size.sm)
                    .color(k.c.icon_muted)
                    .a11y_hidden(),
            )
            .child(input);
        if !st.field.text.is_empty() {
            let clear = self.clone();
            inner = inner.child(
                IconButton::new(look.icon(IconName::Close))
                    .variant(IconButtonVariant::GhostMuted)
                    .control_size(ControlSize::Small)
                    .label(self.words.clear.clone())
                    .on_press_tx(move |s: &mut S, tx: &mut Tx| clear.clear(s, tx))
                    .build(k)
                    // The field stays the one Tab stop; a press keeps the keyboard in it.
                    .focusable(false)
                    .key(format!("{}-clear", self.key)),
            );
        }
        field::frame(look, inner, st.field.focused, false, &menu)
    }

    fn clear(&self, s: &mut S, tx: &mut Tx) {
        let scope = self.grid_key();
        let st = (self.get_mut)(s);
        st.field.replace_text("");
        st.query_changed(Some((&scope, tx)));
    }

    /// The field's keys: Return chooses the grid's cursor (the first result), Down goes to the
    /// grid, Escape closes a popover (inline: clears the text, then goes to
    /// [`IconPicker::on_escape`]), and Ctrl+N and Ctrl+P step the tabs.
    fn field_key_event(&self, s: &mut S, e: &KeyEvent, tx: &mut Tx) -> bool {
        let m = e.mods;
        let bare = !(m.ctrl || m.alt || m.cmd || m.logo || m.shift);
        if escape_key(e) {
            if (self.get)(s).is_open() {
                self.close(s, tx);
                return true;
            }
            if (self.get)(s).query().is_empty() {
                return match &self.on_escape {
                    Some(f) => {
                        f(s, tx);
                        true
                    }
                    None => false,
                };
            }
            self.clear(s, tx);
            return true;
        }
        if let Some(back) = tab_step(e, tx) {
            return self.step_tab(s, back, tx);
        }
        match e.key {
            Key::Enter if bare => {
                if let Some(item) = (self.get)(s).cursor_item() {
                    self.choose(s, item, tx);
                }
                true
            }
            // Down moves to the grid, one focus stop.
            Key::Down if bare => {
                if !(self.get)(s).shown.is_empty() {
                    tx.focus(self.grid_key(), true);
                }
                true
            }
            _ => false,
        }
    }

    /// Under the grid: the cursor's glyph and name, the query's matches lit in the name, and
    /// the keyword it matched when the name didn't ("grinning face · happy").
    fn name_line(&self, s: &S, look: &Look, m: &Metrics) -> El {
        let k = &look.kit;
        let st = (self.get)(s);
        let mut line = row()
            .key(format!("{}-name", self.key))
            .w_full()
            .h(m.tile)
            .gap(look.space.sm)
            .items_center()
            .min_w(0.0);
        let Some(item) = st.cursor_item() else {
            return line;
        };
        let source = st.source();
        let it = &source.items()[item];
        let hit = st.hit(item);
        let words = source.words(item);
        let ranges = hit
            .as_ref()
            .map(|h| name_ranges(&it.name, words, h))
            .unwrap_or_default();
        line = line.child(grid::glyph(look, m, &it.choice)).child(
            MatchedText::new(it.name.to_string(), ranges)
                .font(k.ty.ui)
                .truncate(true)
                .key(format!("{}-name-text", self.key))
                .build(k)
                .shrink()
                .min_w(0.0),
        );
        if let Some(Hit::Keyword(i, chars)) = hit
            && let Some(keyword) = it.keywords.get(i)
        {
            let lit = keyword_range(keyword, chars);
            let at = KEYWORD_MARK.len();
            line = line.child(
                MatchedText::new(
                    format!("{KEYWORD_MARK}{keyword}"),
                    std::iter::once(at + lit.start..at + lit.end),
                )
                .font(k.ty.ui)
                .color(k.c.text_muted)
                .truncate(true)
                .key(format!("{}-name-keyword", self.key))
                .build(k)
                .shrink()
                .min_w(0.0),
            );
        }
        line
    }
}

fn is_current(st: &IconPickerState, rcx: &RowCx<'_>) -> bool {
    st.current_item().is_some() && st.shown.item_of(rcx.entry.id) == st.current_item()
}

//! The search field, and what a host's settings window asks of the search while it draws.

use std::fmt;
use std::rc::Rc;

use rune::{Beat, El, Key, KeyEvent, LiveRegion, Sequence, TextStyle, Tx, el, icon, row, text};
use rune_kit::{ControlSize, IconButton, IconButtonVariant};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::index::SearchIndex;
use super::matching::title_ranges;
use super::state::{Change, FLASH_FOR, FLASH_HOLD, SettingsSearchState};
use crate::contract::{MatchedText, Point};
use crate::filter_bar::field::{self, Handlers, Spec};

/// The highlight's strength over a jumped-to row: the accent at 16 % (OWNER ruling).
pub const FLASH_ALPHA: f32 = 0.16;

const FLASH: &[Beat] = &[
    Beat::new(0.0),
    Beat::new(FLASH_HOLD),
    Beat::new(FLASH_FOR).opacity(0.0),
];

/// Where a search sends the host's settings window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Navigate {
    /// The page to show.
    pub page: String,
    /// The row a jump goes to (Enter in the field); `None` for a query that opens its best
    /// page. A host whose page has its own tabs or tables shows the one holding the row.
    pub row: Option<String>,
}

/// What the search says and shows.
#[derive(Clone)]
pub struct SearchLabels {
    /// The field's name and placeholder.
    pub field: String,
    /// The clear button's name (Strata's shared "Clear search").
    pub clear: String,
    /// The page title while a query found nothing.
    pub results_title: String,
    /// The no-results view's text, also heard as the status.
    pub no_match: String,
    /// The status heard while a query shows rows ("4 settings").
    pub count: Rc<dyn Fn(usize) -> String>,
}

impl fmt::Debug for SearchLabels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchLabels")
            .field("field", &self.field)
            .field("clear", &self.clear)
            .field("results_title", &self.results_title)
            .field("no_match", &self.no_match)
            .finish_non_exhaustive()
    }
}

impl Default for SearchLabels {
    fn default() -> Self {
        Self {
            field: "Search settings".into(),
            clear: "Clear search".into(),
            results_title: "Search results".into(),
            no_match: "No settings match your search.".into(),
            count: Rc::new(|n| match n {
                1 => "1 setting".into(),
                n => format!("{n} settings"),
            }),
        }
    }
}

type NavigateFn<S> = Rc<dyn Fn(&mut S, Navigate, &mut Tx)>;
type MenuFn<S> = Rc<dyn Fn(&mut S, Point, &mut Tx)>;
type EscapeFn<S> = Rc<dyn Fn(&mut S, &mut Tx) -> bool>;

/// The settings window's search: the field, and the filter its rows, headings, pages and
/// navigation follow. See the module docs for the host's part.
pub struct SettingsSearch<S> {
    key: String,
    get: fn(&S) -> &SettingsSearchState,
    get_mut: fn(&mut S) -> &mut SettingsSearchState,
    index: fn(&S) -> &SearchIndex,
    labels: Rc<SearchLabels>,
    page_scroll: Option<String>,
    on_navigate: Option<NavigateFn<S>>,
    on_menu: Option<MenuFn<S>>,
    on_escape: Option<EscapeFn<S>>,
}

impl<S> Clone for SettingsSearch<S> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            get: self.get,
            get_mut: self.get_mut,
            index: self.index,
            labels: self.labels.clone(),
            page_scroll: self.page_scroll.clone(),
            on_navigate: self.on_navigate.clone(),
            on_menu: self.on_menu.clone(),
            on_escape: self.on_escape.clone(),
        }
    }
}

impl<S: 'static> SettingsSearch<S> {
    pub fn new(
        key: impl Into<String>,
        get: fn(&S) -> &SettingsSearchState,
        get_mut: fn(&mut S) -> &mut SettingsSearchState,
        index: fn(&S) -> &SearchIndex,
    ) -> Self {
        Self {
            key: key.into(),
            get,
            get_mut,
            index,
            labels: Rc::new(SearchLabels::default()),
            page_scroll: None,
            on_navigate: None,
            on_menu: None,
            on_escape: None,
        }
    }

    pub fn labels(mut self, labels: SearchLabels) -> Self {
        self.labels = Rc::new(labels);
        self
    }

    /// The key of the scroll view the shown page scrolls in: each change of query scrolls it
    /// to its top, as Strata resets every page's scroll.
    pub fn page_scroll(mut self, key: impl Into<String>) -> Self {
        self.page_scroll = Some(key.into());
        self
    }

    /// Show a page: the best page for a new query, or a jump's page and row.
    pub fn on_navigate(mut self, f: impl Fn(&mut S, Navigate, &mut Tx) + 'static) -> Self {
        self.on_navigate = Some(Rc::new(f));
        self
    }

    /// A menu press on the field, at a window point: the host opens C21's `FieldMenu`.
    pub fn on_menu(mut self, f: impl Fn(&mut S, Point, &mut Tx) + 'static) -> Self {
        self.on_menu = Some(Rc::new(f));
        self
    }

    /// Escape in the empty field: the host closes Settings, returning true when it did. Rune's
    /// text editor takes an Escape a field passes on (it blurs the field), so the field hands
    /// this one to the host itself. Without it, Escape in the empty field leaves the field.
    pub fn on_escape(mut self, f: impl Fn(&mut S, &mut Tx) -> bool + 'static) -> Self {
        self.on_escape = Some(Rc::new(f));
        self
    }

    /// The text field's key.
    pub fn field_key(&self) -> String {
        format!("{}-field", self.key)
    }

    /// The clear button's key.
    pub fn clear_key(&self) -> String {
        format!("{}-clear", self.key)
    }

    /// The hidden status's key.
    pub fn status_key(&self) -> String {
        format!("{}-status", self.key)
    }

    /// The element key [`SettingsSearch::row`] gives the row keyed `row` in the index.
    pub fn row_key(&self, row: &str) -> String {
        format!("{}-row-{row}", self.key)
    }

    /// The no-results view's key.
    pub fn empty_key(&self) -> String {
        format!("{}-empty", self.key)
    }

    /// Search again if the query or the index changed: rows filter, and a new query opens its
    /// best page and scrolls it to the top. The field's edits call this; call it after
    /// [`SettingsSearchState::set_text`] or a `FieldMenu` edit. True when anything changed.
    pub fn refresh(&self, s: &mut S, tx: &mut Tx) -> bool {
        let Some(found) = (self.get)(s).plan((self.index)(s)) else {
            return false;
        };
        let change = (self.get_mut)(s).store(found);
        self.apply(s, change, tx)
    }

    fn apply(&self, s: &mut S, change: Change, tx: &mut Tx) -> bool {
        match change {
            Change::Rows => true,
            Change::Query(page) => {
                if let Some(scroll) = &self.page_scroll {
                    tx.scroll_to_instant(scroll.clone(), 0.0, 0.0);
                }
                if let (Some(page), Some(f)) = (page, &self.on_navigate) {
                    f(s, Navigate { page, row: None }, tx);
                }
                true
            }
        }
    }

    /// From the host's poll every frame, with the clock in seconds: searches a changed index
    /// and times the jump's highlight. True when the view changes.
    pub fn tick(&self, s: &mut S, now: f64, tx: &mut Tx) -> bool {
        let searched = self.refresh(s, tx);
        let flashed = (self.get_mut)(s).tick_flash(now, tx);
        searched || flashed
    }

    /// Enter: go to the best row. Its page shows, it scrolls into view, the keyboard moves to
    /// its first control and it is highlighted for [`FLASH_FOR`]. False when no query or no
    /// match leaves a row to go to. A query or index the host's tick hasn't searched yet is
    /// searched first: the matches name rows by position in the index.
    pub fn jump(&self, s: &mut S, tx: &mut Tx) -> bool {
        self.refresh(s, tx);
        let st = (self.get)(s);
        let Some((_, row, page)) = st.matches().and_then(|m| m.best) else {
            return false;
        };
        let index = (self.index)(s);
        let (Some(r), Some(p)) = (index.rows.get(row), index.pages.get(page)) else {
            return false;
        };
        let (row, page) = (r.key.clone(), p.key.clone());
        let target = self.row_key(&row);
        if let Some(f) = &self.on_navigate {
            f(
                s,
                Navigate {
                    page,
                    row: Some(row.clone()),
                },
                tx,
            );
        }
        // Probe P3 (C20c plan): a reveal in the Tx that shows the page lands on the new row.
        tx.scroll_into_view(target.clone());
        (self.get_mut)(s).flash(row, tx);
        // The page shows in this change, and the focus request resolves against it.
        tx.focus(target, tx.focus_visible() || tx.by_keys());
        true
    }

    /// Whether the row keyed `key` shows: every row while nothing filters, else the matches.
    /// A key the index doesn't list always shows, as Strata leaves untagged rows alone.
    pub fn shows_row(&self, s: &S, key: &str) -> bool {
        let st = (self.get)(s);
        let (Some(m), Some(p)) = (st.matches(), st.prepared()) else {
            return true;
        };
        p.row_by_key.get(key).is_none_or(|&i| m.rows[i])
    }

    /// Whether the heading of section `key` shows: while a row under it does, or while
    /// nothing filters. A section no indexed row is under always shows.
    pub fn shows_section(&self, s: &S, key: &str) -> bool {
        let st = (self.get)(s);
        let (Some(m), Some(p)) = (st.matches(), st.prepared()) else {
            return true;
        };
        let rows = &(self.index)(s).rows;
        let mut under = rows
            .iter()
            .filter(|r| r.section.as_deref() == Some(key))
            .peekable();
        under.peek().is_none()
            || under.any(|r| p.row_by_key.get(&r.key).is_some_and(|&i| m.rows[i]))
    }

    /// Whether the navigation lists page `key`: while it holds a shown row, or while nothing
    /// filters.
    pub fn shows_page(&self, s: &S, key: &str) -> bool {
        let st = (self.get)(s);
        let (Some(m), Some(p)) = (st.matches(), st.prepared()) else {
            return true;
        };
        p.page_by_key.get(key).is_some_and(|&i| m.pages[i])
    }

    /// The row keyed `key` in the index, given the host's `content`, or `None` while the
    /// search hides it. The row is keyed [`SettingsSearch::row_key`], for the jump's reveal and
    /// focus, and draws the jump's highlight behind its content.
    pub fn row(&self, s: &S, look: &Look, key: &str, content: El) -> Option<El> {
        if !self.shows_row(s, key) {
            return None;
        }
        let k = &look.kit;
        let el_key = self.row_key(key);
        let mut wrap = row()
            .key(el_key.clone())
            .w_full()
            .min_w(0.0)
            .equal_columns(1);
        if let Some(f) = (self.get)(s).flash.as_ref().filter(|f| f.row == key) {
            let mut glow = el()
                .key(format!("{el_key}-flash-{}", f.generation))
                .grid_cell(0)
                .radius(k.radius.md)
                .fill(k.alpha(k.c.text_accent, FLASH_ALPHA))
                .a11y_hidden();
            if look.animations {
                glow = glow.sequence(Sequence::new(FLASH));
            }
            wrap = wrap.child(glow);
        }
        Some(wrap.child(content.grid_cell(0)))
    }

    /// A row's title with the query's matched text in the accent. `text` is the title the
    /// host shows (the index row's [`super::IndexRow::title`]).
    pub fn title(&self, s: &S, look: &Look, text: &str, font: TextStyle) -> El {
        let query = (self.get)(s).query();
        let ranges = if query.is_empty() {
            Vec::new()
        } else {
            title_ranges(text, query)
        };
        MatchedText::new(text, ranges).font(font).build(&look.kit)
    }

    /// The view for a query that found nothing: "No settings match your search.". The host
    /// shows it in place of the page while [`SettingsSearchState::no_results`] holds, under the
    /// page title [`SettingsSearch::results_title`] (Strata's "Search results").
    pub fn empty(&self, look: &Look) -> El {
        let k = &look.kit;
        text(self.labels.no_match.clone())
            .key(self.empty_key())
            .font(k.ty.ui)
            .color(k.c.text_muted)
            .w_full()
            .wrap()
    }

    /// The page title while a query found nothing.
    pub fn results_title(&self) -> &str {
        &self.labels.results_title
    }

    /// The search field: a magnifier, the text, and a clear button while there is text. Escape
    /// with text clears it; the next goes to [`SettingsSearch::on_escape`], which closes
    /// Settings. Enter goes to the best row. A hidden status says how many rows the query shows.
    pub fn field(&self, s: &S, look: &Look) -> El {
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
                change.refresh(s, tx);
            }),
            menu: menu.clone(),
        };
        let spec = Spec {
            key: &self.field_key(),
            label: &self.labels.field,
            placeholder: Some(&self.labels.field),
            description: None,
            invalid: false,
            font: k.ty.ui,
        };
        let input = field::input(look, &st.field, &spec, &handlers).role("searchbox");
        let mut inner = field::frame_row(look, format!("{}-frame", self.key))
            .child(
                icon(look.icon(IconName::Search), look.icon_size.sm)
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
                    .label(self.labels.clear.clone())
                    .on_press_tx(move |s: &mut S, tx: &mut Tx| clear.clear(s, tx))
                    .build(k)
                    // The field stays the one Tab stop; a press keeps the keyboard in it.
                    .focusable(false)
                    .key(self.clear_key()),
            );
        }
        let heard = match st.count() {
            None => String::new(),
            Some(0) => self.labels.no_match.clone(),
            Some(n) => (self.labels.count)(n),
        };
        let status = el()
            .key(self.status_key())
            .role("status")
            .live_region(LiveRegion::Polite)
            .label(heard)
            .absolute(0.0, 0.0)
            .w(0.0)
            .h(0.0);
        field::frame(look, inner, st.field.focused, false, &menu).child(status)
    }

    fn clear(&self, s: &mut S, tx: &mut Tx) {
        (self.get_mut)(s).field.replace_text("");
        self.refresh(s, tx);
    }

    fn field_key_event(&self, s: &mut S, ev: &KeyEvent, tx: &mut Tx) -> bool {
        let m = ev.mods;
        match ev.key {
            // Strata's ESCAPE_ANY: any Escape clears a field with text.
            Key::Escape if !(self.get)(s).field.text.is_empty() => {
                self.clear(s, tx);
                true
            }
            Key::Escape => self.on_escape.as_ref().is_some_and(|f| f(s, tx)),
            Key::Enter if !(m.shift || m.ctrl || m.cmd || m.alt || m.logo) => self.jump(s, tx),
            _ => false,
        }
    }
}

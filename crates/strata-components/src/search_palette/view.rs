//! [`SearchPalette`]: the builder that puts the palette in C15's modal shell: the search bar,
//! the body (results or a status) and the footer.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;

use rune::{
    Align, Chord, Color, El, GrowthSpec, LiveRegion, PointerButton, PointerPress, ScrollSpec,
    TextStyle, Tx, col, el, icon, row, text,
};
use rune_kit::{Spinner, TableStyle};
use strata_kit::density::{PALETTE_LINE_GAP, PALETTE_RULE, PALETTE_THUMB};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::data::PaletteItem;
use super::intent::{PaletteIntent, Via};
use super::mode::{PaletteMode, ResultKind};
use super::model::Phase;
use super::state::{PaletteState, Polled};
use super::{
    BAR_GAP, BAR_HEIGHT, FOOTER_GAP, FOOTER_HEIGHT, HINT_GAP, HINT_ICON, PAD_X, PANEL_WIDTH,
    RESULTS_HEIGHT, ROW_GAP, STATUS_PAD_X, STATUS_PAD_Y, field, option_key, results_key,
};
use crate::contract::{MatchedText, Point};
use crate::dialog::DialogShell;
use crate::scroll::{BarLook, overlay};
use crate::thumbnails::{Step, ThumbnailStore, picture};

pub(super) type Emit<S> = Rc<dyn Fn(&mut S, PaletteIntent, &mut Tx)>;
type Visible<S> = Rc<dyn Fn(&mut S, Range<usize>) -> bool>;
/// Where thumbnails come from, and the step they are drawn at.
type Thumbs<S> = (fn(&S) -> &ThumbnailStore, Step);

/// What the result list reads beyond the parts' shared context.
struct List<S> {
    get: fn(&S) -> &PaletteState,
    thumbs: Option<Thumbs<S>>,
    on_visible: Option<Visible<S>>,
}

/// Hands each of `polled`'s intents to the host.
pub(super) fn route<S>(emit: &Emit<S>, s: &mut S, polled: Polled, tx: &mut Tx) {
    for intent in polled.intents {
        emit(s, intent, tx);
    }
}

/// What the parts share.
pub(super) struct Ctx<S> {
    pub key: String,
    pub access: fn(&mut S) -> &mut PaletteState,
    pub emit: Emit<S>,
    pub new_tab: Rc<[Chord]>,
    pub reveal: Rc<[Chord]>,
}

/// The palette: Ctrl+K search, Ctrl+Shift+K folder jump, or any other [`PaletteMode`], over
/// C15's modal shell. See the module docs for hosting it.
///
/// Keys: `{key}-dialog` (role `dialog`, named by the mode's title), `{key}-combobox` (the
/// group around the field), [`field_key`](super::field_key), `{key}-busy` (the spinner), [`results_key`] (the
/// listbox), [`option_key`](super::option_key) (each row, by place), `{key}-status` (the status, a polite live region,
/// hidden while results show, when it says their count), `{key}-partial`, `{key}-footer`.
pub struct SearchPalette<S> {
    key: String,
    get: fn(&S) -> &PaletteState,
    access: fn(&mut S) -> &mut PaletteState,
    new_tab: Vec<Chord>,
    reveal: Vec<Chord>,
    thumbs: Option<Thumbs<S>>,
    on_visible: Option<Visible<S>>,
    return_focus_on_open: Option<String>,
    reduced_motion: bool,
    emit: Emit<S>,
}

impl<S: 'static> SearchPalette<S> {
    /// The palette keyed `key` over the state `get` and `access` reach. Opening a result
    /// takes Enter, and opening it in a new tab Ctrl+Enter ([`Self::new_tab_keys`]); its
    /// folder, Alt+Enter ([`Self::reveal_keys`]).
    pub fn new(
        key: impl Into<String>,
        get: fn(&S) -> &PaletteState,
        access: fn(&mut S) -> &mut PaletteState,
    ) -> Self {
        Self {
            key: key.into(),
            get,
            access,
            new_tab: vec![Chord::parse("mod+enter").expect("a chord")],
            reveal: vec![Chord::parse("alt+enter").expect("a chord")],
            thumbs: None,
            on_visible: None,
            return_focus_on_open: None,
            reduced_motion: false,
            emit: Rc::new(|_, _, _| {}),
        }
    }

    /// The chords that open the highlighted result in a new tab: the host's `open_in_new_tab`
    /// binding (Strata's default, Ctrl+Enter). Shift held too still matches, as in Strata.
    pub fn new_tab_keys(mut self, chords: Vec<Chord>) -> Self {
        self.new_tab = chords;
        self
    }

    /// The chords that open the highlighted result's folder: the host's
    /// `open_containing_folder` binding (Alt+Enter). Super held too still matches.
    pub fn reveal_keys(mut self, chords: Vec<Chord>) -> Self {
        self.reveal = chords;
        self
    }

    /// Draws each result's thumbnail ([`PaletteItem::thumb`]) from `store`, at `step` (see
    /// [`Step::for_tile`] with [`PALETTE_THUMB`]), over its icon. Folders-only modes draw icons.
    pub fn thumbnails(mut self, store: fn(&S) -> &ThumbnailStore, step: Step) -> Self {
        self.thumbs = Some((store, step));
        self
    }

    /// Called with the rows visible in the list (Rune's `on_visible_rows`): sync a
    /// [`ThumbnailStore`] viewer here. True when it changed state the view reads.
    pub fn on_visible(mut self, f: impl Fn(&mut S, Range<usize>) -> bool + 'static) -> Self {
        self.on_visible = Some(Rc::new(f));
        self
    }

    /// On opening a result, focus goes to the element keyed `key` (the revealed item's view)
    /// instead of back to the palette's opener, as Strata hands it to the browser. Escape,
    /// the toggle and a click outside still return it to the opener.
    pub fn return_focus_on_open(mut self, key: impl Into<String>) -> Self {
        self.return_focus_on_open = Some(key.into());
        self
    }

    /// Opens and closes at once while true: pass `cx.is_reduced_motion()` (C15's rule).
    pub fn reduced_motion(mut self, reduced: bool) -> Self {
        self.reduced_motion = reduced;
        self
    }

    /// Where the palette's intents go.
    pub fn on_intent(mut self, f: impl Fn(&mut S, PaletteIntent, &mut Tx) + 'static) -> Self {
        self.emit = Rc::new(f);
        self
    }

    /// Pushes the palette onto `layer`, the overlays `rune_kit::root` mounts, while open.
    pub fn build(self, look: &Look, s: &S, layer: &mut Vec<El>) {
        let state = (self.get)(s);
        if !state.is_open() {
            return;
        }
        state.handler.set(self.emit.clone());
        if *state.key.borrow() != self.key {
            state.key.replace(self.key.clone());
        }
        if *state.return_to.borrow() != self.return_focus_on_open {
            state.return_to.replace(self.return_focus_on_open.clone());
        }
        let cx = Ctx {
            key: self.key.clone(),
            access: self.access,
            emit: self.emit.clone(),
            new_tab: self.new_tab.into(),
            reveal: self.reveal.into(),
        };
        let mode = state.mode();
        let title = mode.map_or(String::new(), |m| m.title.clone());
        let access = self.access;
        let emit = self.emit.clone();
        let mut shell = DialogShell::new(self.key.clone(), title, |_: &mut S| {})
            .on_dismiss_tx(move |s: &mut S, tx: &mut Tx| {
                let polled = access(s).dismissed();
                route(&emit, s, polled, tx);
            })
            .open(true)
            .width(PANEL_WIDTH)
            .reduced_motion(self.reduced_motion);
        if let Some(mode) = mode {
            let phase = state.phase().unwrap_or(Phase::Results);
            let editable = !matches!(phase, Phase::Unavailable(_));
            shell = shell
                .child(bar(&cx, look, state, mode, &phase, editable))
                .child(body(
                    &cx,
                    look,
                    state,
                    mode,
                    &phase,
                    List {
                        get: self.get,
                        thumbs: self.thumbs,
                        on_visible: self.on_visible,
                    },
                ))
                .child(footer(&cx, look, state, mode));
        }
        shell.build(look, layer);
    }
}

/// The search bar: the search icon, the field in its combobox group, the spinner.
fn bar<S: 'static>(
    cx: &Ctx<S>,
    look: &Look,
    state: &PaletteState,
    mode: &PaletteMode,
    phase: &Phase,
    editable: bool,
) -> El {
    let k = &look.kit;
    // A text input is always a text box to Rune's accessibility (U-C6-1), so the combobox is
    // the group around it, expanded while results show.
    let combobox = row()
        .key(format!("{}-combobox", cx.key))
        .role("combobox")
        .label(mode.field_label.clone())
        .expanded(*phase == Phase::Results)
        .grow(1.0)
        .min_w(0.0)
        .items_center()
        .child(field::field(cx, look, state, mode, editable));
    let mut bar = row()
        .key(format!("{}-bar", cx.key))
        .w_full()
        .h(BAR_HEIGHT)
        .px(PAD_X)
        .gap(BAR_GAP)
        .items_center()
        .child(
            icon(look.icon(IconName::Search), look.icon_size.lg)
                .color(k.c.icon_accent)
                .a11y_hidden(),
        )
        .child(combobox);
    if state.busy() {
        bar = bar.child(
            el().key(format!("{}-busy", cx.key))
                .role("progressbar")
                .label(mode.busy.clone())
                // The wrapper carries the name; the spinner itself says nothing more.
                .child(
                    Spinner::new()
                        .size(look.icon_size.md)
                        .color(k.c.icon_accent)
                        .build(k)
                        .a11y_hidden(),
                ),
        );
    }
    col().w_full().child(bar).child(rule(k.c.border_base))
}

/// A 1 pt rule across the panel.
fn rule(color: Color) -> El {
    el().w_full().h(1.0).fill(color)
}

/// The body: the results, or the state's words.
fn body<S: 'static>(
    cx: &Ctx<S>,
    look: &Look,
    state: &PaletteState,
    mode: &PaletteMode,
    phase: &Phase,
    list: List<S>,
) -> El {
    let k = &look.kit;
    let frame = col()
        .key(format!("{}-body", cx.key))
        .w_full()
        .h(RESULTS_HEIGHT)
        .shrink()
        .min_h(0.0);
    let status_key = format!("{}-status", cx.key);
    let Some(words) = phase.text() else {
        // The count assistive technology hears: hidden, as Strata shows none, and polite.
        // Nothing until the query's first answer: the stale list's size isn't its count.
        let heard = state.announced().map(mode.count).unwrap_or_default();
        let count = el()
            .key(status_key)
            .role("status")
            .live_region(LiveRegion::Polite)
            .label(heard)
            .absolute(0.0, 0.0)
            .w(0.0)
            .h(0.0);
        return frame
            .child(results(cx, look, state, mode, list))
            .child(count);
    };
    let mut lines = col()
        .key(status_key)
        .role("status")
        .live_region(LiveRegion::Polite)
        .label(words.to_owned())
        .w_full()
        .min_w(0.0)
        .items_center()
        .gap(look.space.xxs);
    for line in words.lines() {
        lines = lines.child(
            text(line.to_owned())
                .font(k.ty.ui)
                .color(k.c.text_weak)
                .wrap()
                .align(Align::Center)
                .a11y_hidden(),
        );
    }
    frame.child(
        col()
            .w_full()
            .h_full()
            .min_h(0.0)
            .px(STATUS_PAD_X)
            .py(STATUS_PAD_Y)
            .center()
            .child(lines),
    )
}

/// The result list: virtual rows built by index, each a memo keyed by its result.
fn results<S: 'static>(
    cx: &Ctx<S>,
    look: &Look,
    state: &PaletteState,
    mode: &PaletteMode,
    list: List<S>,
) -> El {
    let List {
        get,
        thumbs,
        on_visible,
    } = list;
    let rows_key = format!("{}-rows", cx.key);
    let viewport_key = results_key(&cx.key);
    let shared = Rc::new(Rows {
        key: cx.key.clone(),
        look: look.clone(),
        look_key: look_key(look),
        kind: mode.kind,
        get,
        access: cx.access,
        thumbs: thumbs.filter(|_| mode.kind == ResultKind::Files),
    });
    let (key_rows, build_rows) = (shared.clone(), shared);
    let mut list = col()
        .key(rows_key)
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows(
            state.items().len(),
            look.row.palette,
            move |_: &S, i| option_key(&key_rows.key, i).into(),
            move |s: &S, i| build_rows.row(s, i),
        )
        // Keyed by place (see `option_key`), so the keys never change for a given count. How
        // the list scrolls as answers land is `PaletteState::deliver`'s; arrows reveal their
        // row from the field's key handler. (`reveal_selected` does nothing for rows built by
        // index at the pin: Rune draft U-C17-4.)
        .virtual_rows_version(cx.key.as_str());
    if let Some(f) = on_visible {
        list = list.on_visible_rows(move |s: &mut S, r| f(s, r));
    }
    let viewport = col()
        .key(viewport_key.clone())
        .role("listbox")
        .label(mode.list_label.clone())
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .on_scroll_offset({
            let (bar, feed) = (state.bar.watch(), state.feed.watch());
            move |p| {
                bar(p);
                feed(p);
            }
        })
        // Rows keep the field's keyboard when pressed.
        .preserve_focus_on_press(true)
        .child(list);
    let bar = state.bar.build::<S>(
        &format!("{}-scrollbar", cx.key),
        &viewport_key,
        BarLook::app(look),
    );
    state.bar.area(overlay(viewport, [bar]))
}

/// What a row reads, shared by the list's key and build closures.
struct Rows<S> {
    key: String,
    look: Look,
    look_key: u64,
    kind: ResultKind,
    get: fn(&S) -> &PaletteState,
    access: fn(&mut S) -> &mut PaletteState,
    thumbs: Option<Thumbs<S>>,
}

impl<S: 'static> Rows<S> {
    fn row(&self, s: &S, i: usize) -> El {
        let state = (self.get)(s);
        let item = &state.items()[i];
        let active = state.highlighted() == Some(i);
        let thumb = self
            .thumbs
            .and_then(|(store, step)| Some((store(s), step, item.thumb?)))
            .filter(|(store, _, _)| store.enabled());
        let mut h = DefaultHasher::new();
        item.hash(&mut h);
        active.hash(&mut h);
        self.look_key.hash(&mut h);
        if let Some((store, step, key)) = thumb {
            store.version(key).hash(&mut h);
            step.hash(&mut h);
        }
        // The memo follows the result, not its place, so a row a batch moves isn't built again.
        let memo = format!("{}-result-{}", self.key, item.id);
        rune::memo::memo_with_hash(memo, h.finish(), || {
            let pic = thumb.map(|(store, _, key)| store.thumb(key));
            result_row(self, item, active, pic.as_ref(), &state.handler)
        })
    }
}

/// One result: the icon or thumbnail, the name over the full path, matches in the accent.
fn result_row<S: 'static>(
    rows: &Rows<S>,
    item: &PaletteItem,
    active: bool,
    thumb: Option<&crate::thumbnails::Thumb>,
    handler: &super::state::HandlerSlot,
) -> El {
    #[cfg(test)]
    super::tests::ROW_BUILDS.with(|n| n.set(n.get() + 1));
    let look = &rows.look;
    let k = &look.kit;
    let access = rows.access;
    let glyph = match (rows.kind, item.icon) {
        (ResultKind::Folders, IconName::File) => IconName::Folder,
        (_, name) => name,
    };
    let icon_el = icon(look.icon(glyph), look.icon_size.lg)
        .color(k.c.icon_muted)
        .a11y_hidden();
    let pic = match thumb {
        Some(t) => picture(t, icon_el, PALETTE_THUMB, PALETTE_THUMB),
        None => row()
            .w(PALETTE_THUMB)
            .h(PALETTE_THUMB)
            .center()
            .a11y_hidden()
            .child(icon_el),
    };
    let name_font = TextStyle {
        weight: k.ty.ui_medium.weight,
        ..k.ty.ui_compact
    };
    let name = MatchedText::new(item.label.clone(), item.matched.iter().cloned())
        .font(name_font)
        .color(k.c.text_base)
        .truncate(true)
        .build(k)
        .a11y_hidden();
    let path = MatchedText::new(item.detail.clone(), item.detail_matched.iter().cloned())
        .font(k.ty.small)
        .color(if active {
            k.c.text_muted
        } else {
            k.c.text_weak
        })
        .build(k)
        .truncate_middle()
        .a11y_hidden();
    // The press handlers sit on an inner element keyed by the result, inside the row keyed by
    // place. Rune resolves a release by the pressed element's path and drops it when that path
    // is gone, so a press held while a batch moves another result into the row (the row's key
    // stays, the inner key changes) opens nothing and asks for no menu, for every button, as
    // GTK's ListBox activates only when press and release land on the same row. Rune has no
    // press-down hook for the right button to record the pressed result with instead.
    let on_press = {
        let (press, open_id) = (handler.clone(), item.id.clone());
        move |s: &mut S, tx: &mut Tx| {
            let middle = tx.button() == PointerButton::Middle;
            let via = if middle { Via::Middle } else { Via::Click };
            let polled = access(s).choose(&open_id, middle, via, tx);
            if let Some(emit) = press.get::<Emit<S>>() {
                route(&emit, s, polled, tx);
            }
        }
    };
    let on_menu = {
        let (menu, menu_id) = (handler.clone(), item.id.clone());
        move |s: &mut S, p: PointerPress, tx: &mut Tx| {
            let polled = access(s).menu(&menu_id, Point::new(p.x, p.y));
            if let Some(emit) = menu.get::<Emit<S>>() {
                route(&emit, s, polled, tx);
            }
        }
    };
    let selected = TableStyle::from_kit(k).selected;
    let mut content = row()
        .w_full()
        .h(look.row.palette - PALETTE_RULE)
        .px(PAD_X)
        .gap(ROW_GAP)
        .items_center()
        .child(pic)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(PALETTE_LINE_GAP)
                .child(name)
                .child(path),
        );
    if active {
        content = content.fill(selected).outline(1.0, -1.0, k.c.border_focus);
    }
    let pressable = col()
        .key(format!("result-{}", escape(&item.id)))
        // A layout box to assistive technology: the option around it is the row.
        .role("none")
        .w_full()
        .h_full()
        // A tint only: the pointer never moves the keyboard's highlight. Rune reports a hover
        // when a row scrolls under a still pointer (C6's review round 1, B1).
        .hover(k.c.hover)
        .preserve_focus_on_press(true)
        .pointer_buttons([PointerButton::Primary, PointerButton::Middle])
        .on_press_tx(on_press.clone())
        .on_secondary_press(on_menu.clone())
        .child(content)
        .child(rule(k.c.border_weak));
    col()
        .role("option")
        // Strata names a row by its full path.
        .label(item.detail.clone())
        .focus_selected(active)
        .w_full()
        .h(look.row.palette)
        .preserve_focus_on_press(true)
        // Assistive technology's click and menu actions reach the option, not the inner box;
        // the pointer reaches the inner box first.
        .on_press_tx(on_press)
        .on_secondary_press(on_menu)
        .child(pressable)
}

/// `id` made safe as a key segment: Rune reads `/` in a key as a key path, so `/` and `%`
/// (to keep keys apart) are escaped.
fn escape(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for c in id.chars() {
        match c {
            '%' => out.push_str("%25"),
            '/' => out.push_str("%2F"),
            c => out.push(c),
        }
    }
    out
}

/// The footer: the key hints, then the partial-results notice at its end.
fn footer<S: 'static>(cx: &Ctx<S>, look: &Look, state: &PaletteState, mode: &PaletteMode) -> El {
    let k = &look.kit;
    let mut hints = row()
        .key(format!("{}-footer", cx.key))
        .w_full()
        .h(FOOTER_HEIGHT)
        .px(PAD_X)
        .gap(FOOTER_GAP)
        .items_center()
        .fill(k.c.surface_weak);
    for hint in &mode.hints {
        let mut h = row().gap(HINT_GAP).items_center().min_w(0.0).shrink();
        if let Some(name) = hint.icon {
            h = h.child(
                icon(look.icon(name), HINT_ICON)
                    .color(k.c.icon_muted)
                    .a11y_hidden(),
            );
        }
        hints = hints.child(
            h.child(
                text(hint.text.clone())
                    .font(k.ty.small)
                    .color(k.c.text_muted)
                    .truncate(),
            ),
        );
    }
    if let Some(causes) = state.partial() {
        hints = hints.child(el().grow(1.0)).child(
            row()
                .key(format!("{}-partial", cx.key))
                .role("note")
                .label(mode.partial.clone())
                .description(causes.to_owned())
                .help(causes.to_owned())
                .gap(HINT_GAP)
                .items_center()
                .child(
                    icon(look.icon(IconName::Warning), HINT_ICON + 1.0)
                        .color(k.c.text_accent)
                        .a11y_hidden(),
                )
                .child(
                    text(mode.partial.clone())
                        .font(k.ty.small)
                        .color(k.c.text_accent)
                        .a11y_hidden(),
                ),
        );
    }
    col().w_full().child(rule(k.c.border_base)).child(hints)
}

/// Everything in the look a row reads, as a memo version.
fn look_key(look: &Look) -> u64 {
    let mut h = DefaultHasher::new();
    look.kit.c.hash(&mut h);
    look.kit.ty.hash(&mut h);
    look.space.hash(&mut h);
    look.icon_size.hash(&mut h);
    look.row.hash(&mut h);
    look.icons.name.hash(&mut h);
    (look.icons.icon as usize).hash(&mut h);
    h.finish()
}

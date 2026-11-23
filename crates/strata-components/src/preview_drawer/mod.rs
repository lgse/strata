// SPDX-License-Identifier: MIT

mod divider;
mod find;
mod header;
mod hosted;
mod layout;
mod state;

#[cfg(test)]
mod tests;

use std::rc::Rc;

use rune::{Cx, Drag, El, Icon, KeyEvent, Phase, Transition, Tx, col, el, row, text};
use strata_kit::look::Look;

use crate::contract::{Dnd, DragSource};

pub use hosted::{DocumentView, FindStatus, HostedPreview, NoPreview};
pub use layout::{Arrangement, ColumnsFit, DrawerRules, RightPane, Solved, right_pane, solve};
pub use state::{DrawerState, FIND_MAX_CHARS, FindBar, SLIDE_SECONDS};

pub const CONTENT_MAX_WIDTH: f32 = 1280.0;

/// The split's key: its width is what the rules solve against.
pub const SPLIT_KEY: &str = "preview-split";

/// What the drawer tells the host. The host applies Strata's policy to each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DrawerIntent {
    Closed,
    /// Open in the default application.
    Open,
    /// The wrap toggle, to its new value.
    SetWrap(bool),
    /// The Rendered/Source switch, to the form it offers.
    ShowView(DocumentView),
    Suspend(bool),
    /// A plain primary press landed in the empty reserved slot beside the columns.
    EmptySlotPressed,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DrawerLabels {
    pub region: String,
    pub close: String,
    pub open: String,
    pub wrap: String,
    pub view_source: String,
    pub view_rendered: String,
    pub find: String,
    pub find_field: String,
    pub find_previous: String,
    pub find_next: String,
    pub find_close: String,
    pub no_matches: String,
    /// "Match 1 of 3".
    pub match_of: fn(usize, usize) -> String,
    /// The divider's name and the caption over it.
    pub divider: String,
    /// Accessible descriptions for the three header detail labels (`Size`, `Modified`, `Type`).
    pub detail_descriptions: [String; 3],
}

impl Default for DrawerLabels {
    fn default() -> Self {
        DrawerLabels {
            region: "Preview".into(),
            close: "Close preview (Space)".into(),
            open: "Open in default application".into(),
            wrap: "Toggle word wrap".into(),
            view_source: "View source".into(),
            view_rendered: "View rendered".into(),
            find: "Find in preview (Ctrl+F)".into(),
            find_field: "Find in preview".into(),
            find_previous: "Previous match".into(),
            find_next: "Next match".into(),
            find_close: "Close find".into(),
            no_matches: "No matches".into(),
            match_of: |n, total| format!("Match {n} of {total}"),
            divider: "Preview panel minimum width".into(),
            detail_descriptions: ["Size".into(), "Modified".into(), "Type".into()],
        }
    }
}

#[derive(Clone, Default)]
pub struct HeaderInfo {
    pub icon: Option<&'static Icon>,
    pub title: String,
    pub details: Vec<String>,
}

impl HeaderInfo {
    pub fn new(title: impl Into<String>) -> Self {
        HeaderInfo {
            title: title.into(),
            ..Self::default()
        }
    }
    pub fn icon(mut self, icon: &'static Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn details<T: Into<String>>(mut self, details: impl IntoIterator<Item = T>) -> Self {
        self.details = details.into_iter().map(Into::into).collect();
        self
    }
}

/// Ctrl+F (⌘F on macOS), the default key map's find in preview.
pub fn default_find_keys(ev: &KeyEvent) -> bool {
    ev.is_command('f') && !ev.mods.shift && !ev.mods.alt
}

/// What the slot holds while the drawer is open.
enum Body {
    Empty,
    Placeholder(String),
    View(Box<El>),
}

/// What the hosted view offers the header, read once per build.
#[derive(Clone, Debug, Default)]
pub(crate) struct Caps {
    pub view: Option<DocumentView>,
    pub wrap: Option<bool>,
    pub searchable: bool,
    pub status: Option<FindStatus>,
    pub busy: Option<String>,
}

type HostedAccess<S> = Rc<dyn Fn(&mut S) -> &mut dyn HostedPreview>;
type IntentFn<S> = Rc<dyn Fn(&mut S, DrawerIntent, &mut Tx)>;
type BackgroundDragFn<S> = Rc<dyn Fn(&mut S, &mut Drag, bool, &mut Tx)>;

/// What the parts of the drawer share while they build.
pub(crate) struct Shared<S> {
    pub access: fn(&mut S) -> &mut DrawerState,
    pub hosted: Option<HostedAccess<S>>,
    pub intent: Option<IntentFn<S>>,
    pub background_drag: Option<BackgroundDragFn<S>>,
    pub labels: Rc<DrawerLabels>,
    pub rules: DrawerRules,
    pub animations: bool,
}

impl<S> Clone for Shared<S> {
    fn clone(&self) -> Self {
        Shared {
            access: self.access,
            hosted: self.hosted.clone(),
            intent: self.intent.clone(),
            background_drag: self.background_drag.clone(),
            labels: self.labels.clone(),
            rules: self.rules,
            animations: self.animations,
        }
    }
}

impl<S: 'static> Shared<S> {
    pub fn emit(&self, s: &mut S, intent: DrawerIntent, tx: &mut Tx) {
        if let Some(f) = &self.intent {
            f(s, intent, tx);
        }
    }
    pub fn with_view(&self, s: &mut S, f: impl FnOnce(&mut dyn HostedPreview)) {
        if let Some(access) = &self.hosted {
            f(access(s));
        }
    }
}

pub struct PreviewDrawer<'a, S> {
    state: &'a DrawerState,
    access: fn(&mut S) -> &mut DrawerState,
    arrangement: Arrangement,
    rules: DrawerRules,
    header: HeaderInfo,
    caps: Caps,
    hosted: Option<HostedAccess<S>>,
    body: Body,
    focused: bool,
    open_allowed: bool,
    labels: Rc<DrawerLabels>,
    files_key: String,
    find_keys: fn(&KeyEvent) -> bool,
    drag: Option<(Dnd<S>, DragSource<S>)>,
    intent: Option<IntentFn<S>>,
    background_drag: Option<BackgroundDragFn<S>>,
}

impl<'a, S: 'static> PreviewDrawer<'a, S> {
    /// The drawer for `state`, which `access` finds in the app's state.
    pub fn new(state: &'a DrawerState, access: fn(&mut S) -> &mut DrawerState) -> Self {
        PreviewDrawer {
            state,
            access,
            arrangement: Arrangement::List,
            rules: DrawerRules::STRATA,
            header: HeaderInfo::default(),
            caps: Caps::default(),
            hosted: None,
            body: Body::Empty,
            focused: false,
            open_allowed: true,
            labels: Rc::new(DrawerLabels::default()),
            files_key: "files".into(),
            find_keys: default_find_keys,
            drag: None,
            intent: None,
            background_drag: None,
        }
    }

    /// The view beside the drawer (List unless set).
    pub fn arrangement(mut self, a: Arrangement) -> Self {
        self.arrangement = a;
        self
    }

    pub fn rules(mut self, rules: DrawerRules) -> Self {
        self.rules = rules;
        self
    }

    pub fn header(mut self, header: HeaderInfo) -> Self {
        self.header = header;
        self
    }

    /// The hosted view's state, which decides the header's controls and answers find.
    pub fn hosted<V: HostedPreview + 'static>(
        mut self,
        view: &V,
        access: fn(&mut S) -> &mut V,
    ) -> Self {
        self.caps = Caps {
            view: view.document_view(),
            wrap: view.wrap(),
            searchable: view.searchable(),
            status: view.find_status(),
            busy: view.busy().map(str::to_owned),
        };
        self.hosted = Some(Rc::new(move |s: &mut S| -> &mut dyn HostedPreview {
            access(s)
        }));
        self
    }

    /// The hosted view's element.
    pub fn content(mut self, el: El) -> Self {
        self.body = Body::View(Box::new(el));
        self
    }

    /// A message instead of a view ("No preview for this selection").
    pub fn placeholder(mut self, message: impl Into<String>) -> Self {
        self.body = Body::Placeholder(message.into());
        self
    }

    pub fn focused(mut self, on: bool) -> Self {
        self.focused = on;
        self
    }

    /// Whether the header offers Open (the chooser doesn't).
    pub fn open_allowed(mut self, on: bool) -> Self {
        self.open_allowed = on;
        self
    }

    pub fn labels(mut self, labels: DrawerLabels) -> Self {
        self.labels = Rc::new(labels);
        self
    }

    /// The key of the files' focusable element (or an ancestor of it) that focus returns to.
    pub fn files_key(mut self, key: impl Into<String>) -> Self {
        self.files_key = key.into();
        self
    }

    pub fn find_keys(mut self, f: fn(&KeyEvent) -> bool) -> Self {
        self.find_keys = f;
        self
    }

    pub fn drag_source(mut self, dnd: Dnd<S>, source: DragSource<S>) -> Self {
        self.drag = Some((dnd, source));
        self
    }

    pub fn on_intent(mut self, f: impl Fn(&mut S, DrawerIntent, &mut Tx) + 'static) -> Self {
        self.intent = Some(Rc::new(f));
        self
    }

    pub fn on_background_drag(
        mut self,
        f: impl Fn(&mut S, &mut Drag, bool, &mut Tx) + 'static,
    ) -> Self {
        self.background_drag = Some(Rc::new(f));
        self
    }

    /// The split: `files`, then the divider and the drawer's slot.
    pub fn build(self, files: El, look: &Look, cx: &mut Cx<S>) -> El {
        let PreviewDrawer {
            state: st,
            access,
            arrangement,
            rules,
            header,
            caps,
            hosted,
            body,
            focused,
            open_allowed,
            labels,
            files_key,
            find_keys,
            drag,
            intent,
            background_drag,
        } = self;
        let available = cx.layout_with(SPLIT_KEY, |r| r[2]).unwrap_or(0.0);
        let solved = solve(&rules, &arrangement, available, st.manual_width());
        let columns = matches!(arrangement, Arrangement::Columns(_));
        let has_body = !matches!(body, Body::Empty);
        // The pane shows while the drawer is open, has something to show, and fits.
        let shown = st.is_open() && has_body && solved.fits;
        let slot_width = compute_slot_width(solved, rules.line, shown, (columns, st));
        let shared = Shared {
            access,
            hosted,
            intent,
            background_drag,
            labels,
            rules,
            animations: look.animations,
        };

        let mut slot = el()
            .key("preview-slot")
            .w(slot_width)
            .h_full()
            .clip_soft(0.0)
            .fill(look.kit.c.bg_base);
        if shown {
            let parts = Parts {
                header: &header,
                caps: &caps,
                focused,
                open_allowed,
                // A placeholder has nothing to open, so its Open button stays visible and disabled.
                open_disabled: matches!(body, Body::Placeholder(_)),
                drag: drag.as_ref(),
                find_keys,
            };
            slot = slot
                .child(pane(&shared, st, solved, body, parts, look))
                .child(divider::divider(&shared, st, solved, columns, look))
                .child(stub("preview-open-stub".into(), &files_key));
        } else {
            slot = empty_reserved_slot(
                slot,
                &shared,
                EmptySlot {
                    width: slot_width,
                    lit: st.lit,
                },
                look,
            );
        }
        slot = append_slot_markers(
            slot,
            &shared,
            st,
            SlotMarkers {
                shown,
                has_body,
                fits: solved.fits,
                searchable: caps.searchable,
            },
        );

        let mut split = row()
            .key(SPLIT_KEY)
            .w_full()
            .h_full()
            .min_w(0.0)
            .child(
                el().key("preview-files")
                    .grow(1.0)
                    .min_w(0.0)
                    .h_full()
                    .clip_soft(0.0)
                    .child(files),
            )
            .child(slot)
            .children(focus_stubs(st, &files_key));
        if shown {
            split = split.children(divider::overlays(
                &shared, st, solved, available, slot_width, columns, look,
            ));
        }
        split
    }
}

struct SlotMarkers {
    shown: bool,
    has_body: bool,
    fits: bool,
    searchable: bool,
}

fn hidden_state_needs_reset(st: &DrawerState) -> bool {
    st.dragging || st.hint || st.find.query.focused
}

fn find_bar_unmounted(st: &DrawerState, searchable: bool) -> bool {
    st.find.query.focused && !(st.find.open && searchable)
}

fn append_slot_markers<S: 'static>(
    mut slot: El,
    shared: &Shared<S>,
    st: &DrawerState,
    m: SlotMarkers,
) -> El {
    if st.is_open() && m.has_body {
        slot = slot.child(fit_marker(shared, st, m.fits));
    }
    if !m.shown && hidden_state_needs_reset(st) {
        slot = slot.child(hidden_marker(shared, st));
    } else if find_bar_unmounted(st, m.searchable) {
        slot = slot.child(find_focus_marker(shared, st));
    }
    if st.find_end_pending {
        slot = slot.child(find_end_marker(shared, st));
    }
    slot
}

fn focus_stubs(st: &DrawerState, files_key: &str) -> [El; 3] {
    [
        stub(
            format!("preview-focus-content-{}", st.focus_content),
            BODY_KEY,
        ),
        stub(format!("preview-focus-files-{}", st.focus_files), files_key),
        stub(
            format!("preview-focus-divider-{}", st.focus_divider),
            divider::GRIP_KEY,
        ),
    ]
}

fn compute_slot_width(
    solved: Solved,
    line: f32,
    shown: bool,
    (columns, st): (bool, &DrawerState),
) -> f32 {
    let keeps_slot = st.is_reserved() || st.is_open();
    if shown {
        solved.width + line
    } else if columns && keeps_slot {
        solved.empty_slot
    } else {
        0.0
    }
}

/// The content area's key: where [`DrawerState::focus_content`] sends focus.
pub const BODY_KEY: &str = "preview-body";

/// The header's inputs, borrowed from the builder.
pub(crate) struct Parts<'b, S> {
    pub header: &'b HeaderInfo,
    pub caps: &'b Caps,
    pub focused: bool,
    pub open_allowed: bool,
    pub open_disabled: bool,
    pub drag: Option<&'b (Dnd<S>, DragSource<S>)>,
    pub find_keys: fn(&KeyEvent) -> bool,
}

pub(super) fn slide_exit(animations: bool) -> Transition {
    if animations {
        Transition::identity().springs(rune::SpringPair {
            enter: rune::Spring::NONE,
            exit: rune::Spring::new(SLIDE_SECONDS, 0.0),
        })
    } else {
        Transition::none()
    }
}

fn pane<S: 'static>(
    shared: &Shared<S>,
    st: &DrawerState,
    solved: Solved,
    body: Body,
    parts: Parts<'_, S>,
    look: &Look,
) -> El {
    let k = &look.kit;
    let body = match body {
        Body::View(el) => *el,
        Body::Placeholder(message) => col().w_full().h_full().center().child(
            text(message)
                .key("preview-placeholder")
                .font(k.ty.ui)
                .color(k.c.text_muted),
        ),
        Body::Empty => el(),
    };
    let exit = slide_exit(shared.animations);
    let keys = shared.clone();
    let find_keys = parts.find_keys;
    let searchable = parts.caps.searchable;
    let mut drawer_pane = col()
        .key("preview-drawer")
        .role("region")
        .label(shared.labels.region.clone())
        .w(solved.pane)
        .h_full()
        .pin(Some(shared.rules.line), Some(0.0), None, Some(0.0))
        .clip_soft(0.0)
        .preserve_focus_on_press(true)
        .transition(exit)
        .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            drawer_key(&keys, s, ev, tx, find_keys, searchable)
        });
    if let Some(bg) = shared.background_drag.clone() {
        drawer_pane = drawer_pane.on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
            bg(s, d, false, tx);
        });
    }
    let content = col()
        .child(header::header(shared, st, &parts, look))
        .children(
            (st.find.open && parts.caps.searchable)
                .then(|| find::bar(shared, st, parts.caps, look)),
        )
        .child(
            col()
                .key(BODY_KEY)
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .child(body),
        );
    drawer_pane.child(focus_accent_bar(look, content))
}

fn focus_accent_bar(look: &Look, content: El) -> El {
    let indicator = strata_kit::slots::columns::header_indicator(look);
    let bleed = 1.0;
    el().pin(Some(-bleed), Some(indicator), Some(-bleed), Some(-bleed))
        .focus_within_ring(indicator, 0.0, look.kit.c.bg_accent)
        .child(content.pin(Some(bleed), Some(-indicator), Some(bleed), Some(bleed)))
}

pub(crate) fn return_as_space<S: 'static>(_: &mut S, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if ev.key != rune::Key::Enter || ev.mods != rune::Mods::default() {
        return false;
    }
    if !tx.key_repeat() {
        tx.send_key(KeyEvent {
            key: rune::Key::Text(" ".to_owned()),
            mods: rune::Mods::default(),
        });
    }
    true
}

fn drawer_key<S: 'static>(
    shared: &Shared<S>,
    s: &mut S,
    ev: &KeyEvent,
    tx: &mut Tx,
    find_keys: fn(&KeyEvent) -> bool,
    searchable: bool,
) -> bool {
    // Escape is find's only while its bar shows; otherwise it goes on to the host.
    let finding = (shared.access)(s).find.open && searchable;
    if ev.key == rune::Key::Escape && finding {
        find::close(shared, s, tx);
        return true;
    }
    if (shared.access)(s).find.query.focused && !(ev.mods.ctrl || ev.mods.cmd || ev.mods.alt) {
        return false;
    }
    if searchable && find_keys(ev) {
        find::open(shared, s, tx);
        return true;
    }
    false
}

fn marker(name: &str, st: &DrawerState) -> El {
    el().key(format!("preview-{name}-marker-{}", st.marks))
        .size(0.0, 0.0)
        .a11y_hidden()
        .transition(Transition::none())
}

fn hidden_marker<S: 'static>(shared: &Shared<S>, st: &DrawerState) -> El {
    let access = shared.access;
    marker("hidden", st).on_layout(move |s: &mut S, _| {
        let st = access(s);
        st.dragging = false;
        st.drag_from = None;
        st.hint = false;
        st.find.query.focused = false;
        st.marks = st.marks.wrapping_add(1);
    })
}

fn find_focus_marker<S: 'static>(shared: &Shared<S>, st: &DrawerState) -> El {
    let access = shared.access;
    marker("find-focus", st).on_layout(move |s: &mut S, _| {
        let st = access(s);
        st.find.query.focused = false;
        st.marks = st.marks.wrapping_add(1);
    })
}

fn find_end_marker<S: 'static>(shared: &Shared<S>, st: &DrawerState) -> El {
    let shared = shared.clone();
    marker("find-end", st).on_layout(move |s: &mut S, _| {
        let st = (shared.access)(s);
        st.find_end_pending = false;
        st.marks = st.marks.wrapping_add(1);
        shared.with_view(s, |v| v.find_end());
    })
}

/// A zero-size stub that hands focus to `to` when it is removed (see the module docs).
fn stub(key: String, to: &str) -> El {
    el().key(key)
        .size(0.0, 0.0)
        .a11y_hidden()
        .transition(Transition::none())
        .focus_return(to)
}

struct EmptySlot {
    width: f32,
    lit: bool,
}

fn empty_reserved_slot<S: 'static>(
    slot: El,
    shared: &Shared<S>,
    empty_slot: EmptySlot,
    look: &Look,
) -> El {
    let slot = slot.a11y_hidden();
    if empty_slot.width <= 0.0 {
        return slot;
    }
    let empty = shared.clone();
    let slot = if let Some(bg) = shared.background_drag.clone() {
        slot.preserve_focus_on_press(true)
            .on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
                bg(s, d, true, tx);
                if is_plain_slot_click(d) {
                    empty_slot_press(&empty, s, tx);
                }
            })
    } else {
        slot.on_background_press_tx(move |s: &mut S, tx: &mut Tx| {
            empty_slot_press(&empty, s, tx);
        })
    };
    let line_color = if empty_slot.lit {
        look.kit.c.border_focus
    } else {
        look.kit.c.border_base
    };
    slot.child(
        el().key("preview-empty-line")
            .w(shared.rules.line)
            .h_full()
            .pin(Some(0.0), Some(0.0), None, Some(0.0))
            .fill(line_color)
            .a11y_hidden()
            .inert(),
    )
}

fn is_plain_slot_click(d: &Drag) -> bool {
    if d.phase != Phase::Up || d.cancelled() {
        return false;
    }
    d.dx.abs() <= 4.0 && d.dy.abs() <= 4.0
}

fn empty_slot_press<S: 'static>(shared: &Shared<S>, s: &mut S, tx: &mut Tx) {
    let m = tx.mods();
    if !(m.ctrl || m.shift || m.cmd) {
        shared.emit(s, DrawerIntent::EmptySlotPressed, tx);
    }
}

fn fit_marker<S: 'static>(shared: &Shared<S>, st: &DrawerState, fits: bool) -> El {
    let shared = shared.clone();
    marker(if fits { "fit-shown" } else { "fit-hidden" }, st).on_layout_tx(
        move |s: &mut S, _, tx: &mut Tx| {
            let st = (shared.access)(s);
            if st.suspended == fits {
                st.suspended = !fits;
                shared.emit(s, DrawerIntent::Suspend(!fits), tx);
            } else {
                // The verdict held: nothing to commit.
                tx.live_only();
            }
        },
    )
}

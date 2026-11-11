//! PreviewDrawer (C9): the resizable drawer that hosts every preview, beside the file views in
//! a browser window and in the chooser.
//!
//! Work package: C9, `docs/work/phase-1-leaves/C9-preview-drawer.md`. Feature map nodes
//! `preview/preview-panel` and `preview/preview-panel/layout`.
//!
//! ```ignore
//! PreviewDrawer::new(&s.drawer, |s: &mut App| &mut s.drawer)
//!     .arrangement(Arrangement::List)
//!     .header(HeaderInfo::new("notes.md").details(["2 KB", "Today", "Markdown"]))
//!     .hosted(&s.view, |s: &mut App| &mut s.view)
//!     .content(code_view)
//!     .on_intent(|s: &mut App, intent, tx| s.apply(intent, tx))
//!     .build(listing, &s.look, cx)
//! ```
//!
//! # Why its own split
//!
//! The drawer is a row the component lays out itself, `[files | divider | slot]`, driven by a
//! [`DrawerState`] the host keeps per window. It is neither rune-kit's `SplitView` inspector
//! nor a rune-kit `PaneState`:
//!
//! - `SplitView` is a window's whole shell (title band, toolbar, main card, sidebar). The
//!   drawer sits inside the browser area of both the browser window and the chooser, and a
//!   second `SplitView` would bring a second title band.
//! - The inspector fixes what the drawer has to set: its region is named "Inspector", its grip
//!   "Resize inspector", focus returns to keys of its own (`main-pane`, `sidebar-toggle`), and
//!   the grip never takes focus, which rules out Strata's F8 and arrow keys.
//! - `PaneState`'s rules contradict Strata's: dragging below half the minimum collapses the
//!   pane (Strata keeps at least one standard column), it has one fixed width (Strata has an
//!   automatic width that fills the space beside the focused column, and a session minimum),
//!   its range is fixed (Strata's maximum moves with the window), and only the kit can set its
//!   resizing flag.
//! - Strata slides for 260 ms where the kit springs, a reserved Columns slot never slides, and
//!   the slot lends its width to C2's trailing column. Neither type can say that.
//!
//! From `SplitView` it keeps the zero-size `focus_return` stubs, the grip as a `separator`
//! whose value is the width, the 6 point grip inside the pane's edge, and the slot that clips
//! content held at its full width while it slides.
//!
//! # Focus moves
//!
//! The drawer moves focus with zero-size stubs carrying `focus_return`: each request
//! ([`DrawerState::focus_content`] and kin) bumps a counter in the stub's key, so the old stub
//! is removed and Rune returns focus to the stub's target, as it does when a popover closes. A
//! stub inside the slot returns focus to the files when the preview closes. The stubs stay
//! although Rune 2a44156 has `Tx::focus`: a returned focus goes to the target's
//! `focus_selected` element first (the files' cursor row, an archive's chosen member), while
//! `Tx::focus` takes the target's first focusable element (`runtime/input/focus.rs`).
//!
//! # Deliberate changes
//!
//! - Strata's header shows an accent top border while focus is inside the preview. Rune draws
//!   a focus-within ring only around the whole element, so the drawer draws a focus-within ring
//!   around the pane instead (a Rune issue is drafted for a styled focus-within).
//!
//! # What stays in Strata
//!
//! Which files preview, when the preview opens and closes (Space, `i`, Escape, following the
//! cursor), whether the user dismissed it, the wrap preference, the sidebar rail, and the
//! keymap. The drawer holds the interaction and the placement and reports through
//! [`DrawerIntent`].

mod divider;
mod find;
mod header;
mod hosted;
mod layout;
mod state;

#[cfg(test)]
mod tests;

use std::rc::Rc;

use rune::{Cx, El, Icon, KeyEvent, Transition, Tx, col, el, row, text};
use strata_kit::look::Look;

use crate::contract::{Dnd, DragSource};

pub use hosted::{DocumentView, FindStatus, HostedPreview, NoPreview};
pub use layout::{Arrangement, ColumnsFit, DrawerRules, RightPane, Solved, right_pane, solve};
pub use state::{DrawerState, FIND_MAX_CHARS, FindBar, SLIDE_SECONDS};

/// The widest image, video, PDF page or rendered document text in a preview, centered in a
/// wider drawer (`preview/preview-panel/layout`). C11–C14 lay their content out within it;
/// source views and standalone tables keep the full width.
pub const CONTENT_MAX_WIDTH: f32 = 1280.0;

/// The split's key: its width is what the rules solve against.
pub const SPLIT_KEY: &str = "preview-split";

/// What the drawer tells the host. The host applies Strata's policy to each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DrawerIntent {
    /// The close button closed the drawer (the drawer has already closed). Strata marks the
    /// preview dismissed.
    Closed,
    /// Open in the default application.
    Open,
    /// The wrap toggle, to its new value.
    SetWrap(bool),
    /// The Rendered/Source switch, to the form it offers.
    ShowView(DocumentView),
    /// The narrow-window rule hid (`true`) or showed again (`false`) the open preview's
    /// content: pause media and defer loading, or resume.
    Suspend(bool),
}

/// The words the drawer shows and assistive technology reads. Strata passes translated text
/// and its keymap's keys; the defaults are English with the default key map.
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
        }
    }
}

/// The previewed item in the header: its icon, name and one line of details (size, modified,
/// type). The header is a bar's height, so it holds nothing more: a view's badges (a video's
/// resolution, a route's distance, "2 of 5 in folder") belong in its own content, as C13 and
/// C14b show them.
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
    /// Nothing: in Columns, the empty reserved slot (a focused folder lends it to its child
    /// column).
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

/// What the parts of the drawer share while they build.
pub(crate) struct Shared<S> {
    pub access: fn(&mut S) -> &mut DrawerState,
    pub hosted: Option<HostedAccess<S>>,
    pub intent: Option<IntentFn<S>>,
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

/// The drawer and the files beside it: `PreviewDrawer::new(&s.drawer, access) … .build(files,
/// look, cx)`.
pub struct PreviewDrawer<'a, S> {
    state: &'a DrawerState,
    access: fn(&mut S) -> &mut DrawerState,
    arrangement: Arrangement,
    rules: DrawerRules,
    header: HeaderInfo,
    caps: Caps,
    hosted: Option<HostedAccess<S>>,
    body: Body,
    open_allowed: bool,
    labels: Rc<DrawerLabels>,
    files_key: String,
    find_keys: fn(&KeyEvent) -> bool,
    drag: Option<(Dnd<S>, DragSource<S>)>,
    intent: Option<IntentFn<S>>,
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
            open_allowed: true,
            labels: Rc::new(DrawerLabels::default()),
            files_key: "files".into(),
            find_keys: default_find_keys,
            drag: None,
            intent: None,
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

    /// Which keys open find while focus is in the preview ([`default_find_keys`] unless set;
    /// Strata passes its keymap's, `/` in 10xer mode).
    pub fn find_keys(mut self, f: fn(&KeyEvent) -> bool) -> Self {
        self.find_keys = f;
        self
    }

    /// Dragging the header's icon and name drags the previewed file, through `dnd`. The source
    /// reaches the binder keyed `preview-handle` (W1's `Dnd::drag_source_keyed`), so a window
    /// has one drawer with a drag source.
    pub fn drag_source(mut self, dnd: Dnd<S>, source: DragSource<S>) -> Self {
        self.drag = Some((dnd, source));
        self
    }

    pub fn on_intent(mut self, f: impl Fn(&mut S, DrawerIntent, &mut Tx) + 'static) -> Self {
        self.intent = Some(Rc::new(f));
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
            open_allowed,
            labels,
            files_key,
            find_keys,
            drag,
            intent,
        } = self;
        let available = cx.layout_with(SPLIT_KEY, |r| r[2]).unwrap_or(0.0);
        let solved = solve(&rules, &arrangement, available, st.manual_width());
        let columns = matches!(arrangement, Arrangement::Columns(_));
        let has_body = !matches!(body, Body::Empty);
        // The pane shows while the drawer is open, has something to show, and fits.
        let shown = st.is_open() && has_body && solved.fits;
        // The slot holds the divider's line and the pane beside it.
        let slot_width = if shown {
            solved.width + rules.line
        } else if columns && (st.is_reserved() || st.is_open()) {
            // Columns keeps its slot, down to zero, so the columns keep their offset.
            solved.empty_slot
        } else {
            0.0
        };
        let shared = Shared {
            access,
            hosted,
            intent,
            labels,
            rules,
            animations: look.animations,
        };

        let mut slot = el()
            .key("preview-slot")
            .w(slot_width)
            .h_full()
            .clip_soft(0.0);
        if shown {
            let parts = Parts {
                header: &header,
                caps: &caps,
                // A placeholder has nothing to open.
                open_allowed: open_allowed && matches!(body, Body::View(_)),
                drag: drag.as_ref(),
                find_keys,
            };
            slot = slot
                .child(pane(&shared, st, solved, body, parts, look))
                .child(divider::divider(&shared, st, solved, columns, look))
                .child(stub("preview-open-stub".into(), &files_key));
        } else {
            slot = slot.a11y_hidden();
            if slot_width > 0.0 {
                // The empty reserved slot keeps the divider's line, so the edge beside the
                // columns looks the same with or without a preview.
                slot = slot.child(
                    el().key("preview-empty-line")
                        .w(rules.line)
                        .h_full()
                        .pin(Some(0.0), Some(0.0), None, Some(0.0))
                        .fill(look.kit.c.border_base)
                        .a11y_hidden()
                        .inert(),
                );
            }
        }
        if st.is_open() && has_body {
            slot = slot.child(fit_marker(&shared, st, solved.fits));
        }
        if !shown && (st.dragging || st.hint || st.find.query.focused) {
            slot = slot.child(hidden_marker(&shared, st));
        } else if st.find.query.focused && !(st.find.open && caps.searchable) {
            // The pane shows but the bar doesn't (the view has nothing to search now).
            slot = slot.child(find_focus_marker(&shared, st));
        }
        if st.find_end_pending {
            slot = slot.child(find_end_marker(&shared, st));
        }

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
                    .child(files),
            )
            .child(slot)
            .child(stub(
                format!("preview-focus-content-{}", st.focus_content),
                BODY_KEY,
            ))
            .child(stub(
                format!("preview-focus-files-{}", st.focus_files),
                &files_key,
            ))
            .child(stub(
                format!("preview-focus-divider-{}", st.focus_divider),
                divider::GRIP_KEY,
            ));
        if shown {
            split = split.children(divider::overlays(
                &shared, st, solved, available, slot_width, columns, look,
            ));
        }
        split
    }
}

/// The content area's key: where [`DrawerState::focus_content`] sends focus.
pub const BODY_KEY: &str = "preview-body";

/// The header's inputs, borrowed from the builder.
pub(crate) struct Parts<'b, S> {
    pub header: &'b HeaderInfo,
    pub caps: &'b Caps,
    pub open_allowed: bool,
    pub drag: Option<&'b (Dnd<S>, DragSource<S>)>,
    pub find_keys: fn(&KeyEvent) -> bool,
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
    // The content stays, inert and fading, while the slot slides shut.
    let exit = if shared.animations {
        Transition::identity().springs(rune::SpringPair {
            enter: rune::Spring::NONE,
            exit: rune::Spring::new(SLIDE_SECONDS, 0.0),
        })
    } else {
        Transition::none()
    };
    let keys = shared.clone();
    let find_keys = parts.find_keys;
    let searchable = parts.caps.searchable;
    col()
        .key("preview-drawer")
        .role("region")
        .label(shared.labels.region.clone())
        .w(solved.pane)
        .h_full()
        .pin(Some(shared.rules.line), Some(0.0), None, Some(0.0))
        .fill(k.c.bg_base)
        .focus_within_ring(2.0, -2.0, k.c.border_focus)
        .transition(exit)
        .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            drawer_key(&keys, s, ev, tx, find_keys, searchable)
        })
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
        )
}

/// Keys that reach the drawer from a focused element inside it: Escape closes find, and the
/// find keys open it while the view is searchable. Others pass on.
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
    // A key typed in the find field is the field's (10xer's `/` is literal there).
    if (shared.access)(s).find.query.focused {
        return false;
    }
    if searchable && find_keys(ev) {
        find::open(shared, s);
        return true;
    }
    false
}

// State markers: zero-size elements that run once, after the layout that mounts them, to
// settle state the view can't change while it builds. Each key carries `DrawerState::marks`,
// which every marker that acts bumps (and each open). That is defensive: a marker's own
// handler clears its condition, so it unmounts on the next build anyway; the counter only
// keeps a marker needed again in the same commit from staying mounted unheard.

fn marker(name: &str, st: &DrawerState) -> El {
    el().key(format!("preview-{name}-marker-{}", st.marks))
        .size(0.0, 0.0)
        .a11y_hidden()
        .transition(Transition::none())
}

/// Mounted when the pane has gone (closed, nothing to show, or too narrow) while a drag, the
/// divider's caption or the find field's focus request is left over: it clears them, so the
/// pane doesn't come back mid-drag or take focus when it shows again.
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

/// Mounted while the find field's focus request stands but the bar isn't built: drops the
/// request, so the field doesn't take focus when the view is searchable again.
fn find_focus_marker<S: 'static>(shared: &Shared<S>, st: &DrawerState) -> El {
    let access = shared.access;
    marker("find-focus", st).on_layout(move |s: &mut S, _| {
        let st = access(s);
        st.find.query.focused = false;
        st.marks = st.marks.wrapping_add(1);
    })
}

/// Mounted while find has ended and the hosted view hasn't heard: tells it. Needs the view
/// passed with [`PreviewDrawer::hosted`] in that build.
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

/// Mounted while the drawer is open with something to show, keyed by whether it fits (and by
/// the open, which bumps `marks`), so each change of verdict, and each open, mounts a new
/// marker and reports it.
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

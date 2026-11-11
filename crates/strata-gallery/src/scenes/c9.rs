//! The gallery page for C9 PreviewDrawer: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `c9-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/C9-preview-drawer.md`.
//!
//! The drawer beside a fake listing (List, Icons, or a Columns strip), hosting a fake document
//! that answers find. The page's keys stand in for Strata's policy: Space toggles the preview,
//! Escape closes it, Right moves focus from the listing into the preview and Left back, and F8
//! focuses the divider.

use rune::{Cx, El, Key, KeyEvent, Tx, col, el, row, text};
use strata_components::preview_drawer::{
    Arrangement, ColumnsFit, DocumentView, DrawerIntent, DrawerRules, DrawerState, FindStatus,
    HeaderInfo, HostedPreview, PreviewDrawer,
};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use crate::{Gallery, Scene};

/// The view beside the drawer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    List,
    Icons,
    /// A strip of this many navigated columns.
    Columns(usize),
}

/// What the drawer holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Content {
    #[default]
    Document,
    Image,
    Placeholder,
}

/// C9's demo state, held in `Gallery::c9`.
#[derive(Default)]
pub struct Demo {
    pub drawer: DrawerState,
    pub mode: Mode,
    pub content: Content,
    pub view: FakeView,
    pub chooser: bool,
    /// Strata's "dismissed" flag, which the close button sets.
    pub dismissed: bool,
    /// The last `Suspend` intent: media would pause and loads defer.
    pub suspended: bool,
    pub opened: usize,
}

/// A fake preview. A document in Source is searchable and wraps; Rendered, it is a
/// table-only document: nothing to search and no wrap, as C11 reports one. An image offers
/// neither.
#[derive(Default)]
pub struct FakeView {
    pub document: bool,
    pub form: Option<DocumentView>,
    pub wrap: bool,
    pub busy: Option<String>,
    /// Find has no answer yet (a view still searching).
    pub pending: bool,
    query: String,
    matches: usize,
    current: Option<usize>,
}

const LINES: &[&str] = &[
    "# Preview drawer",
    "The drawer hosts every preview beside the files.",
    "Its header shows only what the preview supports.",
    "Find in preview searches the text the preview shows.",
    "Resize it from its leading edge; the width holds for the window.",
];

impl FakeView {
    fn source(&self) -> bool {
        self.document && self.form != Some(DocumentView::Rendered)
    }
}

impl HostedPreview for FakeView {
    fn document_view(&self) -> Option<DocumentView> {
        self.form
    }
    fn wrap(&self) -> Option<bool> {
        self.source().then_some(self.wrap)
    }
    fn searchable(&self) -> bool {
        self.source()
    }
    fn find(&mut self, query: &str) {
        self.query = query.to_lowercase();
        self.matches = if self.query.is_empty() {
            0
        } else {
            LINES
                .iter()
                .map(|l| l.to_lowercase().matches(&self.query).count())
                .sum()
        };
        self.current = (self.matches > 0).then_some(1);
    }
    fn find_step(&mut self, forward: bool) {
        if let Some(n) = self.current {
            let total = self.matches;
            self.current = Some(if forward {
                n % total + 1
            } else {
                (n + total - 2) % total + 1
            });
        }
    }
    fn find_status(&self) -> Option<FindStatus> {
        (!self.pending).then_some(FindStatus {
            current: self.current,
            total: self.matches,
        })
    }
    fn find_end(&mut self) {
        self.query.clear();
        self.matches = 0;
    }
    fn busy(&self) -> Option<&str> {
        self.busy.as_deref()
    }
}

impl Demo {
    /// Where find stands in the fake view.
    pub fn view_status(&self) -> Option<FindStatus> {
        self.view.find_status()
    }
}

struct Setup {
    dark: bool,
    mode: Mode,
    content: Content,
    open: bool,
}

fn open(g: &mut Gallery, s: Setup) {
    g.open("c9", s.dark);
    let d = &mut g.c9;
    *d = Demo {
        mode: s.mode,
        content: s.content,
        ..Demo::default()
    };
    d.view.document = s.content == Content::Document;
    d.view.form = d.view.document.then_some(DocumentView::Source);
    d.view.wrap = true;
    d.drawer.reserve(matches!(s.mode, Mode::Columns(_)));
    if s.open {
        d.drawer.open_at_once();
    }
}

const fn setup(dark: bool, mode: Mode, content: Content, open: bool) -> Setup {
    Setup {
        dark,
        mode,
        content,
        open,
    }
}

fn find_open(g: &mut Gallery, dark: bool) {
    open(g, setup(dark, Mode::List, Content::Document, true));
    g.c9.drawer.start_find("preview");
    g.c9.view.find("preview");
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c9", dark)`, then fills in `g.c9`.
pub const SCENES: &[Scene] = &[
    ("c9-list-open-light", |g| {
        open(g, setup(false, Mode::List, Content::Document, true))
    }),
    ("c9-list-open-dark", |g| {
        open(g, setup(true, Mode::List, Content::Document, true))
    }),
    ("c9-columns-open-light", |g| {
        open(g, setup(false, Mode::Columns(2), Content::Image, true))
    }),
    ("c9-columns-open-dark", |g| {
        open(g, setup(true, Mode::Columns(2), Content::Image, true))
    }),
    ("c9-columns-reserved-light", |g| {
        open(g, setup(false, Mode::Columns(2), Content::Image, false))
    }),
    ("c9-columns-reserved-dark", |g| {
        open(g, setup(true, Mode::Columns(2), Content::Image, false))
    }),
    ("c9-icons-placeholder-light", |g| {
        open(g, setup(false, Mode::Icons, Content::Placeholder, true))
    }),
    ("c9-icons-placeholder-dark", |g| {
        open(g, setup(true, Mode::Icons, Content::Placeholder, true))
    }),
    ("c9-find-open-light", |g| find_open(g, false)),
    ("c9-find-open-dark", |g| find_open(g, true)),
];

fn minimal(g: &mut Gallery, dark: bool) {
    open(g, setup(dark, Mode::List, Content::Image, true));
    g.c9.chooser = true;
    g.c9.view.busy = Some("Rendering 12,480 triangles…".into());
}

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[
    ("c9-minimal-light", |g| minimal(g, false)),
    ("c9-minimal-dark", |g| minimal(g, true)),
    ("c9-icons-closed-light", |g| {
        open(g, setup(false, Mode::Icons, Content::Placeholder, false))
    }),
    ("c9-icons-closed-dark", |g| {
        open(g, setup(true, Mode::Icons, Content::Placeholder, false))
    }),
];

const COLUMN: f32 = 300.0;

fn arrangement(d: &Demo) -> Arrangement {
    match d.mode {
        Mode::List => Arrangement::List,
        Mode::Icons => Arrangement::Icons,
        Mode::Columns(n) => Arrangement::Columns(ColumnsFit::new(
            &DrawerRules::STRATA,
            n as f32 * COLUMN,
            COLUMN,
        )),
    }
}

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let d = &g.c9;
    let look = g.look();
    let header = match d.content {
        Content::Document => HeaderInfo::new("notes.md")
            .icon(look.icon(IconName::FileCode))
            .details(["2.4 KB", "Today 09:41", "Markdown"]),
        Content::Image => HeaderInfo::new("harbour.jpg")
            .icon(look.icon(IconName::Documents))
            .details(["3.1 MB", "Yesterday", "JPEG image"]),
        Content::Placeholder => HeaderInfo::default(),
    };
    let drawer = PreviewDrawer::new(&d.drawer, |g: &mut Gallery| &mut g.c9.drawer)
        .arrangement(arrangement(d))
        .header(header)
        .open_allowed(!d.chooser)
        .on_intent(intent);
    let drawer = match d.content {
        Content::Document => drawer
            .hosted(&d.view, |g: &mut Gallery| &mut g.c9.view)
            .content(document(d, look)),
        Content::Image => drawer
            .hosted(&d.view, |g: &mut Gallery| &mut g.c9.view)
            .content(image(look)),
        Content::Placeholder => drawer.placeholder("No preview for this selection"),
    };
    drawer.build(files(d, look), look, cx)
}

fn intent(g: &mut Gallery, intent: DrawerIntent, _tx: &mut Tx) {
    let d = &mut g.c9;
    match intent {
        DrawerIntent::Closed => d.dismissed = true,
        DrawerIntent::Open => d.opened += 1,
        DrawerIntent::SetWrap(on) => d.view.wrap = on,
        DrawerIntent::ShowView(form) => {
            d.view.form = Some(form);
            let query = d.drawer.find_query().to_owned();
            d.view.find(&query);
        }
        DrawerIntent::Suspend(on) => d.suspended = on,
        _ => {}
    }
}

/// The fake listing beside the drawer. Its focus group is keyed `files`, the drawer's default
/// return target; row 1 holds the cursor.
fn files(d: &Demo, look: &Look) -> El {
    let k = &look.kit;
    let row_el = |i: usize, name: String| {
        el().key(format!("row-{i}"))
            .role("option")
            .label(name.clone())
            .focusable(true)
            .focus_selected(i == 1)
            .w_full()
            .h(look.row.list)
            .px(look.space.md)
            .items_center()
            .radius(look.round(k.radius.sm))
            .fill(if i == 1 { k.c.layer_02 } else { k.c.bg_base })
            .child(text(name).font(k.ty.ui).color(k.c.text_base))
    };
    let listing = |key: &str, rows: usize, focus: bool| {
        let mut list = col()
            .key(key.to_owned())
            .role("listbox")
            .label(if focus { "Files" } else { "Parent folder" })
            .w_full()
            .h_full()
            .padding(look.space.xs)
            .children((0..rows).map(|i| {
                if focus {
                    row_el(i, format!("file-{i}.md"))
                } else {
                    text(format!("folder-{i}"))
                        .font(k.ty.ui)
                        .color(k.c.text_muted)
                        .h(look.row.list)
                        .px(look.space.md)
                }
            }));
        if focus {
            list = list.focus_group().on_key(listing_key);
        }
        list
    };
    let body = match d.mode {
        Mode::List => listing("files", 12, true),
        Mode::Icons => {
            let tiles = (0..12).map(|i| {
                el().key(format!("row-{i}"))
                    .role("option")
                    .label(format!("photo-{i}.jpg"))
                    .focusable(true)
                    .focus_selected(i == 1)
                    .size(96.0, 96.0)
                    .radius(look.round(k.radius.md))
                    .fill(if i == 1 { k.c.layer_03 } else { k.c.layer_02 })
            });
            row()
                .key("files")
                .role("listbox")
                .label("Files")
                .focus_group()
                .on_key(listing_key)
                .flex_wrap()
                .gap(look.space.md)
                .padding(look.space.md)
                .children(tiles)
        }
        Mode::Columns(n) => row().h_full().children((0..n).map(|c| {
            let last = c + 1 == n;
            // The last column meets the drawer's own divider line.
            let border = if last { 0.0 } else { 1.0 };
            el().w(COLUMN)
                .h_full()
                .border_sides([0.0, border, 0.0, 0.0], k.c.border_muted)
                .child(listing(
                    if last { "files" } else { "parent-column" },
                    10,
                    last,
                ))
        })),
    };
    let mut pane = col().w_full().h_full().clip_soft(0.0).child(body);
    if d.suspended {
        pane = pane.child(
            text("Preview paused: the window is too narrow")
                .key("demo-suspended")
                .font(k.ty.small)
                .color(k.c.text_muted)
                .padding(look.space.md),
        );
    }
    pane
}

/// The fake document: focusable, so Right has somewhere to land.
fn document(d: &Demo, look: &Look) -> El {
    let k = &look.kit;
    let rendered = d.view.form == Some(DocumentView::Rendered);
    col()
        .key("fake-content")
        .role("document")
        .label("notes.md")
        .focusable(true)
        .w_full()
        .h_full()
        .padding(look.space.lg)
        .gap(look.space.sm)
        .children(LINES.iter().enumerate().map(|(i, line)| {
            let style = if rendered && i == 0 {
                k.ty.ui_medium
            } else if rendered {
                k.ty.ui
            } else {
                k.ty.mono
            };
            text(*line).font(style).color(k.c.text_base)
        }))
}

/// A stand-in for an image preview: a picture-shaped block.
fn image(look: &Look) -> El {
    let k = &look.kit;
    col()
        .key("fake-content")
        .role("img")
        .label("harbour.jpg")
        .focusable(true)
        .w_full()
        .h_full()
        .padding(look.space.lg)
        .child(
            el().w_full()
                .aspect_ratio(4.0 / 3.0)
                .radius(look.round(k.radius.md))
                .fill(k.c.layer_03),
        )
}

/// Space on the listing toggles the preview, as Strata's Quick preview does.
fn listing_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    if matches!(&key.key, Key::Text(t) if t == " ") {
        toggle(g, tx);
        return true;
    }
    false
}

fn toggle(g: &mut Gallery, tx: &mut Tx) {
    let Gallery { look, c9: d, .. } = g;
    if d.drawer.is_open() {
        d.drawer.close(look, tx);
    } else {
        d.drawer.open(look, tx);
    }
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let in_content = tx.focused() == Some("fake-content");
    let in_files = tx.focused().is_some_and(|f| f.starts_with("row-"));
    match &key.key {
        Key::Text(t) if t == " " && !in_content => {
            toggle(g, tx);
            true
        }
        Key::Escape if g.c9.drawer.is_open() => {
            let Gallery { look, c9: d, .. } = g;
            d.drawer.close(look, tx);
            true
        }
        Key::Right if in_files && g.c9.drawer.is_open() => {
            g.c9.drawer.focus_content();
            true
        }
        Key::Left if in_content => {
            g.c9.drawer.focus_files();
            true
        }
        Key::Function(8) => {
            g.c9.drawer.focus_divider();
            true
        }
        _ => false,
    }
}

/// Runs at the start of every frame while this page is open, with the clock in seconds.
/// Deliver fake async results here; a worker wakes the loop with `Gallery::waker`. True when
/// state changed.
pub fn poll(_g: &mut Gallery, _t: f64, _tx: &mut Tx) -> bool {
    false
}

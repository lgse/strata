//! The gallery page for C12 PdfView: its scenes, demo state, view and key and poll hooks. Scene
//! names start with `c12-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-2-composites/C12-pdf-view.md`.
//!
//! A PDF in C9's drawer beside a fake listing. A fake document stands in for the file: pages of
//! generated prose, a landscape page, a scan with no text layer and a page that fails. A fake
//! renderer stands in for Strata's sandbox: it renders one page at a time, 60 ms each on the
//! page's clock, setting each line in the bundled JetBrains Mono (through swash, as a PDF
//! rasterizer would) with the text layer to match, at the width the view asks. Under the listing, lines scripts read: what the renderer did
//! (`c12-log`), what copying would take (`c12-selection`), the last copy (`c12-copied`), the
//! last intent or key the view passed on (`c12-last`), and the interface text size the
//! Ctrl+wheel and Ctrl+0 the view didn't take would change (`c12-text-size`).

use rune::{Cx, El, Key, KeyEvent, Tx, WheelEvent, col, el, text};
use strata_components::pdf_view::{
    PageReply, PdfDocument, PdfIntent, PdfState, PdfView, RenderRequest, TextLayer,
};
use strata_components::preview_drawer::{
    DrawerIntent, DrawerState, HeaderInfo, HostedPreview, PreviewDrawer,
};
use strata_components::viewers::viewport::ctrl_zero_key;
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use swash::FontRef;
use swash::scale::{Render, ScaleContext, Source};
use swash::zeno::{Format, Vector};

use crate::{Gallery, Scene};

/// How long the fake renderer takes a page, in seconds.
pub const RENDER_SECONDS: f64 = 0.06;
/// US Letter, in points.
const LETTER: [f32; 2] = [612.0, 792.0];
const MARGIN: f32 = 54.0;
const TOP: f32 = 72.0;
const LINE: f32 = 16.0;
const ADVANCE: f32 = 6.6;
/// The page's type size: JetBrains Mono advances 0.6 em, so 11 pt sets [`ADVANCE`].
const TYPE_SIZE: f32 = 11.0;
/// The baseline, below a line's top.
const BASELINE: f32 = 10.0;
const COLUMNS: usize = 72;

/// What a fake page holds.
#[derive(Clone, Debug, PartialEq)]
pub enum FakeKind {
    /// Prose, with a text layer.
    Text(Vec<String>),
    /// A scanned picture: no text layer.
    Scan,
    /// The renderer fails on it.
    Broken,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FakePage {
    pub size: [f32; 2],
    pub kind: FakeKind,
}

const WORDS: &[&str] = &[
    "strata", "renders", "every", "page", "in", "its", "sandbox", "and", "the", "preview", "draws",
    "glyph", "boxes", "over", "pictures", "quietly", "typed", "jumping", "query", "lines", "of",
    "text", "with", "copy", "paste", "folders", "keep", "their", "order", "while", "you", "scroll",
    "zoom", "select", "find", "words", "by", "hand",
];

/// The lines of prose page `page` holds: a title line, then wrapped text from a fixed stream.
pub fn prose(page: usize, lines: usize) -> Vec<String> {
    let mut out = vec![format!("Page {} of the Strata report", page + 1)];
    let mut seed = (page as u64).wrapping_mul(2_654_435_761).wrapping_add(17);
    let mut line = String::new();
    while out.len() < lines {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let w = WORDS[(seed >> 33) as usize % WORDS.len()];
        if line.len() + w.len() + 1 > COLUMNS {
            out.push(std::mem::take(&mut line));
            // A blank line now and then: a paragraph break.
            if (seed >> 20).is_multiple_of(7) && out.len() < lines {
                out.push(String::new());
            }
            continue;
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(w);
    }
    out.truncate(lines);
    out
}

/// A document of `pages` pages of prose; `mixed` makes page 1 a scan, page 2 landscape and
/// page 3 broken.
pub fn fake_doc(pages: usize, mixed: bool) -> Vec<FakePage> {
    (0..pages)
        .map(|p| match p {
            0 if mixed => FakePage {
                size: LETTER,
                kind: FakeKind::Scan,
            },
            1 if mixed => FakePage {
                size: [LETTER[1], LETTER[0]],
                kind: FakeKind::Text(prose(p, 26)),
            },
            2 if mixed => FakePage {
                size: LETTER,
                kind: FakeKind::Broken,
            },
            _ => FakePage {
                size: LETTER,
                kind: FakeKind::Text(prose(p, 38)),
            },
        })
        .collect()
}

/// Renders `page` to fit `box_px`, as Strata's helper does (`bounded_surface_dimensions`:
/// whole pixels, rounded down), with its text layer in the image's pixels.
pub fn render(page: &FakePage, box_px: [u32; 2], dark_ink: bool) -> PageReply {
    let s = (box_px[0] as f32 / page.size[0]).min(box_px[1] as f32 / page.size[1]);
    let w = (page.size[0] * s).floor().max(1.0) as u32;
    let h = (page.size[1] * s).floor().max(1.0) as u32;
    let s = (w as f32 / page.size[0]).min(h as f32 / page.size[1]);
    let mut rgba = vec![255u8; (w * h * 4) as usize];
    let ink = if dark_ink { 40 } else { 60 };
    let mut fill = |r: [f32; 4], v: u8| {
        let x0 = (r[0] * s).floor().max(0.0) as u32;
        let y0 = (r[1] * s).floor().max(0.0) as u32;
        let x1 = ((r[2] * s).ceil() as u32).min(w);
        let y1 = ((r[3] * s).ceil() as u32).min(h);
        for y in y0..y1 {
            let row = (y * w * 4) as usize;
            for x in x0..x1 {
                let i = row + (x * 4) as usize;
                rgba[i] = v;
                rgba[i + 1] = v;
                rgba[i + 2] = v;
            }
        }
    };
    match &page.kind {
        FakeKind::Broken => PageReply::Failed,
        FakeKind::Scan => {
            for i in 0..12 {
                let y = TOP + i as f32 * 52.0;
                fill(
                    [MARGIN, y, page.size[0] - MARGIN, y + 36.0],
                    150 + (i * 7) as u8,
                );
            }
            PageReply::Rendered {
                width: w,
                height: h,
                rgba,
                text: None,
            }
        }
        FakeKind::Text(lines) => {
            let mut text = String::new();
            let mut glyphs = Vec::new();
            let mut pen = Pen::new(s);
            for (n, line) in lines.iter().enumerate() {
                let top = TOP + n as f32 * LINE;
                for (c, ch) in line.chars().enumerate() {
                    let x = MARGIN + c as f32 * ADVANCE;
                    text.push(ch);
                    glyphs.push([x * s, top * s, (x + ADVANCE) * s, (top + 13.0) * s]);
                    if !ch.is_whitespace() {
                        pen.ink(&mut rgba, [w, h], ch, [x * s, (top + BASELINE) * s], ink);
                    }
                }
                text.push('\n');
                let end = MARGIN + line.chars().count() as f32 * ADVANCE;
                glyphs.push([end * s, top * s, end * s, top * s]);
            }
            PageReply::Rendered {
                width: w,
                height: h,
                rgba,
                text: Some(TextLayer::new(w as f32, h as f32, text, glyphs)),
            }
        }
    }
}

/// Sets glyphs in the bundled JetBrains Mono at [`TYPE_SIZE`] on a page `s` pixels a point.
struct Pen {
    font: FontRef<'static>,
    context: ScaleContext,
    ppem: f32,
}

impl Pen {
    fn new(s: f32) -> Pen {
        let font = FontRef::from_index(strata_kit::fonts::JETBRAINS_MONO, 0)
            .expect("the bundled JetBrains Mono parses");
        Pen {
            font,
            context: ScaleContext::new(),
            ppem: TYPE_SIZE * s,
        }
    }

    /// Inks `ch` with its baseline origin at `at` (pixels), darkening toward `ink` by the
    /// glyph's coverage.
    fn ink(&mut self, rgba: &mut [u8], [w, h]: [u32; 2], ch: char, at: [f32; 2], ink: u8) {
        let id = self.font.charmap().map(ch);
        let mut scaler = self.context.builder(self.font).size(self.ppem).build();
        let Some(image) = Render::new(&[Source::Outline])
            .format(Format::Alpha)
            .offset(Vector::new(at[0].fract(), at[1].fract()))
            .render(&mut scaler, id)
        else {
            return;
        };
        let p = image.placement;
        let x0 = at[0].floor() as i32 + p.left;
        let y0 = at[1].floor() as i32 - p.top;
        for gy in 0..p.height as i32 {
            for gx in 0..p.width as i32 {
                let (x, y) = (x0 + gx, y0 + gy);
                if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                    continue;
                }
                let a = u32::from(image.data[(gy * p.width as i32 + gx) as usize]);
                let v = (255 - a * (255 - u32::from(ink)) / 255) as u8;
                let i = ((y as u32 * w + x as u32) * 4) as usize;
                for c in &mut rgba[i..i + 3] {
                    *c = (*c).min(v);
                }
            }
        }
    }
}

/// The fake renderer: one page at a time, each due [`RENDER_SECONDS`] after it starts.
#[derive(Debug, Default)]
pub struct Renderer {
    pub busy: Option<(RenderRequest, f64)>,
    /// Holds every request: the loading scene.
    pub paused: bool,
    /// Pages rendered, `(page, width)`, in order.
    pub rendered: Vec<(usize, u32)>,
    /// Pages whose requests were cancelled, in order.
    pub cancelled: Vec<usize>,
    /// How many renders ran at once at most: the host's one at a time.
    pub most_at_once: usize,
}

impl Renderer {
    fn line(&self) -> String {
        let tail = |v: &[String]| v[v.len().saturating_sub(8)..].join(" ");
        let rendered: Vec<String> = self
            .rendered
            .iter()
            .map(|(p, w)| format!("p{} {w}", p + 1))
            .collect();
        let cancelled: Vec<String> = self
            .cancelled
            .iter()
            .map(|p| format!("p{}", p + 1))
            .collect();
        format!(
            "Rendered {} · Cancelled {}",
            if rendered.is_empty() {
                "none".into()
            } else {
                tail(&rendered)
            },
            if cancelled.is_empty() {
                "none".into()
            } else {
                tail(&cancelled)
            },
        )
    }
}

/// Which document the page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fixture {
    #[default]
    Pages,
    Mixed,
    /// Short landscape pages: two and a half in view.
    Landscape,
    Long,
    Locked,
    Failed,
}

/// C12's demo state, held in `Gallery::c12`.
#[derive(Default)]
pub struct Demo {
    pub fixture: Fixture,
    pub drawer: DrawerState,
    pub pdf: PdfState,
    pub doc: Vec<FakePage>,
    pub renderer: Renderer,
    /// The chooser's preview: no Open under the locked message.
    pub chooser: bool,
    /// 10xer mode's letters on the message.
    pub letters: bool,
    /// Once pages 1 and 2 have text, select across them (the selection scene).
    pub select_once: bool,
    /// Once the view has laid out, zoom to this (the zoomed scene).
    pub zoom_once: Option<f32>,
    pub last: String,
    pub copied: String,
    pub selection: String,
    pub log: String,
    /// The interface text size Ctrl+wheel and Ctrl+0 would change when the view leaves them.
    pub text_size: i32,
    /// The page's clock at its last poll.
    pub now: f64,
}

fn setup(g: &mut Gallery, dark: bool, fixture: Fixture) {
    g.open("c12", dark);
    let mut pdf = PdfState::new("pdf");
    let doc = match fixture {
        Fixture::Pages => fake_doc(12, false),
        Fixture::Mixed => fake_doc(8, true),
        Fixture::Landscape => (0..6)
            .map(|p| FakePage {
                size: [LETTER[1], LETTER[0]],
                kind: FakeKind::Text(prose(p, 26)),
            })
            .collect(),
        Fixture::Long => fake_doc(500, false),
        Fixture::Locked | Fixture::Failed => Vec::new(),
    };
    match fixture {
        Fixture::Locked => pdf.locked(),
        Fixture::Failed => pdf.failed("The sandboxed preview renderer failed"),
        _ => pdf.open(PdfDocument::new(
            doc.len(),
            doc.first().map_or(LETTER, |p| p.size),
        )),
    }
    let mut drawer = DrawerState::default();
    drawer.open_at_once();
    g.c12 = Demo {
        fixture,
        drawer,
        pdf,
        doc,
        text_size: 13,
        ..Demo::default()
    };
}

fn selection(g: &mut Gallery, dark: bool) {
    setup(g, dark, Fixture::Pages);
    g.c12.select_once = true;
}

fn zoomed(g: &mut Gallery, dark: bool) {
    setup(g, dark, Fixture::Pages);
    g.c12.zoom_once = Some(2.0);
}

fn loading(g: &mut Gallery, dark: bool) {
    setup(g, dark, Fixture::Pages);
    g.c12.renderer.paused = true;
}

/// Find turned on, with "strata" typed.
fn find(g: &mut Gallery, dark: bool) {
    setup(g, dark, Fixture::Pages);
    g.c12.pdf.set_searchable(true);
    g.c12.drawer.start_find("strata");
    g.c12.pdf.find("strata");
}

/// Find turned on, the bar closed: scripts open it.
fn searchable(g: &mut Gallery, dark: bool) {
    setup(g, dark, Fixture::Pages);
    g.c12.pdf.set_searchable(true);
}

fn chooser_locked(g: &mut Gallery, dark: bool) {
    setup(g, dark, Fixture::Locked);
    g.c12.chooser = true;
}

fn letters_locked(g: &mut Gallery, dark: bool) {
    setup(g, dark, Fixture::Locked);
    g.c12.letters = true;
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c12", dark)`, then fills in `g.c12`.
pub const SCENES: &[Scene] = &[
    ("c12-pages-light", |g| setup(g, false, Fixture::Pages)),
    ("c12-pages-dark", |g| setup(g, true, Fixture::Pages)),
    ("c12-loading-light", |g| loading(g, false)),
    ("c12-loading-dark", |g| loading(g, true)),
    ("c12-mixed-light", |g| setup(g, false, Fixture::Mixed)),
    ("c12-mixed-dark", |g| setup(g, true, Fixture::Mixed)),
    ("c12-selection-light", |g| selection(g, false)),
    ("c12-selection-dark", |g| selection(g, true)),
    ("c12-zoomed-light", |g| zoomed(g, false)),
    ("c12-zoomed-dark", |g| zoomed(g, true)),
    ("c12-find-light", |g| find(g, false)),
    ("c12-find-dark", |g| find(g, true)),
    ("c12-locked-light", |g| setup(g, false, Fixture::Locked)),
    ("c12-locked-dark", |g| setup(g, true, Fixture::Locked)),
    ("c12-failed-light", |g| setup(g, false, Fixture::Failed)),
    ("c12-failed-dark", |g| setup(g, true, Fixture::Failed)),
];

/// Scenes for review with `--scene` only, kept out of the goldens: the 500-page document, short
/// landscape pages, find on with its bar closed, and the locked message in the chooser and in
/// 10xer mode.
pub const REVIEW: &[Scene] = &[
    ("c12-landscape-light", |g| {
        setup(g, false, Fixture::Landscape)
    }),
    ("c12-landscape-dark", |g| setup(g, true, Fixture::Landscape)),
    ("c12-long-light", |g| setup(g, false, Fixture::Long)),
    ("c12-long-dark", |g| setup(g, true, Fixture::Long)),
    ("c12-searchable-light", |g| searchable(g, false)),
    ("c12-searchable-dark", |g| searchable(g, true)),
    ("c12-chooser-locked-light", |g| chooser_locked(g, false)),
    ("c12-chooser-locked-dark", |g| chooser_locked(g, true)),
    ("c12-letters-locked-light", |g| letters_locked(g, false)),
    ("c12-letters-locked-dark", |g| letters_locked(g, true)),
];

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let d = &g.c12;
    let look = g.look();
    let content = PdfView::new(|g: &Gallery| &g.c12.pdf, |g: &mut Gallery| &mut g.c12.pdf)
        .open_button(!d.chooser)
        .letters(d.letters)
        .on_intent(intent)
        .build(g, look, cx);
    let drawer = PreviewDrawer::new(&d.drawer, |g: &mut Gallery| &mut g.c12.drawer)
        .header(
            HeaderInfo::new("report.pdf")
                .icon(look.icon(IconName::FileText))
                .details(["1.2 MB", "Today 09:41", "PDF document"]),
        )
        .hosted(&d.pdf, |g: &mut Gallery| &mut g.c12.pdf)
        .content(content)
        .on_intent(drawer_intent)
        .build(files(look, d), look, cx);
    // Stands in for Strata's window: a Ctrl+wheel no one took changes the text size.
    col().w_full().h_full().child(drawer).on_wheel(
        |g: &mut Gallery, e: WheelEvent, _tx: &mut Tx| {
            if !e.mods.ctrl {
                return false;
            }
            g.c12.text_size += if e.dy > 0.0 { 1 } else { -1 };
            true
        },
    )
}

fn intent(g: &mut Gallery, intent: PdfIntent, _tx: &mut Tx) {
    let d = &mut g.c12;
    match intent {
        PdfIntent::Open => d.last = "Open".into(),
        PdfIntent::Leave => {
            d.last = "Leave".into();
            d.drawer.focus_files();
        }
        PdfIntent::Copied(t) => d.copied = t.to_string(),
        _ => {}
    }
}

fn drawer_intent(g: &mut Gallery, intent: DrawerIntent, _tx: &mut Tx) {
    if intent == DrawerIntent::Closed {
        g.c12.last = "Closed".into();
    }
}

fn files(look: &Look, d: &Demo) -> El {
    let k = &look.kit;
    let line = |key: &str, s: &str| {
        let name = if s.is_empty() { "None" } else { s };
        text(name.replace('\n', "⏎"))
            .key(key.to_owned())
            .label(name.to_owned())
            .max_lines(2)
            .wrap()
            .w_full()
            .font(k.ty.small)
            .color(k.c.text_muted)
            .px(look.space.md)
            .py(look.space.xs)
    };
    let text_size = format!("Text size {}", d.text_size);
    let status = col()
        .w_full()
        .child(line("c12-log", &d.log))
        .child(line("c12-selection", &d.selection))
        .child(line("c12-copied", &d.copied))
        .child(line("c12-last", &d.last))
        .child(line("c12-text-size", &text_size));
    let list = col()
        .key("files")
        .role("listbox")
        .label("Files")
        .focusable(true)
        .w_full()
        .grow(1.0)
        .padding(look.space.xs)
        .children((0..4).map(|i| {
            el().role("option")
                .label(format!("report-{i}.pdf"))
                .w_full()
                .h(look.row.list)
                .px(look.space.md)
                .items_center()
                .child(
                    text(format!("report-{i}.pdf"))
                        .font(k.ty.ui)
                        .color(k.c.text_base),
                )
        }));
    col().w_full().h_full().child(list).child(status)
}

/// Keys no focused element took: what the view passed on reaches the window here. Ctrl+0
/// resets the text size, and a Ctrl+C or Ctrl+A is recorded as passed on.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, _tx: &mut Tx) -> bool {
    let d = &mut g.c12;
    if ctrl_zero_key(key).is_some() {
        d.text_size = 13;
        d.last = "Text size reset".into();
        return true;
    }
    for c in ['a', 'c'] {
        if key.is_command(c) {
            d.last = format!("Ctrl+{} passed on", c.to_ascii_uppercase());
            return true;
        }
    }
    if key.key == Key::Escape {
        return false;
    }
    false
}

/// Runs at the start of every frame: the fake renderer's work, the view's clock and find's
/// scrolls, as Strata's poll would. True when state changed.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    let dark = g.dark;
    let d = &mut g.c12;
    d.now = t;
    let mut changed = d.pdf.tick(t);
    for c in d.pdf.take_cancelled() {
        if d.renderer.busy.is_some_and(|(r, _)| r.ticket == c) {
            d.renderer.busy = None;
        }
        d.renderer.cancelled.push(c.page());
    }
    if let Some((req, due)) = d.renderer.busy
        && t + 1e-9 >= due
    {
        d.renderer.busy = None;
        let reply = render(&d.doc[req.page], req.box_px, dark);
        d.renderer.rendered.push((req.page, req.box_px[0]));
        changed |= d.pdf.complete(req.ticket, reply).commits();
    }
    if d.renderer.busy.is_none()
        && !d.renderer.paused
        && let Some(req) = d.pdf.pop()
    {
        d.renderer.busy = Some((req, t + RENDER_SECONDS));
        d.renderer.most_at_once = d.renderer.most_at_once.max(1);
    }
    if let Some((_, due)) = d.renderer.busy {
        tx.wake_after(std::time::Duration::from_secs_f64((due - t).max(0.0)));
    }
    if d.select_once && d.pdf.held(0).is_some() && d.pdf.held(1).is_some() {
        d.select_once = false;
        d.pdf.select((0, 2_150), (1, 160));
        d.pdf.show_page(1, 0.0);
        changed = true;
    }
    if let Some(level) = d.zoom_once
        && !d.pdf.visible().is_empty()
    {
        d.zoom_once = None;
        d.pdf.set_level(level);
        changed = true;
    }
    changed |= d.pdf.flush(tx);
    let log = d.renderer.line();
    let selection = d.pdf.selected_text();
    if log != d.log || selection != d.selection {
        d.log = log;
        d.selection = selection;
        changed = true;
    }
    changed
}

//! The gallery page for C11 DocumentView: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `c11-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-2-composites/C11-document-view.md`.
//!
//! A document in C9's drawer beside a fake listing. Each fixture is what Strata's converters
//! would hand over for one format: Strata's parsers aren't here, so the fixtures are built as
//! units directly, with the shapes Strata's layout gives (list markers in the text, Title as
//! a level-1 heading, ODT's "Column N" titles, notices for omitted content). The page stands
//! in for Strata's policy: it records a link click instead of opening it, keeps the wrap
//! preference, and moves focus back to the files on Leave.

use std::sync::Arc;
use std::time::Duration;

use rune::{Cx, El, KeyEvent, Pixels, Tx, col, el, text};
use strata_components::code_view::highlight::{CodeBlocks, Highlighter};
use strata_components::document_view::{
    Cell, Document, DocumentIntent, DocumentState, DocumentView, ListChildKind, Media, MediaState,
    Span, SpanStyle, Unit, UnitKind,
};
use strata_components::preview_drawer::{
    DrawerIntent, DrawerState, HeaderInfo, HostedPreview, PreviewDrawer,
};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_kit::theme::syntax::syntect_theme;

use crate::{Gallery, Scene, SyntaxKey};

/// How long a golden scene's first build waits for a code block's colours.
const GOLDEN_WAIT: Duration = Duration::from_secs(30);

/// Which fixture the page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fixture {
    #[default]
    Markdown,
    Html,
    Rtf,
    Docx,
    Odt,
    CodeSplit,
    Media,
    WrapOff,
    TableOnly,
    /// A long document: 2,000 units, for the scrolling scripts.
    Long,
}

/// The highlighter, started by the scene (or on the page's first poll).
pub struct Live {
    pub highlighter: Highlighter,
    pub blocks: CodeBlocks,
    syntax: SyntaxKey,
    waker: bool,
}

impl Live {
    fn new(look: &Look, wait: bool) -> Live {
        let highlighter = Highlighter::new(Arc::new(syntect_theme(look)));
        let mut blocks = highlighter.blocks();
        if wait {
            blocks.wait_on_miss(Some(GOLDEN_WAIT));
        }
        Live {
            highlighter,
            blocks,
            syntax: SyntaxKey::of(look),
            waker: false,
        }
    }
}

/// C11's demo state, held in `Gallery::c11`.
#[derive(Default)]
pub struct Demo {
    pub fixture: Fixture,
    pub drawer: DrawerState,
    pub doc: DocumentState,
    pub live: Option<Live>,
    /// The last intent, as the page's status line reads it ("Open https://…", "Leave").
    pub last: String,
    /// What copying the selection would take, read from the page's poll: scripts check
    /// selections and the copy rule through the line keyed `c11-selection`.
    pub selection: String,
}

fn spans(text: &str, marks: &[(&str, SpanStyle)]) -> Vec<Span> {
    marks
        .iter()
        .map(|(part, style)| {
            let at = text.find(part).expect("part of the text");
            Span::new(at..at + part.len(), style.clone())
        })
        .collect()
}

fn para(t: &str, marks: &[(&str, SpanStyle)]) -> Unit {
    Unit::new(UnitKind::Paragraph, t).spans(spans(t, marks))
}

fn heading(level: u8, t: &str) -> Unit {
    Unit::new(UnitKind::Heading(level), t)
}

fn item(depth: usize, marker: &str, t: &str) -> Unit {
    let full = format!("{marker} {t}");
    Unit::new(UnitKind::ListItem { depth }, full.clone())
        .spans([Span::new(0..marker.len(), SpanStyle::Accent)])
}

fn code(language: Option<&str>, t: &str) -> Unit {
    Unit::new(
        UnitKind::Code {
            list_depth: None,
            language: language.map(str::to_owned),
        },
        t,
    )
}

fn table(rows: &[&[&str]], header: bool) -> Unit {
    let rows: Vec<Vec<Cell>> = rows
        .iter()
        .enumerate()
        .map(|(r, cells)| {
            cells
                .iter()
                .map(|c| {
                    if header && r == 0 {
                        Cell::header(*c)
                    } else {
                        Cell::new(*c)
                    }
                })
                .collect()
        })
        .collect();
    let copy = rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|c| c.text.as_str())
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    Unit::new(
        UnitKind::Table {
            list_depth: None,
            rows,
        },
        "",
    )
    .copy_text(copy)
}

fn media(media: Media, alt: &str) -> Unit {
    let copy = match &media {
        Media::Image => format!("[Image: {alt}]\n"),
        Media::Diagram { source } => format!("{source}\n"),
        Media::Equation { source, .. } => format!("${source}$\n"),
    };
    Unit::new(
        UnitKind::Media {
            list_depth: None,
            media,
        },
        alt,
    )
    .copy_text(copy)
}

fn gap() -> Unit {
    Unit::new(UnitKind::Gap, "").copy_text("")
}

const LINK: &str = "https://example.org/strata";

fn link() -> SpanStyle {
    SpanStyle::Link(LINK.into())
}

/// The Markdown fixture: every block kind and inline style.
pub fn markdown() -> Document {
    let math = "Its area is \u{fffc} square units.";
    let at = math.find('\u{fffc}').expect("placeholder");
    Document::new(vec![
        heading(1, "Release notes"),
        para(
            "Strata 1.2 draws documents natively: bold, italic, struck, code and a link.",
            &[
                ("bold", SpanStyle::Bold),
                ("italic", SpanStyle::Italic),
                ("struck", SpanStyle::Strike),
                ("code", SpanStyle::Mono),
                ("a link", link()),
            ],
        ),
        heading(2, "Install"),
        item(0, "•", "Download the release"),
        item(1, "•", "Check its signature"),
        item(0, "1.", "Run the installer"),
        code(
            Some("rust"),
            "fn main() {\n    let name = \"Strata\";\n    println!(\"hello, {name}\");\n}",
        ),
        Unit::new(
            UnitKind::Quote,
            "Previews never run the document's own code.",
        ),
        table(
            &[
                &["Format", "Source view", "Wraps"],
                &["Markdown", "Yes", "Preference"],
                &["ODT", "No", "Always"],
            ],
            true,
        ),
        Unit::new(
            UnitKind::Rule { list_depth: None },
            "────────────────────────",
        ),
        heading(3, "Equations"),
        Unit::new(UnitKind::Paragraph, math)
            .spans([Span::new(at..at + 3, SpanStyle::Math("\\pi r^2".into()))])
            .copy_text("Its area is $\\pi r^2$ square units.\n"),
        media(
            Media::Equation {
                source: "e^{i\\pi} + 1 = 0".into(),
                display: true,
            },
            "",
        ),
        heading(4, "Level four"),
        heading(5, "Level five"),
        heading(6, "Level six"),
    ])
}

fn html() -> Document {
    Document::new(vec![
        heading(1, "Team handbook"),
        para(
            "Underlined and inserted text keep their underline.",
            &[
                ("Underlined", SpanStyle::Underline),
                ("inserted", SpanStyle::Underline),
            ],
        ),
        para(
            "Only web links open: see the handbook online.",
            &[("the handbook online", link())],
        ),
        table(
            &[&["Day", "Room"], &["Monday", "4.02"], &["Friday", "Lab"]],
            true,
        ),
    ])
    .notices(["Unsupported or active HTML content was omitted."])
}

fn rtf() -> Document {
    Document::new(vec![
        para(
            "RTF keeps bold, italic, underline and strikethrough.",
            &[
                ("bold", SpanStyle::Bold),
                ("italic", SpanStyle::Italic),
                ("underline", SpanStyle::Underline),
                ("strikethrough", SpanStyle::Strike),
            ],
        ),
        para("A [😀] B, and Windows-1252: café, naïve.", &[]),
    ])
}

fn docx() -> Document {
    Document::new(vec![
        heading(1, "Quarterly report"),
        heading(2, "Summary for the board"),
        Unit::new(UnitKind::Quote, "Growth held through the quarter."),
        item(0, "•", "Revenue"),
        item(1, "1.", "Subscriptions"),
        item(1, "2.", "Services"),
        gap(),
        table(
            &[
                &["Region", "Q1", "Q2"],
                &["North", "12", "14"],
                &["South", "9", "11"],
            ],
            true,
        ),
    ])
}

fn odt() -> Document {
    Document::new(vec![
        heading(1, "Field notes"),
        heading(6, "Outline level nine, drawn at six"),
        Unit::new(UnitKind::Quote, "A quotation paragraph."),
        code(
            None,
            "    indented preformatted text\n\tand a tab, which keeps its width",
        ),
        item(0, "•", "Level one"),
        item(3, "•", "Level four"),
        item(7, "•", "Level eight, the deepest"),
        Unit::new(
            UnitKind::ListChild {
                depth: 7,
                kind: ListChildKind::Paragraph,
            },
            "A ninth level continues at the eighth.",
        ),
        table(
            &[
                &["Column 1", "Column 2"],
                &["no header-row group", "so Strata names the columns"],
            ],
            true,
        ),
    ])
    .notices(["Document preview reached its size budget; open the file to see all content."])
}

fn code_split() -> Document {
    let block = |first, last, wrap, t: &str| {
        Unit::new(
            UnitKind::Code {
                list_depth: None,
                language: Some("rust".into()),
            },
            t,
        )
        .copy_text(if wrap { format!("{t}\n") } else { t.to_owned() })
        .part(first, last, wrap)
    };
    Document::new(vec![
        para("A block over 32 KiB comes in parts and draws plain:", &[]),
        block(true, false, true, "// part one of a long block\nlet a = 1;"),
        block(false, false, false, "let b = \"a line over 2 KiB, cut here"),
        block(false, true, true, " and carried on\";\nlet c = 3;"),
        para("After the block.", &[]),
    ])
}

fn media_doc() -> Document {
    Document::new(vec![
        para(
            "An image still loading, one that failed, and a diagram:",
            &[],
        ),
        media(Media::Image, "harbour at dawn"),
        media(Media::Image, "a missing photo"),
        media(
            Media::Diagram {
                source: "graph LR\n  A --> B".into(),
            },
            "",
        ),
    ])
}

fn wrap_off() -> Document {
    let long = "This paragraph is longer than the pane, so with wrap off its row scrolls \
                sideways on its own and keeps the whole line on one row.";
    Document::new(vec![
        heading(2, "Wrap off"),
        para(long, &[]),
        code(
            Some("rust"),
            "let wide = [\"a line of code much wider than the preview pane\", \"which scrolls\"];",
        ),
    ])
}

fn table_only() -> Document {
    Document::new(vec![table(
        &[&["Name", "Size"], &["a.txt", "1 KB"], &["b.txt", "2 KB"]],
        true,
    )])
}

/// 2,000 units: headings every 20, paragraphs between, a link in the last.
pub fn long() -> Document {
    let mut units = Vec::new();
    for i in 0..1999 {
        if i % 20 == 0 {
            units.push(heading(2, &format!("Section {}", i / 20 + 1)));
        } else {
            units.push(para(
                &format!(
                    "Paragraph {i} of the long document, long enough to wrap across the pane, \
                     so that a click in the middle of it lands on a word."
                ),
                &[],
            ));
        }
    }
    units.push(para("The end, with a link.", &[("a link", link())]));
    Document::new(units)
}

/// A fake rendered image: a soft two-tone wash in the look's colours.
fn image(look: &Look, w: u32, h: u32) -> Pixels {
    let a = look.kit.c.layer_03;
    let b = look.kit.c.text_accent;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let t = (x + y) as f32 / (w + h) as f32;
            let mix = |p: f32, q: f32| ((p + (q - p) * t) * 255.0) as u8;
            rgba.extend_from_slice(&[mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b), 255]);
        }
    }
    let px = Pixels::new();
    px.set(w, h, rgba);
    px
}

/// The page's state for `fixture`: the document, its media and its form's capabilities.
pub fn setup(g: &mut Gallery, dark: bool, fixture: Fixture) {
    g.open("c11", dark);
    let doc = match fixture {
        Fixture::Markdown => markdown(),
        Fixture::Html => html(),
        Fixture::Rtf => rtf(),
        Fixture::Docx => docx(),
        Fixture::Odt => odt(),
        Fixture::CodeSplit => code_split(),
        Fixture::Media => media_doc(),
        Fixture::WrapOff => wrap_off(),
        Fixture::TableOnly => table_only(),
        Fixture::Long => long(),
    };
    let mut state = DocumentState::new(Arc::new(doc));
    // DOCX and ODT have no Source view and always wrap.
    let rich = matches!(fixture, Fixture::Docx | Fixture::Odt);
    state.set_source_available(!rich);
    state.set_wrap(if rich {
        None
    } else {
        Some(fixture != Fixture::WrapOff)
    });
    match fixture {
        Fixture::Markdown => state.set_media(12, MediaState::Ready(image(&g.look, 220, 48))),
        Fixture::Media => {
            state.set_media(2, MediaState::Failed);
            state.set_media(3, MediaState::Ready(image(&g.look, 180, 90)));
        }
        _ => {}
    }
    let mut drawer = DrawerState::default();
    drawer.open_at_once();
    g.c11 = Demo {
        fixture,
        drawer,
        doc: state,
        live: Some(Live::new(&g.look, true)),
        last: String::new(),
        selection: String::new(),
    };
}

/// Find open on the Markdown fixture at its second match.
fn find_open(g: &mut Gallery, dark: bool) {
    setup(g, dark, Fixture::Markdown);
    g.c11.drawer.start_find("in");
    g.c11.doc.find("in");
    g.c11.doc.find_step(true);
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c11", dark)`, then fills in `g.c11`.
pub const SCENES: &[Scene] = &[
    ("c11-markdown-light", |g| setup(g, false, Fixture::Markdown)),
    ("c11-markdown-dark", |g| setup(g, true, Fixture::Markdown)),
    ("c11-html-light", |g| setup(g, false, Fixture::Html)),
    ("c11-html-dark", |g| setup(g, true, Fixture::Html)),
    ("c11-rtf-light", |g| setup(g, false, Fixture::Rtf)),
    ("c11-rtf-dark", |g| setup(g, true, Fixture::Rtf)),
    ("c11-docx-light", |g| setup(g, false, Fixture::Docx)),
    ("c11-docx-dark", |g| setup(g, true, Fixture::Docx)),
    ("c11-odt-light", |g| setup(g, false, Fixture::Odt)),
    ("c11-odt-dark", |g| setup(g, true, Fixture::Odt)),
    ("c11-code-split-light", |g| {
        setup(g, false, Fixture::CodeSplit)
    }),
    ("c11-code-split-dark", |g| {
        setup(g, true, Fixture::CodeSplit)
    }),
    ("c11-media-light", |g| setup(g, false, Fixture::Media)),
    ("c11-media-dark", |g| setup(g, true, Fixture::Media)),
    ("c11-find-light", |g| find_open(g, false)),
    ("c11-find-dark", |g| find_open(g, true)),
    ("c11-wrap-off-light", |g| setup(g, false, Fixture::WrapOff)),
    ("c11-wrap-off-dark", |g| setup(g, true, Fixture::WrapOff)),
    ("c11-table-only-light", |g| {
        setup(g, false, Fixture::TableOnly)
    }),
    ("c11-table-only-dark", |g| {
        setup(g, true, Fixture::TableOnly)
    }),
];

/// Scenes for review with `--scene` only, kept out of the goldens: the long document the
/// scrolling scripts run over.
pub const REVIEW: &[Scene] = &[
    ("c11-long-light", |g| setup(g, false, Fixture::Long)),
    ("c11-long-dark", |g| setup(g, true, Fixture::Long)),
];

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let d = &g.c11;
    let look = g.look();
    let (name, kind) = match d.fixture {
        Fixture::Markdown | Fixture::CodeSplit | Fixture::Media | Fixture::Long => {
            ("notes.md", "Markdown")
        }
        Fixture::Html => ("handbook.html", "HTML document"),
        Fixture::Rtf => ("letter.rtf", "RTF document"),
        Fixture::Docx => ("report.docx", "Word document"),
        Fixture::Odt => ("report.odt", "OpenDocument text"),
        Fixture::WrapOff => ("wide.md", "Markdown"),
        Fixture::TableOnly => ("sizes.md", "Markdown"),
    };
    let mut view = DocumentView::new(|g: &Gallery| &g.c11.doc, |g: &mut Gallery| &mut g.c11.doc)
        .on_intent(intent);
    if let Some(live) = &d.live {
        view = view.blocks(&live.blocks);
    }
    let content = view.build(g, look, cx);
    PreviewDrawer::new(&d.drawer, |g: &mut Gallery| &mut g.c11.drawer)
        .header(
            HeaderInfo::new(name)
                .icon(look.icon(IconName::FileCode))
                .details(["2.4 KB", "Today 09:41", kind]),
        )
        .hosted(&d.doc, |g: &mut Gallery| &mut g.c11.doc)
        .content(content)
        .on_intent(drawer_intent)
        .build(files(look, &d.last, &d.selection), look, cx)
}

fn intent(g: &mut Gallery, intent: DocumentIntent, _tx: &mut Tx) {
    let d = &mut g.c11;
    match intent {
        DocumentIntent::OpenLink(uri) => d.last = format!("Open {uri}"),
        DocumentIntent::Leave => {
            d.last = "Leave".into();
            d.drawer.focus_files();
        }
        _ => {}
    }
}

fn drawer_intent(g: &mut Gallery, intent: DrawerIntent, _tx: &mut Tx) {
    let d = &mut g.c11;
    match intent {
        DrawerIntent::SetWrap(on) => d.doc.set_wrap(Some(on)),
        DrawerIntent::ShowView(form) => d.last = format!("Show {form:?}"),
        DrawerIntent::Closed => d.last = "Closed".into(),
        _ => {}
    }
}

/// The fake listing beside the drawer, its focus group keyed `files`, and under it two lines
/// scripts read: the last intent (`c11-last-intent`) and what copying the selection would
/// take (`c11-selection`), each named "None" while empty.
fn files(look: &Look, last: &str, selection: &str) -> El {
    let k = &look.kit;
    let line = |key: &str, s: &str| {
        let name = if s.is_empty() { "None" } else { s };
        text(name.replace('\n', "⏎").replace('\t', "⇥"))
            .key(key.to_owned())
            .label(name.to_owned())
            .wrap()
            .w_full()
            .font(k.ty.small)
            .color(k.c.text_muted)
            .px(look.space.md)
            .py(look.space.xs)
    };
    let status = col()
        .w_full()
        .child(line("c11-last-intent", last))
        .child(line("c11-selection", selection));
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
                .label(format!("file-{i}.md"))
                .w_full()
                .h(look.row.list)
                .px(look.space.md)
                .items_center()
                .child(
                    text(format!("file-{i}.md"))
                        .font(k.ty.ui)
                        .color(k.c.text_base),
                )
        }));
    col().w_full().h_full().child(list).child(status)
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
pub fn on_key(_g: &mut Gallery, _key: &KeyEvent, _tx: &mut Tx) -> bool {
    false
}

/// Runs at the start of every frame while this page is open, with the clock in seconds:
/// starts the highlighter, takes the colours that landed, runs Copy code's clock and applies
/// find's selections, as Strata's poll would. True when state changed.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    let d = &mut g.c11;
    let mut changed = false;
    let live = match &mut d.live {
        Some(live) => live,
        None => {
            changed = true;
            d.live.insert(Live::new(&g.look, false))
        }
    };
    if !live.waker
        && let Some(waker) = &g.waker
    {
        live.highlighter.set_waker(waker.clone());
        live.waker = true;
    }
    if live.syntax != SyntaxKey::of(&g.look) {
        live.highlighter.set_theme(Arc::new(syntect_theme(&g.look)));
        live.syntax = SyntaxKey::of(&g.look);
    }
    changed |= live.blocks.poll();
    changed |= d.doc.tick(t);
    changed |= d.doc.flush(tx);
    let selection = tx
        .text_selection()
        .map(|sel| d.doc.copy_selection(sel))
        .unwrap_or_default();
    if selection != d.selection {
        d.selection = selection;
        changed = true;
    }
    changed
}

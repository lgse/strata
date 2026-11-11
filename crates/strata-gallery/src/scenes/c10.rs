//! The gallery page for C10b CodeView: its scenes, demo state, view and key and poll hooks. Scene
//! names start with `c10-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-2-composites/C10b-code-view.md`.
//!
//! A source preview in C9's drawer beside a fake listing, coloured by the real highlighter
//! thread, and the custom-action script editor with C21's field menu. The page stands in for
//! Strata's policy: it keeps the wrap preference and moves focus back to the files on Leave.
//! Golden scenes wait for the colours of the lines on screen before the first frame
//! ([`CodeState::settle`], headless only); the review scenes don't.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use rune::{Cx, El, KeyEvent, TextEdit, Tx, col, el, text};
use strata_components::code_view::highlight::Highlighter;
use strata_components::code_view::{CodeEditor, CodeIntent, CodeState, CodeView, EditorState};
use strata_components::context_menu::{FieldMenu, FieldMenuState};
use strata_components::preview_drawer::{
    DocumentView, DrawerIntent, DrawerState, HeaderInfo, PreviewDrawer,
};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_kit::theme::syntax::syntect_theme;

use crate::{Gallery, Scene, SyntaxKey};

/// How long a golden scene waits for the highlighter (debug builds parse slowly).
const GOLDEN_WAIT: Duration = Duration::from_secs(60);

/// The lines a golden scene waits on: more than the drawer shows.
const ON_SCREEN: usize = 120;

/// The editor's field menu's key.
const FIELD_MENU: &str = "c10-field-menu";

/// Which fixture the page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fixture {
    /// A Rust file, wrap on.
    #[default]
    Rust,
    /// Long lines, wrap off.
    WrapOff,
    /// Plain text with no grammar, cut at Strata's limit.
    Plain,
    /// A 5 KiB line among short ones, split into 2 KiB rows, wrap off.
    LongLine,
    /// 50,000 generated lines.
    Large,
    /// A Markdown file's source: the header offers View rendered.
    MarkdownSource,
    /// The custom-action script editor.
    Editor,
}

/// The editor's fields, for its field menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Script,
}

/// The highlighter, started by the scene (or on the page's first poll).
pub struct Live {
    pub highlighter: Highlighter,
    syntax: SyntaxKey,
    waker: bool,
}

impl Live {
    fn new(look: &Look) -> Live {
        Live {
            highlighter: Highlighter::new(Arc::new(syntect_theme(look))),
            syntax: SyntaxKey::of(look),
            waker: false,
        }
    }
}

/// C10b's demo state, held in `Gallery::c10`.
#[derive(Default)]
pub struct Demo {
    pub fixture: Fixture,
    pub drawer: DrawerState,
    pub code: CodeState,
    pub editor: Option<EditorState>,
    pub fields: FieldMenuState<Field>,
    pub live: Option<Live>,
    /// The last intent, as the page's status line reads it ("Leave", "Show Rendered").
    pub last: String,
    /// What copying the selection would take, read from the page's poll: scripts check
    /// selections and copies through the line keyed `c10-selection`.
    pub selection: String,
}

const RUST: &str = r#"//! Sizes of the entries in a folder.
use std::fs;
use std::path::Path;

/// One entry: its name and size in bytes.
#[derive(Debug, Clone)]
pub struct Entry {
	pub name: String,
	pub size: u64,
}

/// Every file in `dir`, largest first.
pub fn entries(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for item in fs::read_dir(dir)? {
        let item = item?;
        let meta = item.metadata()?;
        if meta.is_file() {
            out.push(Entry {
                name: item.file_name().to_string_lossy().into_owned(),
                size: meta.len(),
            });
        }
    }
    out.sort_by(|a, b| b.size.cmp(&a.size));
    Ok(out)
}

/// The total size, in bytes: "fn" appears in this comment too.
pub fn total(entries: &[Entry]) -> u64 {
    entries.iter().map(|e| e.size).sum() // no overflow check
}

fn main() {
    let found = entries(Path::new(".")).unwrap_or_default();
    println!("{} files, {} bytes", found.len(), total(&found));
}
"#;

const PLAIN: &str = "Shopping list\n\neggs, flour, sugar\nbutter (unsalted)\n\nCall the \
plumber before Friday about the kitchen tap, which still drips at night.\nTake the bottles \
back.\n\tA tabbed line.\nÉCLAIR for dessert, or an éclair from the bakery.\n";

const SCRIPT: &str = r#"#!/usr/bin/env bash
# Rename every photo by the day it was taken.
set -euo pipefail

dir="${1:-$HOME/Pictures}"
count=0
for f in "$dir"/*.jpg; do
    stamp=$(date -r "$f" +%Y-%m-%d)
    mv -n "$f" "$dir/$stamp-$((count++)).jpg"
done

echo "Renamed $count photos in $dir"
"#;

const MARKDOWN: &str = "# Notes\n\nSome **bold** text and a [link](https://example.com).\n\n\
```rust\nfn main() {}\n```\n\n- one\n- two\n";

fn wrap_off() -> String {
    let mut s = String::from("// Wide lines: wrap is off, so they scroll sideways.\n");
    // Wide characters draw wider than their count: the list is measured for them.
    let _ = writeln!(s, "let names = [\"{}\", \"😀🎉\"];", "漢字かな".repeat(12));
    for i in 0..30 {
        let _ = writeln!(
            s,
            "let row_{i} = [{}];",
            (0..24)
                .map(|j| format!("\"cell {i}.{j}\""))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    s
}

/// A 5,000-byte token with no whitespace between two short lines: Strata's source preview
/// splits it on character boundaries into rows of 2,048, 1,476 and 1,476 bytes.
pub fn long_line() -> String {
    format!("before\n{}\nafter\n", "0123456789".repeat(500))
}

/// 50,000 lines of Rust-like text, generated.
pub fn large() -> String {
    let mut s = String::with_capacity(2 << 20);
    for i in 0..50_000 {
        match i % 8 {
            0 => {
                let _ = writeln!(s, "/// Item {i}: a function and its body.");
            }
            1 => {
                let _ = writeln!(s, "pub fn item_{i}(x: u32) -> u32 {{");
            }
            2 => {
                let _ = writeln!(s, "    let name = \"item {i}\";");
            }
            3 => {
                let _ = writeln!(s, "    let y = x.wrapping_mul({i}); // scaled");
            }
            4 => {
                let _ = writeln!(s, "    if y > {i} {{ return y - {i}; }}");
            }
            5 => {
                let _ = writeln!(s, "    y + name.len() as u32");
            }
            6 => s.push_str("}\n"),
            _ => s.push('\n'),
        }
    }
    s
}

fn header(fixture: Fixture) -> (&'static str, &'static str, &'static str) {
    match fixture {
        Fixture::Rust | Fixture::Editor => ("sizes.rs", "Rust source", "1.1 KB"),
        Fixture::WrapOff => ("table.rs", "Rust source", "18 KB"),
        Fixture::Plain => ("notes", "Plain text", "1.0 MB"),
        Fixture::LongLine => ("words.txt", "Plain text", "5 KB"),
        Fixture::Large => ("generated.rs", "Rust source", "1.4 MB"),
        Fixture::MarkdownSource => ("notes.md", "Markdown", "112 bytes"),
    }
}

/// The page's state for `fixture`, its colours landed for the lines on screen when `wait`.
pub fn setup(g: &mut Gallery, dark: bool, fixture: Fixture, wait: bool) {
    g.open("c10", dark);
    let live = Live::new(&g.look);
    let (text, file, syntax): (String, Option<&str>, Option<&str>) = match fixture {
        Fixture::Rust | Fixture::Editor => (RUST.into(), Some("sizes.rs"), None),
        Fixture::WrapOff => (wrap_off(), Some("table.rs"), None),
        Fixture::Plain => (PLAIN.into(), Some("notes"), None),
        Fixture::LongLine => (long_line(), Some("words.txt"), None),
        Fixture::Large => (large(), Some("generated.rs"), None),
        Fixture::MarkdownSource => (MARKDOWN.into(), Some("notes.md"), None),
    };
    let mut code = CodeState::new(text).highlighter(&live.highlighter, file, syntax);
    // The long line unwrapped shows its 2 KiB rows one under another, their gutters "↳".
    code.set_wrap(!matches!(fixture, Fixture::WrapOff | Fixture::LongLine));
    if fixture == Fixture::Plain {
        code.set_notices(vec!["Preview limited to the first 1 MB".into()]);
    }
    if fixture == Fixture::MarkdownSource {
        code.set_document_view(Some(DocumentView::Source));
    }
    let editor = (fixture == Fixture::Editor).then(|| {
        let mut ed = EditorState::new(SCRIPT, &live.highlighter, Some("action.sh"), None);
        if wait {
            ed.settle(GOLDEN_WAIT);
        }
        ed
    });
    if wait {
        code.settle(0..ON_SCREEN, GOLDEN_WAIT);
    }
    let mut drawer = DrawerState::default();
    drawer.open_at_once();
    g.c10 = Demo {
        fixture,
        drawer,
        code,
        editor,
        fields: FieldMenuState::new(),
        live: Some(live),
        last: String::new(),
        selection: String::new(),
    };
}

/// Find open on the Rust fixture at its second match of "fn".
fn find_open(g: &mut Gallery, dark: bool) {
    use strata_components::preview_drawer::HostedPreview;
    setup(g, dark, Fixture::Rust, true);
    g.c10.drawer.start_find("fn");
    g.c10.code.find("fn");
    g.c10.code.find_step(true);
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c10", dark)`, then fills in `g.c10`.
pub const SCENES: &[Scene] = &[
    ("c10-rust-light", |g| setup(g, false, Fixture::Rust, true)),
    ("c10-rust-dark", |g| setup(g, true, Fixture::Rust, true)),
    ("c10-wrap-off-light", |g| {
        setup(g, false, Fixture::WrapOff, true)
    }),
    ("c10-wrap-off-dark", |g| {
        setup(g, true, Fixture::WrapOff, true)
    }),
    ("c10-plain-light", |g| setup(g, false, Fixture::Plain, true)),
    ("c10-plain-dark", |g| setup(g, true, Fixture::Plain, true)),
    ("c10-find-light", |g| find_open(g, false)),
    ("c10-find-dark", |g| find_open(g, true)),
    ("c10-long-line-light", |g| {
        setup(g, false, Fixture::LongLine, true)
    }),
    ("c10-long-line-dark", |g| {
        setup(g, true, Fixture::LongLine, true)
    }),
    ("c10-large-light", |g| setup(g, false, Fixture::Large, true)),
    ("c10-large-dark", |g| setup(g, true, Fixture::Large, true)),
    ("c10-markdown-source-light", |g| {
        setup(g, false, Fixture::MarkdownSource, true)
    }),
    ("c10-markdown-source-dark", |g| {
        setup(g, true, Fixture::MarkdownSource, true)
    }),
    ("c10-editor-light", |g| {
        setup(g, false, Fixture::Editor, true)
    }),
    ("c10-editor-dark", |g| setup(g, true, Fixture::Editor, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens: colours landing as the
/// thread answers.
pub const REVIEW: &[Scene] = &[
    ("c10-async-light", |g| {
        setup(g, false, Fixture::Large, false)
    }),
    ("c10-async-dark", |g| setup(g, true, Fixture::Large, false)),
];

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let d = &g.c10;
    let look = g.look();
    cx.reduced_motion(!look.animations);
    if d.fixture == Fixture::Editor {
        return editor_page(g, look);
    }
    let (name, kind, size) = header(d.fixture);
    let content = CodeView::new(|g: &Gallery| &g.c10.code, |g: &mut Gallery| &mut g.c10.code)
        .on_intent(intent)
        .build(g, look, cx);
    PreviewDrawer::new(&d.drawer, |g: &mut Gallery| &mut g.c10.drawer)
        .header(
            HeaderInfo::new(name)
                .icon(look.icon(IconName::FileCode))
                .details([size, "Today 09:41", kind]),
        )
        .hosted(&d.code, |g: &mut Gallery| &mut g.c10.code)
        .content(content)
        .on_intent(drawer_intent)
        .build(files(look, &d.last, &d.selection), look, cx)
}

fn editor_edit(g: &Gallery, _: Field) -> Option<&TextEdit> {
    g.c10.editor.as_ref().map(|e| &e.edit)
}

fn editor_edit_mut(g: &mut Gallery, _: Field) -> Option<&mut TextEdit> {
    g.c10.editor.as_mut().map(|e| &mut e.edit)
}

fn field_menu() -> FieldMenu<Gallery, Field> {
    FieldMenu::new(
        FIELD_MENU,
        |g: &Gallery| &g.c10.fields,
        |g: &mut Gallery| &mut g.c10.fields,
        editor_edit_mut,
        editor_edit,
    )
    .undo(true)
    .on_change(|g: &mut Gallery, _, _| {
        if let Some(ed) = &mut g.c10.editor {
            ed.edited();
        }
    })
}

fn editor_get(g: &Gallery) -> &EditorState {
    g.c10.editor.as_ref().expect("the editor fixture")
}

fn editor_get_mut(g: &mut Gallery) -> &mut EditorState {
    g.c10.editor.as_mut().expect("the editor fixture")
}

/// The Script tab's editor, as Settings → Actions shows it.
fn editor_page(g: &Gallery, look: &Look) -> El {
    let k = &look.kit;
    let editor = CodeEditor::new(editor_get, editor_get_mut)
        .label("Script")
        .rows(10)
        .menu(field_menu().hook(Field::Script))
        .build(g, look);
    let mut layer = Vec::new();
    let menu = field_menu().build(g, look, &mut layer);
    let page = col()
        .key("c10-editor-page")
        .w_full()
        .h_full()
        .padding(look.space.xl)
        .gap(look.space.md)
        .child(text("Script").font(k.ty.ui_medium).color(k.c.text_base))
        .child(
            text("Runs with bash, found on PATH.")
                .font(k.ty.small)
                .color(k.c.text_muted),
        )
        // The stop before the script, as Strata's runtime choice is.
        .child(
            rune_kit::Button::new("Bash")
                .on_press(|g: &mut Gallery| g.c10.last = "Runtime".into())
                .build(k)
                .key("c10-runtime"),
        )
        .child(col().w_full().grow(1.0).min_h(0.0).child(editor))
        // The Script tab's next stop, as Strata's Library button is.
        .child(
            rune_kit::Button::new("Library")
                .on_press(|g: &mut Gallery| g.c10.last = "Library".into())
                .build(k)
                .key("c10-library"),
        )
        .child(menu);
    rune_kit::root(page, layer)
}

fn intent(g: &mut Gallery, intent: CodeIntent, _tx: &mut Tx) {
    let d = &mut g.c10;
    if intent == CodeIntent::Leave {
        d.last = "Leave".into();
        d.drawer.focus_files();
    }
}

fn drawer_intent(g: &mut Gallery, intent: DrawerIntent, _tx: &mut Tx) {
    let d = &mut g.c10;
    match intent {
        DrawerIntent::SetWrap(on) => {
            d.code.set_wrap(on);
            d.last = format!("Wrap {}", if on { "on" } else { "off" });
        }
        DrawerIntent::ShowView(form) => d.last = format!("Show {form:?}"),
        DrawerIntent::Closed => d.last = "Closed".into(),
        _ => {}
    }
}

/// The fake listing beside the drawer, its focus group keyed `files`, and under it two lines
/// scripts read: the last intent (`c10-last-intent`) and what copying the selection would
/// take (`c10-selection`), each named "None" while empty.
fn files(look: &Look, last: &str, selection: &str) -> El {
    let k = &look.kit;
    let line = |key: &str, s: &str| {
        // A whole large text reads as its start and its length.
        let long;
        let name = if s.is_empty() {
            "None"
        } else if s.len() > 8 * 1024 {
            let cut: String = s.chars().take(200).collect();
            long = format!("{cut}… ({} bytes)", s.len());
            &long
        } else {
            s
        };
        let shown: String = name
            .replace('\n', "⏎")
            .replace('\t', "⇥")
            .chars()
            .take(400)
            .collect();
        text(shown)
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
        .child(line("c10-last-intent", last))
        .child(line("c10-selection", selection));
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
                .label(format!("file-{i}.rs"))
                .w_full()
                .h(look.row.list)
                .px(look.space.md)
                .items_center()
                .child(
                    text(format!("file-{i}.rs"))
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
/// starts the highlighter, takes the colours that landed, applies find's selections and
/// closes the field menu when its field goes, as Strata's poll would. True when state changed.
pub fn poll(g: &mut Gallery, _t: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    if g.c10.live.is_none() {
        let live = Live::new(&g.look);
        g.c10.code = CodeState::new(RUST).highlighter(&live.highlighter, Some("sizes.rs"), None);
        g.c10.drawer.open_at_once();
        g.c10.live = Some(live);
        changed = true;
    }
    let d = &mut g.c10;
    if let Some(live) = &mut d.live {
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
    }
    changed |= d.code.poll(tx);
    if let Some(ed) = &mut d.editor {
        changed |= ed.poll();
    }
    let selection = tx
        .text_selection()
        .map(|sel| d.code.copy_selection(sel))
        .unwrap_or_default();
    if selection != d.selection {
        d.selection = selection;
        changed = true;
    }
    changed |= field_menu().check(g);
    changed
}

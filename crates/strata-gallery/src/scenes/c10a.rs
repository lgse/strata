//! The gallery page for C10a Highlighter: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `c10a-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/C10a-highlighter.md`.
//!
//! Two uses of the shared highlighter: rune-kit `Markdown` with its code blocks coloured by
//! syntect through [`CodeBlocks::markdown`], and a TextArea editing a script, coloured by an
//! editable [`CodeDoc`]'s spans and re-highlighted as it is typed in.
//!
//! The highlighter runs on its own thread, so its colours land at times no golden can rely
//! on. The golden scenes therefore wait for them before the first frame
//! ([`CodeBlocks::wait_on_miss`] and [`CodeDoc::settle`], both headless-only). The review
//! scenes don't wait: open them in a window to see the blocks draw plain, then colour in
//! place.

use std::sync::Arc;
use std::time::Duration;

use rune::{Cx, El, KeyEvent, ScrollSpec, TextEdit, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, Markdown, TextArea};
use strata_components::code_view::highlight::{
    CodeBlocks, CodeDoc, Highlighter, LineEdit, Mode as DocMode, Source,
};
use strata_kit::look::Look;
use strata_kit::theme::syntax::syntect_theme;

use crate::{Gallery, Scene, SyntaxKey};

/// How long a golden scene waits for the highlighter (debug builds parse slowly).
const GOLDEN_WAIT: Duration = Duration::from_secs(30);

const MARKDOWN: &str = r#"Code blocks coloured by **syntect** on the highlighter thread, through rune-kit's
`Markdown` highlight hook. TOML is a language rune-kit's own scanners don't know.

```rust
#[derive(Debug, Clone)]
struct Entry { name: String, size: u64 }

/// The total size, in bytes.
fn total(entries: &[Entry]) -> u64 {
    entries.iter().map(|e| e.size).sum() // no overflow check
}
```

```c
#include <stdio.h>
#define LIMIT 42

int main(void) {
    printf("%d\n", LIMIT); /* done */
    return 0;
}
```

```toml
[package]
name = "strata"   # the app
version = "0.1.0"
```
"#;

const SCRIPT: &str = r#"#!/usr/bin/env bash
# Rename every photo by the day it was taken.
set -euo pipefail

dir="${1:-$HOME/Pictures}"
count=0
for f in "$dir"/*.jpg; do
  stamp=$(date -r "$f" +%Y-%m-%d)
  mv -n "$f" "$dir/$stamp-$((count++)).jpg"
done

cat <<EOF
Renamed $count photos in $dir
EOF
"#;

/// Which use the page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Markdown,
    Editor,
}

/// The highlighter and what it colours, started on the page's first poll (or by a scene).
pub struct Live {
    pub highlighter: Highlighter,
    pub blocks: CodeBlocks,
    pub script: CodeDoc,
    syntax: SyntaxKey,
    waker: bool,
}

impl Live {
    fn new(look: &Look, script: &str, wait: bool) -> Live {
        let highlighter = Highlighter::new(Arc::new(syntect_theme(look)));
        let mut blocks = highlighter.blocks();
        let mut doc = highlighter.open(
            Source::new(script)
                .file_name("rename-photos.sh")
                .mode(DocMode::Editable),
        );
        doc.show(0..doc.line_count());
        if wait {
            blocks.wait_on_miss(Some(GOLDEN_WAIT));
            doc.settle(GOLDEN_WAIT);
        }
        Live {
            highlighter,
            blocks,
            script: doc,
            syntax: SyntaxKey::of(look),
            waker: false,
        }
    }
}

/// C10a's demo state, held in `Gallery::c10a`.
pub struct Demo {
    pub mode: Mode,
    pub script: TextEdit,
    pub live: Option<Live>,
}

impl Default for Demo {
    fn default() -> Demo {
        Demo {
            mode: Mode::Markdown,
            script: TextEdit::new(SCRIPT),
            live: None,
        }
    }
}

impl Demo {
    /// The script was typed in: re-highlight the lines that changed.
    fn edited(&mut self) {
        let Some(live) = &mut self.live else { return };
        let old = live.script.text().clone();
        let new = self.script.text.as_str();
        if *old != *new {
            let edit = LineEdit::diff(&old, new);
            let lines = new.split('\n').count();
            live.script.edit(new.into(), edit, 0..lines);
        }
    }
}

fn scene(g: &mut Gallery, dark: bool, mode: Mode, wait: bool) {
    g.open("c10a", dark);
    let mut demo = Demo {
        mode,
        ..Demo::default()
    };
    demo.live = Some(Live::new(&g.look, SCRIPT, wait));
    g.c10a = demo;
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c10a", dark)`, then fills in `g.c10a`.
pub const SCENES: &[Scene] = &[
    ("c10a-markdown-light", |g| {
        scene(g, false, Mode::Markdown, true)
    }),
    ("c10a-markdown-dark", |g| {
        scene(g, true, Mode::Markdown, true)
    }),
    ("c10a-editor-light", |g| scene(g, false, Mode::Editor, true)),
    ("c10a-editor-dark", |g| scene(g, true, Mode::Editor, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens: the blocks draw plain,
/// then colour in place as the thread answers.
pub const REVIEW: &[Scene] = &[
    ("c10a-markdown-async-light", |g| {
        scene(g, false, Mode::Markdown, false)
    }),
    ("c10a-markdown-async-dark", |g| {
        scene(g, true, Mode::Markdown, false)
    }),
];

pub fn view(g: &Gallery, _cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    let kit = &look.kit;
    let d = &g.c10a;
    let tab = |key: &str, label: &str, mode: Mode| {
        let b = Button::new(label).on_press(move |g: &mut Gallery| g.c10a.mode = mode);
        let b = if d.mode == mode {
            b.variant(ButtonVariant::Contrast)
        } else {
            b
        };
        b.build(kit).key(format!("c10a-show-{key}"))
    };
    let tabs = row()
        .key("c10a-tabs")
        .gap(look.space.md)
        .child(tab("markdown", "Markdown", Mode::Markdown))
        .child(tab("editor", "Editor", Mode::Editor));
    let body = match d.mode {
        Mode::Markdown => {
            let highlighter = match &d.live {
                Some(live) => live.blocks.markdown(),
                // Plain until the page's first poll starts the highlighter.
                None => rune_kit::syntax::Highlighter::new(|_, _| Vec::new()),
            };
            Markdown::new(MARKDOWN)
                .highlighter(highlighter)
                .build(kit)
                .key("c10a-markdown")
        }
        Mode::Editor => {
            let spans = d
                .live
                .as_ref()
                .map(|l| l.script.text_spans(0..l.script.line_count()))
                .unwrap_or_default();
            let rows = SCRIPT.lines().count();
            col()
                .key("c10a-editor")
                .w_full()
                .gap(look.space.sm)
                .child(
                    text("A TextArea coloured by an editable document's spans; type to re-highlight.")
                        .font(kit.ty.ui)
                        .color(kit.c.text_weak),
                )
                .child(
                    TextArea::new(&d.script)
                        .label("Script")
                        .width(None)
                        .rows(rows)
                        .on_edit(|g: &mut Gallery| &mut g.c10a.script)
                        .on_change(|g: &mut Gallery, _tx| g.c10a.edited())
                        .spans(spans)
                        .build(kit)
                        .key("c10a-script"),
                )
        }
    };
    col()
        .key("c10a")
        .w_full()
        .h_full()
        .min_h(0.0)
        .padding(look.space.xl)
        .gap(look.space.lg)
        .child(tabs)
        .child(
            col()
                .key("c10a-body")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(body),
        )
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
pub fn on_key(_g: &mut Gallery, _key: &KeyEvent, _tx: &mut Tx) -> bool {
    false
}

/// Runs at the start of every frame while this page is open, with the clock in seconds.
/// Starts the highlighter on first use, hands it the window's waker and the look's theme,
/// and takes the colours that have landed. True when state changed.
pub fn poll(g: &mut Gallery, _t: f64, _tx: &mut Tx) -> bool {
    let d = &mut g.c10a;
    let mut changed = false;
    let live = match &mut d.live {
        Some(live) => live,
        None => {
            changed = true;
            d.live.insert(Live::new(&g.look, &d.script.text, false))
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
    changed |= live.script.poll();
    changed
}

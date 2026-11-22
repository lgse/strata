// SPDX-License-Identifier: MIT

//! Behavioral regressions for preview text polish:
//! - UI-33: right-click context menu (`Copy`, `Select All`) across `CodeView`, `DocumentView`
//!   (prose and embedded tables), and `PdfView`, preserving selection and restoring focus;
//! - UI-38: 10xer `Right` / `l` into preview scrolling with `Down`/`Up`/`PageDown`/`PageUp`/
//!   `j`/`k`/`Ctrl+D`/`Ctrl+U`/`Home`/`End`/`G`/`g g` and returning via `Left`/`h`, while the
//!   find bar keeps typing and arrow keys.

use std::sync::Arc;

use rune::{Key, Mods, Tx, clipboard::MemoryClipboard};
use strata_core::model::{FileEntry, Location};
use strata_fs::services::PdfTextLayer;

use super::{
    super::{
        document, pdf,
        viewer::{CODE_KEY, Viewer},
    },
    MODULE, disable_auto_preview, focus_view, focused_key, press,
    provider::{Recorded, install, png},
    select, wait_for_code_preview,
};
use crate::{
    App,
    host::offscreen::Offscreen,
    shell,
    test_support::{PATIENCE, fixture, hosted},
};

fn enable_tenxer(host: &mut Offscreen) {
    host.runtime().update(|app: &mut App, _: &mut Tx| {
        shell::saved(app).change(|settings| settings.tenxer_mode = true);
        shell::live_keys(app).follow(shell::preferences(app));
    });
    while host.turn() {}
}

fn attach_clipboard(host: &mut Offscreen) -> MemoryClipboard {
    let mem = rune::clipboard::memory();
    host.runtime().use_clipboard(&mem);
    mem
}

fn element_center(host: &mut Offscreen, key: &str) -> (f32, f32) {
    let info = host
        .runtime()
        .elements()
        .into_iter()
        .find(|el| el.key.as_deref() == Some(key) && !el.exiting)
        .unwrap_or_else(|| panic!("element {key} should exist"));
    let [x, y, w, h] = info.rect;
    (x + w * 0.5, y + h * 0.5)
}

fn clipboard_text(mem: &MemoryClipboard) -> Option<String> {
    mem.text().as_deref().cloned()
}

fn secondary_click_at(host: &mut Offscreen, x: f32, y: f32) {
    host.runtime().secondary_down(x, y);
    while host.turn() {}
    host.runtime().secondary_up(x, y);
    while host.turn() {}
}

fn click_at(host: &mut Offscreen, x: f32, y: f32) {
    host.runtime().pointer_down(x, y);
    while host.turn() {}
    host.runtime().pointer_up(x, y);
    while host.turn() {}
}

fn click_key(host: &mut Offscreen, key: &str) {
    let (x, y) = element_center(host, key);
    click_at(host, x, y);
}

fn menu_items(host: &Offscreen) -> Vec<(String, bool)> {
    let model = host
        .app()
        .menu
        .state
        .model()
        .expect("context menu should be open");
    model
        .sections
        .iter()
        .flat_map(|section| section.items.iter())
        .map(|item| (item.action.to_string(), item.enabled))
        .collect()
}

fn viewer_scroll_offset(host: &Offscreen) -> f32 {
    match &host.app().preview.viewer {
        Viewer::Code(code) => code.scroll_offset(),
        Viewer::Document(doc) => match doc.scroll_offset_for_test() {
            Some(y) => y,
            None => panic!("expected scrollable document viewer"),
        },
        Viewer::Pdf(pdf) => pdf.state.scroll_offset(),
        _ => panic!("expected scrollable preview viewer"),
    }
}

fn open_folder_with_files(host: &mut Offscreen, files: &[(&str, &str)]) -> tempfile::TempDir {
    let folder = fixture();
    for (name, body) in files {
        std::fs::write(folder.path().join(name), body).expect("write fixture file");
    }
    host.app().open(Location::local(folder.path()));
    let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, counted), "listing loads");
    while host.turn() {}
    folder
}

fn wait_for_rendered_document(host: &mut Offscreen) {
    let loaded = host.run_until(PATIENCE, |host| {
        matches!(
            &host.app().preview.viewer,
            Viewer::Document(doc) if doc.is_rendered_document_for_test()
        )
    });
    assert!(loaded, "rendered document preview loads");
    while host.turn() {}
}

fn sample_pdf_layer(text: &str) -> Arc<PdfTextLayer> {
    let glyphs = text
        .char_indices()
        .enumerate()
        .map(|(i, _)| [20.0 + i as f32 * 8.0, 20.0, 28.0 + i as f32 * 8.0, 34.0])
        .collect();
    Arc::new(PdfTextLayer {
        width: 400.0,
        height: 550.0,
        text: text.to_owned(),
        glyphs,
    })
}

fn mount_pdf_viewer(host: &mut Offscreen, pages: &Recorded, layer: Option<Arc<PdfTextLayer>>) {
    install(host, pages);
    host.runtime().update(move |app: &mut App, _: &mut Tx| {
        let entry: FileEntry = app.preview.current.clone().expect("previewed file");
        app.preview.viewer = pdf::viewer(entry, (&png(80, 110), 0, 20), layer);
    });
    while host.turn() {}
}

#[test]
fn code_preview_context_menu_copies_selects_and_dismisses_cleanly() {
    hosted(
        MODULE,
        "text_polish::code_preview_context_menu_copies_selects_and_dismisses_cleanly",
        |mut host| {
            let mem = attach_clipboard(&mut host);
            let body = "first line\nsecond line\nthird line\n";
            let folder = open_folder_with_files(&mut host, &[("sample.txt", body)]);
            select(
                &mut host,
                &Location::local(folder.path().join("sample.txt")),
            );
            wait_for_code_preview(&mut host);

            let (cx, cy) = element_center(&mut host, CODE_KEY);
            secondary_click_at(&mut host, cx, cy);
            assert!(host.app().menu.state.is_open());
            assert_eq!(
                menu_items(&host),
                vec![("copy".into(), false), ("select-all".into(), true)]
            );

            click_key(&mut host, "menu-item1");
            assert!(!host.app().menu.state.is_open());
            assert_eq!(focused_key(&mut host).as_deref(), Some(CODE_KEY));

            secondary_click_at(&mut host, cx, cy);
            assert_eq!(
                menu_items(&host),
                vec![("copy".into(), true), ("select-all".into(), true)]
            );
            click_key(&mut host, "menu-item0");
            assert!(!host.app().menu.state.is_open());
            assert_eq!(clipboard_text(&mem).as_deref(), Some(body));
            assert_eq!(focused_key(&mut host).as_deref(), Some(CODE_KEY));

            // Escape closes the menu and preserves selection and preview focus.
            secondary_click_at(&mut host, cx, cy);
            assert!(host.app().menu.state.is_open());
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().menu.state.is_open());
            assert_eq!(focused_key(&mut host).as_deref(), Some(CODE_KEY));
            secondary_click_at(&mut host, cx, cy);
            assert_eq!(
                menu_items(&host),
                vec![("copy".into(), true), ("select-all".into(), true)]
            );

            // Click-away closes the menu and returns focus to the preview viewer.
            click_at(&mut host, 10.0, 10.0);
            assert!(!host.app().menu.state.is_open());
            assert_eq!(focused_key(&mut host).as_deref(), Some(CODE_KEY));
        },
    );
}

#[test]
fn code_preview_multiline_keyboard_selection_survives_context_menu() {
    hosted(
        MODULE,
        "text_polish::code_preview_multiline_keyboard_selection_survives_context_menu",
        |mut host| {
            let mem = attach_clipboard(&mut host);
            let body = "alpha\nbeta\ngamma\ndelta\n";
            let folder = open_folder_with_files(&mut host, &[("multi.txt", body)]);
            select(&mut host, &Location::local(folder.path().join("multi.txt")));
            wait_for_code_preview(&mut host);

            host.runtime().update(|_: &mut App, tx: &mut Tx| {
                tx.focus(CODE_KEY, true);
            });
            while host.turn() {}

            let ctrl = Mods {
                ctrl: true,
                ..Mods::default()
            };
            let shift = Mods {
                shift: true,
                ..Mods::default()
            };
            // Select across two lines via keyboard (Ctrl+Home, Shift+Down, Shift+End).
            press(&mut host, Key::Home, ctrl);
            press(&mut host, Key::Down, shift);
            press(&mut host, Key::End, shift);
            let code_keyboard_text = |app: &App| {
                app.preview.viewer.code_state().and_then(|c| {
                    let (lo, hi) = c.keyboard_selection()?;
                    c.text().get(lo..hi).map(str::to_owned)
                })
            };

            let expected = "alpha\nbeta";
            assert_eq!(code_keyboard_text(host.app()).as_deref(), Some(expected));

            // Right-click -> Escape must preserve the full two-line keyboard selection.
            let (cx, cy) = element_center(&mut host, CODE_KEY);
            secondary_click_at(&mut host, cx, cy);
            assert_eq!(
                menu_items(&host),
                vec![("copy".into(), true), ("select-all".into(), true)]
            );
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().menu.state.is_open());
            assert_eq!(
                code_keyboard_text(host.app()).as_deref(),
                Some(expected),
                "multi-line keyboard selection must not truncate to row 0 after Escape"
            );

            // Right-click -> Copy copies the full two-line selection and preserves it.
            secondary_click_at(&mut host, cx, cy);
            click_key(&mut host, "menu-item0");
            assert_eq!(clipboard_text(&mem).as_deref(), Some(expected));
            assert_eq!(code_keyboard_text(host.app()).as_deref(), Some(expected));
        },
    );
}

#[test]
fn stale_preview_context_menu_ignores_switched_or_closed_preview() {
    hosted(
        MODULE,
        "text_polish::stale_preview_context_menu_ignores_switched_or_closed_preview",
        |mut host| {
            // Long ASCII file -> short multi-byte UTF-8 file to catch any stale byte-offset panic.
            let long_body = "0123456789abcdef\nsecond line\nthird line\n";
            let short_utf8 = "é\n";
            let folder = open_folder_with_files(
                &mut host,
                &[("a_long.txt", long_body), ("b_short.txt", short_utf8)],
            );
            select(
                &mut host,
                &Location::local(folder.path().join("a_long.txt")),
            );
            wait_for_code_preview(&mut host);

            host.runtime().update(|_: &mut App, tx: &mut Tx| {
                tx.focus(CODE_KEY, true);
            });
            while host.turn() {}
            let ctrl = Mods {
                ctrl: true,
                ..Mods::default()
            };
            let shift = Mods {
                shift: true,
                ..Mods::default()
            };
            // Select bytes 0..23 (inside multi-byte UTF-8 or past end of `b_short.txt`).
            press(&mut host, Key::Home, ctrl);
            press(&mut host, Key::Down, shift);
            press(&mut host, Key::Right, shift);
            press(&mut host, Key::Right, shift);

            let (cx, cy) = element_center(&mut host, CODE_KEY);
            secondary_click_at(&mut host, cx, cy);
            assert!(host.app().menu.state.is_open());

            // Simulate async reload/switch onto a shorter UTF-8 text while the menu stays open.
            host.runtime().update(|app: &mut App, _: &mut Tx| {
                if let Viewer::Code(code) = &mut app.preview.viewer {
                    code.set_text(short_utf8);
                }
            });
            while host.turn() {}

            // Dismissing via Escape must not panic, apply stale selection bytes, or leave
            // owns_focus stuck true after the stale menu closes.
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().menu.state.is_open());
            assert_eq!(
                host.app()
                    .preview
                    .viewer
                    .code_state()
                    .and_then(|c| c.keyboard_selection()),
                None
            );
            assert!(
                !super::super::owns_focus(host.app()),
                "stale preview text menu dismissal must clear owns_focus"
            );

            // Re-open menu, then close preview while menu is open: clicking Select All must not
            // re-arm owns_focus on the closed drawer.
            secondary_click_at(&mut host, cx, cy);
            assert!(host.app().menu.state.is_open());
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                super::super::session::close(app, tx);
            });
            while host.turn() {}
            click_key(&mut host, "menu-item1");
            assert!(!host.app().menu.state.is_open());
            assert!(!host.app().preview.enabled);
            assert!(!super::super::owns_focus(host.app()));
        },
    );
}

#[test]
fn document_preview_context_menu_works_in_prose_and_tables() {
    hosted(
        MODULE,
        "text_polish::document_preview_context_menu_works_in_prose_and_tables",
        |mut host| {
            let mem = attach_clipboard(&mut host);
            let markdown = "# Title\n\nIntro paragraph.\n\n| Name | Value |\n| --- | --- |\n| Alpha | 42 |\n| Beta | 99 |\n";
            let folder = open_folder_with_files(&mut host, &[("notes.md", markdown)]);
            select(&mut host, &Location::local(folder.path().join("notes.md")));
            wait_for_rendered_document(&mut host);

            let (dx, dy) = element_center(&mut host, document::DOCUMENT_KEY);
            secondary_click_at(&mut host, dx, dy);
            assert_eq!(
                menu_items(&host),
                vec![("copy".into(), false), ("select-all".into(), true)]
            );
            click_key(&mut host, "menu-item1");
            assert_eq!(
                focused_key(&mut host).as_deref(),
                Some(document::DOCUMENT_KEY)
            );

            secondary_click_at(&mut host, dx, dy);
            assert_eq!(
                menu_items(&host),
                vec![("copy".into(), true), ("select-all".into(), true)]
            );
            click_key(&mut host, "menu-item0");
            let copied_all = clipboard_text(&mem).unwrap_or_default();
            assert!(copied_all.contains("Intro paragraph."));
            assert!(copied_all.contains("Alpha"));

            // Select inside an embedded table cell via keyed path, right-click on the table, and copy it.
            let cell_selector: String = host
                .runtime()
                .elements()
                .into_iter()
                .find(|el| el.key.as_deref() == Some("r0c0"))
                .expect("table body cell r0c0")
                .path
                .split('/')
                .filter(|seg| !seg.starts_with('#'))
                .collect::<Vec<_>>()
                .join("/");
            host.runtime().update(move |_: &mut App, tx: &mut Tx| {
                tx.clear_text_selection();
                tx.select_text(cell_selector, 0..5);
            });
            while host.turn() {}

            let table_center = element_center(&mut host, "r0c0");
            secondary_click_at(&mut host, table_center.0, table_center.1);
            assert_eq!(
                menu_items(&host),
                vec![("copy".into(), true), ("select-all".into(), true)]
            );
            click_key(&mut host, "menu-item0");
            assert_eq!(clipboard_text(&mem).as_deref(), Some("Alpha"));
        },
    );
}

#[test]
fn pdf_preview_context_menu_copies_and_selects_all() {
    hosted(
        MODULE,
        "text_polish::pdf_preview_context_menu_copies_and_selects_all",
        |mut host| {
            let mem = attach_clipboard(&mut host);
            let folder = open_folder_with_files(&mut host, &[("doc.txt", "placeholder\n")]);
            disable_auto_preview(&mut host);
            select(&mut host, &Location::local(folder.path().join("doc.txt")));
            focus_view(&mut host);
            press(&mut host, Key::Text(" ".into()), Mods::default());
            wait_for_code_preview(&mut host);

            let pages = Recorded::default();
            mount_pdf_viewer(&mut host, &pages, Some(sample_pdf_layer("Hello PDF")));

            let (px, py) = element_center(&mut host, pdf::PDF_KEY);
            secondary_click_at(&mut host, px, py);
            assert_eq!(
                menu_items(&host),
                vec![("copy".into(), false), ("select-all".into(), true)]
            );
            click_key(&mut host, "menu-item1");
            assert_eq!(focused_key(&mut host).as_deref(), Some(pdf::PDF_KEY));

            secondary_click_at(&mut host, px, py);
            assert_eq!(
                menu_items(&host),
                vec![("copy".into(), true), ("select-all".into(), true)]
            );
            click_key(&mut host, "menu-item0");
            assert_eq!(clipboard_text(&mem).as_deref(), Some("Hello PDF"));
        },
    );
}

fn assert_tenxer_scroll_cycle(host: &mut Offscreen, viewer_key: &str) {
    assert_eq!(focused_key(host).as_deref(), Some(viewer_key));
    let start = viewer_scroll_offset(host);

    press(host, Key::Down, Mods::default());
    let after_down = viewer_scroll_offset(host);
    assert!(after_down > start, "Down scrolls {viewer_key} down");

    press(host, Key::PageDown, Mods::default());
    let after_pgdn = viewer_scroll_offset(host);
    assert!(
        after_pgdn > after_down,
        "PageDown scrolls {viewer_key} further down"
    );

    press(host, Key::Up, Mods::default());
    let after_up = viewer_scroll_offset(host);
    assert!(after_up < after_pgdn, "Up scrolls {viewer_key} back up");

    press(host, Key::PageUp, Mods::default());
    let after_pgup = viewer_scroll_offset(host);
    assert!(
        after_pgup < after_up,
        "PageUp scrolls {viewer_key} further up"
    );

    // 10x vim motions: j / k / Ctrl+D / Ctrl+U / G / g g.
    let ctrl = Mods {
        ctrl: true,
        ..Mods::default()
    };
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    press(host, Key::Text("j".into()), Mods::default());
    let after_j = viewer_scroll_offset(host);
    assert!(after_j > after_pgup, "j scrolls {viewer_key} down");

    press(host, Key::Text("d".into()), ctrl);
    let after_ctrl_d = viewer_scroll_offset(host);
    assert!(
        after_ctrl_d > after_j,
        "Ctrl+D scrolls {viewer_key} half-page down"
    );

    press(host, Key::Text("u".into()), ctrl);
    let after_ctrl_u = viewer_scroll_offset(host);
    assert!(
        after_ctrl_u < after_ctrl_d,
        "Ctrl+U scrolls {viewer_key} half-page up"
    );

    press(host, Key::Text("k".into()), Mods::default());
    let after_k = viewer_scroll_offset(host);
    assert!(after_k < after_ctrl_u, "k scrolls {viewer_key} up");

    press(host, Key::Text("G".into()), shift);
    let at_end = viewer_scroll_offset(host);
    assert!(at_end > after_k, "G jumps {viewer_key} to end");

    press(host, Key::Text("g".into()), Mods::default());
    press(host, Key::Text("g".into()), Mods::default());
    assert!(
        host.run_until(PATIENCE, |h| viewer_scroll_offset(h) == 0.0),
        "g g jumps {viewer_key} back to top"
    );
}

#[test]
fn tenxer_right_arrow_into_code_document_and_pdf_scrolls_and_returns() {
    hosted(
        MODULE,
        "text_polish::tenxer_right_arrow_into_code_document_and_pdf_scrolls_and_returns",
        |mut host| {
            let long_code: String = (0..250).map(|i| format!("let line_{i} = {i};\n")).collect();
            let long_md: String = (0..120)
                .map(|i| format!("## Section {i}\n\nParagraph {i} with scrollable content.\n\n"))
                .collect();
            let folder = open_folder_with_files(
                &mut host,
                &[("long.rs", &long_code), ("long.md", &long_md)],
            );
            enable_tenxer(&mut host);

            // 1. CodeView: enter via Right, scroll, return via Left, re-enter via l, test find bar, return via h.
            select(&mut host, &Location::local(folder.path().join("long.rs")));
            wait_for_code_preview(&mut host);
            focus_view(&mut host);
            press(&mut host, Key::Right, Mods::default());
            assert_tenxer_scroll_cycle(&mut host, CODE_KEY);

            // Open find bar with `/` in 10xer mode: typing/arrows stay in find input.
            press(&mut host, Key::Text("/".into()), Mods::default());
            assert!(host.app().preview.drawer.is_find_focused());
            press(&mut host, Key::Text("j".into()), Mods::default());
            press(&mut host, Key::Text("k".into()), Mods::default());
            press(&mut host, Key::Left, Mods::default());
            assert_eq!(host.app().preview.drawer.find_query(), "jk");
            assert!(host.app().preview.drawer.is_find_focused());
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().preview.drawer.is_find_focused());

            press(&mut host, Key::Left, Mods::default());
            assert_eq!(
                focused_key(&mut host).as_deref(),
                Some(crate::columns::VIEW)
            );
            press(&mut host, Key::Text("l".into()), Mods::default());
            assert_eq!(focused_key(&mut host).as_deref(), Some(CODE_KEY));
            press(&mut host, Key::Text("h".into()), Mods::default());
            assert_eq!(
                focused_key(&mut host).as_deref(),
                Some(crate::columns::VIEW)
            );

            // 2. DocumentView: enter via Right, scroll, return via Left.
            select(&mut host, &Location::local(folder.path().join("long.md")));
            wait_for_rendered_document(&mut host);
            focus_view(&mut host);
            press(&mut host, Key::Right, Mods::default());
            assert_tenxer_scroll_cycle(&mut host, document::DOCUMENT_KEY);
            press(&mut host, Key::Left, Mods::default());
            assert_eq!(
                focused_key(&mut host).as_deref(),
                Some(crate::columns::VIEW)
            );

            // 3. PdfView: enter via Right, scroll, return via h.
            let pages = Recorded::default();
            mount_pdf_viewer(&mut host, &pages, Some(sample_pdf_layer("Page one")));
            focus_view(&mut host);
            press(&mut host, Key::Right, Mods::default());
            assert_tenxer_scroll_cycle(&mut host, pdf::PDF_KEY);
            press(&mut host, Key::Text("h".into()), Mods::default());
            assert_eq!(
                focused_key(&mut host).as_deref(),
                Some(crate::columns::VIEW)
            );
        },
    );
}

fn live_selected_text(host: &mut Offscreen) -> Option<String> {
    let mut out = None;
    host.runtime().update(|_: &mut App, tx: &mut Tx| {
        if let Some(sel) = tx.text_selection() {
            let text = sel.text();
            if !text.is_empty() {
                out = Some(text.to_owned());
            }
        }
    });
    out
}

fn drag_select(host: &mut Offscreen, from: (f32, f32), to: (f32, f32)) {
    host.runtime().pointer_move(from.0, from.1);
    while host.turn() {}
    host.runtime().pointer_down(from.0, from.1);
    while host.turn() {}
    host.runtime().pointer_move(to.0, to.1);
    while host.turn() {}
    host.runtime().pointer_up(to.0, to.1);
    while host.turn() {}
}

fn step_recording_owns_focus(host: &mut Offscreen, log: &mut Vec<bool>) {
    for _ in 0..8 {
        let worked = host.turn();
        log.push(super::super::owns_focus(host.app()));
        if !worked {
            break;
        }
    }
}

fn assert_no_focus_bounce(samples: &[bool], context: &str) {
    let mut seen_true = false;
    let mut dropped_after_true = false;
    for &on in samples {
        if on {
            assert!(
                !dropped_after_true,
                "{context}: owns_focus bounced true -> false -> true: {samples:?}"
            );
            seen_true = true;
        } else if seen_true {
            dropped_after_true = true;
        }
    }
    assert!(
        seen_true && !dropped_after_true,
        "{context}: expected monotonic false -> true without drop: {samples:?}"
    );
}

fn find_text_rect(host: &mut Offscreen, needle: &str) -> [f32; 4] {
    host.runtime()
        .elements()
        .into_iter()
        .find(|el| {
            !el.exiting
                && el.text.as_deref().is_some_and(|t| t.contains(needle))
                && el.rect[2] > 10.0
                && el.rect[3] > 5.0
        })
        .unwrap_or_else(|| panic!("expected rendered text element containing {needle:?}"))
        .rect
}

fn secondary_click_recording(host: &mut Offscreen, at: (f32, f32), trace: &mut Vec<bool>) {
    host.runtime().secondary_down(at.0, at.1);
    step_recording_owns_focus(host, trace);
    host.runtime().secondary_up(at.0, at.1);
    step_recording_owns_focus(host, trace);
}

fn click_key_recording(host: &mut Offscreen, key: &str, trace: &mut Vec<bool>) {
    let (x, y) = element_center(host, key);
    host.runtime().pointer_down(x, y);
    step_recording_owns_focus(host, trace);
    host.runtime().pointer_up(x, y);
    step_recording_owns_focus(host, trace);
}

#[test]
fn document_preview_context_menu_keeps_drawer_focused_and_selection_visible() {
    hosted(
        MODULE,
        "text_polish::document_preview_context_menu_keeps_drawer_focused_and_selection_visible",
        |mut host| {
            let mem = attach_clipboard(&mut host);
            let markdown =
                "# Heading\n\nFirst paragraph alpha beta.\n\nSecond paragraph gamma delta.\n";
            let folder = open_folder_with_files(&mut host, &[("notes.md", markdown)]);
            select(&mut host, &Location::local(folder.path().join("notes.md")));
            wait_for_rendered_document(&mut host);

            focus_view(&mut host);
            press(&mut host, Key::Right, Mods::default());
            assert_eq!(
                focused_key(&mut host).as_deref(),
                Some(document::DOCUMENT_KEY)
            );
            assert!(super::super::owns_focus(host.app()));

            // 1. Single-paragraph drag selection survives open + Escape with zero focus drop.
            let p1 = find_text_rect(&mut host, "First paragraph");
            let mid = (p1[0] + p1[2] * 0.5, p1[1] + p1[3] * 0.5);
            drag_select(
                &mut host,
                (p1[0] + 4.0, mid.1),
                (p1[0] + p1[2] - 4.0, mid.1),
            );
            assert!(
                live_selected_text(&mut host)
                    .as_deref()
                    .is_some_and(|s| s.contains("First paragraph"))
            );

            let mut trace = vec![super::super::owns_focus(host.app())];
            secondary_click_recording(&mut host, mid, &mut trace);
            assert!(host.app().menu.state.is_open());
            assert!(live_selected_text(&mut host).is_some());
            host.runtime().key(rune::KeyEvent {
                key: Key::Escape,
                mods: Mods::default(),
            });
            step_recording_owns_focus(&mut host, &mut trace);
            assert!(!host.app().menu.state.is_open());
            assert!(live_selected_text(&mut host).is_some());
            assert!(trace.iter().all(|&on| on), "focus dropped: {trace:?}");

            // 2. Multi-paragraph drag selection stays visible while open, copies whole, and
            //    dismisses cleanly on secondary scrim click.
            let p2 = find_text_rect(&mut host, "Second paragraph");
            drag_select(
                &mut host,
                (p1[0] + 4.0, mid.1),
                (p2[0] + p2[2] - 4.0, p2[1] + p2[3] * 0.5),
            );
            let mut multi_trace = vec![super::super::owns_focus(host.app())];
            secondary_click_recording(&mut host, mid, &mut multi_trace);
            let while_open = live_selected_text(&mut host).expect("selection live while open");
            assert!(
                while_open.contains("First paragraph") && while_open.contains("Second paragraph")
            );

            click_key_recording(&mut host, "menu-item0", &mut multi_trace);
            let copied = clipboard_text(&mem).unwrap_or_default();
            assert!(copied.contains("First paragraph") && copied.contains("Second paragraph"));
            assert!(live_selected_text(&mut host).is_some());
            assert!(
                multi_trace.iter().all(|&on| on),
                "focus dropped: {multi_trace:?}"
            );

            secondary_click_at(&mut host, mid.0, mid.1);
            assert!(host.app().menu.state.is_open());
            secondary_click_at(&mut host, 10.0, 10.0);
            assert!(!host.app().menu.state.is_open());
            assert!(live_selected_text(&mut host).is_some());
            assert!(super::super::owns_focus(host.app()));
        },
    );
}

fn switch_to_list_view(host: &mut Offscreen) {
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        crate::commands::run(app, "view-list", crate::commands::On::Focus, tx);
    });
    while host.turn() {}
}

fn enter_preview_and_record_focus(
    host: &mut Offscreen,
    key: Key,
    expected_focus: &str,
) -> Vec<bool> {
    let mut samples = vec![super::super::owns_focus(host.app())];
    host.runtime().key(rune::KeyEvent {
        key,
        mods: Mods::default(),
    });
    let deadline = std::time::Instant::now() + PATIENCE;
    let mut settled_frames = 0usize;
    loop {
        let worked = host.turn();
        samples.push(super::super::owns_focus(host.app()));
        let landed = focused_key(host).as_deref() == Some(expected_focus)
            && super::super::owns_focus(host.app());
        if landed {
            settled_frames += 1;
            if settled_frames >= 6 && !worked {
                break;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {expected_focus}; samples={samples:?}, focused={:?}",
            focused_key(host)
        );
        if !worked {
            host.wait(std::time::Duration::from_millis(10));
        }
    }
    samples
}

fn assert_monotonic_entry(host: &mut Offscreen, key: Key, expected: &str, label: &str) {
    assert!(!super::super::owns_focus(host.app()));
    let samples = enter_preview_and_record_focus(host, key, expected);
    assert_no_focus_bounce(&samples, label);
}

#[test]
fn tenxer_right_arrow_into_preview_focuses_monotonically_without_bounce() {
    hosted(
        MODULE,
        "text_polish::tenxer_right_arrow_into_preview_focuses_monotonically_without_bounce",
        |mut host| {
            let folder = open_folder_with_files(
                &mut host,
                &[
                    ("doc.md", "# Title\n\nBody paragraph.\n"),
                    ("code.rs", "fn main() {}\n"),
                ],
            );
            enable_tenxer(&mut host);

            // 1. Columns: already-open (Right, l) and fresh-open (Right).
            select(&mut host, &Location::local(folder.path().join("doc.md")));
            wait_for_rendered_document(&mut host);
            focus_view(&mut host);
            assert_monotonic_entry(
                &mut host,
                Key::Right,
                document::DOCUMENT_KEY,
                "Columns open Right",
            );

            press(&mut host, Key::Left, Mods::default());
            assert_monotonic_entry(
                &mut host,
                Key::Text("l".into()),
                document::DOCUMENT_KEY,
                "Columns open l",
            );

            press(&mut host, Key::Escape, Mods::default());
            focus_view(&mut host);
            assert_monotonic_entry(
                &mut host,
                Key::Right,
                document::DOCUMENT_KEY,
                "Columns fresh Right",
            );

            // 2. List: already-open (Right) and fresh-open (l).
            switch_to_list_view(&mut host);
            select(&mut host, &Location::local(folder.path().join("code.rs")));
            wait_for_code_preview(&mut host);
            focus_view(&mut host);
            assert_monotonic_entry(&mut host, Key::Right, CODE_KEY, "List open Right");

            press(&mut host, Key::Escape, Mods::default());
            focus_view(&mut host);
            assert_monotonic_entry(&mut host, Key::Text("l".into()), CODE_KEY, "List fresh l");
        },
    );
}

#[test]
fn suspended_or_cancelled_preview_load_clears_armed_focus_handoff() {
    hosted(
        MODULE,
        "text_polish::suspended_or_cancelled_preview_load_clears_armed_focus_handoff",
        |mut host| {
            let folder = open_folder_with_files(&mut host, &[("slow.txt", "pending body\n")]);
            disable_auto_preview(&mut host);
            let pending = Recorded::default();
            install(&mut host, &pending);
            select(&mut host, &Location::local(folder.path().join("slow.txt")));
            enable_tenxer(&mut host);
            focus_view(&mut host);

            // Right-arrow starts an async load and arms focus_handoff while loader is active.
            host.runtime().key(rune::KeyEvent {
                key: Key::Right,
                mods: Mods::default(),
            });
            host.turn();
            assert!(host.app().preview.loader.is_active());
            assert!(super::super::owns_focus(host.app()));

            // Suspending the drawer cancels the load and must clear owns_focus immediately.
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                super::super::on_intent(
                    app,
                    strata_components::preview_drawer::DrawerIntent::Suspend(true),
                    tx,
                );
            });
            while host.turn() {}
            assert!(
                !super::super::owns_focus(host.app()),
                "suspending drawer during an armed handoff must clear owns_focus"
            );

            // Even if a load is cancelled without suspend while viewer stays Empty, handoff must
            // expire rather than wedging owns_focus indefinitely.
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                super::super::arm_focus_handoff(app, tx);
            });
            while host.turn() {}
            assert!(
                !super::super::owns_focus(host.app()),
                "armed handoff with idle Empty viewer must expire"
            );
        },
    );
}

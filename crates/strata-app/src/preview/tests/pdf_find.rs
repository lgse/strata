// SPDX-License-Identifier: MIT

//! End-to-end tests for searching text inside PDF previews (`UI-63`): Find button visibility,
//! `Ctrl+F` and 10xer `/` shortcuts, case-insensitive multi-page search via `RenderKind::TextOnly`
//! without PNG decoding, live match count updates, wrapping `Enter` / `Shift+Enter` navigation,
//! `Escape` cleanup with preserved selection for `Ctrl+C`, cancellation on file switch / close,
//! and non-searchable locked / failed PDFs.

use std::{sync::Arc, time::Duration};

use rune::{Key, Mods, Tx};
use strata_components::preview_drawer::{FindStatus, HostedPreview};
use strata_core::model::{FileEntry, Location};
use strata_fs::services::{MediaPreviewSize, PdfTextIndex, PdfTextLayer, PreviewContent};
use strata_preview::sandbox::PDF_PASSWORD_PROTECTED;

use super::{
    super::pdf,
    MODULE, disable_auto_preview, focus_view, listed, press,
    provider::{Recorded, install, png},
    select, space, wait_for_code_preview,
};
use crate::{
    App,
    host::offscreen::Offscreen,
    shell,
    test_support::{PATIENCE, hosted},
};

const CTRL: Mods = Mods {
    ctrl: true,
    cmd: true,
    shift: false,
    alt: false,
    logo: false,
};

const SHIFT: Mods = Mods {
    ctrl: false,
    cmd: false,
    shift: true,
    alt: false,
    logo: false,
};

const QUIET: Duration = Duration::from_millis(120);

fn text_layer(text: &str, width: f32, height: f32) -> Arc<PdfTextLayer> {
    let glyphs: Vec<[f32; 4]> = text
        .chars()
        .enumerate()
        .map(|(i, ch)| {
            let x = 10.0 + i as f32 * 8.0;
            if ch == '\n' {
                [x, 12.0, x, 12.0]
            } else {
                [x, 12.0, x + 8.0, 26.0]
            }
        })
        .collect();
    Arc::new(PdfTextLayer {
        width,
        height,
        text: text.to_owned(),
        glyphs,
    })
}

fn page_png(width: u32) -> Vec<u8> {
    png(width, width * 11 / 8)
}

fn show_pdf_with_text(
    host: &mut Offscreen,
    pages: &Recorded,
    total_pages: i32,
    page0_text: Option<&str>,
) -> tempfile::TempDir {
    let folder = listed(host);
    disable_auto_preview(host);
    select(host, &Location::local(folder.path().join("todo.txt")));
    focus_view(host);
    space(host);
    wait_for_code_preview(host);
    install(host, pages);
    let layer = page0_text.map(|t| text_layer(t, 400.0, 550.0));
    host.runtime().update(move |app: &mut App, _: &mut Tx| {
        let entry: FileEntry = app.preview.current.clone().expect("a previewed file");
        app.preview.viewer = pdf::viewer(entry, (&page_png(80), 0, total_pages), layer);
    });
    while host.turn() {}
    folder
}

fn focus_pdf(host: &mut Offscreen) {
    host.runtime().update(|_: &mut App, tx: &mut Tx| {
        tx.focus(pdf::PDF_KEY, true);
    });
    while host.turn() {}
}

fn focused_key(host: &mut Offscreen) -> Option<String> {
    host.runtime()
        .elements()
        .into_iter()
        .find(|el| el.focused && !el.exiting)
        .and_then(|el| el.key)
}

fn has_element(host: &mut Offscreen, key: &str) -> bool {
    host.runtime()
        .elements()
        .into_iter()
        .any(|el| !el.exiting && el.key.as_deref() == Some(key))
}

fn type_text(host: &mut Offscreen, text: &str) {
    for ch in text.chars() {
        press(host, Key::Text(ch.to_string()), Mods::default());
    }
}

fn pdf_selected_text(host: &Offscreen) -> String {
    match &host.app().preview.viewer {
        super::Viewer::Pdf(pdf) => pdf.state.selected_text(),
        _ => panic!("expected PDF viewer"),
    }
}

fn pdf_held(host: &Offscreen, page: usize) -> Option<[u32; 2]> {
    match &host.app().preview.viewer {
        super::Viewer::Pdf(pdf) => pdf.state.held(page),
        _ => None,
    }
}

fn pdf_scroll_offset(host: &Offscreen) -> f32 {
    match &host.app().preview.viewer {
        super::Viewer::Pdf(pdf) => pdf.state.scroll_offset(),
        _ => panic!("expected PDF viewer"),
    }
}

fn answer_pending_pages(
    host: &mut Offscreen,
    pages: &Recorded,
    total_pages: i32,
    page_texts: &[Option<&str>],
    start_index: usize,
) -> usize {
    let mut idx = start_index;
    while host.run_until(QUIET, |_| pages.count() > idx) {
        let req = pages.request(idx);
        let page = usize::try_from(req.pdf_page).expect("non-negative page");
        let is_text_only = req.media_size == MediaPreviewSize::new(16, 16);
        let text = page_texts.get(page).copied().flatten().map(|t| {
            let (w, h) = if is_text_only {
                (16.0, 22.0)
            } else {
                (req.media_size.width as f32, req.media_size.height as f32)
            };
            text_layer(t, w, h)
        });
        // Intentionally supply empty bytes for TextOnly to prove PNG decoding is skipped.
        let png_bytes = if is_text_only {
            Vec::new()
        } else {
            page_png(u32::try_from(req.media_size.width).expect("width"))
        };
        pages.answer(
            idx,
            PreviewContent::Pdf {
                png: png_bytes,
                page: req.pdf_page,
                pages: total_pages,
                text_layer: text,
            },
        );
        while host.turn() {}
        idx += 1;
        if idx >= start_index + total_pages as usize * 2 {
            break;
        }
    }
    idx
}

#[test]
fn pdf_find_searches_pages_asynchronously_steps_wraps_and_cleans_up_on_escape() {
    hosted(
        MODULE,
        "pdf_find::pdf_find_searches_pages_asynchronously_steps_wraps_and_cleans_up_on_escape",
        |mut host| {
            let pages = Recorded::default();
            let page_texts = [
                Some("Alpha intro on page one"),
                Some("No match on page two"),
                Some("Second ALPHA hit on page three"),
                None, // Scanned page with no text layer.
                Some("Final alpha match on page five"),
            ];
            let _folder = show_pdf_with_text(&mut host, &pages, 5, page_texts[0]);

            assert!(
                host.app().preview.viewer.searchable(),
                "loaded PDF preview is searchable"
            );
            assert!(
                has_element(&mut host, "preview-find"),
                "header displays the Find button"
            );

            // Answer initial visible/margin page renders before starting find.
            let next_idx = answer_pending_pages(&mut host, &pages, 5, &page_texts, 0);

            focus_pdf(&mut host);
            press(&mut host, Key::Command('f'), CTRL);
            assert!(host.app().preview.drawer.is_finding());
            assert!(host.app().preview.drawer.is_find_focused());
            assert!(has_element(&mut host, "preview-find-field"));

            // Typing matches page 0 immediately and kicks off TextOnly fetches for remaining pages.
            type_text(&mut host, "alpha");
            assert!(
                host.run_until(PATIENCE, |h| {
                    h.app().preview.viewer.find_status()
                        == Some(FindStatus {
                            current: Some(1),
                            total: 1,
                        })
                }),
                "page 0 match appears immediately"
            );
            assert_eq!(pdf_selected_text(&host), "Alpha");

            // Drain background requests: unvisited offscreen pages use minimal 16x16 TextOnly
            // requests and succeed even with empty (non-PNG) payloads.
            pages.wait_for(&mut host, next_idx);
            let after_idx = answer_pending_pages(&mut host, &pages, 5, &page_texts, next_idx);
            assert!(
                (next_idx..after_idx)
                    .any(|i| pages.request(i).media_size == MediaPreviewSize::new(16, 16)),
                "unvisited pages requested with minimal 16x16 TextOnly size"
            );

            assert!(
                host.run_until(PATIENCE, |h| {
                    h.app().preview.viewer.find_status()
                        == Some(FindStatus {
                            current: Some(1),
                            total: 3,
                        })
                }),
                "match count updates live in page order as background text layers arrive"
            );
            // Offscreen page 4 had only a TextOnly reply, so no image should be held yet.
            assert_eq!(
                pdf_held(&host, 4),
                None,
                "TextOnly reply must not store a rendered image for unvisited pages"
            );

            // Step forward across pages and wrap around.
            press(&mut host, Key::Enter, Mods::default());
            assert_eq!(
                host.app().preview.viewer.find_status(),
                Some(FindStatus {
                    current: Some(2),
                    total: 3,
                })
            );
            assert_eq!(pdf_selected_text(&host), "ALPHA");

            press(&mut host, Key::Enter, Mods::default());
            assert_eq!(
                host.app().preview.viewer.find_status(),
                Some(FindStatus {
                    current: Some(3),
                    total: 3,
                })
            );
            assert_eq!(pdf_selected_text(&host), "alpha");

            press(&mut host, Key::Enter, Mods::default());
            assert_eq!(
                host.app().preview.viewer.find_status(),
                Some(FindStatus {
                    current: Some(1),
                    total: 3,
                })
            );
            assert_eq!(pdf_selected_text(&host), "Alpha");

            // Shift+Enter steps backward and wraps to the last match.
            press(&mut host, Key::Enter, SHIFT);
            assert_eq!(
                host.app().preview.viewer.find_status(),
                Some(FindStatus {
                    current: Some(3),
                    total: 3,
                })
            );
            assert_eq!(pdf_selected_text(&host), "alpha");

            // Escape closes find bar, returns focus to PDF viewer, clears find hits, and preserves
            // the current match's selection for Ctrl+C.
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().preview.drawer.is_finding());
            assert!(
                host.app().preview.enabled,
                "Escape closes find without closing the preview"
            );
            assert_eq!(focused_key(&mut host).as_deref(), Some(pdf::PDF_KEY));
            assert_eq!(
                host.app().preview.viewer.find_status(),
                Some(FindStatus {
                    current: None,
                    total: 0,
                }),
                "find_end clears find hits"
            );
            assert_eq!(
                pdf_selected_text(&host),
                "alpha",
                "selected match stays selected after closing find"
            );
        },
    );
}

#[test]
fn pdf_find_reveals_first_async_match_on_offscreen_page() {
    hosted(
        MODULE,
        "pdf_find::pdf_find_reveals_first_async_match_on_offscreen_page",
        |mut host| {
            let pages = Recorded::default();
            let page_texts = [
                Some("Page zero without target"),
                Some("Page one without target"),
                Some("Page two without target"),
                Some("Page three without target"),
                Some("Page four contains TREASURE"),
            ];
            let _folder = show_pdf_with_text(&mut host, &pages, 5, page_texts[0]);
            let idx = answer_pending_pages(&mut host, &pages, 5, &page_texts, 0);

            assert_eq!(pdf_scroll_offset(&host), 0.0);

            focus_pdf(&mut host);
            press(&mut host, Key::Command('f'), CTRL);
            type_text(&mut host, "treasure");
            assert_eq!(
                host.app().preview.viewer.find_status(),
                Some(FindStatus {
                    current: None,
                    total: 0,
                })
            );

            pages.wait_for(&mut host, idx);
            answer_pending_pages(&mut host, &pages, 5, &page_texts, idx);

            assert!(
                host.run_until(PATIENCE, |h| {
                    h.app().preview.viewer.find_status()
                        == Some(FindStatus {
                            current: Some(1),
                            total: 1,
                        })
                }),
                "asynchronous offscreen match becomes current"
            );
            assert_eq!(pdf_selected_text(&host), "TREASURE");
            assert!(
                host.run_until(PATIENCE, |h| pdf_scroll_offset(h) > 0.0),
                "viewport scrolls to reveal the first asynchronous match"
            );
        },
    );
}

#[test]
fn pdf_find_cancels_inflight_text_only_requests_on_escape_close_and_switch() {
    hosted(
        MODULE,
        "pdf_find::pdf_find_cancels_inflight_text_only_requests_on_escape_close_and_switch",
        |mut host| {
            let pages = Recorded::default();
            let page_texts = [Some("First page text"); 8];
            let folder = show_pdf_with_text(&mut host, &pages, 8, page_texts[0]);
            let mut idx = answer_pending_pages(&mut host, &pages, 8, &page_texts, 0);

            // 1. Escape cancels an in-flight TextOnly request.
            focus_pdf(&mut host);
            press(&mut host, Key::Command('f'), CTRL);
            type_text(&mut host, "missing");
            pages.wait_for(&mut host, idx);
            assert_eq!(
                pages.request(idx).media_size,
                MediaPreviewSize::new(16, 16),
                "find issues a TextOnly request"
            );
            press(&mut host, Key::Escape, Mods::default());
            assert!(
                host.run_until(PATIENCE, |_| pages.dropped_unanswered(idx)),
                "closing find bar cancels in-flight TextOnly request"
            );
            idx += 1;

            // 2. Switching to another file cancels an in-flight TextOnly request.
            press(&mut host, Key::Command('f'), CTRL);
            pages.wait_for(&mut host, idx);
            assert_eq!(pages.request(idx).media_size, MediaPreviewSize::new(16, 16));
            select(&mut host, &Location::local(folder.path().join("readme.md")));
            assert!(
                host.run_until(PATIENCE, |_| pages.dropped_unanswered(idx)),
                "switching previewed file cancels in-flight TextOnly request"
            );

            // 3. Closing the preview cancels an in-flight TextOnly request.
            let pages2 = Recorded::default();
            install(&mut host, &pages2);
            host.runtime().update(move |app: &mut App, _: &mut Tx| {
                let entry: FileEntry = app.preview.current.clone().expect("a previewed file");
                app.preview.viewer = pdf::viewer(
                    entry,
                    (&page_png(80), 0, 8),
                    Some(text_layer("p0", 400.0, 550.0)),
                );
            });
            while host.turn() {}
            let idx2 = answer_pending_pages(&mut host, &pages2, 8, &page_texts, 0);
            focus_pdf(&mut host);
            press(&mut host, Key::Command('f'), CTRL);
            type_text(&mut host, "missing");
            pages2.wait_for(&mut host, idx2);
            assert_eq!(
                pages2.request(idx2).media_size,
                MediaPreviewSize::new(16, 16)
            );
            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                super::super::dismiss_escape(app, tx);
            });
            assert!(
                host.run_until(PATIENCE, |_| pages2.dropped_unanswered(idx2)),
                "closing preview cancels in-flight TextOnly request"
            );
        },
    );
}

#[test]
fn pdf_find_works_with_tenxer_slash_and_rejects_locked_or_failed_pdfs() {
    hosted(
        MODULE,
        "pdf_find::pdf_find_works_with_tenxer_slash_and_rejects_locked_or_failed_pdfs",
        |mut host| {
            let pages = Recorded::default();
            let folder = show_pdf_with_text(&mut host, &pages, 2, Some("Tenxer slash needle"));

            // 10xer mode: `/` opens find on a focused PDF and Escape returns focus to the PDF.
            host.runtime().update(|app: &mut App, _: &mut Tx| {
                shell::saved(app).change(|s| s.tenxer_mode = true);
            });
            while host.turn() {}

            focus_pdf(&mut host);
            press(&mut host, Key::Text("/".into()), Mods::default());
            assert!(host.app().preview.drawer.is_finding());
            assert!(host.app().preview.drawer.is_find_focused());
            type_text(&mut host, "needle");
            assert_eq!(
                host.app().preview.viewer.find_status(),
                Some(FindStatus {
                    current: Some(1),
                    total: 1,
                })
            );
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().preview.drawer.is_finding());
            assert_eq!(focused_key(&mut host).as_deref(), Some(pdf::PDF_KEY));

            // Back to default mode and load a password-protected (locked) PDF -> not searchable.
            host.runtime().update(|app: &mut App, _: &mut Tx| {
                shell::saved(app).change(|s| s.tenxer_mode = false);
            });
            while host.turn() {}

            let locked = Recorded::default();
            install(&mut host, &locked);
            select(&mut host, &Location::local(folder.path().join("readme.md")));
            locked.wait_for(&mut host, 0);
            locked.fail(0, PDF_PASSWORD_PROTECTED);
            assert!(
                host.run_until(PATIENCE, |h| {
                    matches!(
                        h.app().preview.viewer,
                        super::Viewer::Message { can_open: true, .. }
                    )
                }),
                "password-protected PDF shows locked message card"
            );
            assert!(
                !host.app().preview.viewer.searchable(),
                "locked PDF is not searchable"
            );
            assert!(
                !has_element(&mut host, "preview-find"),
                "locked PDF hides header Find button"
            );
            press(&mut host, Key::Command('f'), CTRL);
            assert!(
                !host.app().preview.drawer.is_finding(),
                "Ctrl+F does not open find on a locked PDF"
            );

            // Failed PDF is also not searchable.
            let failed = Recorded::default();
            install(&mut host, &failed);
            select(&mut host, &Location::local(folder.path().join("todo.txt")));
            failed.wait_for(&mut host, 0);
            failed.fail(0, "Unable to parse damaged PDF");
            assert!(
                host.run_until(PATIENCE, |h| {
                    matches!(
                        h.app().preview.viewer,
                        super::Viewer::Message {
                            can_open: false,
                            ..
                        }
                    )
                }),
                "failed PDF shows error message card"
            );
            assert!(
                !host.app().preview.viewer.searchable(),
                "failed PDF is not searchable"
            );
            assert!(!has_element(&mut host, "preview-find"));
        },
    );
}

#[test]
fn pdf_find_uses_single_pass_text_index_and_upgrades_unrendered_page_on_jump() {
    hosted(
        MODULE,
        "pdf_find::pdf_find_uses_single_pass_text_index_and_upgrades_unrendered_page_on_jump",
        |mut host| {
            let pages = Recorded::default();
            pages.enable_pdf_text();
            let page_texts = [
                Some("Page zero intro"),
                Some("Page one no hit"),
                Some("Page two no hit"),
                Some("Page three no hit"),
                Some("Page four contains TREASURE"),
            ];
            let _folder = show_pdf_with_text(&mut host, &pages, 5, page_texts[0]);
            let after_initial = answer_pending_pages(&mut host, &pages, 5, &page_texts, 0);

            focus_pdf(&mut host);
            press(&mut host, Key::Command('f'), CTRL);
            type_text(&mut host, "treasure");

            // Single-pass text extraction starts immediately and shows the header busy indicator.
            pages.wait_for_pdf_text(&mut host, 0);
            assert!(
                host.run_until(PATIENCE, |h| h.app().preview.viewer.busy().is_some()),
                "busy indicator shows while single-pass text index loads"
            );
            assert_eq!(
                pages.count(),
                after_initial,
                "single-pass text index suppresses per-page 16x16 TextOnly requests"
            );

            // Answer single-pass index with all 5 pages.
            pages.answer_pdf_text(
                0,
                Ok(PdfTextIndex {
                    pages: page_texts.iter().map(|t| t.map(str::to_owned)).collect(),
                    truncated: false,
                }),
            );
            assert!(
                host.run_until(PATIENCE, |h| {
                    h.app().preview.viewer.find_status()
                        == Some(FindStatus {
                            current: Some(1),
                            total: 1,
                        })
                        && h.app().preview.viewer.busy().is_none()
                }),
                "single-pass index populates offscreen match immediately and clears busy"
            );
            assert!(
                host.run_until(PATIENCE, |h| pdf_scroll_offset(h) > 0.0),
                "viewport scrolls to reveal the unrendered match on page 4"
            );

            // Scrolling page 4 into view triggers a normal full-width Page render (never 16x16),
            // which upgrades the match with its real glyph layer and selects "TREASURE".
            pages.wait_for(&mut host, after_initial);
            let after_jump = answer_pending_pages(&mut host, &pages, 5, &page_texts, after_initial);
            assert!(
                (after_initial..after_jump)
                    .all(|i| pages.request(i).media_size != MediaPreviewSize::new(16, 16)),
                "no 16x16 TextOnly requests are issued once single-pass index is loaded"
            );
            assert!(
                host.run_until(PATIENCE, |h| pdf_selected_text(h) == "TREASURE"),
                "full page render upgrades the current match's glyph selection"
            );

            // Closing and reopening find reuses the loaded index without another extraction pass.
            press(&mut host, Key::Escape, Mods::default());
            press(&mut host, Key::Command('f'), CTRL);
            type_text(&mut host, "intro");
            assert_eq!(
                host.app().preview.viewer.find_status(),
                Some(FindStatus {
                    current: Some(1),
                    total: 1,
                })
            );
            assert_eq!(
                pages.pdf_text_count(),
                1,
                "loaded text index is reused across find sessions"
            );
        },
    );
}

#[test]
fn pdf_find_cancels_inflight_text_index_and_skips_single_page_pdfs() {
    hosted(
        MODULE,
        "pdf_find::pdf_find_cancels_inflight_text_index_and_skips_single_page_pdfs",
        |mut host| {
            let pages = Recorded::default();
            pages.enable_pdf_text();
            let folder = show_pdf_with_text(&mut host, &pages, 4, Some("Page zero"));
            let _ = answer_pending_pages(&mut host, &pages, 4, &[Some("Page zero"); 4], 0);

            // 1. Escape cancels an in-flight single-pass text extraction.
            focus_pdf(&mut host);
            press(&mut host, Key::Command('f'), CTRL);
            type_text(&mut host, "missing");
            pages.wait_for_pdf_text(&mut host, 0);
            press(&mut host, Key::Escape, Mods::default());
            assert!(
                host.run_until(PATIENCE, |_| pages.pdf_text_dropped_unanswered(0)),
                "Escape cancels in-flight single-pass text extraction"
            );

            // 2. Switching files cancels an in-flight single-pass text extraction.
            press(&mut host, Key::Command('f'), CTRL);
            pages.wait_for_pdf_text(&mut host, 1);
            select(&mut host, &Location::local(folder.path().join("readme.md")));
            assert!(
                host.run_until(PATIENCE, |_| pages.pdf_text_dropped_unanswered(1)),
                "switching previewed file cancels in-flight single-pass text extraction"
            );

            // 3. Single-page PDFs skip single-pass text extraction completely.
            let single = Recorded::default();
            single.enable_pdf_text();
            install(&mut host, &single);
            host.runtime().update(move |app: &mut App, _: &mut Tx| {
                let entry: FileEntry = app.preview.current.clone().expect("a previewed file");
                app.preview.viewer = pdf::viewer(
                    entry,
                    (&page_png(80), 0, 1),
                    Some(text_layer("Only page", 400.0, 550.0)),
                );
            });
            while host.turn() {}
            focus_pdf(&mut host);
            press(&mut host, Key::Command('f'), CTRL);
            type_text(&mut host, "only");
            assert_eq!(
                host.app().preview.viewer.find_status(),
                Some(FindStatus {
                    current: Some(1),
                    total: 1,
                })
            );
            assert_eq!(
                single.pdf_text_count(),
                0,
                "single-page PDF already loaded must skip PreviewPdfText"
            );
        },
    );
}

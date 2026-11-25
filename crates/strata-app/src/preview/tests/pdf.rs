// SPDX-License-Identifier: MIT

use rune::{Key, Mods, Tx};
use strata_core::model::{FileEntry, Location};
use strata_desktop::settings::{KeymapOverrides, TextSize};
use strata_fs::services::PreviewContent;

use super::{
    super::pdf,
    provider::{Recorded, install, png},
    wait_for_code_preview,
};
use super::{MODULE, disable_auto_preview, focus_view, listed, press, select, space};
use crate::{
    App, shell,
    test_support::{PATIENCE, hosted, hosted_after},
};

const PAGES: i32 = 30;

/// Answers request `index` with its page rendered at the width it asked, and says which page.
fn answer_page(pages: &Recorded, index: usize) -> usize {
    let request = pages.request(index);
    let width = u32::try_from(request.media_size.width).expect("a width");
    pages.answer(
        index,
        PreviewContent::Pdf {
            png: page_png(width),
            page: request.pdf_page,
            pages: PAGES,
            text_layer: None,
        },
    );
    usize::try_from(request.pdf_page).expect("a page")
}

/// A blank page `width` pixels wide, Letter-shaped.
fn page_png(width: u32) -> Vec<u8> {
    png(width, width * 11 / 8)
}

fn show_pdf(host: &mut crate::host::offscreen::Offscreen, pages: &Recorded) -> tempfile::TempDir {
    let folder = listed(host);
    disable_auto_preview(host);
    select(host, &Location::local(folder.path().join("todo.txt")));
    focus_view(host);
    space(host);
    wait_for_code_preview(host);
    open_pdf(host, pages);
    folder
}

fn open_pdf(host: &mut crate::host::offscreen::Offscreen, pages: &Recorded) {
    install(host, pages);
    host.runtime().update(move |app: &mut App, _: &mut Tx| {
        let entry: FileEntry = app.preview.current.clone().expect("a previewed file");
        app.preview.viewer = pdf::viewer(entry, (&page_png(80), 0, PAGES), None);
    });
    while host.turn() {}
}

fn held(host: &crate::host::offscreen::Offscreen, page: usize) -> Option<[u32; 2]> {
    match &host.app().preview.viewer {
        super::Viewer::Pdf(pdf) => pdf.state.held(page),
        _ => None,
    }
}

fn level(host: &crate::host::offscreen::Offscreen) -> f32 {
    match &host.app().preview.viewer {
        super::Viewer::Pdf(pdf) => pdf.state.level(),
        _ => panic!("the PDF preview"),
    }
}

#[test]
fn pages_render_one_at_a_time_and_stale_or_unwanted_ones_are_dropped() {
    hosted(
        MODULE,
        "pdf::pages_render_one_at_a_time_and_stale_or_unwanted_ones_are_dropped",
        |mut host| {
            let pages = Recorded::default();
            let _folder = show_pdf(&mut host, &pages);
            assert!(
                host.run_until(PATIENCE, |_| pages.count() == 1),
                "a page asked"
            );
            for _ in 0..5 {
                host.turn();
            }
            assert_eq!(pages.count(), 1, "one render at a time");

            let page = answer_page(&pages, 0);
            let width = u32::try_from(pages.request(0).media_size.width).unwrap();
            let landed = host.run_until(PATIENCE, |host| {
                held(host, page).is_some_and(|[w, _]| w == width)
            });
            assert!(landed, "page {page} lands at the width it asked");
            assert!(
                host.run_until(PATIENCE, |_| pages.count() == 2),
                "the next page"
            );

            host.runtime().update(|_: &mut App, tx: &mut Tx| {
                tx.scroll_to_y(format!("{}-scroll", pdf::PDF_KEY), f32::MAX);
            });
            let dropped = host.run_until(PATIENCE, |_| pages.dropped_unanswered(1));
            assert!(
                dropped,
                "a page scrolled out of view is no longer asked for"
            );

            let last = pages.count() - 1;
            open_pdf(&mut host, &pages);
            let stale = answer_page(&pages, last);
            for _ in 0..20 {
                host.turn();
            }
            assert_eq!(held(&host, stale), None, "an answer for the old viewer");
        },
    );
}

/// Writes `saved` as the test process's `settings.toml`.
fn save(saved: &str) {
    let file = strata_core::storage::config_directory().join("settings.toml");
    std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
    std::fs::write(file, saved).expect("settings");
}

/// Zooms the PDF in, from the poll as a host restoring a zoom would.
fn zoom_in(host: &mut crate::host::offscreen::Offscreen) {
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        if let super::Viewer::Pdf(pdf) = &mut app.preview.viewer {
            pdf.state.set_level(2.0);
        }
        tx.focus(pdf::PDF_KEY, true);
    });
    let zoomed = host.run_until(PATIENCE, |host| level(host) > 1.5);
    assert!(zoomed, "the PDF zoomed in");
}

fn text_size(host: &crate::host::offscreen::Offscreen) -> TextSize {
    shell::preferences(host.app()).text_size
}

#[test]
fn the_pdf_zoom_reset_follows_the_key_map_ahead_of_the_text_size() {
    hosted_after(
        MODULE,
        "pdf::the_pdf_zoom_reset_follows_the_key_map_ahead_of_the_text_size",
        || save("text_size = 21\n[keymap.default]\npdf_zoom_reset = [\"<Control>e\"]\n"),
        |mut host| {
            let ctrl = Mods {
                ctrl: true,
                cmd: true,
                ..Mods::default()
            };
            let _folder = show_pdf(&mut host, &Recorded::default());

            zoom_in(&mut host);
            press(&mut host, Key::Command('e'), ctrl);
            assert!(level(&host) < 1.01, "the rebound key resets the PDF");
            assert_eq!(text_size(&host), TextSize::new(21));

            zoom_in(&mut host);
            press(&mut host, Key::Command('0'), ctrl);
            assert!(level(&host) > 1.5, "Ctrl+0 is no longer the PDF's");
            assert_eq!(
                text_size(&host),
                TextSize::default(),
                "Ctrl+0 resets the text"
            );

            host.runtime().update(|app: &mut App, _: &mut Tx| {
                shell::saved(app).change(|settings| {
                    settings.keymap = KeymapOverrides::default();
                    settings.text_size = TextSize::new(21);
                });
            });
            zoom_in(&mut host);
            press(&mut host, Key::Command('0'), ctrl);
            assert!(level(&host) < 1.01, "the shipped Ctrl+0 resets the PDF");
            assert_eq!(
                text_size(&host),
                TextSize::new(21),
                "and leaves the text size"
            );
        },
    );
}

/// Puts the pointer over the middle of the PDF's pages.
fn hover_pages(host: &mut crate::host::offscreen::Offscreen) {
    let scroll = format!("{}-scroll", pdf::PDF_KEY);
    let elements = host.runtime().elements();
    let pages = elements
        .iter()
        .find(|element| !element.exiting && element.key.as_deref() == Some(scroll.as_str()));
    let [x, y, w, h] = pages.expect("the pages are mounted").visible;
    host.runtime().pointer_move(x + w / 2.0, y + h / 2.0);
    while host.turn() {}
}

#[test]
fn escape_closes_a_pdf_preview_under_the_pointer() {
    hosted(
        MODULE,
        "pdf::escape_closes_a_pdf_preview_under_the_pointer",
        |mut host| {
            let _folder = show_pdf(&mut host, &Recorded::default());
            hover_pages(&mut host);
            focus_view(&mut host);
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().preview.enabled, "Escape closes the preview");
        },
    );
}

fn current_page(host: &crate::host::offscreen::Offscreen) -> usize {
    match &host.app().preview.viewer {
        super::Viewer::Pdf(pdf) => pdf.state.current_page(),
        _ => panic!("the PDF preview"),
    }
}

fn page_input_text(host: &crate::host::offscreen::Offscreen) -> String {
    match &host.app().preview.viewer {
        super::Viewer::Pdf(pdf) => pdf.state.page_input_text().to_owned(),
        _ => panic!("the PDF preview"),
    }
}

fn page_input_focused(host: &crate::host::offscreen::Offscreen) -> bool {
    match &host.app().preview.viewer {
        super::Viewer::Pdf(pdf) => pdf.state.is_page_input_focused(),
        _ => panic!("the PDF preview"),
    }
}

fn focus_key(host: &mut crate::host::offscreen::Offscreen, key: &str) {
    let key = key.to_owned();
    host.runtime().update(move |_: &mut App, tx: &mut Tx| {
        tx.focus(key, true);
    });
    while host.turn() {}
}

fn click_key(host: &mut crate::host::offscreen::Offscreen, key: &str) {
    let target = host
        .runtime()
        .elements()
        .into_iter()
        .find(|el| !el.exiting && el.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("element `{key}` should be mounted"));
    let [x, y, w, h] = target.visible;
    host.runtime().pointer_move(x + w / 2.0, y + h / 2.0);
    host.runtime().pointer_down(x + w / 2.0, y + h / 2.0);
    host.runtime().pointer_up(x + w / 2.0, y + h / 2.0);
    for _ in 0..8 {
        host.turn();
    }
}

#[test]
fn jump_to_page_box_navigates_and_escape_cancels_without_closing_preview() {
    hosted(
        MODULE,
        "pdf::jump_to_page_box_navigates_and_escape_cancels_without_closing_preview",
        |mut host| {
            let _folder = show_pdf(&mut host, &Recorded::default());
            let input_key = format!("{}-page-input", pdf::PDF_KEY);
            let next_key = format!("{}-page-next", pdf::PDF_KEY);

            assert_eq!(current_page(&host), 0);
            assert_eq!(page_input_text(&host), "1");

            // Focus page input, type "15", press Enter -> jumps to page 15 and refocuses PDF.
            focus_key(&mut host, &input_key);
            assert!(page_input_focused(&host));
            for ch in ['1', '5'] {
                press(&mut host, Key::Text(ch.to_string()), Mods::default());
            }
            press(&mut host, Key::Enter, Mods::default());
            for _ in 0..8 {
                host.turn();
            }
            assert_eq!(current_page(&host), 14, "jumped to page 15 (0-based 14)");
            assert_eq!(page_input_text(&host), "15");
            assert!(!page_input_focused(&host));
            assert!(host.app().preview.enabled, "preview remains open");

            // Next button steps to page 16.
            click_key(&mut host, &next_key);
            assert_eq!(current_page(&host), 15);
            assert_eq!(page_input_text(&host), "16");

            // Typing into page input and pressing Escape cancels edit without closing preview.
            focus_key(&mut host, &input_key);
            press(&mut host, Key::Text("2".into()), Mods::default());
            press(&mut host, Key::Escape, Mods::default());
            while host.turn() {}
            assert!(
                host.app().preview.enabled,
                "Escape while editing page input must not close the preview drawer"
            );
            assert!(!page_input_focused(&host));
            assert_eq!(current_page(&host), 15);
            assert_eq!(page_input_text(&host), "16");

            // A subsequent Escape (outside the page input) closes the preview drawer.
            press(&mut host, Key::Escape, Mods::default());
            while host.turn() {}
            assert!(
                !host.app().preview.enabled,
                "Escape outside page input closes the preview drawer"
            );
        },
    );
}

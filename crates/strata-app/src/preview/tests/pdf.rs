// SPDX-License-Identifier: MIT

//! The PDF preview's host (`preview::pdf`): pages asked of the provider one at a time, a later
//! page landing, a request the view no longer wants dropped, a reply for a viewer that has
//! gone ignored; and the zoom reset read from the key map at the window's stage, ahead of the
//! text size.

use std::{
    cell::{Cell, RefCell},
    io::Cursor,
    rc::Rc,
};

use rune::{Key, Mods, Tx};
use strata_core::model::{FileEntry, Location};
use strata_desktop::settings::{KeymapOverrides, TextSize};
use strata_fs::services::{
    LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
};

use super::{super::pdf, wait_for_code_preview};
use super::{MODULE, disable_auto_preview, focus_view, listed, press, select, space};
use crate::{
    App, shell,
    test_support::{PATIENCE, hosted, hosted_after},
};

/// A request the fake provider was handed: what it asked, how to answer, and whether its load
/// was dropped before an answer.
struct Asked {
    request: PreviewRequest,
    emit: Rc<dyn Fn(PreviewEvent)>,
    answered: Rc<Cell<bool>>,
    dropped_unanswered: Rc<Cell<bool>>,
}

/// A provider that keeps every request for the test to answer.
#[derive(Clone, Default)]
struct Pages(Rc<RefCell<Vec<Asked>>>);

impl PreviewProvider for Pages {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        let answered = Rc::new(Cell::new(false));
        let dropped_unanswered = Rc::new(Cell::new(false));
        let (seen, dropped) = (answered.clone(), dropped_unanswered.clone());
        self.0.borrow_mut().push(Asked {
            request,
            emit,
            answered,
            dropped_unanswered,
        });
        LoadHandle::new(move || dropped.set(!seen.get()))
    }
}

impl Pages {
    fn count(&self) -> usize {
        self.0.borrow().len()
    }

    /// Answers request `index` with its page rendered at the width it asked.
    fn answer(&self, index: usize) -> usize {
        let asked = self.0.borrow();
        let asked = &asked[index];
        let width = u32::try_from(asked.request.media_size.width).expect("a width");
        asked.answered.set(true);
        (asked.emit)(PreviewEvent::Ready(Preview {
            request_id: asked.request.id,
            entry: asked.request.entry.clone(),
            content_type: "application/pdf".to_owned(),
            content: PreviewContent::Pdf {
                png: page_png(width),
                page: asked.request.pdf_page,
                pages: PAGES,
                text_layer: None,
            },
        }));
        usize::try_from(asked.request.pdf_page).expect("a page")
    }

    fn dropped_unanswered(&self, index: usize) -> bool {
        self.0.borrow()[index].dropped_unanswered.get()
    }
}

const PAGES: i32 = 30;

/// A blank page `width` pixels wide, Letter-shaped.
fn page_png(width: u32) -> Vec<u8> {
    let height = width * 11 / 8;
    let page = image::RgbaImage::from_pixel(width, height, image::Rgba([255, 255, 255, 255]));
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(page)
        .write_to(&mut png, image::ImageFormat::Png)
        .expect("a PNG");
    png.into_inner()
}

/// Opens the preview on `todo.txt`, then shows a PDF of [`PAGES`] pages in it, opened from a
/// small first page, with its later pages asked of `pages`.
fn show_pdf(host: &mut crate::host::offscreen::Offscreen, pages: &Pages) -> tempfile::TempDir {
    let folder = listed(host);
    disable_auto_preview(host);
    select(host, &Location::local(folder.path().join("todo.txt")));
    focus_view(host);
    space(host);
    wait_for_code_preview(host);
    open_pdf(host, pages);
    folder
}

fn open_pdf(host: &mut crate::host::offscreen::Offscreen, pages: &Pages) {
    let pages = pages.clone();
    host.runtime().update(move |app: &mut App, _: &mut Tx| {
        app.preview.loader.set_provider(Rc::new(pages));
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

/// The view asks one page at a time; a page it is answered lands and the next is asked; a
/// request for a page scrolled far out of view is dropped unanswered; and an answer for a
/// viewer that has been replaced changes nothing in the new one.
#[test]
fn pages_render_one_at_a_time_and_stale_or_unwanted_ones_are_dropped() {
    hosted(
        MODULE,
        "pdf::pages_render_one_at_a_time_and_stale_or_unwanted_ones_are_dropped",
        |mut host| {
            let pages = Pages::default();
            let _folder = show_pdf(&mut host, &pages);
            assert!(
                host.run_until(PATIENCE, |_| pages.count() == 1),
                "a page asked"
            );
            for _ in 0..5 {
                host.turn();
            }
            assert_eq!(pages.count(), 1, "one render at a time");

            let page = pages.answer(0);
            let width = u32::try_from(pages.0.borrow()[0].request.media_size.width).unwrap();
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
            let stale = pages.answer(last);
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

/// With `pdf_zoom_reset` rebound to Ctrl+E, Ctrl+E resets the focused PDF and Ctrl+0 is the
/// text size's again; with the binding back to Ctrl+0, Ctrl+0 resets the PDF and leaves the
/// text size, as GTK's `zoom_shortcut` reads the key map.
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
            let _folder = show_pdf(&mut host, &Pages::default());

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

/// Escape closes a PDF preview with the pointer over its pages: the pages' hover ends as
/// they go, after the viewer has.
#[test]
fn escape_closes_a_pdf_preview_under_the_pointer() {
    hosted(
        MODULE,
        "pdf::escape_closes_a_pdf_preview_under_the_pointer",
        |mut host| {
            let _folder = show_pdf(&mut host, &Pages::default());
            hover_pages(&mut host);
            focus_view(&mut host);
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().preview.enabled, "Escape closes the preview");
        },
    );
}

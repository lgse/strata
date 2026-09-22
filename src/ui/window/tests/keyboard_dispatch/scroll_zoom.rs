// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{
    LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
};
use crate::ui::preferences::TextSize;

struct PdfPreview;

impl PreviewProvider for PdfPreview {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        let png = gtk::gdk::MemoryTexture::new(
            1,
            1,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(vec![255u8; 4]),
            4,
        )
        .save_to_png_bytes()
        .to_vec();
        emit(PreviewEvent::Ready(Preview {
            request_id: request.id,
            entry: request.entry,
            content_type: "application/pdf".into(),
            content: PreviewContent::Pdf {
                png,
                page: request.pdf_page,
                pages: 2,
            },
        }));
        LoadHandle::new(|| {})
    }
}

#[test]
fn discrete_scroll_steps_respect_modifiers_and_text_size_limits() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::scroll_zoom::discrete_scroll_steps_respect_modifiers_and_text_size_limits",
        || {
            let fixture = KeyboardFixture::new();
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            let ctrl = ModifierType::CONTROL_MASK;
            for (initial, modifiers, dy, expected, handled) in [
                (13, ctrl, -1.0, 14, true),
                (13, ctrl, 1.0, 12, true),
                (13, ctrl, -3.0, 16, true),
                (13, ctrl, 0.0, 13, false),
                (48, ctrl, -1.0, 48, true),
                (8, ctrl, 1.0, 8, true),
                (13, ModifierType::empty(), -1.0, 13, false),
                (13, ctrl | ModifierType::ALT_MASK, -1.0, 13, false),
                (13, ctrl | ModifierType::SUPER_MASK, -1.0, 13, false),
            ] {
                preferences.set_text_size(TextSize::new(initial));
                let result = keyboard::handle_text_zoom_scroll(
                    &preferences,
                    modifiers,
                    Some(fixture.view.widget()),
                    dy,
                );
                assert_eq!(result == glib::Propagation::Stop, handled);
                assert_eq!(preferences.text_size(), TextSize::new(expected));
            }
        },
    );
}

#[test]
fn smooth_scroll_deltas_accumulate_into_wheel_steps() {
    let surface = gtk::gdk::ScrollUnit::Surface;
    let wheel = gtk::gdk::ScrollUnit::Wheel;

    let zoom = keyboard::TextZoomScroll::default();
    assert_eq!(zoom.accumulate(surface, 4.0), 0);
    assert_eq!(zoom.accumulate(surface, 4.0), 0);
    assert_eq!(zoom.accumulate(surface, 4.0), 1);
    assert_eq!(
        zoom.accumulate(surface, 4.0),
        0,
        "a completed step must not repeat on the next delta"
    );

    assert_eq!(zoom.accumulate(wheel, -1.0), -1);
    assert_eq!(zoom.accumulate(wheel, -3.0), -3);

    let reversed = keyboard::TextZoomScroll::default();
    assert_eq!(reversed.accumulate(surface, 6.0), 0);
    assert_eq!(reversed.accumulate(surface, -6.0), 0);
    assert_eq!(reversed.accumulate(surface, -6.0), -1);
}

#[test]
fn pdf_pages_and_scrollbars_keep_scroll_zoom_ownership() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::scroll_zoom::pdf_pages_and_scrollbars_keep_scroll_zoom_ownership",
        || {
            let fixture = KeyboardFixture::with_provider(Rc::new(PdfPreview));
            assert!(fixture.press(Key::space, ModifierType::empty()));
            let list =
                widget_with_class(&fixture.preview.widget(), "preview-pdf-list").expect("PDF list");
            let mut parent = list.parent();
            let scroll = loop {
                let widget = parent.expect("PDF scroll container");
                if let Ok(scroll) = widget.clone().downcast::<gtk::ScrolledWindow>() {
                    break scroll;
                }
                parent = widget.parent();
            };
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            let initial = preferences.text_size();
            for target in [
                list,
                scroll.clone().upcast(),
                scroll.hscrollbar(),
                scroll.vscrollbar(),
            ] {
                assert_eq!(
                    keyboard::handle_text_zoom_scroll(
                        &preferences,
                        ModifierType::CONTROL_MASK,
                        Some(target),
                        -1.0,
                    ),
                    glib::Propagation::Proceed,
                );
                assert_eq!(preferences.text_size(), initial);
            }
        },
    );
}

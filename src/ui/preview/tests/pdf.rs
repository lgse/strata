// SPDX-License-Identifier: MIT

use gtk::gdk::{Key, ModifierType as M};

use super::*;

fn pdf_keys(drawer: &PreviewDrawer) -> gtk::EventControllerKey {
    let mut child = drawer.state.content.first_child();
    while let Some(widget) = child {
        if widget.has_css_class("preview-pdf-scroll") {
            let controllers = widget.observe_controllers();
            return (0..controllers.n_items())
                .find_map(|index| {
                    controllers
                        .item(index)
                        .and_downcast::<gtk::EventControllerKey>()
                })
                .expect("PDF key controller");
        }
        child = widget.next_sibling();
    }
    panic!("no PDF view");
}

fn clipboard_text(clipboard: &gtk::gdk::Clipboard) -> String {
    clipboard
        .content()
        .and_then(|content| content.value(glib::Type::STRING).ok())
        .and_then(|value| value.get::<String>().ok())
        .unwrap_or_default()
}

#[test]
fn pdf_text_keys_select_and_copy_only_on_their_exact_chords() {
    crate::test_support::gtk_test(
        "ui::preview::tests::pdf::pdf_text_keys_select_and_copy_only_on_their_exact_chords",
        || {
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            drawer.show(entry("paper.pdf"), None);
            let text = "page text";
            {
                let pending = provider.0.borrow();
                let pending = &pending[0];
                (pending.emit)(PreviewEvent::Ready(Preview {
                    request_id: pending.request.id,
                    entry: pending.request.entry.clone(),
                    content_type: "application/pdf".into(),
                    content: PreviewContent::Pdf {
                        png: Vec::new(),
                        page: 0,
                        pages: 1,
                        text_layer: Some(Arc::new(PdfTextLayer {
                            width: 100.0,
                            height: 20.0,
                            text: text.into(),
                            glyphs: (0..text.len())
                                .map(|index| {
                                    let x = index as f32 * 10.0;
                                    [x, 0.0, x + 10.0, 12.0]
                                })
                                .collect(),
                        })),
                    },
                }));
            }
            let keys = pdf_keys(&drawer);
            let press = |key: Key, modifiers: M| {
                keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers])
            };
            let near_misses = [
                M::empty(),
                M::CONTROL_MASK | M::SHIFT_MASK,
                M::CONTROL_MASK | M::ALT_MASK,
            ];
            for modifiers in near_misses {
                assert!(!press(Key::a, modifiers), "{modifiers:?}+a");
            }
            assert!(!press(Key::c, M::CONTROL_MASK), "nothing is selected yet");
            assert!(press(Key::a, M::CONTROL_MASK));

            let clipboard = drawer.widget().clipboard();
            clipboard.set_text("kept");
            for modifiers in near_misses {
                assert!(!press(Key::c, modifiers), "{modifiers:?}+c");
            }
            assert_eq!(clipboard_text(&clipboard), "kept");
            assert!(press(Key::c, M::CONTROL_MASK));
            assert_eq!(clipboard_text(&clipboard), text);
            drawer.close();
        },
    );
}

fn labelled_button(root: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    if let Some(button) = root.downcast_ref::<gtk::Button>()
        && button.label().is_some_and(|text| text == label)
    {
        return Some(button.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = labelled_button(&widget, label) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
fn password_protected_pdf_failures_offer_external_opening_only_where_allowed() {
    crate::test_support::gtk_test(
        "ui::preview::tests::pdf::password_protected_pdf_failures_offer_external_opening_only_where_allowed",
        || {
            for (allow_external_open, message, offers_open) in [
                (true, crate::sandbox::PDF_PASSWORD_PROTECTED, true),
                (false, crate::sandbox::PDF_PASSWORD_PROTECTED, false),
                (true, "The sandboxed preview renderer failed", false),
            ] {
                let provider = Rc::new(Provider::default());
                let drawer = PreviewDrawer::new(provider.clone(), allow_external_open);
                let opened = Rc::new(Cell::new(0));
                let count = opened.clone();
                drawer
                    .state
                    .open
                    .connect_clicked(move |_| count.set(count.get() + 1));
                drawer.show(entry("locked.pdf"), None);
                {
                    let pending = provider.0.borrow();
                    (pending[0].emit)(PreviewEvent::Failed {
                        request_id: pending[0].request.id,
                        entry: pending[0].request.entry.clone(),
                        message: message.into(),
                    });
                }
                let open = labelled_button(drawer.state.content.upcast_ref(), "Open");
                assert_eq!(
                    open.is_some(),
                    offers_open,
                    "external open {allow_external_open}: {message}"
                );
                if let Some(open) = open {
                    open.emit_clicked();
                    assert_eq!(
                        opened.get(),
                        1,
                        "the Open button hands off like the header action"
                    );
                }
            }
        },
    );
}

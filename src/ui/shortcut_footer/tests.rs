// SPDX-License-Identifier: MIT

use super::*;

mod clipboard;
mod localized;
mod modes;
mod reference;

fn press(key: gdk::Key, modifiers: gdk::ModifierType) -> KeyPress {
    KeyPress::from_signal(
        crate::ui::keys::key_from_gdk(key),
        0,
        crate::ui::keys::modifiers_from_gdk(modifiers),
    )
}

fn expect_key(
    footer: &ShortcutFooter,
    key: gdk::Key,
    modifiers: gdk::ModifierType,
    expected: Option<glib::Propagation>,
) {
    assert_eq!(
        footer.handle_key(&press(key, modifiers)),
        expected,
        "{key:?} with {modifiers:?}"
    );
}

impl ShortcutFooter {
    pub(crate) fn assert_hints_visible(&self, visible: bool) {
        assert_eq!(
            self.widget().is_visible(),
            visible || self.paste.is_visible() || self.count.is_visible()
        );
        assert_eq!(self.more.is_visible(), visible);
    }
}

fn press_reference(footer: &ShortcutFooter, key: gdk::Key) {
    let controllers = footer.panel.observe_controllers();
    let controller = (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .next()
        .expect("reference key controller");
    assert!(
        controller.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &gdk::ModifierType::empty()]),
        "{key:?} should be handled by the open reference"
    );
}

fn reference_pairs(labels: &[String], key: &str, action: &str) -> bool {
    labels
        .windows(2)
        .any(|pair| pair[0] == action && pair[1] == key)
}

fn has_label(labels: &[String], text: &str) -> bool {
    labels.iter().any(|label| label == text)
}

/// The reference shows every `present` label and none of the `absent` ones.
fn assert_rows(labels: &[String], present: &[&str], absent: &[&str]) {
    for text in present {
        assert!(has_label(labels, text), "missing {text:?}: {labels:?}");
    }
    for text in absent {
        assert!(!has_label(labels, text), "unexpected {text:?}: {labels:?}");
    }
}

fn reference_labels(footer: &ShortcutFooter) -> Vec<String> {
    let mut labels = Vec::new();
    collect_label_text(footer.reference.upcast_ref(), &mut labels);
    labels
}

fn collect_label_text(widget: &gtk::Widget, labels: &mut Vec<String>) {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        labels.push(label.text().to_string());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        collect_label_text(&current, labels);
        child = current.next_sibling();
    }
}

fn category_named(footer: &ShortcutFooter, name: &str) -> gtk::Button {
    footer
        .category_buttons()
        .into_iter()
        .find(|button| {
            let mut labels = Vec::new();
            collect_label_text(button.upcast_ref(), &mut labels);
            has_label(&labels, name)
        })
        .unwrap_or_else(|| panic!("{name} category"))
}

fn focus_within(window: &gtk::Window, widget: &impl IsA<gtk::Widget>) -> bool {
    gtk::prelude::RootExt::focus(window).is_some_and(|focus| {
        focus == *widget.upcast_ref::<gtk::Widget>() || focus.is_ancestor(widget)
    })
}

fn settle() {
    let until = std::time::Instant::now() + std::time::Duration::from_millis(200);
    while std::time::Instant::now() < until {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

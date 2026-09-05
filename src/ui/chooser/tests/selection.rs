// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::ui::browser_modes::BrowserMode;
use std::time::{Duration, Instant};

fn selection_models(widget: &gtk::Widget) -> Vec<gtk::SelectionModel> {
    let mut models = Vec::new();
    if widget.is_mapped() {
        if let Some(view) = widget.downcast_ref::<gtk::ListView>()
            && let Some(model) = view.model()
        {
            models.push(model);
        }
        if let Some(view) = widget.downcast_ref::<gtk::GridView>()
            && let Some(model) = view.model()
        {
            models.push(model);
        }
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        models.extend(selection_models(&widget));
        child = widget.next_sibling();
    }
    models
}

fn new_folder_button(widget: &gtk::Widget) -> Option<gtk::Button> {
    if widget.is_mapped() && widget.has_css_class("chooser-new-folder") {
        return widget.clone().downcast().ok();
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(button) = new_folder_button(&widget) {
            return Some(button);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
#[ignore = "requires a GTK display and isolated XDG directories; run this test alone"]
fn every_view_enforces_single_selection_including_type_groups() {
    gtk::init().expect("GTK display");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("exclusive GTK main context");
    let root = tempfile::tempdir().expect("fixture directory");
    std::fs::write(root.path().join("notes.txt"), "notes").expect("text fixture");
    std::fs::write(root.path().join("data.json"), "{}").expect("JSON fixture");
    for mode in [
        BrowserMode::Columns,
        BrowserMode::Grid,
        BrowserMode::Explorer,
    ] {
        for multiple in [false, true] {
            let view = BrowserView::new_chooser(ChooserFileSource::new(), multiple);
            view.set_view_mode(mode);
            view.set_group_by_type(true);
            let browser = view.browser();
            let window = gtk::Window::builder()
                .default_width(1000)
                .default_height(700)
                .child(&view.widget())
                .build();
            window.present();
            browser.navigate(Location::local(root.path()));
            let deadline = Instant::now() + Duration::from_secs(5);
            let models = loop {
                while context.pending() {
                    context.iteration(false);
                }
                let models = selection_models(&view.widget());
                if models.iter().map(|model| model.n_items()).sum::<u32>() == 2 {
                    break models;
                }
                assert!(Instant::now() < deadline, "{mode:?} did not load");
                std::thread::sleep(Duration::from_millis(5));
            };
            for model in &models {
                for position in 0..model.n_items() {
                    model.select_item(position, false);
                }
            }
            let expected = if multiple { 2 } else { 1 };
            assert_eq!(
                models
                    .iter()
                    .map(|model| model.selection().size())
                    .sum::<u64>(),
                expected,
                "{mode:?}, multiple={multiple}"
            );
            assert_eq!(
                browser.selected_entries().len() as u64,
                expected,
                "{mode:?}, multiple={multiple}"
            );
            let new_folder =
                new_folder_button(&view.widget()).expect("chooser toolbar folder action");
            assert!(new_folder.has_css_class("column-header-action"));
            new_folder.emit_clicked();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !view.new_entry_is_active() {
                while context.pending() {
                    context.iteration(false);
                }
                assert!(
                    Instant::now() < deadline,
                    "{mode:?} toolbar opens the inline folder entry"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(view.cancel_new_entry());
            window.destroy();
            browser.clear_observer();
        }
    }
}

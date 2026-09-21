// SPDX-License-Identifier: MIT

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
    for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
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

fn mapped_item_view(widget: &gtk::Widget) -> Option<gtk::Widget> {
    if widget.is_mapped() && (widget.is::<gtk::ListView>() || widget.is::<gtk::GridView>()) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(view) = mapped_item_view(&current) {
            return Some(view);
        }
        child = current.next_sibling();
    }
    None
}

fn ancestor_scroll(widget: &gtk::Widget) -> Option<gtk::ScrolledWindow> {
    let mut current = widget.parent();
    while let Some(widget) = current {
        if let Ok(scroll) = widget.clone().downcast::<gtk::ScrolledWindow>() {
            return Some(scroll);
        }
        current = widget.parent();
    }
    None
}

fn selection_model(widget: &gtk::Widget) -> Option<gtk::SelectionModel> {
    widget
        .clone()
        .downcast::<gtk::ListView>()
        .ok()
        .and_then(|list| list.model())
        .or_else(|| {
            widget
                .clone()
                .downcast::<gtk::GridView>()
                .ok()
                .and_then(|grid| grid.model())
        })
}

fn drag_on(widget: &gtk::Widget) -> Option<gtk::GestureDrag> {
    let controllers = widget.observe_controllers();
    (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .filter_map(|controller| controller.downcast::<gtk::GestureDrag>().ok())
        .find(|gesture| gesture.button() == 1)
}

fn find_descendant(
    widget: &gtk::Widget,
    predicate: &impl Fn(&gtk::Widget) -> bool,
) -> Option<gtk::Widget> {
    if predicate(widget) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(found) = find_descendant(&widget, predicate) {
            return Some(found);
        }
    }
    None
}

/// The point must resolve to inert chrome: picking it inside `surface` reaches the
/// surface without crossing an item or a control.
fn inert_point(surface: &gtk::Widget, y: f64) -> Option<f64> {
    let x = f64::from(surface.width()) / 2.0;
    let picked = surface.pick(x, y, gtk::PickFlags::DEFAULT)?;
    let mut current = Some(picked);
    while let Some(widget) = current {
        if widget == *surface {
            return Some(y);
        }
        if widget.is::<gtk::Button>()
            || widget.is::<gtk::Editable>()
            || widget.is::<gtk::Range>()
            || widget.is::<gtk::Scrollbar>()
            || widget.is::<gtk::TextView>()
            || widget.is::<gtk::ListView>()
            || widget.is::<gtk::GridView>()
            || widget.is::<gtk::ColumnView>()
            || widget.is::<gtk::ListBox>()
            || widget.is::<gtk::FlowBox>()
        {
            return None;
        }
        current = widget.parent();
    }
    None
}

fn chrome_click(surface: &gtk::Widget, y_hint: f64, mode: BrowserMode, area: &str) {
    let y = (y_hint.max(0.0) as i32..surface.height())
        .map(f64::from)
        .find_map(|y| inert_point(surface, y));
    let y = y.unwrap_or_else(|| panic!("{mode:?} {area}: no inert point found"));
    let drag = drag_on(surface)
        .unwrap_or_else(|| panic!("{mode:?} {area}: surface exposes no marquee drag"));
    let x = f64::from(surface.width()) / 2.0;
    drag.emit_by_name::<()>("drag-begin", &[&x, &y]);
    drag.emit_by_name::<()>("drag-end", &[&0.0, &0.0]);
}

fn background_click(scroll: &gtk::ScrolledWindow, mode: BrowserMode) {
    let x = f64::from(scroll.width()) / 2.0;
    let y = (0..scroll.height())
        .rev()
        .map(f64::from)
        .find(|y| crate::ui::pointer::is_background(scroll.upcast_ref(), x, *y));
    let y = y.unwrap_or_else(|| panic!("{mode:?} file view: no background point found"));
    let drag = drag_on(scroll.upcast_ref())
        .unwrap_or_else(|| panic!("{mode:?} file view: scroll exposes no marquee drag"));
    drag.emit_by_name::<()>("drag-begin", &[&x, &y]);
    drag.emit_by_name::<()>("drag-end", &[&0.0, &0.0]);
}

#[test]
fn clicking_blank_chrome_clears_the_selection() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::selection::clicking_blank_chrome_clears_the_selection",
        || {
            crate::ui::prepare_portal_ui();
            let context = glib::MainContext::default();
            let root = tempfile::tempdir().expect("fixture directory");
            std::fs::write(root.path().join("notes.txt"), "notes").expect("text fixture");
            std::fs::write(
                root.path().join("sample.png"),
                [
                    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49,
                    0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06,
                    0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44,
                    0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d,
                    0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42,
                    0x60, 0x82,
                ],
            )
            .expect("image fixture");
            for mode in [BrowserMode::Icons, BrowserMode::List] {
                let request = ChooserRequest {
                    token: format!("empty-click-{mode:?}"),
                    title: "Empty click regression".into(),
                    accept_label: "Open".into(),
                    modal: false,
                    parent: None,
                    parent_size_hint: None,
                    initial_directory: root.path().into(),
                    kind: ChooserKind::Open {
                        directory: false,
                        multiple: true,
                    },
                    filters: Vec::new(),
                    current_filter: None,
                    choices: Vec::new(),
                };
                let state = build_chooser(request, Arc::new(AtomicBool::new(false)), |_| {})
                    .expect("chooser");
                let view = state.view.clone();
                let browser = view.browser();
                view.set_view_mode(mode);
                let initialized = Rc::new(Cell::new(false));
                let initialized_at_idle = initialized.clone();
                glib::idle_add_local_once(move || initialized_at_idle.set(true));
                while !initialized.get() {
                    context.iteration(true);
                }
                let deadline = Instant::now() + Duration::from_secs(10);
                let item_view = loop {
                    while context.pending() {
                        context.iteration(false);
                    }
                    if let Some(item_view) = mapped_item_view(&view.widget())
                        && selection_model(&item_view).is_some_and(|model| model.n_items() == 2)
                    {
                        break item_view;
                    }
                    assert!(Instant::now() < deadline, "{mode:?} did not load");
                    std::thread::sleep(Duration::from_millis(5));
                };
                let scroll = ancestor_scroll(&item_view).expect("pane scroll");
                let model = selection_model(&item_view).expect("item model");

                // Blank area inside the file view itself.
                model.select_item(0, true);
                while context.pending() {
                    context.iteration(false);
                }
                assert_eq!(browser.selected_entries().len(), 1, "{mode:?} selection");
                background_click(&scroll, mode);
                while context.pending() {
                    context.iteration(false);
                }
                assert!(
                    browser.selected_entries().is_empty(),
                    "{mode:?} file view background click clears the selection"
                );

                // Blank space below the places in the sidebar.
                model.select_item(0, true);
                while context.pending() {
                    context.iteration(false);
                }
                assert_eq!(
                    browser.selected_entries().len(),
                    1,
                    "{mode:?} selection before sidebar click"
                );
                let sidebar = find_descendant(state.window.upcast_ref(), &|widget| {
                    widget.has_css_class("sidebar-shell")
                })
                .expect("sidebar shell");
                chrome_click(
                    &sidebar,
                    f64::from(sidebar.height()) - 12.0,
                    mode,
                    "sidebar",
                );
                while context.pending() {
                    context.iteration(false);
                }
                assert!(
                    browser.selected_entries().is_empty(),
                    "{mode:?} sidebar blank click clears the selection"
                );

                // Blank body of the preview pane once it is open; the image fixture
                // gives the pane an inert body in both modes.
                let depth = browser.active_depth().expect("active depth");
                let image = (0usize..)
                    .map_while(|position| {
                        browser
                            .entry_at(depth, position)
                            .map(|entry| (position, entry))
                    })
                    .find(|(_, entry)| entry.native_name == "sample.png")
                    .map(|(position, _)| position)
                    .expect("image fixture position");
                model.select_item(
                    u32::try_from(image).expect("image position fits in u32"),
                    true,
                );
                browser.set_selection(depth, &[image], Some(image));
                view.focus_items_from_header();
                while context.pending() {
                    context.iteration(false);
                }
                assert_eq!(
                    browser.selected_entries().len(),
                    1,
                    "{mode:?} selection before preview click"
                );
                let keys: Vec<gtk::EventControllerKey> = {
                    let controllers = state.window.observe_controllers();
                    (0..controllers.n_items())
                        .filter_map(|index| controllers.item(index))
                        .filter_map(|controller| {
                            controller.downcast::<gtk::EventControllerKey>().ok()
                        })
                        .collect()
                };
                assert!(keys.iter().any(|keys| keys.emit_by_name::<bool>(
                    "key-pressed",
                    &[
                        &gtk::gdk::Key::space,
                        &0u32,
                        &gtk::gdk::ModifierType::empty()
                    ]
                )));
                let split = find_descendant(state.window.upcast_ref(), &|widget| {
                    widget.has_css_class("preview-split")
                })
                .and_then(|widget| widget.downcast::<gtk::Paned>().ok())
                .expect("preview split");
                let preview = split.end_child().expect("preview pane");
                let deadline = Instant::now() + Duration::from_secs(5);
                while !preview.is_mapped() || preview.width() <= 0 {
                    while context.pending() {
                        context.iteration(false);
                    }
                    assert!(Instant::now() < deadline, "{mode:?} preview did not open");
                    std::thread::sleep(Duration::from_millis(5));
                }
                chrome_click(&preview, f64::from(preview.height()) / 2.0, mode, "preview");
                while context.pending() {
                    context.iteration(false);
                }
                assert!(
                    browser.selected_entries().is_empty(),
                    "{mode:?} preview blank click clears the selection"
                );

                state.window.destroy();
                browser.clear_observer();
            }
        },
    );
}

// SPDX-License-Identifier: MIT

use super::*;
use std::time::{Duration, Instant};

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut widgets = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(next) = child {
        widgets.extend(descendants(&next));
        child = next.next_sibling();
    }
    widgets
}

fn by_class(root: &gtk::Widget, class: &str) -> gtk::Widget {
    descendants(root)
        .into_iter()
        .find(|widget| widget.has_css_class(class) && widget.is_mapped())
        .unwrap_or_else(|| panic!("missing {class}"))
}

fn settle() {
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn capture(window: &gtk::Window, name: &str) {
    let output = std::path::PathBuf::from("target/ui-evidence");
    if !output.is_dir() {
        return;
    }
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        f64::from(window.width()),
        f64::from(window.height()),
    );
    let texture = window
        .renderer()
        .expect("renderer")
        .render_texture(snapshot.to_node().expect("rendered browser"), None);
    texture
        .save_to_png(output.join(format!("{name}.png")))
        .expect("save evidence");
}

#[test]
fn browser_chrome_insets_and_list_name_alignment() {
    crate::test_support::gtk_test(
        "ui::browser::tests::layout::browser_chrome_insets_and_list_name_alignment",
        || {
            crate::ui::prepare_portal_ui();
            let fixture = tempfile::tempdir().expect("fixture");
            std::fs::create_dir(fixture.path().join("alpha")).expect("folder");
            std::fs::create_dir(fixture.path().join("beta")).expect("folder");
            std::fs::write(fixture.path().join("report.txt"), b"report").expect("file");
            std::fs::write(
                fixture
                    .path()
                    .join("a filename that wraps onto two lines.txt"),
                b"wrapped",
            )
            .expect("file");
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            let browser = view.browser();
            let root = view.widget();
            let window = gtk::Window::builder()
                .child(&root)
                .default_width(1000)
                .default_height(380)
                .build();
            window.present();
            browser.navigate(Location::local(fixture.path()));
            let deadline = Instant::now() + Duration::from_secs(5);
            while !browser
                .column_snapshot(0)
                .is_some_and(|snapshot| !snapshot.loading)
            {
                assert!(Instant::now() < deadline, "directory load");
                glib::MainContext::default().iteration(false);
            }
            view.set_view_mode(BrowserMode::List);
            settle();
            let list = by_class(&root, "file-list-mode");
            let row = by_class(&list, "list-row")
                .parent()
                .expect("list row wrapper");
            let bounds = row.compute_bounds(&root).expect("row bounds");
            let viewport = list
                .parent()
                .expect("scroller")
                .compute_bounds(&root)
                .expect("viewport bounds");
            let left = bounds.x() - viewport.x();
            let right = viewport.x() + viewport.width() - bounds.x() - bounds.width();
            let top = bounds.y() - viewport.y();
            assert!((left - right).abs() <= 1.0, "left {left} right {right}");
            assert!((top - left).abs() <= 1.0, "top {top} left {left}");
            let pane_header_height = by_class(&root, "mode-pane-header")
                .compute_bounds(&root)
                .expect("List toolbar bounds")
                .height();
            assert_eq!(pane_header_height, 41.0);
            let name = by_class(&root, "list-heading-button");
            let label = descendants(&name)
                .into_iter()
                .find(|widget| {
                    widget
                        .clone()
                        .downcast::<gtk::Label>()
                        .is_ok_and(|label| label.text() == "Name")
                })
                .expect("Name label");
            let icon = by_class(&list, "list-name-cell")
                .first_child()
                .expect("file icon");
            let row_content = by_class(&list, "list-row");
            let icon_in_row = icon.compute_bounds(&row_content).expect("icon inset");
            let bottom_inset = row_content.height() as f32 - icon_in_row.y() - icon_in_row.height();
            assert!(
                (icon_in_row.y() - icon_in_row.x()).abs() <= 1.0,
                "row vertical and horizontal content insets must match: {icon_in_row:?}"
            );
            assert!(
                (bottom_inset - icon_in_row.x()).abs() <= 1.0,
                "row bottom inset {bottom_inset}, left {}",
                icon_in_row.x()
            );
            let label_x = label.compute_bounds(&root).expect("label bounds").x();
            let icon_x = icon.compute_bounds(&root).expect("icon bounds").x();
            assert!(
                (label_x - icon_x).abs() <= 1.0,
                "Name {label_x}, icon {icon_x}"
            );
            capture(&window, "list");
            view.set_view_mode(BrowserMode::Columns);
            settle();
            let header = by_class(&root, "column-header");
            let actions = descendants(&header)
                .into_iter()
                .filter(|widget| widget.has_css_class("column-header-action") && widget.is_mapped())
                .collect::<Vec<_>>();
            let last = actions.last().expect("header action");
            let bounds = last.compute_bounds(&root).expect("action bounds");
            let header_bounds = header.compute_bounds(&root).expect("header bounds");
            assert_eq!(header_bounds.height(), pane_header_height);
            let heading = header
                .first_child()
                .expect("heading box")
                .first_child()
                .expect("folder name");
            let heading_center = heading
                .compute_bounds(&root)
                .expect("heading bounds")
                .center()
                .y();
            assert!(
                (heading_center - header_bounds.center().y()).abs() <= 1.0,
                "folder title must remain optically centered"
            );
            assert!(
                (heading_center - bounds.center().y()).abs() <= 1.0,
                "folder title and buttons must remain aligned"
            );
            let right = header_bounds.x() + header_bounds.width() - bounds.x() - bounds.width();
            assert!((right - 5.0).abs() <= 1.0, "header right inset {right}");
            let top = bounds.y() - header_bounds.y();
            let bottom = header_bounds.y() + header_bounds.height() - bounds.y() - bounds.height();
            assert!(
                (top - bottom).abs() <= 1.0,
                "header top {top}, bottom {bottom}"
            );
            for action in actions {
                assert_eq!(
                    action.cursor().and_then(|cursor| cursor.name()).as_deref(),
                    Some("pointer")
                );
            }
            let column = by_class(&root, "directory-column")
                .compute_bounds(&root)
                .expect("column bounds");
            let first_row = by_class(&root, "file-row")
                .parent()
                .expect("column row")
                .compute_bounds(&root)
                .expect("column row bounds");
            let left = first_row.x() - column.x();
            let right = column.x() + column.width() - first_row.x() - first_row.width();
            let top = first_row.y() - header_bounds.y() - header_bounds.height();
            assert!(
                (left - right).abs() <= 1.0,
                "Columns left {left} right {right}"
            );
            assert!((top - left).abs() <= 1.0, "Columns top {top} left {left}");
            capture(&window, "columns");
            for (density, expected_height) in
                [(BrowserDensity::Compact, 26), (BrowserDensity::Airy, 42)]
            {
                view.set_density(density);
                for (mode, class) in [
                    (BrowserMode::Columns, "file-row"),
                    (BrowserMode::List, "list-row"),
                ] {
                    view.set_view_mode(mode);
                    settle();
                    let row = by_class(&root, class).parent().expect("row wrapper");
                    assert_eq!(
                        row.height(),
                        expected_height,
                        "{density:?} {mode:?} row height"
                    );
                }
            }
            view.set_density(BrowserDensity::Compact);
            view.set_view_mode(BrowserMode::Icons);
            settle();
            capture(&window, "icons");
            let menu = by_class(&root, "icons-thumbnail-menu")
                .downcast::<gtk::MenuButton>()
                .expect("thumbnail menu");
            let popover = menu.popover().expect("thumbnail popover");
            let scale = descendants(popover.upcast_ref())
                .into_iter()
                .find_map(|widget| widget.downcast::<gtk::Scale>().ok())
                .expect("thumbnail scale");
            scale.set_value(32.0);
            settle();
            capture(&window, "icons-32");
            for card in descendants(&root)
                .into_iter()
                .filter(|widget| widget.has_css_class("icons-card") && widget.is_mapped())
            {
                assert!(
                    card.height() >= card.height_request(),
                    "GridView must retain the reserved card height"
                );
                let (icon, label) = crate::ui::icons_cell::parts(&card).expect("card parts");
                let top = icon.compute_bounds(&card).expect("icon bounds").y();
                let caption = label.compute_bounds(&card).expect("caption bounds");
                let bottom = card.height() as f32 - caption.y() - caption.height();
                assert!(
                    (top - bottom).abs() <= 1.0,
                    "card content top {top}, bottom {bottom}"
                );
            }
            view.set_view_mode(BrowserMode::List);
            view.set_view_mode(BrowserMode::Icons);
            settle();
            let slots = descendants(&root)
                .into_iter()
                .filter_map(|widget| {
                    widget
                        .downcast::<crate::ui::thumbnail::ThumbnailSlot>()
                        .ok()
                })
                .filter(|slot| slot.is_mapped())
                .collect::<Vec<_>>();
            assert!(!slots.is_empty());
            assert!(slots.iter().all(|slot| slot.slot_size() == 32));
            window.destroy();
            browser.clear_observer();
        },
    );
}

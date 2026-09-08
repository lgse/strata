// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::{Duration, Instant};

use super::*;
use crate::{
    model::{EntryKind, MetadataValue},
    services::{
        DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError,
        MetadataOutcome, MetadataRequest,
    },
    test_support::gtk_test,
};

struct Source {
    entries: RefCell<Vec<FileEntry>>,
    fills: RefCell<Vec<MetadataRequest>>,
}

impl FileSource for Source {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }
    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: self.entries.borrow().clone(),
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }
    fn supports_metadata_fill(&self, _: &Location) -> bool {
        true
    }
    fn fill_metadata(
        &self,
        request: MetadataRequest,
        emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        let id = request.id;
        self.fills.borrow_mut().push(request);
        emit(DirectoryEvent::MetadataFinished {
            request_id: id,
            outcome: MetadataOutcome::Unsupported,
        });
        LoadHandle::new(|| {})
    }
}

fn entry(name: &str, size: u64) -> FileEntry {
    FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(),
        display_name: name.into(),
        thumbnail_path: None,
        kind: EntryKind::File,
        size: MetadataValue::Known(size),
        modified_unix_seconds: MetadataValue::Known(1),
        mode: MetadataValue::Known(0o100644),
        is_hidden: false,
    }
}

struct Fixture {
    browser: Option<Rc<Browser>>,
    source: Rc<Source>,
    index: SourceIndexMap,
    columns: ListColumnLayout,
    cuts: Rc<RefCell<HashSet<Location>>>,
    scrolling: Rc<Cell<bool>>,
    items: Rc<RefCell<Vec<BoundModeItem>>>,
    factory: gtk::SignalListItemFactory,
    view: gtk::ListView,
    window: gtk::Window,
}

impl Fixture {
    fn new() -> Self {
        crate::ui::theme::ThemeManager::shared();
        thumbnail::hold_thumbnail_workers();
        let entries = vec![entry("a.txt", 100), entry("b.png", 200), entry("c.rs", 300)];
        let values: Vec<_> = entries.iter().map(browser::entry_model_value).collect();
        let source = Rc::new(Source {
            entries: RefCell::new(entries),
            fills: RefCell::new(Vec::new()),
        });
        let browser = Browser::new(source.clone());
        browser.navigate(Location::local("/fixture"));
        let model = gtk::StringList::new(&values.iter().map(String::as_str).collect::<Vec<_>>());
        let index = SourceIndexMap::watch(&model);
        let sorter = gtk::CustomSorter::new(|left, right| {
            super::super::model_value(right)
                .cmp(&super::super::model_value(left))
                .into()
        });
        let sorted = gtk::SortListModel::new(Some(model), Some(sorter));
        let selection = gtk::MultiSelection::new(Some(sorted.clone()));
        let columns = ListColumnLayout::new();
        let cuts = Rc::new(RefCell::new(HashSet::new()));
        let scrolling = Rc::new(Cell::new(false));
        let items = Rc::new(RefCell::new(Vec::new()));
        let factory = ListFactory {
            browser: Rc::downgrade(&browser),
            depth: 0,
            positions: PanePositions {
                index: index.clone(),
                view: sorted.upcast(),
            },
            selection: selection.clone(),
            previews: Rc::new(Cell::new(true)),
            activation: Rc::new(Cell::new(ClickActivation::default_for(
                super::super::BrowserMode::List,
            ))),
            transfers: Rc::new(RefCell::new(None)),
            cuts: cuts.clone(),
            columns: columns.clone(),
            scrolling: scrolling.clone(),
            bound_items: items.clone(),
        }
        .build();
        let view = gtk::ListView::new(Some(selection), Some(factory.clone()));
        let window = gtk::Window::builder()
            .default_width(800)
            .default_height(400)
            .child(&view)
            .build();
        window.present();
        let fixture = Self {
            browser: Some(browser),
            source,
            index,
            columns,
            cuts,
            scrolling,
            items,
            factory,
            view,
            window,
        };
        pump_until(|| fixture.item_at(0).is_some());
        fixture
    }

    fn item_at(&self, position: u32) -> Option<gtk::ListItem> {
        self.items
            .borrow()
            .iter()
            .filter_map(|bound| bound.item.upgrade())
            .find(|item| item.item().is_some() && item.position() == position)
    }

    fn first(&self) -> (gtk::ListItem, ListRow) {
        let item = self.item_at(0).expect("bound first item");
        let row = ListRow::from_widget(item.child().and_downcast::<gtk::Box>().expect("row"))
            .expect("row parts");
        (item, row)
    }

    fn bind(&self, item: &gtk::ListItem) {
        self.factory.emit_by_name::<()>("bind", &[item]);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.view.set_model(None::<&gtk::SelectionModel>);
        self.window.close();
        thumbnail::clear_thumbnail_runtime();
    }
}

fn pump_until(done: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "GTK state did not settle");
        context.iteration(false);
    }
}

fn assert_cleared(row: &ListRow) {
    assert!(row.name.get_visible());
    assert!(!row.field.get_visible());
    for label in [&row.name, &row.mode, &row.size, &row.kind, &row.modified] {
        assert_eq!(label.label(), "");
    }
}

#[test]
fn setup_and_binding_follow_source_positions_and_shared_column_widths() {
    gtk_test(
        "ui::browser_modes::list_factory::tests::setup_and_binding_follow_source_positions_and_shared_column_widths",
        || {
            let fixture = Fixture::new();
            let (item, row) = fixture.first();
            assert_eq!(
                fixture.index.of_item(&item.item().expect("model item")),
                Some(2)
            );
            let expected = &fixture.source.entries.borrow()[2];
            assert_eq!(row.name.label(), expected.display_name);
            assert_eq!(row.size.label(), entry_size(expected));
            assert_eq!(row.mode.label(), entry_mode(expected));
            assert_eq!(row.kind.label(), "File");
            assert_eq!(row.icon.slot_size(), 18);
            super::super::set_list_column_width(&fixture.columns, 0, 240);
            super::super::set_list_column_width(&fixture.columns, 2, 115);
            assert_eq!(row.name_cell.width_request(), 240);
            assert_eq!(row.size.width_request(), 115);
        },
    );
}

#[test]
fn scrolling_defers_details_and_settling_preserves_rename_state() {
    gtk_test(
        "ui::browser_modes::list_factory::tests::scrolling_defers_details_and_settling_preserves_rename_state",
        || {
            let fixture = Fixture::new();
            let (item, row) = fixture.first();
            row.widget.add_css_class("cut");
            row.modified.set_label("old date");
            row.name.set_visible(false);
            row.field.set_visible(true);
            fixture.scrolling.set(true);
            fixture.bind(&item);
            assert!(row.widget.has_css_class("cut"));
            assert_eq!(
                row.modified.label(),
                crate::util::modified_date(&fixture.source.entries.borrow()[2])
            );
            assert!(row.name.get_visible());
            assert!(!row.field.get_visible());
            row.name.set_visible(false);
            row.field.set_visible(true);
            row.field.set_text("unfinished rename");
            row.modified.set_label("old date");
            let section = PaneSection {
                view: fixture.view.clone().upcast(),
                view_model: fixture.view.model().expect("selection").upcast(),
                selection: fixture
                    .view
                    .model()
                    .expect("selection")
                    .downcast()
                    .expect("multi selection"),
                bound_items: fixture.items.clone(),
                syncing: Rc::new(Cell::new(false)),
                visit: super::super::bound_item_visitor(fixture.items.clone()),
            };
            refresh_list_section(
                fixture.browser.as_ref().expect("browser"),
                0,
                &fixture.index,
                &section,
                &fixture.cuts.borrow(),
            );
            assert!(!row.widget.has_css_class("cut"));
            assert_eq!(
                row.modified.label(),
                crate::util::modified_date(&fixture.source.entries.borrow()[2])
            );
            assert!(!row.name.get_visible());
            assert!(row.field.get_visible());
            assert_eq!(row.field.text(), "unfinished rename");
        },
    );
}

#[test]
fn missing_source_mapping_or_entry_clears_recycled_fields() {
    gtk_test(
        "ui::browser_modes::list_factory::tests::missing_source_mapping_or_entry_clears_recycled_fields",
        || {
            for missing_mapping in [true, false] {
                let fixture = Fixture::new();
                let (item, row) = fixture.first();
                row.name.set_visible(false);
                row.field.set_visible(true);
                if missing_mapping {
                    fixture.index.by_item.borrow_mut().clear();
                } else {
                    fixture.source.entries.borrow_mut().clear();
                    fixture.browser.as_ref().expect("browser").refresh_all();
                }
                fixture.bind(&item);
                assert_cleared(&row);
            }
        },
    );
}

#[test]
fn factory_does_not_keep_the_browser_alive() {
    gtk_test(
        "ui::browser_modes::list_factory::tests::factory_does_not_keep_the_browser_alive",
        || {
            let mut fixture = Fixture::new();
            let (item, row) = fixture.first();
            let weak = Rc::downgrade(fixture.browser.as_ref().expect("browser"));
            fixture.browser.take();
            assert!(weak.upgrade().is_none());
            fixture.bind(&item);
            assert_cleared(&row);
        },
    );
}

#[test]
fn permissions_fill_is_deferred_during_scroll_and_uses_the_source_location() {
    gtk_test(
        "ui::browser_modes::list_factory::tests::permissions_fill_is_deferred_during_scroll_and_uses_the_source_location",
        || {
            let fixture = Fixture::new();
            let (item, _) = fixture.first();
            fixture.source.entries.borrow_mut()[2].mode = MetadataValue::Unknown;
            fixture.browser.as_ref().expect("browser").refresh_all();
            fixture.scrolling.set(true);
            fixture.bind(&item);
            assert!(fixture.source.fills.borrow().is_empty());
            fixture.scrolling.set(false);
            fixture.bind(&item);
            pump_until(|| !fixture.source.fills.borrow().is_empty());
            let fills = fixture.source.fills.borrow();
            assert_eq!(fills.len(), 1);
            assert_eq!(fills[0].entries, vec![Location::local("/fixture/c.rs")]);
            assert!(!fills[0].full);
        },
    );
}

#[test]
fn unbind_cancels_pending_thumbnail_work() {
    gtk_test(
        "ui::browser_modes::list_factory::tests::unbind_cancels_pending_thumbnail_work",
        || {
            let fixture = Fixture::new();
            let path = std::path::Path::new("/fixture/b.png");
            pump_until(|| fixture.item_at(1).is_some());
            let item = fixture.item_at(1).expect("image row");
            fixture.bind(&item);
            pump_until(|| thumbnail::has_pending_thumbnail(path));
            fixture.factory.emit_by_name::<()>("unbind", &[&item]);
            assert!(!thumbnail::has_pending_thumbnail(path));
        },
    );
}

#[test]
fn appearance_animation_is_suppressed_while_scrolling() {
    gtk_test(
        "ui::browser_modes::list_factory::tests::appearance_animation_is_suppressed_while_scrolling",
        || {
            let fixture = Fixture::new();
            for scrolling in [false, true] {
                fixture.scrolling.set(scrolling);
                let item: gtk::ListItem = glib::Object::new();
                fixture.factory.emit_by_name::<()>("setup", &[&item]);
                let row = item.child().expect("row");
                assert_eq!(row.has_css_class("file-appear"), !scrolling);
                pump_until(|| !row.has_css_class("file-appear"));
            }
        },
    );
}

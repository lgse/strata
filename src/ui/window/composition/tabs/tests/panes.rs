// SPDX-License-Identifier: MIT

use gdk::{Key, ModifierType as M};

use super::*;
use crate::ui::browser_modes::BrowserMode::{Icons, List};
use crate::ui::window::sidebar::PlaceNavigation;

mod terminal;

struct Split {
    root: tempfile::TempDir,
    window: gtk::ApplicationWindow,
    tabs: Rc<TabWindow>,
}

impl Split {
    /// A window whose tab shows `left` in the main pane, with `notes.txt`
    /// and `draft.txt` in it and an empty `right` beside it.
    fn open() -> Self {
        let root = tempfile::tempdir().expect("pane fixture");
        for folder in ["left", "right"] {
            std::fs::create_dir_all(root.path().join(folder)).expect("pane folder");
        }
        for file in ["notes.txt", "draft.txt"] {
            std::fs::write(root.path().join("left").join(file), file).expect("pane file");
        }
        let (window, tabs) = super::open();
        tabs.preferences.set_browser_mode(List);
        let split = Self { root, window, tabs };
        load(&split.tabs.active_browser(), split.folder("left"));
        split
    }

    fn folder(&self, name: &str) -> Location {
        Location::local(self.root.path().join(name))
    }

    fn content(&self) -> Rc<WindowContent> {
        self.tabs.active_tab().content.clone()
    }

    fn main(&self) -> BrowserView {
        self.content().browser.clone()
    }

    fn active(&self) -> BrowserView {
        self.content().active_view()
    }

    fn key(&self, key: Key, modifiers: M) -> glib::Propagation {
        self.tabs.handle_key(&press(key, modifiers))
    }

    fn title(&self) -> Option<String> {
        self.window.title().map(|title| title.to_string())
    }

    /// Keys reach the active pane's listing, as the dispatcher it gates expects.
    fn await_focus_in(&self, view: &BrowserView) {
        let widget = view.widget();
        wait_until_or(
            || {
                gtk::prelude::RootExt::focus(&self.window)
                    .is_some_and(|focus| focus == widget || focus.is_ancestor(&widget))
            },
            || "keyboard focus stayed outside the active pane".to_owned(),
        );
    }

    fn settle_at(view: &BrowserView, location: &Location) {
        wait_until_or(
            || {
                view.browser().active_location().as_ref() == Some(location)
                    && view
                        .browser()
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading)
            },
            || format!("pane never settled at {}", location.diagnostic_path()),
        );
    }
}

#[test]
fn the_extra_pane_opens_on_the_active_folder_and_closing_keeps_the_active_one() {
    gtk_test(
        "ui::window::composition::tabs::tests::panes::the_extra_pane_opens_on_the_active_folder_and_closing_keeps_the_active_one",
        || {
            let split = Split::open();
            let (left, right) = (split.folder("left"), split.folder("right"));
            for key in [Key::F3, Key::F6] {
                assert!(is_tab_shortcut(&press(key, M::empty())), "{key:?}");
            }
            assert!(!split.content().is_split());
            split.content().header.extra_pane.emit_clicked();
            assert!(split.content().is_split());
            let extra = split.active();
            assert!(!extra.ptr_eq(&split.main()), "the new pane is active");
            Split::settle_at(&extra, &left);
            split.await_focus_in(&extra);
            assert_eq!(extra.view_mode(), split.main().view_mode());
            crate::ui::window::apply_browser_mode(&extra, &split.tabs.preferences, Icons);
            assert_eq!(extra.view_mode(), Icons, "the active pane switches");
            assert_eq!(
                split.main().view_mode(),
                List,
                "the other pane keeps its view"
            );
            assert_eq!(
                split.tabs.preferences.browser_mode(),
                List,
                "nor is it saved"
            );

            extra.navigate_location(right.clone());
            Split::settle_at(&extra, &right);
            wait_until(|| split.title().as_deref() == Some("right — Strata"));
            assert_eq!(split.main().browser().active_location(), Some(left.clone()));

            split.key(Key::F6, M::empty());
            assert!(split.active().ptr_eq(&split.main()));
            split.await_focus_in(&split.main());
            wait_until(|| split.title().as_deref() == Some("left — Strata"));
            split.key(Key::F6, M::empty());
            assert!(split.active().ptr_eq(&extra));
            split.main().browser().focus_active();
            wait_until(|| split.active().ptr_eq(&split.main()));
            wait_until(|| split.title().as_deref() == Some("left — Strata"));
            extra.browser().focus_active();
            wait_until(|| split.active().ptr_eq(&extra));

            split.key(Key::F3, M::empty());
            assert!(!split.content().is_split());
            assert!(!split.content().header.extra_pane.is_active());
            assert!(split.active().ptr_eq(&split.main()));
            Split::settle_at(&split.main(), &right);
            split.await_focus_in(&split.main());
            assert_eq!(
                split.main().view_mode(),
                Icons,
                "the main pane takes the closed one's view"
            );
            wait_until(|| split.title().as_deref() == Some("right — Strata"));

            split.key(Key::F3, M::empty());
            Split::settle_at(&extra, &right);
            split.key(Key::F6, M::empty());
            split.main().navigate_location(left.clone());
            Split::settle_at(&split.main(), &left);
            split.key(Key::F3, M::empty());
            assert!(!split.content().is_split());
            Split::settle_at(&split.main(), &left);
            split.window.destroy();
        },
    );
}

#[test]
fn sidebar_places_open_in_the_active_pane() {
    gtk_test(
        "ui::window::composition::tabs::tests::panes::sidebar_places_open_in_the_active_pane",
        || {
            let split = Split::open();
            let (left, right) = (split.folder("left"), split.folder("right"));
            let place = |location: &Location| {
                let row = gtk::Button::new();
                split.content().sidebar.state.bind_place_row(
                    &row,
                    location.clone(),
                    PlaceNavigation::Direct,
                );
                row
            };
            split.key(Key::F3, M::empty());
            let extra = split.active();
            Split::settle_at(&extra, &left);

            place(&right).emit_clicked();
            Split::settle_at(&extra, &right);
            assert_eq!(split.main().browser().active_location(), Some(left.clone()));

            split.key(Key::F6, M::empty());
            let root = Location::local(split.root.path());
            place(&root).emit_clicked();
            Split::settle_at(&split.main(), &root);
            assert_eq!(extra.browser().active_location(), Some(right));
            split.window.destroy();
        },
    );
}

#[test]
fn copy_and_move_to_the_other_pane_take_the_active_selection() {
    gtk_test(
        "ui::window::composition::tabs::tests::panes::copy_and_move_to_the_other_pane_take_the_active_selection",
        || {
            let split = Split::open();
            let (left, right) = (split.folder("left"), split.folder("right"));
            let (source, destination) = (
                split.root.path().join("left"),
                split.root.path().join("right"),
            );
            let ctrl_shift = M::CONTROL_MASK | M::SHIFT_MASK;
            split
                .main()
                .browser()
                .select_entries_by_name(&["notes.txt".into()]);
            assert_eq!(split.key(Key::C, ctrl_shift), glib::Propagation::Stop);
            assert_eq!(
                split.content().footer().feedback_text(),
                crate::i18n::tr("Open the other pane first")
            );

            split.key(Key::F3, M::empty());
            let extra = split.active();
            extra.navigate_location(right.clone());
            Split::settle_at(&extra, &right);
            split.key(Key::F6, M::empty());
            let main = split.main();
            Split::settle_at(&main, &left);

            main.browser().select_entries_by_name(&["notes.txt".into()]);
            split.key(Key::C, ctrl_shift);
            wait_until(|| destination.join("notes.txt").exists());
            assert!(
                source.join("notes.txt").exists(),
                "copying keeps the source"
            );

            main.browser().select_entries_by_name(&["draft.txt".into()]);
            split.key(Key::X, ctrl_shift);
            wait_until(|| {
                destination.join("draft.txt").exists() && !source.join("draft.txt").exists()
            });
            let listed = |view: &BrowserView| {
                view.browser()
                    .column_snapshot(0)
                    .map_or(0, |column| column.count)
            };
            wait_until_or(
                || listed(&extra) == 2 && listed(&main) == 1,
                || "both panes should list what the transfers changed".to_owned(),
            );
            split.window.destroy();
        },
    );
}

#[test]
fn both_location_bars_navigate_their_own_pane() {
    gtk_test(
        "ui::window::composition::tabs::tests::panes::both_location_bars_navigate_their_own_pane",
        || {
            let split = Split::open();
            split.key(Key::F3, M::empty());
            let extra = split.active();
            let main = split.main();
            Split::settle_at(&extra, &split.folder("left"));
            for (view, target, other, unchanged) in [
                (&main, split.folder("right"), &extra, split.folder("left")),
                (
                    &extra,
                    Location::local(split.root.path()),
                    &main,
                    split.folder("right"),
                ),
            ] {
                view.begin_location_edit();
                wait_until(|| view.location_has_focus());
                let field = location_field(&view.location_widget()).expect("location entry");
                field.set_text(&target.display_path());
                field.emit_activate();
                Split::settle_at(view, &target);
                assert_eq!(other.browser().active_location(), Some(unchanged));
                wait_until(|| split.active().ptr_eq(view));
            }
            split.key(Key::F3, M::empty());
            main.begin_location_edit();
            wait_until(|| main.location_has_focus());
            split.window.destroy();
        },
    );
}

fn location_field(widget: &gtk::Widget) -> Option<gtk::Entry> {
    if let Some(entry) = widget.downcast_ref::<gtk::Entry>()
        && entry.has_css_class("location-entry")
    {
        return Some(entry.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(entry) = location_field(&widget) {
            return Some(entry);
        }
        child = widget.next_sibling();
    }
    None
}

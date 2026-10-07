// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    adapters::LocalFileSource,
    test_support::gtk_test,
    ui::{browser::BrowserView, preferences::PreferenceManager},
};

struct Fixture {
    views: [BrowserView; 2],
    windows: [gtk::Window; 2],
    preferences: Rc<PreferenceManager>,
    directory: tempfile::TempDir,
    motion: Cell<u32>,
}

impl Fixture {
    fn new() -> Self {
        let settings = glib::user_config_dir().join("strata/settings.toml");
        std::fs::create_dir_all(settings.parent().expect("settings directory"))
            .expect("create settings directory");
        std::fs::write(
            &settings,
            "folder_peeking = true\nbrowser_mode = \"icons\"\ncolumns_mirror_selection = false\n",
        )
        .expect("saved browsing preferences");
        let preferences = PreferenceManager::shared();
        let directory = tempfile::tempdir().expect("peek fixture");
        std::fs::create_dir(directory.path().join("child")).expect("child folder");
        std::fs::write(directory.path().join("child/note.txt"), "note").expect("peek entry");
        let views = std::array::from_fn(|_| {
            BrowserView::new(
                Rc::new(LocalFileSource),
                PeekBehavior {
                    open_delay: Duration::from_millis(10),
                    close_delay: Duration::ZERO,
                    fade_duration: Duration::ZERO,
                    ..PeekBehavior::default()
                },
            )
        });
        let windows = std::array::from_fn(|index| {
            let window = gtk::Window::builder()
                .default_width(900)
                .default_height(650)
                .child(&views[index].widget())
                .build();
            window.present();
            views[index].navigate_location(Location::local(directory.path()));
            window
        });
        wait_until(|| {
            views.iter().all(|view| {
                view.browser()
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
            })
        });
        settle();
        Self {
            views,
            windows,
            preferences,
            directory,
            motion: Cell::new(0),
        }
    }

    fn hover(&self, index: usize) {
        let view = &self.views[index];
        view.state.cancel_peek();
        let motion = self.motion.get() + 1;
        self.motion.set(motion);
        view.record_pointer_hover((f64::from(motion), 0.0), Some(0));
        let anchor = view.widget();
        anchor.set_state_flags(gtk::StateFlags::PRELIGHT, false);
        view.state.schedule_peek(
            0,
            Location::local(self.directory.path().join("child")),
            anchor,
        );
    }

    fn assert_peek(&self, index: usize, shown: bool) {
        let location = self.views[index]
            .state
            .peek
            .borrow()
            .as_ref()
            .map(|peek| peek.location.clone());
        assert_eq!(
            location,
            shown.then(|| Location::local(self.directory.path().join("child")))
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for (view, window) in self.views.iter().zip(&self.windows) {
            view.state.cancel_peek();
            view.browser().bump_navigation_generation();
            view.browser().clear_observer();
            self.preferences.release_bindings_within(&view.widget());
            window.destroy();
        }
    }
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "peek state did not settle"
        );
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn settle() {
    let ready = Rc::new(Cell::new(false));
    let finished = ready.clone();
    glib::timeout_add_local_once(Duration::from_millis(40), move || finished.set(true));
    wait_until(|| ready.get());
}

#[test]
fn columns_ignore_saved_and_live_peeking_while_icons_and_list_follow_it() {
    gtk_test(
        "ui::browser::peek::tests::columns_ignore_saved_and_live_peeking_while_icons_and_list_follow_it",
        || {
            let fixture = Fixture::new();
            for index in 0..2 {
                fixture.hover(index);
            }
            settle();
            for index in 0..2 {
                fixture.assert_peek(index, true);
            }
            for enabled in [false, true] {
                fixture.preferences.set_folder_peeking(enabled);
                if !enabled {
                    for index in 0..2 {
                        fixture.assert_peek(index, false);
                    }
                }
                for mode in [
                    BrowserMode::Columns,
                    BrowserMode::List,
                    BrowserMode::Icons,
                    BrowserMode::Columns,
                ] {
                    for (index, view) in fixture.views.iter().enumerate() {
                        view.set_view_mode(mode);
                        fixture.hover(index);
                    }
                    settle();
                    for index in 0..2 {
                        fixture.assert_peek(index, enabled && mode != BrowserMode::Columns);
                    }
                }
            }
        },
    );
}

#[test]
fn switching_to_columns_cancels_pending_and_open_peeks() {
    gtk_test(
        "ui::browser::peek::tests::switching_to_columns_cancels_pending_and_open_peeks",
        || {
            let fixture = Fixture::new();
            fixture.hover(0);
            fixture.views[0].set_view_mode(BrowserMode::Columns);
            fixture.hover(1);
            settle();
            fixture.assert_peek(0, false);
            fixture.assert_peek(1, true);
            fixture.views[1].set_view_mode(BrowserMode::Columns);
            fixture.assert_peek(1, false);
            settle();
            fixture.assert_peek(0, false);
            fixture.assert_peek(1, false);
            fixture.views[0].set_view_mode(BrowserMode::Icons);
            fixture.hover(0);
            settle();
            fixture.assert_peek(0, true);
        },
    );
}

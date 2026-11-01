// SPDX-License-Identifier: MIT

//! Opening browser windows and their first folder.

use std::{
    cell::{Cell, RefCell},
    env,
    path::PathBuf,
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::{
        LocalFileSource, LocalOperationProvider, RevealRequest, location_for_file, mount_lanes,
        probes,
    },
    app::Browser,
    model::Location,
    services::LocationValidationError,
    ui::{
        browser::{BrowserView, PeekBehavior},
        preferences::PreferenceManager,
    },
};

use super::{SidebarView, composition};

pub fn present(application: &gtk::Application) {
    present_target(application, None, Vec::new(), false, true, true);
}

/// Opens the requested directory with the named items selected.
pub fn present_reveal(application: &gtk::Application, request: RevealRequest) {
    present_target(
        application,
        Some(request.directory),
        request.selection,
        request.properties,
        true,
        false,
    );
}

pub(super) fn bind_update_notice_preferences(
    anchor: &impl IsA<gtk::Widget>,
    manager: &PreferenceManager,
    notice: &crate::ui::settings::UpdateNoticeHandler,
) {
    let initial = Cell::new(true);
    let notice = Rc::downgrade(notice);
    manager.bind_preference(
        anchor,
        |manager| (manager.checks_for_updates(), manager.release_channel()),
        move |_, _| {
            if !initial.replace(false)
                && let Some(notice) = notice.upgrade()
            {
                crate::ui::settings::clear_cached_update_notice();
                notice(None);
            }
        },
    );
}

pub(super) fn browser_for_window() -> BrowserView {
    let browser = BrowserView::new(Rc::new(LocalFileSource), PeekBehavior::default());
    browser.set_operation_provider(Rc::new(LocalOperationProvider));
    browser
}

pub(in crate::ui) fn present_target(
    application: &gtk::Application,
    location: Option<Location>,
    selection: Vec<Location>,
    properties: bool,
    auto_navigate: bool,
    restore_tabs: bool,
) -> BrowserView {
    let present_started = std::time::Instant::now();
    crate::assets::register_icon_theme();
    let _theme_manager = crate::ui::theme::ThemeManager::shared();
    let preference_manager = PreferenceManager::shared();
    crate::ui::actions::shared();
    tracing::debug!(
        elapsed_ms = present_started.elapsed().as_millis() as u64,
        "present theme and styles ready"
    );

    let window = gtk::ApplicationWindow::builder()
        .application(application)
        .title("Strata")
        .default_width(1200)
        .default_height(760)
        .build();

    let tabs = composition::TabWindow::new(&window, &preference_manager, restore_tabs);
    let browser = tabs.active_browser();
    window.present();
    crate::metrics::mark_window_presented();
    if auto_navigate {
        let first_folder = FirstFolder {
            restore: restore_tabs && location.is_none(),
            location,
            selection,
            properties,
            preference_manager,
        };
        let idle_browser = browser.clone();
        glib::idle_add_local_once(move || {
            let started = std::time::Instant::now();
            first_folder.open(&tabs, &idle_browser, started);
            tracing::debug!(
                elapsed_ms = started.elapsed().as_millis() as u64,
                "present navigation started"
            );
        });
    }
    browser
}

/// What a newly presented window shows once it is on screen.
struct FirstFolder {
    restore: bool,
    location: Option<Location>,
    selection: Vec<Location>,
    properties: bool,
    preference_manager: Rc<PreferenceManager>,
}

impl FirstFolder {
    fn open(
        self,
        tabs: &Rc<composition::TabWindow>,
        browser: &BrowserView,
        started: std::time::Instant,
    ) {
        if self.restore && tabs.try_restore() {
            tracing::debug!(
                elapsed_ms = started.elapsed().as_millis() as u64,
                "present tab session restored"
            );
            return;
        }
        match self.location {
            Some(location) if !self.selection.is_empty() => {
                browser.reveal_locations(location, self.selection, self.properties);
            }
            Some(location) => browser.navigate_location(location),
            None => open_default_folder(&browser.browser(), &self.preference_manager),
        }
    }
}

pub(super) fn schedule_after_first_paint(
    window: &gtk::ApplicationWindow,
    sidebar: &SidebarView,
    manager: &Rc<PreferenceManager>,
) {
    let state = Rc::downgrade(&sidebar.state);
    let manager = manager.clone();
    let armed = Cell::new(false);
    window.connect_map(move |window| {
        if armed.get() {
            return;
        }
        let Some(clock) = window.frame_clock() else {
            return;
        };
        armed.set(true);
        let handler = Rc::new(RefCell::new(None));
        let handler_for_paint = handler.clone();
        let state = state.clone();
        let manager = manager.clone();
        let id = clock.connect_after_paint(move |clock| {
            if let Some(id) = handler_for_paint.borrow_mut().take() {
                clock.disconnect(id);
            }
            crate::metrics::mark_first_themed_frame();
            let state = state.clone();
            glib::idle_add_local_once(move || {
                if let Some(state) = state.upgrade() {
                    state.rebuild();
                }
            });
            let manager = manager.clone();
            glib::idle_add_local_once(move || {
                crate::ui::settings::maybe_run_due_update_check(&manager);
            });
        });
        handler.replace(Some(id));
    });
}

pub(super) fn navigate_to_gio_file(browser: &Rc<Browser>, file: &gio::File) {
    if let Some(location) = location_for_file(file) {
        browser.navigate(location);
    }
}

pub(crate) fn home_directory() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub(crate) fn default_save_folder() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Downloads).unwrap_or_else(home_directory)
}

pub(super) fn open_checked(browser: &Rc<Browser>, location: Location, absent: impl FnOnce() + 'static) {
    let Some(path) = location.native_path().map(PathBuf::from) else {
        browser.navigate_location(location, true);
        return;
    };
    let generation = browser.bump_navigation_generation();
    let weak = Rc::downgrade(browser);
    glib::MainContext::default().spawn_local(async move {
        let gone = glib::future_with_timeout(mount_lanes::STALLED_AFTER, async {
            match probes::folder(&path).await {
                Err(LocationValidationError::NotDirectory) => true,
                Err(LocationValidationError::Missing) => probes::folder_was_deleted(&path).await,
                _ => false,
            }
        })
        .await
        .unwrap_or(false);
        let Some(browser) = weak.upgrade() else {
            return;
        };
        if browser.navigation_generation() != generation {
            return;
        }
        if gone {
            absent();
        } else {
            browser.navigate_location(location, true);
        }
    });
}

pub(super) fn open_default_folder(browser: &Rc<Browser>, manager: &Rc<PreferenceManager>) {
    let home = Location::local(home_directory());
    let Some(saved) = manager.default_directory() else {
        browser.navigate_location(home, true);
        return;
    };
    let weak = Rc::downgrade(browser);
    let manager = manager.clone();
    open_checked(browser, Location::local(&saved), move || {
        if manager.default_directory().as_ref() == Some(&saved) {
            manager.set_default_directory(None);
        }
        if let Some(browser) = weak.upgrade() {
            browser.navigate_location(home, true);
        }
    });
}

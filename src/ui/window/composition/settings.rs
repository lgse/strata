// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

use gtk::{glib, prelude::*};

use crate::{
    model::Location,
    services::{
        BuildKind, InstallRequest, InstallSource, ManagedInstall, ReleaseMetadata, UpdateMethod,
    },
    ui::{
        blur::BlurBin,
        browser::WeakPaneTarget,
        keys::{self, bindings::SETTINGS},
        preferences::PreferenceManager,
        settings::{self, InstallGuard, UpdateNoticeHandler},
    },
};

use super::{
    super::{SidebarView, bind_update_notice_preferences},
    WindowContent,
};

#[cfg(test)]
mod tests;

type AvailableUpdate = Rc<RefCell<Option<(ReleaseMetadata, InstallRequest, UpdateMethod)>>>;

thread_local! {
    static LAYER_PREPARATION_CLAIMED: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn install(
    window: &gtk::ApplicationWindow,
    content: &WindowContent,
    preferences: &Rc<PreferenceManager>,
) -> UpdateNoticeHandler {
    // The same process-wide guard covers this window's notice, lazy Settings
    // layer, and every other window's update and rollback controls.
    let guard = settings::install_guard();
    let notice = bind_update_notice(window, &content.sidebar, &guard);
    settings::register_update_notice(&notice);
    bind_update_notice_preferences(&content.overlay, preferences, &notice);
    let launcher = Rc::new(SettingsLauncher {
        layer: RefCell::new(None),
        focus_restore: RefCell::new(None),
        target: content.target().downgrade(),
        button: content.header.settings.clone(),
        blurred_root: content.blurred_root.clone(),
        overlay: content.overlay.clone(),
        preferences: preferences.clone(),
        notice: notice.clone(),
        guard,
        cancel_chords: {
            let panes = Rc::downgrade(&content.panes);
            Rc::new(move || {
                if let Some(panes) = panes.upgrade() {
                    panes.cancel_chords();
                }
            })
        },
    });
    prepare_after_first_paint(&launcher);
    let clicked_settings = Rc::downgrade(&launcher);
    content.header.settings.connect_clicked(move |_| {
        if let Some(launcher) = clicked_settings.upgrade() {
            launcher.show();
        }
    });
    let shortcut = gtk::EventControllerKey::new();
    let active_root = content.overlay.downgrade();
    keys::on_press(&shortcut, move |_, press| {
        if !active_root.upgrade().is_some_and(|root| root.is_mapped()) || !press.is(SETTINGS) {
            return glib::Propagation::Proceed;
        }
        launcher.show();
        glib::Propagation::Stop
    });
    window.add_controller(shortcut.clone());
    let controller: gtk::EventController = shortcut.upcast();
    super::super::keyboard::release_controllers_on_close(
        window.upcast_ref(),
        &content.overlay,
        std::slice::from_ref(&controller),
    );
    content.key_controllers.borrow_mut().push(controller);
    notice
}

/// Most of the first open's cost is paid once per process, so only the first tab shown
/// prepares its layer: later tabs still open quickly and do not each keep a copy of Settings.
fn prepare_after_first_paint(launcher: &Rc<SettingsLauncher>) {
    let launcher_for_map = Rc::downgrade(launcher);
    launcher.overlay.connect_map(move |overlay| {
        if LAYER_PREPARATION_CLAIMED.get() {
            return;
        }
        let Some(clock) = overlay.frame_clock() else {
            return;
        };
        LAYER_PREPARATION_CLAIMED.set(true);
        let launcher = launcher_for_map.clone();
        let handler = Rc::new(RefCell::new(None));
        let handler_for_paint = handler.clone();
        let id = clock.connect_after_paint(move |clock| {
            if let Some(id) = handler_for_paint.borrow_mut().take() {
                clock.disconnect(id);
            }
            let launcher = launcher.clone();
            glib::idle_add_local_once(move || {
                if let Some(launcher) = launcher.upgrade() {
                    launcher.prepare();
                }
            });
        });
        handler.replace(Some(id));
    });
}

struct SettingsLauncher {
    layer: RefCell<Option<settings::SettingsLayer>>,
    focus_restore: RefCell<Option<Rc<Cell<bool>>>>,
    target: WeakPaneTarget,
    button: gtk::Button,
    blurred_root: BlurBin,
    overlay: gtk::Overlay,
    preferences: Rc<PreferenceManager>,
    notice: UpdateNoticeHandler,
    guard: InstallGuard,
    /// Drops a chord armed in any of the tab's pane footers.
    cancel_chords: Rc<dyn Fn()>,
}

impl SettingsLauncher {
    fn layer(&self) -> settings::SettingsLayer {
        if let Some(layer) = self.layer.borrow().clone() {
            return layer;
        }
        let target = self.target.clone();
        let open_folder: settings::OpenFolder = Rc::new(move |path: PathBuf| {
            if let Some(target) = target.upgrade() {
                target.view().navigate_location(Location::local(path));
            }
        });
        let layer = settings::build_layer(
            &self.button,
            &self.blurred_root,
            self.preferences.clone(),
            self.notice.clone(),
            self.guard.clone(),
            open_folder,
        );
        self.overlay.add_overlay(&layer.widget);
        self.focus_restore
            .replace(Some(crate::ui::modal::remember_persistent_modal_focus(
                layer.widget.upcast_ref(),
            )));
        self.layer.borrow_mut().replace(layer.clone());
        layer
    }

    /// Separate idle callbacks let input and frames through between the two blocking steps.
    fn prepare(&self) {
        if self.layer.borrow().is_some() {
            return;
        }
        let layer = self.layer().widget.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(layer) = layer.upgrade() {
                settings::prepare_layout(&layer);
            }
        });
    }

    fn show(&self) {
        if self
            .overlay
            .root()
            .and_downcast::<gtk::Window>()
            .is_some_and(|window| super::super::visible_modal_layer(&window).is_some())
        {
            return;
        }
        (self.cancel_chords)();
        // Closing Settings returns focus to the file list, not to the gear or other chrome
        // that opened it; only a focused filter session gets its field or results back.
        let filter_had_focus = self
            .target
            .upgrade()
            .is_some_and(|target| target.view().filter_focus().is_some());
        let layer = self.layer();
        self.blurred_root.set_blurred(true);
        layer.widget.set_visible(true);
        if let Some(restore_origin) = self.focus_restore.borrow().as_ref() {
            restore_origin.set(filter_had_focus);
        }
        layer.focus_current_page();
        self.button.add_css_class("active");
    }
}

fn bind_update_notice(
    window: &gtk::ApplicationWindow,
    sidebar: &SidebarView,
    guard: &InstallGuard,
) -> UpdateNoticeHandler {
    let available: AvailableUpdate = Rc::new(RefCell::new(None));
    let available_for_click = available.clone();
    let parent = window.upcast_ref::<gtk::Window>().downgrade();
    let guard = guard.clone();
    sidebar.update_notice.connect_clicked(move |_| {
        let Some((release, install, update_method)) = available_for_click.borrow().clone() else {
            return;
        };
        let Some(parent) = parent.upgrade() else {
            return;
        };
        settings::show_update_dialog(&parent, &release, install, guard.clone(), update_method);
    });
    notice_handler(sidebar, available)
}

fn notice_handler(sidebar: &SidebarView, available: AvailableUpdate) -> UpdateNoticeHandler {
    let button = sidebar.update_notice.clone();
    let label = sidebar.update_label.clone();
    let version = sidebar.update_version.clone();
    let area = sidebar.update_area.clone();
    Rc::new(move |release| {
        if let Some((release, install, update_method)) = release {
            crate::ui::accessibility::set_description(
                &button,
                Some(&update_tooltip(&release, update_method)),
            );
            label.set_text(&crate::i18n::tr("Update available"));
            version.set_text(&format!("v{}", release.version));
            if release.kind == BuildKind::Stable {
                button.remove_css_class("preview");
            } else {
                button.add_css_class("preview");
            }
            *available.borrow_mut() = Some((release, install, update_method));
            area.set_visible(true);
        } else {
            available.borrow_mut().take();
            area.set_visible(false);
        }
    })
}

fn update_tooltip(release: &ReleaseMetadata, method: UpdateMethod) -> String {
    match method {
        UpdateMethod::InPlace => {
            rust_i18n::t!("Install Strata v%{value1}", value1 = release.version).into_owned()
        }
        UpdateMethod::Aur => rust_i18n::t!(
            "Strata v%{value1} is available through %{value2}",
            value1 = release.version,
            value2 = InstallSource::detect().managed().map_or_else(
                || crate::i18n::tr("your package manager"),
                ManagedInstall::manager
            )
        )
        .into_owned(),
        UpdateMethod::Omarchy => rust_i18n::t!(
            "Strata v%{value1} is available through Omarchy",
            value1 = release.version
        )
        .into_owned(),
        UpdateMethod::Pacman => rust_i18n::t!(
            "Strata v%{value1} is available through pacman",
            value1 = release.version
        )
        .into_owned(),
    }
}

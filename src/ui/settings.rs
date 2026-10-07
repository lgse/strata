// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    process::Command,
    rc::{Rc, Weak},
    sync::{OnceLock, mpsc::TryRecvError},
    time::{Duration, Instant},
};

use gtk::{gdk, gio, glib, prelude::*, subclass::prelude::*};

use crate::{
    assets::icons,
    services::{
        self, BuildKind, Channel, DocumentBlock, InstallCancel, InstallRequest, InstallSource,
        ManagedInstall, ReleaseMetadata, ReleaseNotes, UpdateCheck, UpdateInstall, UpdateMethod,
        Version,
    },
};

#[cfg(test)]
mod tests;

mod about;
mod actions;
mod bindings;
mod exclusions;
mod general;
mod search;
mod theme;
mod wrap;
use about::about_page;
use bindings::bind_switch;
use general::general_page;
use theme::theme_page;

use super::{
    blur::BlurBin, browser::dismiss_modal_layer, controls::modal_layout,
    modal::modal_layer_with_backdrop, preferences::PreferenceManager, terminal,
};

type AvailableUpdate = (ReleaseMetadata, InstallRequest, UpdateMethod);
type CachedUpdate = Option<AvailableUpdate>;
pub(super) type UpdateNoticeHandler = Rc<dyn Fn(CachedUpdate)>;
type WeakUpdateNoticeHandler = Weak<dyn Fn(CachedUpdate)>;

struct UpdateCheckRow {
    row: gtk::Box,
    run_check: Rc<dyn Fn(bool)>,
    responsive_action: (gtk::Box, gtk::Button),
    install_underway: Rc<dyn Fn() -> bool>,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "tests read the row's status text")
    )]
    status: gtk::Label,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "tests seed and inspect the cached install offer")
    )]
    pending_download: Rc<RefCell<Option<PendingInstall>>>,
}

struct ResponsiveContent {
    flows: Vec<(gtk::FlowBox, u32)>,
    actions: Vec<(gtk::Box, gtk::Button)>,
    setting_rows: Vec<gtk::Box>,
    activation_rows: Vec<ResponsiveActivationRow>,
}

pub struct ResponsiveActivationRow {
    row: gtk::Box,
    options: Vec<gtk::Box>,
}

/// Shared "an install is running" guard across the update row and update
/// dialog, the two places [`services::install_update`] is called. Without one
/// process-wide guard, separate windows could replace the executable at the
/// same time. See [`start_install`].
pub(super) type InstallGuard = Rc<Cell<bool>>;

const RESTART_GRACE_SECONDS: u64 = 20;
// Must outlast the restart waiter's recovery window.
const ROLLBACK_RETENTION: Duration = Duration::from_secs(60);

pub(crate) fn schedule_rollback_cleanup() {
    let Ok(current_exe) = services::installed_executable() else {
        return;
    };
    let Some(rollback) = current_exe.parent().map(services::rollback_path) else {
        return;
    };
    use std::os::unix::fs::MetadataExt;
    let Ok(preserved) = std::fs::symlink_metadata(&rollback) else {
        return;
    };
    if !preserved.is_file() {
        return;
    }
    glib::timeout_add_local_once(ROLLBACK_RETENTION, move || {
        // A later install owns its own rollback copy, even in this process.
        if !std::fs::symlink_metadata(&rollback).is_ok_and(|current| {
            current.dev() == preserved.dev() && current.ino() == preserved.ino()
        }) {
            return;
        }
        if let Err(error) = std::fs::remove_file(&rollback) {
            tracing::warn!(%error, "could not remove the preserved previous version");
        }
    });
}

thread_local! {
    static INSTALL_GUARD: InstallGuard = Rc::new(Cell::new(false));
}

/// The one [`InstallGuard`] for this process.
///
/// Every install writes the *same* target -- the running executable -- so
/// the guard has to span every window, not just the controls within one. A
/// per-window guard would let installs started in separate windows replace
/// that executable concurrently, leaving the last writer as the installed
/// build.
///
/// A `thread_local` `Rc` (rather than a `Mutex`) is the whole story here
/// because every window is built on the single GTK main thread, from
/// `connect_activate`; this mirrors [`PreferenceManager::shared`]. It is
/// deliberately never released: it is one `bool`, and the guard's
/// correctness should not depend on some window or in-flight install
/// happening to still hold a strong reference.
pub(super) fn install_guard() -> InstallGuard {
    INSTALL_GUARD.with(|guard| guard.clone())
}

type InstallLauncher =
    Rc<dyn Fn(InstallRequest, InstallCancel) -> std::sync::mpsc::Receiver<UpdateInstall>>;

fn default_install_launcher() -> InstallLauncher {
    Rc::new(services::install_update)
}

thread_local! {
    /// Shared by the due scheduler so every window uses one TTL.
    static LAST_COMPLETED_CHECK: Cell<Option<Instant>> = const { Cell::new(None) };
    static CHECK_IN_FLIGHT: Cell<bool> = const { Cell::new(false) };
    static CHECK_GENERATION: Cell<u64> = const { Cell::new(0) };
    static LAST_UPDATE_RESULT: RefCell<CachedUpdate> = const { RefCell::new(None) };
    static UPDATE_NOTICE_HANDLERS: RefCell<Vec<WeakUpdateNoticeHandler>> = const { RefCell::new(Vec::new()) };
}

fn next_check_generation() -> u64 {
    CHECK_IN_FLIGHT.set(true);
    let next = CHECK_GENERATION.get().saturating_add(1);
    CHECK_GENERATION.set(next);
    next
}

/// Detection spawns a package-manager child, so it resolves asynchronously
/// on first need, never during startup.
static UPDATE_METHOD_CACHE: OnceLock<UpdateMethod> = OnceLock::new();

/// Invokes `callback` on the GTK thread, detecting on a worker thread on a cache miss.
pub(super) fn resolve_update_method_async(callback: impl FnOnce(UpdateMethod) + 'static) {
    if let Some(method) = UPDATE_METHOD_CACHE.get().copied() {
        callback(method);
        return;
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("strata-update-method".into())
        .spawn(move || {
            let method = *UPDATE_METHOD_CACHE.get_or_init(services::update_method);
            let _sent = sender.send(method);
        });
    if spawned.is_err() {
        let method = *UPDATE_METHOD_CACHE.get_or_init(services::update_method);
        callback(method);
        return;
    }
    let mut callback = Some(callback);
    glib::timeout_add_local(Duration::from_millis(50), move || {
        match receiver.try_recv() {
            Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
            resolved => {
                let method = match resolved {
                    Ok(method) => method,
                    Err(TryRecvError::Disconnected) => {
                        *UPDATE_METHOD_CACHE.get_or_init(services::update_method)
                    }
                    Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
                };
                if let Some(callback) = callback.take() {
                    callback(method);
                }
                glib::ControlFlow::Break
            }
        }
    });
}
const UPDATE_DUE_INTERVAL: Duration = Duration::from_secs(24 * 3600);

fn update_check_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|completed| now.duration_since(completed) >= UPDATE_DUE_INTERVAL)
}

fn force_due_update_check(last: Option<Instant>) -> bool {
    last.is_none()
}

pub(super) fn maybe_run_due_update_check(manager: &Rc<PreferenceManager>) {
    if !manager.checks_for_updates() || CHECK_IN_FLIGHT.get() {
        return;
    }
    let last_completed = LAST_COMPLETED_CHECK.get();
    if !update_check_due(last_completed, Instant::now()) {
        return;
    }
    let force = force_due_update_check(last_completed);
    let generation = next_check_generation();
    let channel = manager.release_channel();
    let weak_manager = Rc::downgrade(manager);
    resolve_update_method_async(move |method| {
        if is_stale_check(generation, CHECK_GENERATION.get()) {
            return;
        }
        let receiver = services::check_for_updates(
            channel,
            crate::build_info::installed_version(),
            method,
            force,
        );
        glib::timeout_add_local(Duration::from_millis(100), move || {
            if is_stale_check(generation, CHECK_GENERATION.get()) {
                return glib::ControlFlow::Break;
            }
            match receiver.try_recv() {
                Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(TryRecvError::Disconnected) => {
                    CHECK_IN_FLIGHT.set(false);
                    glib::ControlFlow::Break
                }
                Ok(result) => {
                    complete_due_update_check(&weak_manager, channel, result, method, generation);
                    glib::ControlFlow::Break
                }
            }
        });
    });
}

fn complete_due_update_check(
    manager: &Weak<PreferenceManager>,
    channel: Channel,
    result: UpdateCheck,
    method: UpdateMethod,
    generation: u64,
) {
    if is_stale_check(generation, CHECK_GENERATION.get()) {
        return;
    }
    CHECK_IN_FLIGHT.set(false);
    if !manager
        .upgrade()
        .is_some_and(|manager| manager.checks_for_updates() && manager.release_channel() == channel)
    {
        return;
    }
    LAST_COMPLETED_CHECK.set(Some(Instant::now()));
    if let UpdateCheck::Available { release, install } = result {
        publish_update_notice(Some((release, install, method)));
    }
}

pub(super) fn register_update_notice(notice: &UpdateNoticeHandler) {
    UPDATE_NOTICE_HANDLERS.with(|handlers| {
        handlers.borrow_mut().push(Rc::downgrade(notice));
    });
    LAST_UPDATE_RESULT.with(|cache| {
        if let Some(result) = cache.borrow().clone() {
            notice(Some(result));
        }
    });
}

fn publish_update_notice(result: CachedUpdate) {
    LAST_UPDATE_RESULT.with(|cache| *cache.borrow_mut() = result.clone());
    UPDATE_NOTICE_HANDLERS.with(|handlers| {
        handlers.borrow_mut().retain(|notice| {
            let Some(notice) = notice.upgrade() else {
                return false;
            };
            notice(result.clone());
            true
        });
    });
}

pub(super) fn clear_cached_update_notice() {
    LAST_UPDATE_RESULT.with(|cache| *cache.borrow_mut() = None);
}

const DIALOG_WIDTH: i32 = 1400;
const DIALOG_HEIGHT: i32 = 1024;
const DIALOG_MARGIN: i32 = 24;
const COMPACT_NAVIGATION_BREAKPOINT: i32 = 900;
// Reflow the content before collapsing navigation: desktop toolbars need more room.
const COMPACT_CONTENT_BREAKPOINT: i32 = 1250;
const MIN_SIDE_BY_SIDE_ACTIVATION_WIDTH: i32 = 620;
const STACK_ACTIVATION_OPTIONS_BREAKPOINT: i32 = 350;
const STACK_TEXT_SIZE_BREAKPOINT: i32 = 600;
const STACK_EXCLUSION_INPUT_BREAKPOINT: i32 = 1000;

mod responsive_bin {
    use super::*;

    #[derive(Default)]
    pub struct ResponsiveBin {
        pub compact_navigation: Cell<bool>,
        pub compact_content: Cell<bool>,
        pub typography_scale: Cell<f64>,
        // Matrix widths below which the translated choices were seen squeezed: side by side,
        // or with each label beside its control.
        pub activation_side_by_side_min: Cell<i32>,
        pub activation_inline_options_min: Cell<i32>,
        pub navigation: RefCell<Option<gtk::Box>>,
        pub navigation_heading: RefCell<Option<gtk::Label>>,
        pub navigation_labels: RefCell<Vec<gtk::Label>>,
        pub navigation_contents: RefCell<Vec<gtk::Box>>,
        pub responsive_flows: RefCell<Vec<(gtk::FlowBox, u32)>>,
        pub responsive_actions: RefCell<Vec<(gtk::Box, gtk::Button)>>,
        pub responsive_setting_rows: RefCell<Vec<gtk::Box>>,
        pub responsive_activation_rows: RefCell<Vec<ResponsiveActivationRow>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ResponsiveBin {
        const NAME: &'static str = "StrataSettingsResponsiveBin";
        type Type = super::ResponsiveBin;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ResponsiveBin {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for ResponsiveBin {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let natural = match orientation {
                gtk::Orientation::Horizontal => DIALOG_WIDTH + DIALOG_MARGIN * 2,
                gtk::Orientation::Vertical => DIALOG_HEIGHT + DIALOG_MARGIN * 2,
                _ => unreachable!("GTK orientations are horizontal or vertical"),
            };
            (1, natural, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let Some(child) = self.obj().first_child() else {
                return;
            };
            let (child_width, child_height) = responsive_dialog_size(width, height);
            let logical_width = f64::from(child_width) / self.typography_scale.get().max(0.1);
            let compact_content = logical_width < f64::from(COMPACT_CONTENT_BREAKPOINT);
            let compact = uses_compact_navigation(
                (f64::from(child_width) / self.typography_scale.get().max(0.1)) as i32,
            );
            if self.compact_navigation.replace(compact) != compact {
                if let Some(navigation) = self.navigation.borrow().as_ref() {
                    search::set_compact(navigation, compact);
                    if compact {
                        navigation.add_css_class("compact");
                    } else {
                        navigation.remove_css_class("compact");
                    }
                }
                if let Some(heading) = self.navigation_heading.borrow().as_ref() {
                    heading.set_visible(!compact);
                }
                for label in self.navigation_labels.borrow().iter() {
                    label.set_visible(!compact);
                }
                for content in self.navigation_contents.borrow().iter() {
                    content.set_halign(if compact {
                        gtk::Align::Center
                    } else {
                        gtk::Align::Fill
                    });
                }
            }
            if self.compact_content.replace(compact_content) != compact_content {
                let compact = compact_content;
                for (flow, expanded_columns) in self.responsive_flows.borrow().iter() {
                    flow.set_max_children_per_line(if compact { 1 } else { *expanded_columns });
                }
                for (row, action) in self.responsive_actions.borrow().iter() {
                    row.set_orientation(if compact {
                        gtk::Orientation::Vertical
                    } else {
                        gtk::Orientation::Horizontal
                    });
                    action.set_halign(gtk::Align::Fill);
                }
                for row in self.responsive_setting_rows.borrow().iter() {
                    row.set_orientation(if compact {
                        gtk::Orientation::Vertical
                    } else {
                        gtk::Orientation::Horizontal
                    });
                    row.set_spacing(if compact { 8 } else { 16 });
                }
            }
            let available_activation_width = child_width - if compact { 100 } else { 330 };
            let scaled_activation_width = MIN_SIDE_BY_SIDE_ACTIVATION_WIDTH as f64
                + (self.typography_scale.get() - 1.0).max(0.0) * 320.0;
            let matrix_width = activation_matrix_width(&self.responsive_activation_rows.borrow());
            let activation_compact = f64::from(available_activation_width)
                < scaled_activation_width
                || matrix_width < self.activation_side_by_side_min.get();
            let stack_activation_options = available_activation_width
                < STACK_ACTIVATION_OPTIONS_BREAKPOINT
                || matrix_width < self.activation_inline_options_min.get();
            for responsive_row in self.responsive_activation_rows.borrow().iter() {
                responsive_row.row.set_orientation(if activation_compact {
                    gtk::Orientation::Vertical
                } else {
                    gtk::Orientation::Horizontal
                });
                responsive_row
                    .row
                    .set_spacing(if activation_compact { 4 } else { 24 });
                for option in &responsive_row.options {
                    option.set_orientation(if stack_activation_options {
                        gtk::Orientation::Vertical
                    } else {
                        gtk::Orientation::Horizontal
                    });
                    option.set_spacing(if stack_activation_options { 2 } else { 6 });
                    option.set_halign(if activation_compact && !stack_activation_options {
                        gtk::Align::End
                    } else {
                        gtk::Align::Fill
                    });
                }
                if activation_compact {
                    responsive_row.row.add_css_class("compact");
                } else {
                    responsive_row.row.remove_css_class("compact");
                }
            }
            reflow_settings(
                &child,
                compact_content,
                activation_compact,
                logical_width < f64::from(STACK_TEXT_SIZE_BREAKPOINT),
                logical_width < f64::from(STACK_EXCLUSION_INPUT_BREAKPOINT),
            );
            let x = ((width - child_width) / 2) as f32;
            let y = ((height - child_height) / 2) as f32;
            let transform = gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(x, y));
            child.allocate(child_width, child_height, baseline, Some(transform));
            self.stack_squeezed_activation_choices(activation_compact, stack_activation_options);
        }
    }

    impl ResponsiveBin {
        /// Long translated choices can need more room than the fixed breakpoints allow; once
        /// a choice gets less than its natural width, stack the matrix further at this width.
        fn stack_squeezed_activation_choices(&self, compact: bool, stacked_options: bool) {
            if compact && stacked_options {
                return;
            }
            let rows = self.responsive_activation_rows.borrow();
            let squeezed = rows.iter().filter(|row| row.row.is_mapped()).any(|row| {
                row.options
                    .iter()
                    .filter_map(|option| option.last_child())
                    .any(|control| choice_label_wraps(&control))
            });
            if !squeezed {
                return;
            }
            let threshold = activation_matrix_width(&rows) + 1;
            if compact {
                self.activation_inline_options_min.set(threshold);
            } else {
                self.activation_side_by_side_min.set(threshold);
            }
            let bin = self.obj().downgrade();
            glib::idle_add_local_once(move || {
                if let Some(bin) = bin.upgrade() {
                    bin.queue_allocate();
                }
            });
        }
    }
}

glib::wrapper! {
    pub struct ResponsiveBin(ObjectSubclass<responsive_bin::ResponsiveBin>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ResponsiveBin {
    fn new(
        child: &impl IsA<gtk::Widget>,
        navigation: &gtk::Box,
        navigation_heading: &gtk::Label,
        navigation_labels: Vec<gtk::Label>,
        navigation_contents: Vec<gtk::Box>,
        responsive: ResponsiveContent,
    ) -> Self {
        let bin: Self = glib::Object::new();
        let imp = bin.imp();
        imp.navigation.replace(Some(navigation.clone()));
        imp.navigation_heading
            .replace(Some(navigation_heading.clone()));
        imp.navigation_labels.replace(navigation_labels);
        imp.navigation_contents.replace(navigation_contents);
        imp.responsive_flows.replace(responsive.flows);
        imp.responsive_actions.replace(responsive.actions);
        imp.responsive_setting_rows.replace(responsive.setting_rows);
        imp.responsive_activation_rows
            .replace(responsive.activation_rows);
        child.set_parent(&bin);
        PreferenceManager::shared().bind_interface_scale(&bin, |widget, scale| {
            let bin = widget
                .downcast_ref::<ResponsiveBin>()
                .expect("settings bin");
            bin.imp().typography_scale.set(scale);
            bin.imp().activation_side_by_side_min.set(0);
            bin.imp().activation_inline_options_min.set(0);
            bin.queue_allocate();
        });
        bin
    }

    fn add_navigation(&self, label: gtk::Label, content: gtk::Box) {
        let imp = self.imp();
        imp.navigation_labels.borrow_mut().push(label);
        imp.navigation_contents.borrow_mut().push(content);
    }

    fn add_flow(&self, flow: gtk::FlowBox, columns: u32) {
        flow.set_max_children_per_line(if self.imp().compact_content.get() {
            1
        } else {
            columns
        });
        self.imp()
            .responsive_flows
            .borrow_mut()
            .push((flow, columns));
    }

    fn add_action(&self, row: gtk::Box, button: gtk::Button) {
        row.set_orientation(if self.imp().compact_content.get() {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        });
        button.set_halign(gtk::Align::Fill);
        self.imp()
            .responsive_actions
            .borrow_mut()
            .push((row, button));
    }
}

/// Whether a segmented control gives any choice less than its one-line width.
fn choice_label_wraps(control: &gtk::Widget) -> bool {
    let mut choice = control.first_child();
    while let Some(button) = choice {
        choice = button.next_sibling();
        if let Some(label) = button.downcast_ref::<gtk::Button>().and_then(|b| b.child())
            && label.width() < label.measure(gtk::Orientation::Horizontal, -1).1
        {
            return true;
        }
    }
    false
}

fn activation_matrix_width(rows: &[ResponsiveActivationRow]) -> i32 {
    rows.first()
        .and_then(|row| row.row.parent())
        .map_or(0, |matrix| matrix.width())
}

fn reflow_settings(
    widget: &gtk::Widget,
    compact: bool,
    activation_compact: bool,
    stack_text_size: bool,
    stack_exclusion_input: bool,
) {
    if widget.has_css_class("settings-dialog") {
        if compact {
            widget.add_css_class("compact");
        } else {
            widget.remove_css_class("compact");
        }
    }
    if let Some(row) = widget.downcast_ref::<gtk::Box>()
        && [
            "settings-option",
            "settings-library-toolbar",
            "about-identity",
            "theme-library-footer",
            "settings-inline-description",
            "settings-update-summary",
        ]
        .iter()
        .any(|class| row.has_css_class(class))
    {
        let switch_row = row
            .last_child()
            .is_some_and(|child| child.is::<gtk::Switch>());
        // Short numeric controls can stay beside wrapping copy after other rows stack.
        let stack_row = if row.has_css_class("settings-renderer-row")
            || row.has_css_class("settings-exclusions-row")
        {
            true
        } else if row.has_css_class("settings-text-size-row") {
            stack_text_size
        } else {
            compact
        };
        let orientation = if stack_row && !switch_row {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        };
        // Release notes and sidebar chips always stay below their row's copy.
        if !row.has_css_class("settings-update-status")
            && !row.has_css_class("settings-sidebar-places")
            && row.orientation() != orientation
        {
            row.set_orientation(orientation);
        }
    }
    if widget.has_css_class("settings-integration-actions")
        && let Some(actions) = widget.downcast_ref::<gtk::Box>()
    {
        actions.set_orientation(if compact {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        });
    }
    if widget.has_css_class("theme-appearance-filter")
        || widget.has_css_class("settings-integration-actions")
    {
        widget.set_halign(if compact {
            gtk::Align::Fill
        } else {
            gtk::Align::End
        });
        widget.set_hexpand(compact);
        let mut child = widget.first_child();
        while let Some(button) = child {
            child = button.next_sibling();
            button.set_hexpand(compact);
        }
    }
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && (label.has_css_class("settings-nowrap") || label.has_css_class("menu-heading"))
    {
        label.set_wrap(compact && !label.has_css_class("settings-keycap"));
    }
    if let Some(button) = widget.downcast_ref::<gtk::Button>()
        && !widget.is::<gtk::ToggleButton>()
        && let Some(label) = button.child().and_downcast::<gtk::Label>()
    {
        label.set_wrap(compact);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    }
    // Keep the exclusion placeholder readable instead of squeezing it beside translated buttons.
    if widget.has_css_class("settings-exclusions-input")
        && let Some(row) = widget.downcast_ref::<gtk::Box>()
    {
        let orientation = if stack_exclusion_input {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        };
        if row.orientation() != orientation {
            row.set_orientation(orientation);
        }
    }
    if widget.has_css_class("activation-header") {
        widget.set_visible(!activation_compact);
    }
    if widget.has_css_class("activation-inline-label") {
        widget.set_visible(activation_compact);
    }
    // Beside the choices, keep long single-word view names whole; stacked, they have the full width.
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && label.has_css_class("click-activation-title")
    {
        let mode = if activation_compact {
            gtk::pango::WrapMode::WordChar
        } else {
            gtk::pango::WrapMode::Word
        };
        if label.wrap_mode() != mode {
            label.set_wrap_mode(mode);
        }
    }
    let mut child = widget.first_child();
    while let Some(next) = child {
        child = next.next_sibling();
        reflow_settings(
            &next,
            compact,
            activation_compact,
            stack_text_size,
            stack_exclusion_input,
        );
    }
}

fn responsive_dialog_size(width: i32, height: i32) -> (i32, i32) {
    (
        DIALOG_WIDTH.min((width - DIALOG_MARGIN * 2).max(1)),
        DIALOG_HEIGHT.min((height - DIALOG_MARGIN * 2).max(1)),
    )
}

fn uses_compact_navigation(dialog_width: i32) -> bool {
    dialog_width < COMPACT_NAVIGATION_BREAKPOINT
}

#[expect(
    deprecated,
    reason = "GTK 4.12 deprecated translate_coordinates and allocation without a replacement for click-in-bounds checks"
)]
pub fn build_layer(
    settings_button: &gtk::Button,
    root: &BlurBin,
    preferences: Rc<PreferenceManager>,
    update_notice: UpdateNoticeHandler,
    install_guard: InstallGuard,
) -> gtk::Box {
    let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layer.add_css_class("app-modal-layer");
    layer.add_css_class("settings-backdrop");
    layer.set_halign(gtk::Align::Fill);
    layer.set_valign(gtk::Align::Fill);
    layer.set_hexpand(true);
    layer.set_vexpand(true);
    layer.set_focusable(true);
    layer.set_visible(false);

    let panel = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    panel.add_css_class("settings-dialog");
    panel.set_overflow(gtk::Overflow::Hidden);

    let navigation = gtk::Box::new(gtk::Orientation::Vertical, 2);
    navigation.add_css_class("settings-navigation");
    let navigation_heading = append_heading(&navigation, "SETTINGS");
    let settings_search = search::append(&navigation);

    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.add_css_class("settings-page");
    page.set_hexpand(true);
    let titlebar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    titlebar.add_css_class("settings-titlebar");
    let title = gtk::Label::new(Some(&crate::i18n::tr("General")));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.add_css_class("settings-title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(1);
    let close = gtk::Button::builder()
        .tooltip_text(crate::i18n::tr("Close settings"))
        .build();
    close.set_child(Some(&crate::assets::primary_icon(icons::X, 18)));
    close.add_css_class("settings-close");
    close.set_valign(gtk::Align::Center);
    titlebar.append(&title);
    titlebar.append(&close);
    page.append(&titlebar);

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(120)
        .hhomogeneous(false)
        .vhomogeneous(false)
        .hexpand(true)
        .vexpand(true)
        .build();
    let (general, responsive_setting_rows, responsive_activation_rows) =
        general_page(preferences.clone());
    stack.add_named(&general, Some("general"));
    stack.add_named(&about_page(), Some("about"));
    // Heavy pages build on first selection, never during startup: the
    // Updates page spawns package-manager detection plus release-note
    // network work, and the theme page builds its swatch flows.
    let updates_container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    updates_container.set_hexpand(true);
    updates_container.set_vexpand(true);
    let updates_spinner = gtk::Spinner::new();
    updates_spinner.start();
    updates_spinner.set_halign(gtk::Align::Center);
    updates_spinner.set_valign(gtk::Align::Center);
    updates_spinner.set_vexpand(true);
    updates_container.append(&updates_spinner);
    stack.add_named(&updates_container, Some("updates"));
    page.append(&stack);

    // Created before the navigation loop so lazy builders can register
    // their responsive rows and flows as pages materialize.
    let responsive_panel = ResponsiveBin::new(
        &panel,
        &navigation,
        &navigation_heading,
        Vec::new(),
        Vec::new(),
        ResponsiveContent {
            flows: Vec::new(),
            actions: Vec::new(),
            setting_rows: responsive_setting_rows,
            activation_rows: responsive_activation_rows,
        },
    );
    // Navigation entries register as their buttons are created, so the
    // responsive panel compacts correctly even with lazy pages.
    let responsive_for_nav = responsive_panel.clone();
    let built: Rc<RefCell<std::collections::HashSet<&'static str>>> =
        Rc::new(RefCell::new(["general", "about"].into_iter().collect()));
    let dismiss_hooks = DismissHooks::default();
    let nav_buttons: Rc<RefCell<Vec<gtk::Button>>> = Rc::new(RefCell::new(Vec::new()));
    for (label, icon, name) in [
        ("General", icons::SLIDERS, "general"),
        ("Appearance", icons::PALETTE, "theme"),
        ("Actions", icons::PLAY, "actions"),
        ("Updates", icons::DOWNLOADS, "updates"),
        ("About", icons::INFO, "about"),
    ] {
        let active = name == "general";
        let (button, navigation_label, navigation_content) = navigation_button(icon, label);
        responsive_for_nav.add_navigation(navigation_label, navigation_content);
        if active {
            button.add_css_class("settings-nav-active");
        }
        nav_buttons.borrow_mut().push(button.clone());
        let buttons = nav_buttons.clone();
        let stack = stack.clone();
        let title = title.clone();
        let page_title = label.to_owned();
        let built = built.clone();
        let preferences = preferences.clone();
        let update_notice = update_notice.clone();
        let install_guard = install_guard.clone();
        let updates_container = updates_container.clone();
        let responsive_panel = responsive_panel.clone();
        let search_state = settings_search.state.clone();
        let dismiss_hooks = dismiss_hooks.clone();
        button.connect_clicked(move |clicked| {
            for candidate in buttons.borrow().iter() {
                if candidate == clicked {
                    candidate.add_css_class("settings-nav-active");
                } else {
                    candidate.remove_css_class("settings-nav-active");
                }
            }
            if built.borrow_mut().insert(name) {
                match name {
                    "theme" => {
                        let page =
                            theme_page(preferences.clone(), super::theme::ThemeManager::shared());
                        search::apply(&page.widget, &search_state);
                        stack.add_named(&page.widget, Some("theme"));
                        for (flow, columns) in page.flows {
                            responsive_panel.add_flow(flow, columns);
                        }
                        dismiss_hooks.add(page.dismiss);
                    }
                    "actions" => {
                        let page = actions::actions_page();
                        search::apply(&page, &search_state);
                        stack.add_named(&page, Some("actions"));
                    }
                    "updates" => {
                        let container = updates_container.clone();
                        let panel = responsive_panel.clone();
                        let preferences = preferences.clone();
                        let update_notice = update_notice.clone();
                        let install_guard = install_guard.clone();
                        let _ = stack;
                        let search_state = search_state.clone();
                        resolve_update_method_async(move |method| {
                            maybe_run_due_update_check(&preferences);
                            let (updates, actions) =
                                updates_page(preferences, update_notice, install_guard, method);
                            while let Some(child) = container.first_child() {
                                container.remove(&child);
                            }
                            search::apply(&updates, &search_state);
                            container.append(&updates);
                            for (row, button) in actions {
                                panel.add_action(row, button);
                            }
                        });
                    }
                    _ => {}
                }
            }
            stack.set_visible_child_name(name);
            title.set_text(&crate::i18n::tr(&page_title));
        });
        navigation.append(&button);
    }

    settings_search.install(&stack, &title, &nav_buttons);
    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    navigation.append(&spacer);

    let navigation_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_width(true)
        .child(&navigation)
        .build();
    panel.append(&navigation_scroll);
    panel.append(&page);
    responsive_panel.set_hexpand(false);
    responsive_panel.set_vexpand(false);
    let top = gtk::Box::new(gtk::Orientation::Vertical, 0);
    top.set_vexpand(true);
    let bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bottom.set_vexpand(true);
    let left = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    left.set_hexpand(true);
    let right = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    right.set_hexpand(true);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.append(&left);
    row.append(&responsive_panel);
    row.append(&right);
    layer.append(&top);
    layer.append(&row);
    layer.append(&bottom);

    let close_settings: Rc<dyn Fn()> = {
        let layer = layer.clone();
        let button = settings_button.clone();
        let root = root.clone();
        let hooks = dismiss_hooks.clone();
        Rc::new(move || hide(&layer, &button, &root, &hooks))
    };
    // A window closed mid-preview never runs `hide`; the preview is process-wide.
    layer.connect_unrealize(move |_| dismiss_hooks.run());
    let close_hide = close_settings.clone();
    close.connect_clicked(move |_| close_hide());
    let escape_hide = close_settings.clone();
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key != gdk::Key::Escape {
            return gtk::glib::Propagation::Proceed;
        }
        escape_hide();
        gtk::glib::Propagation::Stop
    });
    layer.add_controller(keys);

    let click_layer = layer.clone();
    let click_dialog = responsive_panel.clone();
    let click = gtk::GestureClick::new();
    click.connect_pressed(move |_, _, x, y| {
        let on_dialog = click_dialog
            .translate_coordinates(&click_layer, 0.0, 0.0)
            .is_some_and(|(dx, dy)| {
                let alloc = click_dialog.allocation();
                x >= dx
                    && x < dx + alloc.width() as f64
                    && y >= dy
                    && y < dy + alloc.height() as f64
            });
        if !on_dialog {
            close_settings();
        }
    });
    layer.add_controller(click);
    super::focus_navigation::install(&layer);
    layer
}

/// Work that every route closing Settings must run, such as discarding a theme preview.
type DismissHookList = RefCell<Vec<Rc<dyn Fn()>>>;

#[derive(Clone, Default)]
struct DismissHooks(Rc<DismissHookList>);

impl DismissHooks {
    fn add(&self, hook: Rc<dyn Fn()>) {
        self.0.borrow_mut().push(hook);
    }

    /// Hooks are idempotent: they run on every close and again when the layer unrealizes.
    fn run(&self) {
        let hooks = self.0.borrow().clone();
        for hook in hooks {
            hook();
        }
    }
}

fn hide(layer: &gtk::Box, button: &gtk::Button, root: &BlurBin, hooks: &DismissHooks) {
    hooks.run();
    if layer.has_css_class("dismissing") {
        return;
    }
    layer.add_css_class("dismissing");
    layer.set_sensitive(false);
    let layer_for_anim = layer.clone();
    let layer = layer.clone();
    let root = root.clone();
    let button = button.clone();
    super::browser::animate_out(&layer_for_anim, move || {
        layer.set_visible(false);
        layer.remove_css_class("dismissing");
        layer.set_sensitive(true);
        root.set_blurred(false);
        button.remove_css_class("active");
    });
}

fn updates_page(
    manager: Rc<PreferenceManager>,
    update_notice: UpdateNoticeHandler,
    install_guard: InstallGuard,
    update_method: UpdateMethod,
) -> (gtk::Widget, Vec<(gtk::Box, gtk::Button)>) {
    let preferences = page_content();
    preferences.add_css_class("settings-updates-page");
    append_heading(&preferences, "STATUS");
    let managed = InstallSource::detect().managed();
    if let Some(managed) = managed {
        preferences.append(&managed_install_row(managed));
    }

    let available_notes = release_notes_card(
        ReleaseNotesKind::Available,
        &crate::i18n::tr("Available release"),
        &crate::i18n::tr("Check for updates to see the latest release notes."),
    );
    let UpdateCheckRow {
        row: update_row,
        run_check,
        responsive_action,
        install_underway,
        ..
    } = update_check_row(
        manager.clone(),
        available_notes.clone(),
        install_guard.clone(),
        update_method,
    );

    preferences.append(&update_row);
    let options = settings_group(&preferences, "PREFERENCES");
    options.append(&automatic_updates_option(&manager, update_method));
    let channel_row = append_channel_option(&options, manager.clone(), managed, update_method);
    append_current_release_notes(&preferences);
    bind_updates_auto_check(
        &manager,
        &channel_row,
        &preferences,
        run_check.clone(),
        update_notice,
    );

    let page = scrollable_page(&preferences, None);
    wire_channel_change_check(&manager, &page, run_check, install_underway);
    (page, vec![responsive_action])
}

fn append_channel_option(
    preferences: &gtk::Box,
    manager: Rc<PreferenceManager>,
    managed: Option<&ManagedInstall>,
    update_method: UpdateMethod,
) -> gtk::Box {
    let channel_row = channel_option(manager.clone(), managed);
    channel_row.set_sensitive(manager.checks_for_updates());
    search::set_available(
        &channel_row,
        managed.is_some() || !update_method.is_package_managed(),
    );
    preferences.append(&channel_row);
    channel_row
}

fn append_current_release_notes(preferences: &gtk::Box) {
    append_heading(preferences, "RELEASE NOTES");
    let current_notes = release_notes_card(
        ReleaseNotesKind::Current,
        &rust_i18n::t!(
            "What's new in v%{value1}",
            value1 = crate::build_info::installed_version()
        ),
        &crate::i18n::tr("Loading release notes…"),
    );
    current_notes.container.remove(&current_notes.title);
    current_notes.container.remove(&current_notes.summary);
    let header = gtk::Box::new(gtk::Orientation::Vertical, 6);
    header.append(&current_notes.title);
    header.append(&current_notes.summary);
    header.set_hexpand(true);
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    heading.append(&header);
    heading.append(&crate::assets::primary_icon(icons::CHEVRON_RIGHT, 16));
    let toggle = gtk::Button::builder().child(&heading).build();
    toggle.add_css_class("release-notes-toggle");
    let details = gtk::Revealer::builder()
        .child(&current_notes.container)
        .transition_duration(0)
        .reveal_child(false)
        .build();
    let expander = gtk::Box::new(gtk::Orientation::Vertical, 0);
    expander.add_css_class("settings-release-expander");
    search::tag(&expander, "Release notes");
    expander.append(&toggle);
    expander.append(&details);
    toggle.update_state(&[gtk::accessible::State::Expanded(Some(false))]);
    toggle.connect_clicked(move |button| {
        let expanded = !details.reveals_child();
        details.set_reveal_child(expanded);
        button.update_state(&[gtk::accessible::State::Expanded(Some(expanded))]);
        if expanded {
            button.add_css_class("expanded");
        } else {
            button.remove_css_class("expanded");
        }
    });
    preferences.append(&expander);
    load_current_release_notes(&current_notes);
}

fn bind_updates_auto_check(
    manager: &Rc<PreferenceManager>,
    channel_row: &gtk::Box,
    preferences: &gtk::Box,
    run_check: Rc<dyn Fn(bool)>,
    update_notice: UpdateNoticeHandler,
) {
    manager.bind_preference(
        channel_row,
        PreferenceManager::checks_for_updates,
        |widget, enabled| widget.set_sensitive(enabled),
    );
    let initial = Cell::new(true);
    manager.bind_preference(
        preferences,
        PreferenceManager::checks_for_updates,
        move |_, enabled| {
            if initial.replace(false) {
                return;
            }
            if enabled {
                run_check(false);
            } else {
                update_notice(None);
            }
        },
    );
}

fn wire_channel_change_check(
    manager: &Rc<PreferenceManager>,
    page: &impl IsA<gtk::Widget>,
    run_check: Rc<dyn Fn(bool)>,
    install_underway: Rc<dyn Fn() -> bool>,
) {
    // No automatic check here: the due scheduler owns background checks process-wide.
    manager.on_release_channel_changed(
        page,
        Rc::new(move || {
            if !install_underway() {
                run_check(false);
            }
        }),
    );
}

fn automatic_updates_option(manager: &Rc<PreferenceManager>, method: UpdateMethod) -> gtk::Box {
    let (row, toggle) = settings_option(
        "Check for updates automatically",
        match method {
            UpdateMethod::InPlace => "Look for a new release on GitHub when Strata starts.",
            UpdateMethod::Aur => "Check the AUR for a newer packaged release when Strata starts.",
            UpdateMethod::Omarchy => {
                "Check the Omarchy package repository for a newer release when Strata starts."
            }
            UpdateMethod::Pacman => {
                "Check the configured package repositories for a newer release when Strata starts."
            }
        },
        manager.checks_for_updates(),
    );
    bind_switch(
        manager,
        &toggle,
        PreferenceManager::checks_for_updates,
        PreferenceManager::set_checks_for_updates,
    );
    row
}

const RELEASE_CHANNEL_TITLE: &str = "Release channel";
const STABLE_CHANNEL: &str = "Stable";
// The release channel, distinct from the file-preview sense of "Preview".
const PREVIEW_CHANNEL: &str = "release_channel.preview";
const NIGHTLY_CHANNEL: &str = "Nightly";

fn channel_label(channel: Channel) -> String {
    crate::i18n::tr(match channel {
        Channel::Stable => STABLE_CHANNEL,
        Channel::Preview => PREVIEW_CHANNEL,
        Channel::Nightly => NIGHTLY_CHANNEL,
    })
}

/// The manifest's channel id, shown by its localized name when Strata knows it.
fn managed_channel_label(managed: &ManagedInstall) -> Option<String> {
    let id = managed.channel()?;
    Some(
        managed
            .tracked_channel()
            .map_or_else(|| id.to_owned(), channel_label),
    )
}

fn channel_option(manager: Rc<PreferenceManager>, managed: Option<&ManagedInstall>) -> gtk::Box {
    let control = bindings::choice_menu(
        &manager,
        RELEASE_CHANNEL_TITLE,
        &[
            (STABLE_CHANNEL, Channel::Stable),
            (PREVIEW_CHANNEL, Channel::Preview),
            (NIGHTLY_CHANNEL, Channel::Nightly),
        ],
        PreferenceManager::release_channel,
        PreferenceManager::set_release_channel,
    );
    control.set_sensitive(managed.is_none());
    let description = managed
        .map(managed_channel_description)
        .unwrap_or_else(|| channel_description(manager.release_channel()).to_owned());
    let row = control_row(RELEASE_CHANNEL_TITLE, &description, &control);
    if managed.is_none() {
        let label = row
            .first_child()
            .and_downcast::<gtk::Box>()
            .expect("channel row copy")
            .last_child()
            .and_downcast::<gtk::Label>()
            .expect("channel description");
        manager.bind_preference(
            &label,
            PreferenceManager::release_channel,
            |widget, channel| {
                widget
                    .downcast_ref::<gtk::Label>()
                    .expect("channel description binding")
                    .set_text(&crate::i18n::tr(channel_description(channel)));
            },
        );
    }
    row
}

fn channel_description(channel: Channel) -> &'static str {
    match channel {
        Channel::Stable => "Tested releases recommended for everyday use.",
        Channel::Preview => "Try upcoming releases with alpha, beta, and release-candidate builds.",
        Channel::Nightly => "Everything in Preview, plus daily development builds. May break.",
    }
}

fn managed_channel_description(managed: &ManagedInstall) -> String {
    let tracked = match managed_channel_label(managed) {
        Some(channel) => rust_i18n::t!(
            "This install tracks the %{channel} release channel.",
            channel = channel
        )
        .into_owned(),
        None => crate::i18n::tr("The installed package decides the release channel."),
    };
    match managed.alternate_instruction() {
        Some(alternate) => format!("{tracked} {alternate}"),
        None => tracked,
    }
}

fn release_notes_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class("release-notes-content");
    label.set_xalign(0.0);
    label.set_yalign(0.0);
    label.set_hexpand(true);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_selectable(true);
    label.set_use_markup(true);
    label
}

fn clear_release_notes(notes: &gtk::Box) {
    while let Some(child) = notes.first_child() {
        notes.remove(&child);
    }
}

fn set_release_notes_message(notes: &gtk::Box, message: &str) {
    clear_release_notes(notes);
    let label = release_notes_label();
    label.set_text(message);
    notes.append(&label);
}

fn set_release_note_blocks(notes: &gtk::Box, blocks: &[DocumentBlock]) {
    clear_release_notes(notes);
    for block in blocks {
        match block {
            DocumentBlock::Heading { level, markup } => {
                let label = release_notes_label();
                label.add_css_class("release-notes-heading");
                label.add_css_class(&format!("level-{level}"));
                label.set_markup(markup);
                notes.append(&label);
            }
            DocumentBlock::Paragraph(markup) => {
                let label = release_notes_label();
                label.set_markup(markup);
                notes.append(&label);
            }
            DocumentBlock::ListItem {
                marker,
                depth,
                markup,
            } => {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                row.set_valign(gtk::Align::Start);
                row.set_margin_start(i32::try_from(depth.saturating_mul(18)).unwrap_or(i32::MAX));
                let bullet = gtk::Label::new(Some(marker));
                bullet.add_css_class("release-notes-bullet");
                bullet.set_valign(gtk::Align::Start);
                let copy = release_notes_label();
                copy.set_markup(markup);
                row.append(&bullet);
                row.append(&copy);
                notes.append(&row);
            }
            DocumentBlock::ListChild {
                depth,
                kind,
                markup,
            } => {
                let copy = release_notes_label();
                copy.set_margin_start(
                    i32::try_from(depth.saturating_add(1).saturating_mul(18)).unwrap_or(i32::MAX),
                );
                match kind {
                    services::DocumentListChildKind::Heading(level) => {
                        copy.add_css_class("release-notes-heading");
                        copy.add_css_class(&format!("level-{level}"));
                        copy.set_markup(markup);
                    }
                    services::DocumentListChildKind::Code(_) => {
                        copy.add_css_class("release-notes-code");
                        copy.set_markup(&format!("<tt>{markup}</tt>"));
                    }
                    services::DocumentListChildKind::Paragraph
                    | services::DocumentListChildKind::Quote => copy.set_markup(markup),
                }
                notes.append(&copy);
            }
            DocumentBlock::Code { markup, .. } => {
                let label = release_notes_label();
                label.add_css_class("release-notes-code");
                label.set_markup(&format!("<tt>{markup}</tt>"));
                notes.append(&label);
            }
            DocumentBlock::Rule => {
                let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
                separator.add_css_class("release-notes-rule");
                notes.append(&separator);
            }
            DocumentBlock::ListRule { depth } => {
                let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
                separator.add_css_class("release-notes-rule");
                separator.set_margin_start(
                    i32::try_from(depth.saturating_add(1).saturating_mul(18)).unwrap_or(i32::MAX),
                );
                notes.append(&separator);
            }
            DocumentBlock::Quote(_)
            | DocumentBlock::TableRow { .. }
            | DocumentBlock::ListTableRow { .. }
            | DocumentBlock::ContainerBoundary
            | DocumentBlock::Image { .. } => {}
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReleaseNotesKind {
    Current,
    Available,
}

#[derive(Clone)]
struct ReleaseNotesCard {
    kind: ReleaseNotesKind,
    container: gtk::Box,
    title: gtk::Label,
    summary: gtk::Label,
    badge: gtk::Label,
    notes: gtk::Box,
    fallback: gtk::LinkButton,
}

fn release_notes_card(kind: ReleaseNotesKind, title: &str, initial: &str) -> ReleaseNotesCard {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 8);
    container.add_css_class("release-notes-card");
    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("release-notes-title");
    title_label.set_xalign(0.0);
    title_label.set_wrap(true);
    title_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    let badge = gtk::Label::new(None);
    badge.add_css_class("prerelease-badge");
    badge.set_xalign(0.0);
    badge.set_halign(gtk::Align::Start);
    badge.set_visible(false);
    let notes = gtk::Box::new(gtk::Orientation::Vertical, 6);
    set_release_notes_message(&notes, initial);
    let fallback = gtk::LinkButton::with_label(
        "https://github.com/lgse/strata/releases",
        &crate::i18n::tr("View on GitHub"),
    );
    fallback.set_has_tooltip(false);
    fallback.add_css_class("release-notes-fallback");
    fallback.set_halign(gtk::Align::Start);
    fallback.set_visible(false);
    let summary = gtk::Label::new(Some(initial));
    summary.set_xalign(0.0);
    summary.set_wrap(true);
    summary.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    summary.add_css_class("settings-option-description");
    container.append(&title_label);
    container.append(&summary);
    container.append(&badge);
    container.append(&notes);
    container.append(&fallback);
    ReleaseNotesCard {
        kind,
        container,
        title: title_label,
        summary,
        badge,
        notes,
        fallback,
    }
}

/// Shows `release`'s notes in `card`, including a visible prerelease badge
/// above the notes whenever `release.kind` is not [`BuildKind::Stable`] --
/// release notes and update surfaces must visibly label prerelease software.
fn show_release_notes(card: &ReleaseNotesCard, release: &ReleaseMetadata) {
    card.container.set_visible(true);
    card.title.set_text(&match card.kind {
        ReleaseNotesKind::Current => {
            rust_i18n::t!("What's new in v%{value1}", value1 = release.version)
        }
        ReleaseNotesKind::Available => {
            rust_i18n::t!("Available release · v%{version}", version = release.version)
        }
    });
    let changes = release
        .note_blocks
        .iter()
        .filter(|block| matches!(block, DocumentBlock::ListItem { .. }))
        .count();
    let published = release.published_at.as_deref().map(release_date);
    card.summary.set_text(&match (changes, published) {
        (0, None) => crate::i18n::tr("Release notes"),
        (0, Some(date)) => rust_i18n::t!("Published %{date}", date = date).into_owned(),
        (count, None) => crate::i18n::count("changes", count),
        (count, Some(date)) => rust_i18n::t!(
            "%{changes} · published %{date}",
            changes = crate::i18n::count("changes", count),
            date = date
        )
        .into_owned(),
    });
    if release.kind == BuildKind::Stable {
        card.badge.set_visible(false);
    } else {
        card.badge.set_text(&release.kind.localized_label());
        card.badge.set_visible(true);
    }
    if release.notes.trim().is_empty() {
        set_release_notes_message(
            &card.notes,
            &crate::i18n::tr("No release notes were provided for this release."),
        );
    } else {
        set_release_note_blocks(&card.notes, &release.note_blocks);
    }
    card.fallback.set_uri(&release.url);
    card.fallback.set_visible(true);
}

fn load_current_release_notes(card: &ReleaseNotesCard) {
    let receiver = services::fetch_release_notes(crate::build_info::RELEASE_TAG);
    let card = card.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        match receiver.try_recv() {
            Ok(ReleaseNotes::Found(release)) => {
                show_release_notes(&card, &release);
                glib::ControlFlow::Break
            }
            Ok(ReleaseNotes::Unavailable { url }) => {
                card.summary
                    .set_text(&crate::i18n::tr("Release notes unavailable for this build"));
                set_release_notes_message(
                    &card.notes,
                    &crate::i18n::tr(
                        "Release notes are unavailable because this version’s tag was not found.",
                    ),
                );
                card.fallback.set_uri(&url);
                card.fallback.set_visible(true);
                glib::ControlFlow::Break
            }
            Ok(ReleaseNotes::Failed { message, url }) => {
                card.summary
                    .set_text(&crate::i18n::tr("Couldn’t load release notes"));
                set_release_notes_message(
                    &card.notes,
                    &rust_i18n::t!("Couldn’t load release notes: %{message}", message = message),
                );
                card.fallback.set_uri(&url);
                card.fallback.set_visible(true);
                glib::ControlFlow::Break
            }
            Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => {
                card.summary
                    .set_text(&crate::i18n::tr("Couldn’t load release notes"));
                set_release_notes_message(
                    &card.notes,
                    &crate::i18n::tr(
                        "Couldn’t load release notes because the request ended unexpectedly.",
                    ),
                );
                glib::ControlFlow::Break
            }
        }
    });
}

/// Whether a check's result -- issued under `result_generation` -- has been
/// superseded by a newer check, whose generation is `current_generation`.
///
/// A toggle mid-check must start a fresh check rather than being silently
/// dropped (see `run_check`'s doc comment), which means an older check's
/// result can still land after a newer one has already started or even
/// finished. Applying that stale result regardless is exactly how a Preview
/// fetch in flight when the user flips back to Stable could still offer an
/// RC to a Stable user: the result carries no channel of its own, so
/// nothing but generation order distinguishes it from a current one.
fn managed_install_row(managed: &ManagedInstall) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
    row.add_css_class("settings-option");
    let title = gtk::Label::new(Some(&crate::i18n::tr("Package-managed installation")));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let description = gtk::Label::new(Some(&managed_install_summary(managed)));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.set_selectable(true);
    description.add_css_class("settings-option-description");
    row.append(&title);
    row.append(&description);
    row
}

fn managed_install_summary(managed: &ManagedInstall) -> String {
    let mut lines = vec![managed.ownership_summary()];
    if let Some(channel) = managed_channel_label(managed) {
        lines.push(
            rust_i18n::t!(
                "Tracking the %{channel} release channel.",
                channel = channel
            )
            .into_owned(),
        );
    }
    lines.push(managed.update_instruction());
    lines.extend(managed.alternate_instruction());
    lines.join("\n")
}

fn is_stale_check(result_generation: u64, current_generation: u64) -> bool {
    result_generation != current_generation
}

struct PendingInstall {
    kind: BuildKind,
    returns_to_stable: bool,
    request: InstallRequest,
}

/// Whether an offer for a `kind` build may still be installed by a user now
/// on `channel`.
///
/// `is_stale_check` only covers a check whose *result* has not landed yet,
/// and only within the one row that started it. This covers the other half:
/// an offer that already landed and is sitting in a window's "Install
/// update" button or an open update dialog. The channel preference is
/// process-wide ([`PreferenceManager::shared`]), so switching back to Stable in
/// one window leaves every other window holding a cached RC offer it would
/// otherwise happily install. Re-testing at the moment of the click is what
/// makes the preference authoritative regardless of how many views cached
/// an offer under the old one.
fn effective_update_channel(selected: Channel, update_method: UpdateMethod) -> Channel {
    match update_method {
        UpdateMethod::InPlace | UpdateMethod::Aur => selected,
        UpdateMethod::Omarchy | UpdateMethod::Pacman => Channel::Stable,
    }
}

fn offer_still_eligible(channel: Channel, kind: BuildKind) -> bool {
    match channel {
        Channel::Stable => kind == BuildKind::Stable,
        Channel::Preview => kind != BuildKind::Nightly,
        Channel::Nightly => true,
    }
}

fn update_check_row(
    manager: Rc<PreferenceManager>,
    available_notes: ReleaseNotesCard,
    install_guard: InstallGuard,
    update_method: UpdateMethod,
) -> UpdateCheckRow {
    update_check_row_with(
        manager,
        available_notes,
        install_guard,
        update_method,
        default_install_launcher(),
    )
}

fn update_check_row_with(
    manager: Rc<PreferenceManager>,
    available_notes: ReleaseNotesCard,
    install_guard: InstallGuard,
    update_method: UpdateMethod,
    launcher: InstallLauncher,
) -> UpdateCheckRow {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    row.add_css_class("settings-option");
    row.add_css_class("settings-update-status");
    search::tag(&row, "Check for updates");
    let summary = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    summary.add_css_class("settings-update-summary");
    summary.set_vexpand(true);
    row.set_vexpand(false);
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(&crate::i18n::tr("Check for updates")));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let status = gtk::Label::new(Some(&installed_version_status(
        &crate::build_info::installed_version(),
        crate::build_info::build_kind(),
        update_method,
    )));
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.set_use_markup(true);
    status.add_css_class("settings-option-description");
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let status_icon = crate::assets::primary_icon(icons::INFO, 18);
    status_icon.set_valign(gtk::Align::Center);
    title.set_valign(gtk::Align::Center);
    heading.append(&status_icon);
    heading.append(&title);
    copy.append(&heading);
    copy.append(&status);
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("settings-update-progress");
    progress.set_hexpand(true);
    progress.set_visible(false);
    copy.append(&progress);
    let button = gtk::Button::with_label(&crate::i18n::tr("Check now"));
    button.add_css_class("settings-update-check");
    button.set_valign(gtk::Align::Center);
    summary.append(&copy);
    summary.append(&button);
    row.append(&summary);
    available_notes.container.add_css_class("inline");
    available_notes.container.set_visible(false);
    row.append(&available_notes.container);

    let checking = Rc::new(Cell::new(false));
    let row_generation = Rc::new(Cell::new(0u64));
    // Set once a check finds an update this platform can install; consumed by the
    // button's next click instead of re-running a check.
    let pending_download = Rc::new(RefCell::new(None::<PendingInstall>));
    // Set once an install finishes, so the next click restarts instead of re-checking.
    let installed = Rc::new(Cell::new(false));
    let installing = Rc::new(Cell::new(false));
    let cancel_handle = Rc::new(RefCell::new(None::<InstallCancel>));
    let install_underway: Rc<dyn Fn() -> bool> = Rc::new({
        let installed = installed.clone();
        let installing = installing.clone();
        move || installed.get() || installing.get()
    });
    let managed_update_available = Rc::new(Cell::new(false));
    let run_check: Rc<dyn Fn(bool)> = Rc::new({
        let title = title.clone();
        let status_icon = status_icon.clone();
        let checking = checking.clone();
        let status = status.clone();
        let button = button.clone();
        let pending_download = pending_download.clone();
        let installed = installed.clone();
        let managed_update_available = managed_update_available.clone();
        let progress = progress.clone();
        let available_notes = available_notes.clone();
        let manager = manager.clone();
        let row_generation = row_generation.clone();
        move |force: bool| {
            // Always start a fresh check rather than dropping it: a channel
            // toggle must never be silently ignored just because a previous
            // check (for the old channel) is still in flight. The stale
            // check's own result is discarded below instead, once its
            // generation no longer matches.
            let my_generation = next_check_generation();
            let my_row_generation = row_generation.get().saturating_add(1);
            row_generation.set(my_row_generation);
            checking.set(true);
            *pending_download.borrow_mut() = None;
            installed.set(false);
            managed_update_available.set(false);
            button.set_label(&crate::i18n::tr("Check now"));
            progress.set_fraction(0.0);
            progress.set_visible(false);
            progress.remove_css_class("error");
            title.set_text(&crate::i18n::tr("Checking for updates…"));
            status.set_text(&crate::i18n::tr("Checking for updates…"));
            available_notes.container.set_visible(false);
            available_notes.fallback.set_visible(false);
            // Clear any previously offered release immediately, not only once
            // this check's own result lands: otherwise the sidebar keeps
            // showing a (possibly prerelease) offer from before the channel
            // was switched for the whole duration of this check.
            publish_update_notice(None);
            button.set_sensitive(false);
            // Read the channel now, not once when the row was built: a
            // mid-session channel toggle must be reflected by the very next
            // check, including this one if it was triggered by that toggle.
            let channel = effective_update_channel(manager.release_channel(), update_method);
            let receiver = services::check_for_updates(
                channel,
                crate::build_info::installed_version(),
                update_method,
                force,
            );
            let title = title.clone();
            let status_icon = status_icon.clone();
            let checking = checking.clone();
            let status = status.clone();
            let button = button.clone();
            let pending_download = pending_download.clone();
            let managed_update_available = managed_update_available.clone();
            let available_notes = available_notes.clone();
            let row_generation = row_generation.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                if is_stale_check(my_generation, CHECK_GENERATION.get()) {
                    // Only a newer check on this row owns its disabled button.
                    if is_stale_check(my_row_generation, row_generation.get()) {
                        return glib::ControlFlow::Break;
                    }
                    button.set_sensitive(true);
                    checking.set(false);
                    return glib::ControlFlow::Break;
                }
                match receiver.try_recv() {
                    Ok(result) => {
                        CHECK_IN_FLIGHT.set(false);
                        LAST_COMPLETED_CHECK.set(Some(Instant::now()));
                        crate::assets::set_primary_icon(
                            &status_icon,
                            match &result {
                                UpdateCheck::UpToDate => icons::CIRCLE_CHECK,
                                UpdateCheck::Available { .. } => icons::DOWNLOADS,
                                UpdateCheck::Failed(_) => icons::TRIANGLE_ALERT,
                            },
                        );
                        title.set_text(&crate::i18n::tr(match &result {
                            UpdateCheck::UpToDate => "Strata is up to date",
                            UpdateCheck::Available { .. } => "An update is available",
                            UpdateCheck::Failed(_) => "Couldn’t check for updates",
                        }));
                        let returns_to_stable = matches!(
                            &result,
                            UpdateCheck::Available { release, .. }
                                if channel == Channel::Stable
                                    && crate::build_info::build_kind() != BuildKind::Stable
                                    && release.kind == BuildKind::Stable
                        );
                        let message = if returns_to_stable
                            && matches!(update_method, UpdateMethod::InPlace | UpdateMethod::Aur)
                        {
                            let UpdateCheck::Available { release, .. } = &result else {
                                unreachable!();
                            };
                            rust_i18n::t!(
                                "Stable channel target: <a href=\"%{value1}\">v%{value2}</a>",
                                value1 = glib::markup_escape_text(&release.url),
                                value2 = glib::markup_escape_text(&release.version)
                            )
                            .into_owned()
                        } else {
                            update_check_message(&result, update_method)
                        };
                        status.set_markup(&update_status_markup(
                            message,
                            &result,
                            InstallSource::detect(),
                        ));
                        available_notes
                            .container
                            .set_visible(shows_available_release_notes(&result));
                        match &result {
                            UpdateCheck::Available { release, install } => publish_update_notice(
                                Some((release.clone(), install.clone(), update_method)),
                            ),
                            UpdateCheck::UpToDate | UpdateCheck::Failed(_) => {
                                publish_update_notice(None)
                            }
                        }
                        match &result {
                            UpdateCheck::Available { release, install } => {
                                show_release_notes(&available_notes, release);
                                if update_method.is_package_managed() {
                                    managed_update_available.set(true);
                                    button.set_label(&crate::i18n::tr(match update_method {
                                        UpdateMethod::Aur => aur_update_action_label(),
                                        UpdateMethod::Omarchy => "Open Omarchy Update",
                                        UpdateMethod::Pacman => "Check again",
                                        UpdateMethod::InPlace => unreachable!(),
                                    }));
                                } else {
                                    *pending_download.borrow_mut() = Some(PendingInstall {
                                        kind: release.kind,
                                        returns_to_stable,
                                        request: install.clone(),
                                    });
                                    button.set_label(&crate::i18n::tr(if returns_to_stable {
                                        "Return to stable"
                                    } else {
                                        "Install update"
                                    }));
                                }
                            }
                            UpdateCheck::UpToDate | UpdateCheck::Failed(_) => {}
                        }
                        button.set_sensitive(true);
                        checking.set(false);
                        glib::ControlFlow::Break
                    }
                    Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(TryRecvError::Disconnected) => {
                        CHECK_IN_FLIGHT.set(false);
                        title.set_text(&crate::i18n::tr("Couldn’t check for updates"));
                        crate::assets::set_primary_icon(&status_icon, icons::TRIANGLE_ALERT);
                        status.set_markup(&crate::i18n::tr(
                            "Couldn't check for updates · <a href=\"https://github.com/lgse/strata/releases/latest\">View releases on GitHub</a>",
                        ));
                        available_notes.container.set_visible(false);
                        button.set_sensitive(true);
                        checking.set(false);
                        glib::ControlFlow::Break
                    }
                }
            });
        }
    });

    let clicked_check = run_check.clone();
    let row_status = status.clone();
    let row_pending_download = pending_download.clone();
    button.connect_clicked(move |button| {
        if update_method == UpdateMethod::Aur && managed_update_available.get() {
            match launch_aur_update() {
                Ok(message) => status.set_text(&message),
                Err(error) => status.set_text(&rust_i18n::t!(
                    "Couldn’t open AUR update: %{error}",
                    error = error
                )),
            }
            return;
        }
        if update_method == UpdateMethod::Omarchy && managed_update_available.get() {
            match launch_omarchy_update() {
                Ok(()) => {
                    status.set_text(&crate::i18n::tr("Omarchy Update opened in your terminal."))
                }
                Err(error) => status.set_text(&rust_i18n::t!(
                    "Couldn’t open Omarchy Update: %{error}",
                    error = error
                )),
            }
            return;
        }
        if installed.get() {
            restart_application(button);
            return;
        }
        if let Some(cancel) = cancel_handle.borrow_mut().take() {
            status.set_text(&crate::i18n::tr(if cancel.cancel() {
                "Cancelling…"
            } else {
                FINALIZING_STATUS
            }));
            button.set_sensitive(false);
            return;
        }
        // Both branches can reborrow this cell; an `if let` scrutinee would retain the RefMut.
        let pending = pending_download.take();
        if let Some(pending) = pending {
            if !offer_still_eligible(manager.release_channel(), pending.kind) {
                // The channel was switched back to Stable -- possibly from
                // another window, which this row never hears about -- after
                // this offer was cached. Drop it and re-check rather than
                // installing a prerelease the user has since opted out of;
                // `clicked_check` clears the sidebar notice and relabels the
                // button on its way.
                clicked_check(true);
                return;
            }
            let PendingInstall {
                kind: offered_kind,
                returns_to_stable,
                request,
            } = pending;
            if checking.replace(true) {
                return;
            }
            installing.set(true);
            status.set_text(&crate::i18n::tr(if returns_to_stable {
                "Downloading stable release…"
            } else {
                "Downloading update…"
            }));
            progress.set_fraction(0.0);
            progress.set_visible(true);
            progress.remove_css_class("error");
            button.set_label(&crate::i18n::tr("Cancel"));
            let progress_for_progress = progress.clone();
            let status_for_progress = status.clone();
            let button_for_progress = button.clone();
            let checking_for_installed = checking.clone();
            let status_for_installed = status.clone();
            let button_for_installed = button.clone();
            let installed_for_installed = installed.clone();
            let installing_for_failed = installing.clone();
            let checking_for_failed = checking.clone();
            let status_for_failed = status.clone();
            let button_for_failed = button.clone();
            let progress_for_failed = progress.clone();
            let cancel_for_installed = cancel_handle.clone();
            let cancel_for_cancelled = cancel_handle.clone();
            let cancel_for_failed = cancel_handle.clone();
            let checking_for_cancelled = checking.clone();
            let installing_for_cancelled = installing.clone();
            let status_for_cancelled = status.clone();
            let button_for_cancelled = button.clone();
            let progress_for_cancelled = progress.clone();
            let started = start_install(
                &install_guard,
                request,
                &launcher,
                move |event| {
                    if matches!(event, InstallProgress::Finalizing) {
                        button_for_progress.set_sensitive(false);
                    }
                    apply_install_progress(&status_for_progress, &progress_for_progress, event)
                },
                move || {
                    let _taken = cancel_for_installed.borrow_mut().take();
                    status_for_installed.set_text(&crate::i18n::tr(if returns_to_stable {
                        "Stable release installed — restart to apply"
                    } else {
                        "Update installed — restart to apply"
                    }));
                    button_for_installed.set_label(&crate::i18n::tr("Restart now"));
                    button_for_installed.set_sensitive(true);
                    installed_for_installed.set(true);
                    checking_for_installed.set(false);
                    restart_application(&button_for_installed);
                },
                move || {
                    let _taken = cancel_for_cancelled.borrow_mut().take();
                    status_for_cancelled.set_text(&crate::i18n::tr("Update cancelled"));
                    progress_for_cancelled.set_visible(false);
                    button_for_cancelled.set_label(&crate::i18n::tr("Check now"));
                    button_for_cancelled.set_sensitive(true);
                    checking_for_cancelled.set(false);
                    installing_for_cancelled.set(false);
                },
                move |message| {
                    let _taken = cancel_for_failed.borrow_mut().take();
                    match message {
                        Some(message) => status_for_failed.set_text(&rust_i18n::t!(
                            "Couldn't install update: %{message}",
                            message = crate::services::error_detail(message)
                        )),
                        None => {
                            status_for_failed.set_text(&crate::i18n::tr("Couldn't install update"))
                        }
                    }
                    progress_for_failed.add_css_class("error");
                    button_for_failed.set_label(&crate::i18n::tr("Check now"));
                    button_for_failed.set_sensitive(true);
                    checking_for_failed.set(false);
                    installing_for_failed.set(false);
                },
            );
            let started = match started {
                Ok(cancel) => {
                    *cancel_handle.borrow_mut() = Some(cancel);
                    Ok(())
                }
                Err(request) => Err(request),
            };
            if let Err(request) = started {
                // An install from an update dialog or another window is
                // already running. Leave this row
                // re-triable rather than stuck mid-"downloading" with
                // nothing actually happening.
                status.set_text(&crate::i18n::tr(
                    "Another install is already running — try again shortly.",
                ));
                progress.set_visible(false);
                button.set_label(&crate::i18n::tr(if returns_to_stable {
                    "Return to stable"
                } else {
                    "Install update"
                }));
                button.set_sensitive(true);
                checking.set(false);
                installing.set(false);
                *pending_download.borrow_mut() = Some(PendingInstall {
                    kind: offered_kind,
                    returns_to_stable,
                    request,
                });
            }
        } else {
            clicked_check(true);
        }
    });
    UpdateCheckRow {
        row,
        run_check,
        responsive_action: (summary, button),
        install_underway,
        status: row_status,
        pending_download: row_pending_download,
    }
}

/// The three non-terminal states [`drive_install`] reports through
/// `on_progress`. Keeping this separate from [`UpdateInstall`] means callers
/// never need to (incorrectly) handle `Installed`/`Failed` in that closure --
/// those terminal states are always reported through the driver's other two
/// callbacks instead.
enum InstallProgress {
    Downloading { downloaded: u64, total: Option<u64> },
    Verifying,
    Installing,
    Finalizing,
}

const FINALIZING_STATUS: &str = "Finalizing update…";

/// Drives an install `receiver` on the GTK main loop until it reports a
/// terminal outcome, then stops.
///
/// This is the shared shape behind `update_check_row`'s and
/// `show_update_dialog`'s install flows: poll `receiver` every 100ms, forward
/// non-terminal updates to `on_progress`, and invoke exactly one of
/// `on_installed`/`on_failed` once a terminal state is reached. Deliberately
/// does *not* format status text itself because the two call sites use
/// different wording. `on_failed` receives `Some(message)` for an explicit
/// [`UpdateInstall::Failed`] or `None` when the receiver disconnected
/// without ever reporting one, since callers render those two cases
/// differently too.
fn drive_install(
    receiver: std::sync::mpsc::Receiver<UpdateInstall>,
    on_progress: impl Fn(InstallProgress) + 'static,
    on_installed: impl Fn() + 'static,
    on_cancelled: impl Fn() + 'static,
    on_failed: impl Fn(Option<String>) + 'static,
) {
    glib::timeout_add_local(Duration::from_millis(100), move || {
        loop {
            match receiver.try_recv() {
                Ok(UpdateInstall::Downloading { downloaded, total }) => {
                    on_progress(InstallProgress::Downloading { downloaded, total });
                }
                Ok(UpdateInstall::Verifying) => on_progress(InstallProgress::Verifying),
                Ok(UpdateInstall::Installing) => on_progress(InstallProgress::Installing),
                Ok(UpdateInstall::Finalizing) => on_progress(InstallProgress::Finalizing),
                Ok(UpdateInstall::Installed) => {
                    on_installed();
                    return glib::ControlFlow::Break;
                }
                Ok(UpdateInstall::Cancelled) => {
                    on_cancelled();
                    return glib::ControlFlow::Break;
                }
                Ok(UpdateInstall::Failed(message)) => {
                    on_failed(Some(message));
                    return glib::ControlFlow::Break;
                }
                Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(TryRecvError::Disconnected) => {
                    on_failed(None);
                    return glib::ControlFlow::Break;
                }
            }
        }
    });
}

/// The shared guard prevents concurrent replacements from different windows.
/// Guard rejection leaves the request unused and must keep the caller retryable.
fn start_install(
    guard: &InstallGuard,
    request: InstallRequest,
    launcher: &InstallLauncher,
    on_progress: impl Fn(InstallProgress) + 'static,
    on_installed: impl Fn() + 'static,
    on_cancelled: impl Fn() + 'static,
    on_failed: impl Fn(Option<String>) + 'static,
) -> Result<InstallCancel, InstallRequest> {
    if guard.replace(true) {
        return Err(request);
    }
    let cancel = InstallCancel::new();
    let receiver = launcher(request, cancel.clone());
    let guard_for_installed = guard.clone();
    let guard_for_cancelled = guard.clone();
    let guard_for_failed = guard.clone();
    drive_install(
        receiver,
        on_progress,
        move || {
            guard_for_installed.set(false);
            on_installed();
        },
        move || {
            guard_for_cancelled.set(false);
            on_cancelled();
        },
        move |message| {
            guard_for_failed.set(false);
            on_failed(message);
        },
    );
    Ok(cancel)
}

/// The update row's compact progress rendering, distinct from the update
/// dialog's dialog-specific wording.
fn apply_install_progress(
    status: &gtk::Label,
    progress: &gtk::ProgressBar,
    event: InstallProgress,
) {
    match event {
        InstallProgress::Downloading { downloaded, total } => {
            if let Some(total) = total.filter(|total| *total > 0) {
                let fraction = (downloaded as f64 / total as f64).clamp(0.0, 1.0);
                progress.set_fraction(fraction);
                status.set_text(&rust_i18n::t!(
                    "Downloading update… %{value1}%",
                    value1 = format!("{:.0}", fraction * 100.0)
                ));
            } else {
                progress.pulse();
                status.set_text(&rust_i18n::t!(
                    "Downloading update… %{value1} MB",
                    value1 = crate::i18n::decimal(downloaded as f64 / 1_048_576.0, 1)
                ));
            }
        }
        InstallProgress::Verifying => status.set_text(&crate::i18n::tr("Verifying update…")),
        InstallProgress::Installing => {
            progress.set_fraction(1.0);
            status.set_text(&crate::i18n::tr("Installing update…"));
        }
        InstallProgress::Finalizing => {
            progress.set_fraction(1.0);
            status.set_text(&crate::i18n::tr(FINALIZING_STATUS));
        }
    }
}

/// Relaunches the (just-updated) executable and quits the current instance.
fn restart_application(button: &gtk::Button) {
    let application = button
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok())
        .and_then(|window| window.application());
    restart(application.as_ref());
}

fn process_start_time(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // comm (field 2) may contain spaces or parentheses; the remaining fields
    // begin after its final closing parenthesis. starttime is field 22.
    stat.rsplit_once(") ")?
        .1
        .split_whitespace()
        .nth(19)
        .map(str::to_owned)
}

fn restart_waiter(current_exe: &std::path::Path, parent_pid: u32) -> Option<Command> {
    use std::{os::unix::process::CommandExt, process::Stdio};

    let mut command = crate::trusted_command::command("sh").ok()?;
    let sleep = crate::trusted_command::resolve("sleep").ok()?;
    let start_time = process_start_time(parent_pid).unwrap_or_default();
    let date = crate::trusted_command::resolve("date").ok()?;
    let mv = crate::trusted_command::resolve("mv").ok()?;
    let rollback = current_exe.parent().map(services::rollback_path)?;
    command
        .args([
            "-c",
            "pid=$1; binary=$2; sleeper=$3; born=$4; rollback=$5; clock=$6; mover=$7; grace=$8; while kill -0 \"$pid\" 2>/dev/null && [ -r \"/proc/$pid/stat\" ]; do IFS= read -r stat < \"/proc/$pid/stat\" || break; rest=${stat##*) }; set -- $rest; shift 19; [ \"${1:-}\" = \"$born\" ] || break; \"$sleeper\" 0.1; done; \"$sleeper\" 0.5; started=$(\"$clock\" +%s); \"$binary\" && exit 0; status=$?; [ $(($(\"$clock\" +%s) - started)) -lt \"$grace\" ] || exit \"$status\"; [ -f \"$rollback\" ] || exit \"$status\"; \"$mover\" -f -- \"$rollback\" \"$binary\" && exec \"$binary\"; exit \"$status\"",
            "strata-restart",
        ])
        .arg(parent_pid.to_string())
        .arg(current_exe)
        .arg(sleep)
        .arg(start_time)
        .arg(rollback)
        .arg(date)
        .arg(mv)
        .arg(RESTART_GRACE_SECONDS.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    if std::env::var_os(crate::CAIRO_SELECTED_BY_STRATA).as_deref()
        == Some(std::ffi::OsStr::new("1"))
        && std::env::var_os("GSK_RENDERER").as_deref() == Some(std::ffi::OsStr::new("cairo"))
    {
        // The next process must read the saved renderer, not inherit our default.
        command.env_remove("GSK_RENDERER");
        command.env_remove(crate::CAIRO_SELECTED_BY_STRATA);
    }
    Some(command)
}

/// A closed window may no longer have an application; never exit the process in that case.
fn restart(application: Option<&gtk::Application>) {
    let Some(application) = application else {
        return;
    };
    let Ok(current_exe) = crate::services::installed_executable() else {
        return;
    };
    // Wait for this process to exit completely before relaunching. A fixed
    // delay could overlap the old and new GTK/Wayland clients and rapidly hand
    // keyboard focus through an underlying terminal. Besides re-activating the
    // old GApplication instance, that exposed a Foot/libxkbcommon crash on
    // affected systems. Detach the waiter from inherited terminal streams and
    // put it in its own process group so applying an update cannot disturb the
    // terminal that launched Strata.
    let Some(mut waiter) = restart_waiter(&current_exe, std::process::id()) else {
        return;
    };
    if restart_blocker_shown(application) {
        return;
    }
    let Ok(mut waiter) = waiter.spawn() else {
        return;
    };
    // Do not leave a waiter behind when an unregistered close handler refuses.
    if !close_windows_for_restart(application) {
        let _ = waiter.kill();
        let _ = waiter.wait();
        return;
    }
    application.quit();
}

/// Checks every window before closing any, so a refusal cannot leave some closed.
fn restart_blocker_shown(application: &gtk::Application) -> bool {
    let Some((window, blocker)) = crate::ui::close_guard::application_blocker(application) else {
        return false;
    };
    window.present();
    blocker.show(&window);
    true
}

fn close_windows_for_restart(application: &gtk::Application) -> bool {
    if restart_blocker_shown(application) {
        return false;
    }
    for window in application.windows() {
        window.close();
    }
    application.windows().is_empty()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateDialogPhase {
    Ready,
    Downloading,
    Finalizing,
    Cancelled,
    Failed,
    Installed,
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "tests drive the dialog through these handles")
)]
struct UpdateDialog {
    layer: gtk::Box,
    cancel: gtk::Button,
    close: gtk::Button,
    action: gtk::Button,
    status: gtk::Label,
    escape: gtk::EventControllerKey,
}

pub(super) fn show_update_dialog(
    parent: &gtk::Window,
    release: &ReleaseMetadata,
    install: InstallRequest,
    install_guard: InstallGuard,
    update_method: UpdateMethod,
) {
    let _dialog = build_update_dialog(
        parent,
        release,
        install,
        install_guard,
        update_method,
        default_install_launcher(),
    );
}

fn build_update_dialog(
    parent: &gtk::Window,
    release: &ReleaseMetadata,
    install: InstallRequest,
    install_guard: InstallGuard,
    update_method: UpdateMethod,
    launcher: InstallLauncher,
) -> Option<UpdateDialog> {
    let window_overlay = parent.child().and_downcast::<gtk::Overlay>()?;
    let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
    if let Some(root) = blurred_root.as_ref() {
        root.set_blurred(true);
    }

    let aur_action = aur_update_action_label();
    let layout = modal_layout(
        icons::DOWNLOADS,
        &rust_i18n::t!("Strata v%{value1} is available", value1 = release.version),
        &rust_i18n::t!(
            "Installed v%{value1}  →  Available v%{value2}",
            value1 = crate::build_info::installed_version(),
            value2 = release.version
        ),
        &crate::i18n::tr(match update_method {
            UpdateMethod::InPlace => "Download update",
            UpdateMethod::Aur => aur_action,
            UpdateMethod::Omarchy => "Open Omarchy Update",
            UpdateMethod::Pacman => "Close",
        }),
    );
    layout.content.add_css_class("update-dialog");
    layout.content.set_size_request(560, -1);
    // A prerelease offer must be visibly labelled, and must let the user
    // confirm exactly what they are about to install before doing so: which
    // channel it is, its precise tag, the source commit, and when it was
    // published.
    if release.kind != BuildKind::Stable {
        let badge = gtk::Label::new(Some(&release.kind.localized_label()));
        badge.add_css_class("prerelease-badge");
        badge.set_xalign(0.0);
        badge.set_halign(gtk::Align::Start);
        layout.body.append(&badge);
        layout.body.append(&update_dialog_details(release));
    }
    let notes_heading = gtk::Label::new(Some(&crate::i18n::tr("What’s new")));
    notes_heading.add_css_class("release-notes-title");
    notes_heading.set_xalign(0.0);
    let notes = gtk::Box::new(gtk::Orientation::Vertical, 6);
    if release.notes.trim().is_empty() {
        set_release_notes_message(
            &notes,
            &crate::i18n::tr(
                "No release notes were provided. Review this release on GitHub before continuing.",
            ),
        );
    } else {
        set_release_note_blocks(&notes, &release.note_blocks);
    }
    let notes_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(120)
        .max_content_height(300)
        .propagate_natural_height(true)
        .child(&notes)
        .build();
    notes_scroll.add_css_class("update-dialog-notes");
    let fallback =
        gtk::LinkButton::with_label(&release.url, &crate::i18n::tr("View release on GitHub"));
    fallback.set_has_tooltip(false);
    fallback.add_css_class("release-notes-fallback");
    fallback.set_halign(gtk::Align::Start);
    let status_message = match update_method {
        UpdateMethod::InPlace => {
            crate::i18n::tr("Review the release notes before downloading the update.")
        }
        UpdateMethod::Aur => InstallSource::detect()
            .managed()
            .map(update_dialog_status)
            .unwrap_or_else(|| {
                crate::i18n::tr("This installation is managed by its package manager.")
            }),
        UpdateMethod::Omarchy => crate::i18n::tr(
            "This installation is managed by Omarchy. Run “omarchy update” to install it.",
        ),
        UpdateMethod::Pacman => crate::i18n::tr(
            "This installation is managed by pacman. Install it through a full system update.",
        ),
    };
    let status = gtk::Label::new(Some(&status_message));
    status.add_css_class("update-dialog-status");
    status.set_xalign(0.0);
    status.set_wrap(true);
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("update-dialog-progress");
    progress.set_fraction(0.0);
    progress.set_visible(false);
    layout.body.append(&notes_heading);
    layout.body.append(&notes_scroll);
    layout.body.append(&fallback);
    layout.body.append(&status);
    layout.body.append(&progress);
    let content = layout.content;
    let close = layout.close;
    let cancel = layout.cancel;
    let action = layout.confirm;

    let phase = Rc::new(Cell::new(UpdateDialogPhase::Ready));
    let cancel_handle = Rc::new(RefCell::new(None::<InstallCancel>));
    let set_dismissible: Rc<dyn Fn(bool)> = Rc::new({
        let cancel = cancel.clone();
        let close = close.clone();
        move |enabled| {
            cancel.set_sensitive(enabled);
            close.set_sensitive(enabled);
        }
    });
    let enter_finalizing: Rc<dyn Fn()> = Rc::new({
        let phase = phase.clone();
        let status = status.clone();
        let progress = progress.clone();
        let set_dismissible = set_dismissible.clone();
        move || {
            phase.set(UpdateDialogPhase::Finalizing);
            progress.set_fraction(1.0);
            status.set_text(&crate::i18n::tr(FINALIZING_STATUS));
            set_dismissible(false);
        }
    });
    let close_dialog: Rc<dyn Fn(&gtk::Box)> = Rc::new({
        let phase = phase.clone();
        let cancel_handle = cancel_handle.clone();
        let enter_finalizing = enter_finalizing.clone();
        let overlay = window_overlay.clone();
        let root = blurred_root.clone();
        move |layer| {
            match phase.get() {
                UpdateDialogPhase::Finalizing => return,
                UpdateDialogPhase::Downloading => {
                    let committed = cancel_handle
                        .borrow()
                        .as_ref()
                        .is_some_and(|handle| !handle.cancel());
                    if committed {
                        // The installer can commit before its Finalizing event reaches the UI.
                        enter_finalizing();
                        return;
                    }
                    cancel_handle.take();
                    phase.set(UpdateDialogPhase::Cancelled);
                }
                UpdateDialogPhase::Ready
                | UpdateDialogPhase::Cancelled
                | UpdateDialogPhase::Failed
                | UpdateDialogPhase::Installed => {}
            }
            dismiss_modal_layer(layer, &overlay, root.as_ref());
        }
    });
    let layer = modal_layer_with_backdrop(&content, close_dialog.clone());
    window_overlay.add_overlay(&layer);
    action.grab_focus();

    for button in [&cancel, &close] {
        let close_dialog = close_dialog.clone();
        let layer = layer.clone();
        button.connect_clicked(move |_| close_dialog(&layer));
    }
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed({
        let close_dialog = close_dialog.clone();
        let layer = layer.clone();
        move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                close_dialog(&layer);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    });
    layer.add_controller(escape.clone());

    // Set when the offer this dialog was opened with is no longer eligible on
    // the current channel, which turns the action button into a plain Close.
    let withdrawn = Rc::new(Cell::new(false));
    let offered_kind = release.kind;
    let withdraw: Rc<dyn Fn()> = Rc::new({
        let withdrawn = withdrawn.clone();
        let status = status.clone();
        let action = action.clone();
        move || {
            withdrawn.set(true);
            status.set_text(&crate::i18n::tr(
                "This build is no longer offered on your update channel — check for updates again.",
            ));
            action.set_label(&crate::i18n::tr("Close"));
        }
    });
    PreferenceManager::shared().on_release_channel_changed(&layer, {
        let withdraw = withdraw.clone();
        let withdrawn = withdrawn.clone();
        let phase = phase.clone();
        Rc::new(move || {
            if phase.get() != UpdateDialogPhase::Ready || withdrawn.get() {
                return;
            }
            if !offer_still_eligible(PreferenceManager::shared().release_channel(), offered_kind) {
                withdraw();
            }
        })
    });
    let action_layer = layer.clone();
    let action_overlay = window_overlay.clone();
    let action_root = blurred_root.clone();
    let application = parent.application();
    let dialog_status = status.clone();
    action.connect_clicked(move |button| {
        if update_method == UpdateMethod::Aur {
            if aur_action == "Close" {
                dismiss_modal_layer(&action_layer, &action_overlay, action_root.as_ref());
                button.set_sensitive(false);
                return;
            }
            match launch_aur_update() {
                Ok(_) => {
                    dismiss_modal_layer(&action_layer, &action_overlay, action_root.as_ref());
                    button.set_sensitive(false);
                }
                Err(error) => status.set_text(&rust_i18n::t!(
                    "Couldn’t open AUR update: %{error}",
                    error = error
                )),
            }
            return;
        }
        if update_method == UpdateMethod::Omarchy {
            match launch_omarchy_update() {
                Ok(()) => {
                    dismiss_modal_layer(&action_layer, &action_overlay, action_root.as_ref());
                    button.set_sensitive(false);
                }
                Err(error) => status.set_text(&rust_i18n::t!(
                    "Couldn’t open Omarchy Update: %{error}",
                    error = error
                )),
            }
            return;
        }
        if update_method == UpdateMethod::Pacman {
            dismiss_modal_layer(&action_layer, &action_overlay, action_root.as_ref());
            button.set_sensitive(false);
            return;
        }
        match phase.get() {
            UpdateDialogPhase::Installed => {
                restart(application.as_ref());
                button.set_sensitive(false);
                return;
            }
            UpdateDialogPhase::Failed | UpdateDialogPhase::Cancelled => {
                dismiss_modal_layer(&action_layer, &action_overlay, action_root.as_ref());
                button.set_sensitive(false);
                return;
            }
            UpdateDialogPhase::Downloading | UpdateDialogPhase::Finalizing => return,
            UpdateDialogPhase::Ready => {}
        }
        if withdrawn.get() {
            dismiss_modal_layer(&action_layer, &action_overlay, action_root.as_ref());
            button.set_sensitive(false);
            return;
        }
        // Read the channel at the click, not when the dialog was opened: this
        // dialog is driven by the sidebar notice, whose cached offer survives
        // a channel switch made anywhere in the process -- including in
        // another window. `withdrawn` rather than a phase change so Cancel and
        // Escape keep dismissing normally.
        if !offer_still_eligible(PreferenceManager::shared().release_channel(), offered_kind) {
            withdraw();
            return;
        }

        phase.set(UpdateDialogPhase::Downloading);
        button.set_sensitive(false);
        progress.set_visible(true);
        status.set_text(&crate::i18n::tr("Starting download…"));
        let progress_for_progress = progress.clone();
        let status_for_progress = status.clone();
        let progress_for_installed = progress.clone();
        let status_for_installed = status.clone();
        let action_for_installed = button.clone();
        let phase_for_installed = phase.clone();
        let dismissible_for_installed = set_dismissible.clone();
        let progress_for_failed = progress.clone();
        let status_for_failed = status.clone();
        let action_for_failed = button.clone();
        let phase_for_failed = phase.clone();
        let dismissible_for_failed = set_dismissible.clone();
        let phase_for_progress = phase.clone();
        let enter_finalizing = enter_finalizing.clone();
        let install_guard = install_guard.clone();
        let cancel_for_installed = cancel_handle.clone();
        let cancel_for_cancelled = cancel_handle.clone();
        let cancel_for_failed = cancel_handle.clone();
        let phase_for_cancelled = phase.clone();
        let layer_for_cancelled = action_layer.clone();
        let overlay_for_cancelled = action_overlay.clone();
        let root_for_cancelled = action_root.clone();
        let outcome = start_install(
            &install_guard,
            install.clone(),
            &launcher,
            move |event| match event {
                InstallProgress::Downloading { downloaded, total } => {
                    if let Some(total) = total.filter(|total| *total > 0) {
                        let fraction = (downloaded as f64 / total as f64).clamp(0.0, 1.0);
                        progress_for_progress.set_fraction(fraction);
                        status_for_progress.set_text(&rust_i18n::t!(
                            "Downloading… %{value1}%  (%{value2} of %{value3} MB)",
                            value1 = format!("{:.0}", fraction * 100.0),
                            value2 = crate::i18n::decimal(downloaded as f64 / 1_048_576.0, 1),
                            value3 = crate::i18n::decimal(total as f64 / 1_048_576.0, 1)
                        ));
                    } else {
                        progress_for_progress.pulse();
                        status_for_progress.set_text(&rust_i18n::t!(
                            "Downloading… %{value1} MB",
                            value1 = crate::i18n::decimal(downloaded as f64 / 1_048_576.0, 1)
                        ));
                    }
                }
                InstallProgress::Verifying => {
                    status_for_progress.set_text(&crate::i18n::tr("Verifying update…"))
                }
                InstallProgress::Installing => {
                    progress_for_progress.set_fraction(1.0);
                    status_for_progress.set_text(&crate::i18n::tr("Installing update…"));
                }
                InstallProgress::Finalizing => {
                    if phase_for_progress.get() == UpdateDialogPhase::Downloading {
                        enter_finalizing();
                    }
                }
            },
            move || {
                let _taken = cancel_for_installed.borrow_mut().take();
                phase_for_installed.set(UpdateDialogPhase::Installed);
                progress_for_installed.set_fraction(1.0);
                status_for_installed
                    .set_text(&crate::i18n::tr("Update installed — restart to apply"));
                action_for_installed.set_label(&crate::i18n::tr("Restart now"));
                action_for_installed.add_css_class("suggested-action");
                action_for_installed.set_sensitive(true);
                // Restart can fail; leave a way to close the dialog.
                dismissible_for_installed(true);
                restart_application(&action_for_installed);
            },
            move || {
                let _taken = cancel_for_cancelled.borrow_mut().take();
                phase_for_cancelled.set(UpdateDialogPhase::Cancelled);
                dismiss_modal_layer(
                    &layer_for_cancelled,
                    &overlay_for_cancelled,
                    root_for_cancelled.as_ref(),
                );
            },
            move |message| {
                let _taken = cancel_for_failed.borrow_mut().take();
                phase_for_failed.set(UpdateDialogPhase::Failed);
                dismissible_for_failed(true);
                match message {
                    Some(message) => {
                        status_for_failed.set_text(&rust_i18n::t!(
                            "Couldn’t install update: %{message}",
                            message = crate::services::error_detail(message)
                        ));
                        progress_for_failed.add_css_class("error");
                    }
                    None => status_for_failed.set_text(&crate::i18n::tr("Couldn’t install update")),
                }
                action_for_failed.set_label(&crate::i18n::tr("Close"));
                action_for_failed.set_sensitive(true);
            },
        );
        match outcome {
            Ok(handle) => *cancel_handle.borrow_mut() = Some(handle),
            Err(_request) => {
                phase.set(UpdateDialogPhase::Ready);
                status.set_text(&crate::i18n::tr(
                    "Another install is already running — try again shortly.",
                ));
                progress.set_visible(false);
                button.set_sensitive(true);
            }
        }
    });
    Some(UpdateDialog {
        layer,
        cancel,
        close,
        action,
        status: dialog_status,
        escape,
    })
}

fn aur_update_action_label() -> &'static str {
    match InstallSource::detect().managed() {
        Some(managed) if managed.aur_update_target().is_some() => "Open AUR Update",
        Some(managed) if managed.package().is_some() => "View on AUR",
        _ => "Close",
    }
}

fn aur_update_command(terminal: &terminal::Terminal, helper: &str, package: &str) -> Command {
    terminal.exec_command(&[helper, "-Syu", package])
}

fn launch_aur_update() -> Result<String, String> {
    let managed = InstallSource::detect()
        .managed()
        .ok_or_else(|| crate::i18n::tr("missing package metadata"))?;
    if let Some((helper, package)) = managed.aur_update_target() {
        let Some(terminal) = terminal::Terminal::resolve() else {
            return Err(terminal::no_terminal_message());
        };
        return aur_update_command(&terminal, helper, package)
            .spawn()
            .map(|_child| crate::i18n::tr("AUR update opened in your terminal."))
            .map_err(|error| terminal.launch_failure(&error));
    }
    let package = managed
        .package()
        .ok_or_else(|| crate::i18n::tr("missing AUR package name"))?;
    let uri = format!("https://aur.archlinux.org/packages/{package}");
    gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
        .map(|()| crate::i18n::tr("AUR package page opened."))
        .map_err(|error| error.to_string())
}

fn omarchy_update_command(terminal: &terminal::Terminal) -> Command {
    terminal.exec_command(&["omarchy", "update"])
}

fn launch_omarchy_update() -> Result<(), String> {
    let Some(terminal) = terminal::Terminal::resolve() else {
        return Err(terminal::no_terminal_message());
    };
    omarchy_update_command(&terminal)
        .spawn()
        .map(|_child| ())
        .map_err(|error| terminal.launch_failure(&error))
}

fn or_unknown(value: Option<String>) -> String {
    value.unwrap_or_else(|| crate::i18n::tr("Unknown"))
}

/// Formats a GitHub `published_at` timestamp as a date in the app language,
/// keeping the raw value when it cannot be parsed.
fn release_date(published_at: &str) -> String {
    let Ok(date) = glib::DateTime::from_iso8601(published_at, None) else {
        return published_at
            .split('T')
            .next()
            .unwrap_or(published_at)
            .to_owned();
    };
    let date = date.to_local().unwrap_or(date);
    crate::util::localized_date(&date, &crate::i18n::tr("dates.older"))
}

fn release_detail_rows(release: &ReleaseMetadata) -> [(&'static str, String); 4] {
    [
        ("Channel", release.kind.localized_label()),
        ("Tag", release.tag.clone()),
        ("Commit", or_unknown(release.commit.clone())),
        (
            "Published",
            or_unknown(release.published_at.as_deref().map(release_date)),
        ),
    ]
}

/// Renders `release`'s channel, tag, source commit, and publication date as
/// a small identity block, for the dialog to show above the notes whenever
/// it is offering a prerelease -- the issue requires the user be able to
/// confirm exactly what they are about to install before doing so.
fn update_dialog_details(release: &ReleaseMetadata) -> gtk::Box {
    let details = gtk::Box::new(gtk::Orientation::Vertical, 2);
    details.add_css_class("update-dialog-details");
    for (label, value) in release_detail_rows(release) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("update-dialog-detail-row");
        let label_widget = gtk::Label::new(Some(&crate::i18n::tr(label)));
        label_widget.add_css_class("update-dialog-detail-label");
        label_widget.set_xalign(0.0);
        label_widget.set_hexpand(true);
        let value_widget = gtk::Label::new(Some(&value));
        value_widget.add_css_class("update-dialog-detail-value");
        value_widget.set_xalign(0.0);
        value_widget.set_selectable(true);
        value_widget.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        if label == "Commit" {
            value_widget.add_css_class("monospace");
        }
        row.append(&label_widget);
        row.append(&value_widget);
        details.append(&row);
    }
    details
}

fn shows_available_release_notes(result: &UpdateCheck) -> bool {
    matches!(result, UpdateCheck::Available { .. })
}

/// The installed build's identity, for the update row's idle status line:
/// just the version for a stable build, or `Version {version} · {label}`
/// when running a prerelease -- so a user on an RC or nightly always sees
/// what they currently have installed, not just a bare version number.
fn installed_version_status(
    version: &Version,
    kind: BuildKind,
    update_method: UpdateMethod,
) -> String {
    let version = if kind == BuildKind::Stable {
        rust_i18n::t!("Version %{version}", version = version).into_owned()
    } else {
        rust_i18n::t!(
            "Version %{version} · %{value1}",
            version = version,
            value1 = kind.localized_label()
        )
        .into_owned()
    };
    match update_method {
        UpdateMethod::InPlace => version,
        UpdateMethod::Aur => match InstallSource::detect().managed() {
            Some(managed) => rust_i18n::t!(
                "%{version} · Managed by %{manager}",
                version = version,
                manager = managed.manager()
            ),
            None => rust_i18n::t!(
                "%{version} · Managed by a package manager",
                version = version
            ),
        }
        .into_owned(),
        UpdateMethod::Omarchy => {
            rust_i18n::t!("%{version} · Managed by Omarchy", version = version).into_owned()
        }
        UpdateMethod::Pacman => {
            rust_i18n::t!("%{version} · Managed by pacman", version = version).into_owned()
        }
    }
}

fn update_status_markup(message: String, result: &UpdateCheck, source: &InstallSource) -> String {
    match (source.managed(), result) {
        (Some(managed), UpdateCheck::Available { .. }) => format!(
            "{message}\n{}",
            glib::markup_escape_text(&managed.update_instruction())
        ),
        _ => message,
    }
}

fn update_dialog_status(managed: &ManagedInstall) -> String {
    format!(
        "{} {}",
        managed.ownership_summary(),
        managed.update_instruction()
    )
}

fn update_check_message(result: &UpdateCheck, update_method: UpdateMethod) -> String {
    match result {
        UpdateCheck::UpToDate => {
            rust_i18n::t!("Up to date — version %{value1}", value1 = crate::build_info::installed_version()).into_owned()
        }
        UpdateCheck::Available { release, .. } => {
            let url = glib::markup_escape_text(&release.url);
            let version = glib::markup_escape_text(&release.version);
            match update_method {
                UpdateMethod::InPlace | UpdateMethod::Aur => rust_i18n::t!("Update available: <a href=\"%{value1}\">v%{value2}</a>", value1 = url, value2 = version),
                UpdateMethod::Omarchy => rust_i18n::t!("Update available: <a href=\"%{value1}\">v%{value2}</a> · Run “omarchy update” to install", value1 = url, value2 = version),
                UpdateMethod::Pacman => rust_i18n::t!("Update available: <a href=\"%{value1}\">v%{value2}</a> · Install through a full system update", value1 = url, value2 = version),
            }
            .into_owned()
        }
        UpdateCheck::Failed(message) => rust_i18n::t!("Couldn't check for updates: %{value1} · <a href=\"https://github.com/lgse/strata/releases/latest\">View releases on GitHub</a>", value1 = glib::markup_escape_text(message)).into_owned(),
    }
}

fn navigation_button(icon: &str, label: &str) -> (gtk::Button, gtk::Label, gtk::Box) {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let icon_image = crate::assets::primary_icon(icon, 18);
    let subtitle = match label {
        "General" => "Browsing, search, files",
        "Appearance" => "Theme, text, motion",
        "Updates" => "Channel, release notes",
        "Actions" => "Custom actions",
        _ => "Version and links",
    };
    let text = gtk::Label::new(None);
    text.set_markup(&format!(
        "{}\n<span size=\"small\" weight=\"normal\">{}</span>",
        glib::markup_escape_text(&crate::i18n::tr(label)),
        glib::markup_escape_text(&crate::i18n::tr(subtitle))
    ));
    text.set_xalign(0.0);
    text.add_css_class("settings-nav-copy");
    content.append(&icon_image);
    content.append(&text);
    let button = gtk::Button::builder().child(&content).build();
    button.set_widget_name(label);
    button.set_has_frame(false);
    button.set_cursor_from_name(Some("pointer"));
    super::accessibility::set_label(
        &button,
        &crate::i18n::tr(if label == "Appearance" {
            "Appearance settings"
        } else {
            label
        }),
    );
    (button, text, content)
}

fn scrollable_page(content: &gtk::Box, class: Option<&str>) -> gtk::Widget {
    let empty = gtk::Label::new(Some(&crate::i18n::tr(
        "No matching settings are available on this installation.",
    )));
    empty.add_css_class("settings-search-page-empty");
    empty.add_css_class("settings-option-description");
    empty.set_visible(false);
    content.append(&empty);
    constrain_page_text(content.upcast_ref());
    content.set_hexpand(true);
    let scroller = gtk::ScrolledWindow::builder()
        .child(content)
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();
    scroller.add_css_class("settings-content-scroll");
    if let Some(class) = class {
        scroller.add_css_class(class);
    }
    scroller.upcast()
}

// Prose wraps to the viewport; long editable values scroll inside their entry,
// rather than making the whole settings page wider.
fn constrain_page_text(widget: &gtk::Widget) {
    // Action buttons keep native label sizing; segmented choices may wrap.
    if widget.is::<gtk::Button>() && !widget.is::<gtk::ToggleButton>() {
        return;
    }
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && label.ellipsize() == gtk::pango::EllipsizeMode::None
    {
        label.set_wrap(
            !label.has_css_class("settings-nowrap")
                && !label.has_css_class("menu-heading")
                && !label.has_css_class("settings-control-label"),
        );
        label.set_wrap_mode(if label.has_css_class("settings-word-wrap") {
            gtk::pango::WrapMode::Word
        } else {
            gtk::pango::WrapMode::WordChar
        });
        crate::ui::controls::keep_words_whole(label);
    }
    if let Some(entry) = widget.downcast_ref::<gtk::Entry>() {
        entry.set_width_chars(1);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        constrain_page_text(&widget);
    }
}

fn page_content() -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("settings-preferences");
    content
}

fn settings_option(title: &str, description: &str, active: bool) -> (gtk::Box, gtk::Switch) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    search::tag(&row, title);
    row.add_css_class("settings-option");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title_label = gtk::Label::new(Some(&crate::i18n::tr(title)));
    title_label.set_xalign(0.0);
    title_label.set_wrap(true);
    title_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title_label.add_css_class("settings-option-title");
    let description_label = gtk::Label::new(Some(&crate::i18n::tr(description)));
    description_label.set_xalign(0.0);
    description_label.set_wrap(true);
    description_label.add_css_class("settings-option-description");
    description_label.set_visible(!description.is_empty());
    copy.append(&title_label);
    copy.append(&description_label);
    let toggle = gtk::Switch::builder()
        .active(active)
        .halign(gtk::Align::End)
        .valign(gtk::Align::Center)
        .build();
    toggle.update_property(&[
        gtk::accessible::Property::Label(&crate::i18n::tr(title)),
        gtk::accessible::Property::Description(&crate::i18n::tr(description)),
    ]);
    row.append(&copy);
    row.append(&toggle);
    (row, toggle)
}

fn selects_all_in_settings_search(key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    modifiers.contains(gdk::ModifierType::CONTROL_MASK) && matches!(key, gdk::Key::a | gdk::Key::A)
}

fn search_field(placeholder: &str) -> (gtk::Overlay, gtk::Entry, gtk::Button) {
    let theme_search = gtk::Entry::new();
    theme_search.add_css_class("form-control");
    theme_search.add_css_class("settings-search");
    theme_search.set_placeholder_text(Some(&crate::i18n::tr(placeholder)));
    super::accessibility::set_label(&theme_search, &crate::i18n::tr(placeholder));
    let search_keys = gtk::EventControllerKey::new();
    search_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let selected_search = theme_search.downgrade();
    search_keys.connect_key_pressed(move |_, key, _, modifiers| {
        if !selects_all_in_settings_search(key, modifiers) {
            return glib::Propagation::Proceed;
        }
        let Some(search) = selected_search.upgrade() else {
            return glib::Propagation::Proceed;
        };
        search.select_region(0, -1);
        glib::Propagation::Stop
    });
    theme_search.add_controller(search_keys);
    let clear_search = gtk::Button::builder()
        .child(&crate::assets::primary_icon(icons::X, 15))
        .tooltip_text(crate::i18n::tr("Clear search"))
        .halign(gtk::Align::End)
        .valign(gtk::Align::Center)
        .margin_end(6)
        .visible(false)
        .build();
    clear_search.add_css_class("theme-search-clear");
    clear_search.set_has_frame(false);
    let search_overlay = gtk::Overlay::new();
    search_overlay.set_child(Some(&theme_search));
    let icon = crate::assets::primary_icon(icons::SEARCH, 16);
    icon.set_halign(gtk::Align::Start);
    icon.set_valign(gtk::Align::Center);
    icon.set_margin_start(12);
    icon.add_css_class("settings-search-icon");
    icon.set_can_target(false);
    search_overlay.add_overlay(&icon);
    search_overlay.add_overlay(&clear_search);
    let search = theme_search.downgrade();
    clear_search.connect_clicked(move |_| {
        if let Some(search) = search.upgrade() {
            search.set_text("");
            search.grab_focus();
        }
    });
    (search_overlay, theme_search, clear_search)
}

fn settings_group(content: &gtk::Box, heading: &str) -> gtk::Box {
    if !heading.is_empty() {
        append_heading(content, heading);
    }
    let group = gtk::Box::new(gtk::Orientation::Vertical, 0);
    group.add_css_class("settings-group");
    group.set_overflow(gtk::Overflow::Hidden);
    content.append(&group);
    group
}

fn control_row(title: &str, description: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let (row, placeholder) = settings_option(title, description, false);
    row.remove(&placeholder);
    row.append(control);
    row
}

fn indent_row(row: &gtk::Box) {
    let arrow = crate::assets::primary_icon(icons::CORNER_DOWN_RIGHT, 18);
    arrow.add_css_class("settings-indent");
    arrow.set_valign(gtk::Align::Center);
    // Keep the dependency marker with its heading when the control stacks below.
    if let Some(copy) = row.first_child().and_downcast::<gtk::Box>()
        && let Some(title) = copy.first_child()
    {
        copy.remove(&title);
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        heading.append(&arrow);
        heading.append(&title);
        copy.prepend(&heading);
    }
    row.add_css_class("settings-dependent");
}

fn append_heading(container: &gtk::Box, text: &str) -> gtk::Label {
    let heading = gtk::Label::new(Some(&crate::i18n::tr(text)));
    heading.set_xalign(0.0);
    heading.add_css_class("menu-heading");
    container.append(&heading);
    heading
}

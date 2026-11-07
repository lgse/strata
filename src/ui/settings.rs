// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use crate::{
    assets::icons,
    services::{BuildKind, Channel, InstallSource, UpdateCheck, UpdateMethod},
};

#[cfg(test)]
mod tests;

mod about;
mod actions;
mod bindings;
mod channel;
mod compact_bin;
mod diagnostics;
mod exclusions;
mod general;
pub(crate) use general::abbreviate_home;
mod install;
mod install_command;
mod integrations;
mod keyboard;
mod managed_update;
mod navigation;
mod page;
mod release_notes;
mod release_picker;
mod responsive;
mod restart;
mod search;
mod theme;
mod update_check;
mod update_dialog;
mod update_messages;
mod update_status;
mod updates;
mod wrap;
use about::about_page;
use bindings::bind_switch;
use general::general_page;
use theme::theme_page;

pub(crate) use install::{InstallGuard, install_guard};
use install::{InstallLauncher, default_install_launcher};
use navigation::{NAVIGATION, navigation_button, page_shown};
use page::{page_content, scrollable_page};
use release_notes::release_date;
use responsive::{ResponsiveActivationRow, ResponsiveBin, ResponsiveContent};
use restart::restart;
pub(crate) use restart::schedule_rollback_cleanup;
use update_check::resolve_update_method_async;
pub(crate) use update_check::{
    UpdateNoticeHandler, clear_cached_update_notice, maybe_run_due_update_check,
    register_update_notice,
};
pub(crate) use update_dialog::show_update_dialog;
use update_dialog::{UpdateOffer, build_update_dialog};
use updates::updates_page;

// Read by `update_status` and the test modules through their `super::*` globs.
#[cfg(test)]
use self::{
    channel::managed_install_summary,
    install::FINALIZING_STATUS,
    managed_update::launch_aur_update,
    release_notes::{ReleaseNotesKind, release_notes_card},
    restart::{
        close_windows_for_restart, remove_unless_replaced, restart_waiter, restart_with,
        schedule_rollback_cleanup_with,
    },
    update_check::{complete_due_update_check, next_check_generation},
    update_dialog::UpdateDialog,
    updates::{UpdateCheckRow, update_check_row_with},
};
#[cfg(test)]
use crate::services::{InstallCancel, InstallRequest, ReleaseMetadata, UpdateInstall};
#[cfg(test)]
use gtk::gio;
use managed_update::aur_update_action_label;
use release_notes::{ReleaseNotesCard, show_release_notes};
use update_check::{CachedUpdate, effective_update_channel};
use update_messages::{
    installed_version_status, shows_available_release_notes, update_check_message,
    update_status_markup,
};
use updates::PendingInstall;

use super::{
    blur::BlurBin,
    keys::{self, bindings::SELECT_ALL},
    modal::close_on_escape,
    preferences::PreferenceManager,
};

/// Shows a directory in the hosting window's browser.
pub(crate) type OpenFolder = Rc<dyn Fn(std::path::PathBuf)>;

#[derive(Clone)]
pub struct SettingsLayer {
    pub widget: gtk::Box,
    navigation: CurrentPage,
}

impl SettingsLayer {
    /// Settings opens on its current page's navigation entry, which tells a screen
    /// reader where the user is; the layer itself is an unnamed panel.
    pub fn focus_current_page(&self) {
        let focused = self.navigation.button().is_some_and(|button| {
            button.is_visible()
                && button.is_sensitive()
                && button.is_mapped()
                && button.grab_focus()
        });
        if !focused {
            super::modal::focus_modal_layer(&self.widget);
        }
    }
}

pub fn build_layer(
    settings_button: &gtk::Button,
    root: &BlurBin,
    preferences: Rc<PreferenceManager>,
    update_notice: UpdateNoticeHandler,
    install_guard: InstallGuard,
    open_folder: OpenFolder,
) -> SettingsLayer {
    let layer = backdrop_layer();
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
    let (titlebar, title, close) = titlebar();
    page.append(&titlebar);

    let stack = page_stack();
    let (general, responsive_setting_rows, responsive_activation_rows) =
        general_page(preferences.clone());
    stack.add_named(&general, Some("general"));
    stack.add_named(&diagnostics::diagnostics_page(), Some("diagnostics"));
    stack.add_named(&about_page(), Some("about"));
    // Heavy pages build on first selection, never during startup: the
    // Updates page spawns package-manager detection plus release-note
    // network work, and the theme page builds its swatch flows.
    let updates_container = updates_placeholder();
    if page_shown("updates") {
        stack.add_named(&updates_container, Some("updates"));
    }
    page.append(&stack);

    // Created before the navigation loop so lazy builders can register
    // their responsive rows and flows as pages materialize.
    let responsive_panel = ResponsiveBin::new(
        &panel,
        &navigation,
        &navigation_heading,
        ResponsiveContent {
            flows: Vec::new(),
            actions: Vec::new(),
            setting_rows: responsive_setting_rows,
            activation_rows: responsive_activation_rows,
        },
    );
    let dismiss_hooks = DismissHooks::default();
    let close_settings: Rc<dyn Fn()> = {
        let layer = layer.clone();
        let button = settings_button.clone();
        let root = root.clone();
        let hooks = dismiss_hooks.clone();
        Rc::new(move || hide(&layer, &button, &root, &hooks))
    };
    let pages = PageHost {
        stack: stack.clone(),
        title: title.clone(),
        built: Rc::new(RefCell::new(
            ["general", "diagnostics", "about"].into_iter().collect(),
        )),
        navigation: CurrentPage::default(),
        preferences,
        update_notice,
        install_guard,
        updates_container,
        responsive_panel: responsive_panel.clone(),
        search_state: settings_search.state.clone(),
        dismiss_hooks: dismiss_hooks.clone(),
        open_folder,
        close_settings: close_settings.clone(),
    };
    // Navigation entries register as their buttons are created, so the
    // responsive panel compacts correctly even with lazy pages.
    for entry in NAVIGATION.iter().filter(|entry| page_shown(entry.page)) {
        navigation.append(&pages.navigation_button(entry));
    }

    settings_search.install(&stack, &title, &pages.navigation.buttons);
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
    center_in_layer(&layer, &responsive_panel);

    // A window closed mid-preview never runs `hide`; the preview is process-wide.
    layer.connect_unrealize(move |_| dismiss_hooks.run());
    let close_hide = close_settings.clone();
    close.connect_clicked(move |_| close_hide());
    let escape_hide = close_settings.clone();
    close_on_escape(&layer, move || escape_hide());
    close_on_outside_click(&layer, &panel, close_settings);
    super::focus_navigation::install(&layer);
    SettingsLayer {
        widget: layer,
        navigation: pages.navigation,
    }
}

/// Computes a hidden layer's styles and layout without mapping it, so that its first
/// open mostly only has to render. GTK does not run between the visibility changes,
/// so nothing can see the layer as open.
pub fn prepare_layout(layer: &gtk::Box) {
    let Some(parent) = layer.parent().filter(|parent| parent.width() > 0) else {
        return;
    };
    if layer.is_visible() {
        return;
    }
    layer.set_child_visible(false);
    layer.set_visible(true);
    let (min_width, ..) = layer.measure(gtk::Orientation::Horizontal, -1);
    let width = parent.width().max(min_width);
    let (min_height, ..) = layer.measure(gtk::Orientation::Vertical, width);
    layer.allocate(width, parent.height().max(min_height), -1, None);
    layer.set_visible(false);
    layer.set_child_visible(true);
}

fn backdrop_layer() -> gtk::Box {
    let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layer.add_css_class("app-modal-layer");
    layer.add_css_class("settings-backdrop");
    layer.set_halign(gtk::Align::Fill);
    layer.set_valign(gtk::Align::Fill);
    layer.set_hexpand(true);
    layer.set_vexpand(true);
    layer.set_focusable(true);
    layer.set_visible(false);
    layer
}

fn titlebar() -> (gtk::Box, gtk::Label, gtk::Button) {
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
    super::accessibility::set_own_label(&close, &crate::i18n::tr("Close settings"));
    close.set_child(Some(&crate::assets::primary_icon(icons::X, 18)));
    close.add_css_class("settings-close");
    close.set_valign(gtk::Align::Center);
    titlebar.append(&title);
    titlebar.append(&close);
    (titlebar, title, close)
}

fn page_stack() -> gtk::Stack {
    gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(120)
        .hhomogeneous(false)
        .vhomogeneous(false)
        .hexpand(true)
        .vexpand(true)
        .build()
}

fn updates_placeholder() -> gtk::Box {
    let updates_container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    updates_container.set_hexpand(true);
    updates_container.set_vexpand(true);
    let updates_spinner = gtk::Spinner::new();
    updates_spinner.start();
    updates_spinner.set_halign(gtk::Align::Center);
    updates_spinner.set_valign(gtk::Align::Center);
    updates_spinner.set_vexpand(true);
    updates_container.append(&updates_spinner);
    updates_container
}

/// The responsive bin fills the window; the dialog is the panel inside its margin.
fn center_in_layer(layer: &gtk::Box, responsive_panel: &ResponsiveBin) {
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
    row.append(responsive_panel);
    row.append(&right);
    layer.append(&top);
    layer.append(&row);
    layer.append(&bottom);
}

#[expect(
    deprecated,
    reason = "GTK 4.12 deprecated translate_coordinates and allocation without a replacement for click-in-bounds checks"
)]
fn close_on_outside_click(layer: &gtk::Box, panel: &gtk::Box, close_settings: Rc<dyn Fn()>) {
    let click_layer = layer.clone();
    let click_dialog = panel.clone();
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
}

type NavigationButtons = Rc<RefCell<Vec<(gtk::Button, &'static str)>>>;

#[derive(Clone)]
struct CurrentPage {
    buttons: NavigationButtons,
    page: Rc<Cell<&'static str>>,
}

impl Default for CurrentPage {
    fn default() -> Self {
        Self {
            buttons: Rc::default(),
            page: Rc::new(Cell::new("general")),
        }
    }
}

impl CurrentPage {
    fn button(&self) -> Option<gtk::Button> {
        let page = self.page.get();
        self.buttons
            .borrow()
            .iter()
            .find(|(_, name)| *name == page)
            .map(|(button, _)| button.clone())
    }
}

/// The page stack and everything a lazily built page needs when first shown.
#[derive(Clone)]
struct PageHost {
    stack: gtk::Stack,
    title: gtk::Label,
    built: Rc<RefCell<std::collections::HashSet<&'static str>>>,
    navigation: CurrentPage,
    preferences: Rc<PreferenceManager>,
    update_notice: UpdateNoticeHandler,
    install_guard: InstallGuard,
    updates_container: gtk::Box,
    responsive_panel: ResponsiveBin,
    search_state: search::State,
    dismiss_hooks: DismissHooks,
    open_folder: OpenFolder,
    close_settings: Rc<dyn Fn()>,
}

impl PageHost {
    fn navigation_button(&self, entry: &navigation::NavigationEntry) -> gtk::Button {
        let name = entry.page;
        let (button, navigation_label, navigation_content) = navigation_button(entry);
        self.responsive_panel
            .add_navigation(navigation_label, navigation_content);
        if name == self.navigation.page.get() {
            button.add_css_class("settings-nav-active");
        }
        self.navigation
            .buttons
            .borrow_mut()
            .push((button.clone(), name));
        let host = self.clone();
        let page_title = entry.label;
        button.connect_clicked(move |clicked| host.show(clicked, name, page_title));
        button
    }

    fn show(&self, clicked: &gtk::Button, name: &'static str, page_title: &str) {
        self.navigation.page.set(name);
        for (candidate, _) in self.navigation.buttons.borrow().iter() {
            if candidate == clicked {
                candidate.add_css_class("settings-nav-active");
            } else {
                candidate.remove_css_class("settings-nav-active");
            }
        }
        let first_shown = self.built.borrow_mut().insert(name);
        if first_shown {
            self.build(name);
        }
        self.stack.set_visible_child_name(name);
        self.title.set_text(&crate::i18n::tr(page_title));
    }

    fn build(&self, name: &'static str) {
        match name {
            "theme" => self.build_theme_page(),
            "actions" => self.add_page(name, &actions::actions_page()),
            keyboard::PAGE => {
                self.add_page(name, &keyboard::keyboard_page(self.preferences.clone()));
            }
            "integrations" => self.build_integrations_page(),
            "updates" => self.build_updates_page(),
            _ => {}
        }
    }

    fn add_page(&self, name: &str, page: &gtk::Widget) {
        search::apply(page, &self.search_state);
        self.stack.add_named(page, Some(name));
    }

    fn build_theme_page(&self) {
        let page = theme_page(
            self.preferences.clone(),
            super::theme::ThemeManager::shared(),
        );
        self.add_page("theme", &page.widget);
        for (flow, columns) in page.flows {
            self.responsive_panel.add_flow(flow, columns);
        }
        self.dismiss_hooks.add(page.dismiss);
    }

    fn build_integrations_page(&self) {
        let open_folder = self.open_folder.clone();
        let close_settings = self.close_settings.clone();
        let page = integrations::integrations_page(Some(Rc::new(move |path| {
            close_settings();
            open_folder(path);
        })));
        self.add_page("integrations", &page);
    }

    fn build_updates_page(&self) {
        let host = self.clone();
        resolve_update_method_async(move |method| host.fill_updates_page(method));
    }

    fn fill_updates_page(&self, method: UpdateMethod) {
        maybe_run_due_update_check(&self.preferences);
        let (updates, actions) = updates_page(
            self.preferences.clone(),
            self.update_notice.clone(),
            self.install_guard.clone(),
            method,
        );
        while let Some(child) = self.updates_container.first_child() {
            self.updates_container.remove(&child);
        }
        search::apply(&updates, &self.search_state);
        self.updates_container.append(&updates);
        for (row, button) in actions {
            self.responsive_panel.add_action(row, button);
        }
    }
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
    if !layer.is_visible() {
        return;
    }
    // A `dismissing` hide hands focus back to where Settings was opened from.
    layer.add_css_class("dismissing");
    layer.set_visible(false);
    layer.remove_css_class("dismissing");
    root.set_blurred(false);
    button.remove_css_class("active");
}

/// Translates Settings copy, rendering the shortcuts it names.
fn translated(message: &str) -> String {
    translated_in(&rust_i18n::locale(), message)
}

fn translated_in(locale: &str, message: &str) -> String {
    use super::keys::{
        Keys::One,
        bindings::{REFERENCE, SIDEBAR_FOCUS, TENXER_MODE},
        label_in,
    };
    rust_i18n::t!(
        message,
        locale = locale,
        reference_keys = label_in(locale, &One(REFERENCE.alternate(0).shown())),
        sidebar_focus_keys = label_in(locale, &One(SIDEBAR_FOCUS.shown())),
        tenxer_mode_keys = label_in(locale, &One(TENXER_MODE.shown()))
    )
    .into_owned()
}

fn settings_option(title: &str, description: &str, active: bool) -> (gtk::Box, gtk::Switch) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    search::tag(&row, title);
    row.add_css_class("settings-option");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title_label = gtk::Label::new(None);
    title_label.set_xalign(0.0);
    title_label.set_wrap(true);
    title_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title_label.add_css_class("settings-option-title");
    let description_label = gtk::Label::new(None);
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
    let keyed = [title, description]
        .iter()
        .any(|message| message.contains("_keys}"));
    let (title, description) = (title.to_owned(), description.to_owned());
    let (title_label, description_label) = (title_label.downgrade(), description_label.downgrade());
    let render = move |toggle: &gtk::Switch| {
        if let Some(label) = title_label.upgrade() {
            label.set_text(&translated(&title));
        }
        if let Some(label) = description_label.upgrade() {
            label.set_text(&translated(&description));
        }
        super::accessibility::set_own_label(toggle, &translated(&title));
        super::accessibility::set_description(toggle, Some(&translated(&description)));
    };
    if keyed {
        super::shortcut_reference::follow_keys(&toggle, render);
    } else {
        render(&toggle);
    }
    row.append(&copy);
    row.append(&toggle);
    (row, toggle)
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
    keys::on_press(&search_keys, move |_, press| {
        if !press.is(SELECT_ALL) {
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
        .halign(gtk::Align::End)
        .valign(gtk::Align::Center)
        .margin_end(6)
        .visible(false)
        .build();
    crate::ui::accessibility::set_tooltip_and_label(
        &clear_search,
        &crate::i18n::tr("Clear search"),
    );
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

pub(super) fn unit_system_control(manager: &Rc<PreferenceManager>) -> gtk::Box {
    use super::preferences::UnitSystem;
    let (control, buttons) = super::controls::segmented_control(
        &["Metric", "Imperial"],
        usize::from(manager.unit_system() == UnitSystem::Imperial),
    );
    super::accessibility::set_label(&control, &crate::i18n::tr("Units"));
    for (button, value) in buttons
        .iter()
        .zip([UnitSystem::Metric, UnitSystem::Imperial])
    {
        bindings::bind_choice(
            manager,
            button,
            value,
            PreferenceManager::unit_system,
            PreferenceManager::set_unit_system,
        );
    }
    control
}

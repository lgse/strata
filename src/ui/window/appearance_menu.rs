// SPDX-License-Identifier: MIT

//! The header's Appearance menu: view mode, preview, density, text size and
//! listing options.

use std::rc::Rc;

use gtk::{gio, glib, prelude::*};

use crate::ui::{
    browser::{BrowserView, PaneTarget},
    browser_modes::BrowserMode,
    keys::{
        Keys,
        bindings::{HIDDEN_FILES, TEXT_LARGER, TEXT_RESET, TEXT_SMALLER},
    },
    preferences::{PreferenceManager, TextSize},
    preview::PreviewDrawer,
    shortcut_reference::with_keys,
};

use super::{apply_browser_mode, density_slider};

/// The menu button, its popover and the column the sections are appended to.
struct MenuShell {
    content: gtk::Box,
    popover: gtk::Popover,
    button: gtk::MenuButton,
}

impl MenuShell {
    fn new() -> Self {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("appearance-menu");
        let popover = gtk::Popover::builder()
            .has_arrow(false)
            .halign(gtk::Align::End)
            .position(gtk::PositionType::Bottom)
            .build();
        popover.add_css_class("appearance-popover");
        crate::ui::scrolling::popover::dismiss_on_outside_scroll(&popover);
        let button = gtk::MenuButton::builder()
            .tooltip_text(crate::i18n::tr("Appearance"))
            .popover(&popover)
            .build();
        // Without an explicit name GTK builds one from the whole open popover, so
        // a screen reader reads the entire menu back as the button's label.
        crate::ui::accessibility::set_label(&button, &crate::i18n::tr("Appearance"));
        Self {
            content,
            popover,
            button,
        }
    }

    fn popover(&self) -> glib::WeakRef<gtk::Popover> {
        self.popover.downgrade()
    }

    fn append_separator(&self) {
        self.content
            .append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    }

    fn append_heading(&self, text: &str) {
        append_menu_heading(&self.content, &crate::i18n::tr(text));
    }

    /// Fits the popover to the window with its contents scrolling.
    fn finish(self, button_icon: &gtk::Image) -> gtk::MenuButton {
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_width(true)
            .propagate_natural_height(true)
            .max_content_height(600)
            .child(&self.content)
            .build();
        let anchor = self.button.downgrade();
        let constrained = scroll.downgrade();
        self.popover.connect_show(move |_| {
            if let (Some(anchor), Some(scroll)) = (anchor.upgrade(), constrained.upgrade())
                && let Some(window) = anchor.root().and_downcast::<gtk::Window>()
            {
                scroll.set_max_content_height((window.height() - anchor.height() - 48).max(1));
                scroll.set_max_content_width((window.width() - 24).max(1));
            }
        });
        self.popover.set_child(Some(&scroll));
        self.button.set_child(Some(button_icon));
        self.button.add_css_class("header-action");
        self.button.set_cursor_from_name(Some("pointer"));
        self.button
    }
}

fn popdown(popover: &glib::WeakRef<gtk::Popover>) {
    if let Some(popover) = popover.upgrade() {
        popover.popdown();
    }
}

/// A view-mode radio row and its check mark.
struct ModeOption {
    mode: BrowserMode,
    button: gtk::Button,
    check: gtk::Image,
}

/// Rows that only apply to some view modes, shown or enabled as the mode changes.
struct ListingOptions {
    group_by_type: gtk::Button,
    compact_full_names: gtk::Button,
    column_sizes: gtk::Button,
    customize_columns: gtk::Button,
}

pub(in crate::ui) fn build_appearance_menu(
    view: &BrowserView,
    preferences: Rc<PreferenceManager>,
    preview: &PreviewDrawer,
) -> gtk::MenuButton {
    build_pane_appearance_menu(&PaneTarget::new(view), preferences, preview)
}

/// The menu for a tab whose panes share it: every row acts on the active pane.
pub(in crate::ui) fn build_pane_appearance_menu(
    target: &PaneTarget,
    preferences: Rc<PreferenceManager>,
    preview: &PreviewDrawer,
) -> gtk::MenuButton {
    let shell = MenuShell::new();
    let popover = shell.popover();
    shell.append_heading("VIEW");
    let current_mode = target.view().view_mode();
    let button_icon = crate::assets::chrome_icon(browser_mode_icon(current_mode));
    let modes = mode_options(current_mode);
    let listing = listing_options(target, &preferences, &popover, current_mode);
    connect_mode_buttons(&modes, target, &preferences, &popover);
    follow_view_mode(target, &modes, &listing, &button_icon);
    for option in &modes {
        shell.content.append(&option.button);
    }
    shell
        .content
        .append(&preview_toggle(preview, &preferences, &popover));

    shell.append_separator();
    append_density_section(&shell, &preferences);
    shell.append_separator();
    append_text_size_section(&shell, &preferences);

    shell.append_separator();
    shell.content.append(&listing.group_by_type);
    shell.content.append(&listing.compact_full_names);
    shell.content.append(&listing.customize_columns);
    shell.content.append(&listing.column_sizes);
    shell
        .content
        .append(&hidden_files_option(target, &preferences, &popover));
    shell.finish(&button_icon)
}

fn mode_options(current_mode: BrowserMode) -> [ModeOption; 4] {
    [
        (
            crate::assets::icons::COLUMNS,
            "Columns",
            BrowserMode::Columns,
        ),
        (crate::assets::icons::ICONS, "Icons", BrowserMode::Icons),
        (crate::assets::icons::LIST, "List", BrowserMode::List),
        (
            crate::assets::icons::COMPACT,
            "view_mode.compact",
            BrowserMode::Compact,
        ),
    ]
    .map(|(icon, label, mode)| {
        let option = appearance_option(OptionSpec {
            icon,
            label,
            checked: current_mode == mode,
            sensitive: true,
            role: gtk::AccessibleRole::MenuItemRadio,
        });
        ModeOption {
            mode,
            button: option.button,
            check: option.check,
        }
    })
}

fn listing_options(
    target: &PaneTarget,
    preferences: &Rc<PreferenceManager>,
    popover: &glib::WeakRef<gtk::Popover>,
    current_mode: BrowserMode,
) -> ListingOptions {
    ListingOptions {
        group_by_type: group_by_type_option(preferences, popover, current_mode),
        compact_full_names: compact_full_names_option(preferences, popover, current_mode),
        column_sizes: column_sizes_option(preferences, popover, current_mode),
        customize_columns: customize_columns_option(target, popover, current_mode),
    }
}

fn compact_full_names_option(
    preferences: &Rc<PreferenceManager>,
    popover: &glib::WeakRef<gtk::Popover>,
    current_mode: BrowserMode,
) -> gtk::Button {
    let option = appearance_option(OptionSpec {
        icon: crate::assets::icons::COMPACT,
        label: "Show full names in Compact view",
        checked: preferences.compact_full_names(),
        sensitive: true,
        role: gtk::AccessibleRole::MenuItemCheckbox,
    });
    option
        .button
        .set_visible(current_mode == BrowserMode::Compact);
    preferences.bind_preference(
        &option.check,
        PreferenceManager::compact_full_names,
        |widget, enabled| widget.set_visible(enabled),
    );
    let preferences = preferences.clone();
    let popover = popover.clone();
    option.button.connect_clicked(move |_| {
        preferences.set_compact_full_names(!preferences.compact_full_names());
        popdown(&popover);
    });
    option.button
}

fn group_by_type_option(
    preferences: &Rc<PreferenceManager>,
    popover: &glib::WeakRef<gtk::Popover>,
    current_mode: BrowserMode,
) -> gtk::Button {
    let option = appearance_option(OptionSpec {
        icon: crate::assets::icons::LIST_CHECKS,
        label: "Group by file type",
        checked: preferences.group_by_type(),
        sensitive: current_mode.supports_type_grouping(),
        role: gtk::AccessibleRole::MenuItemCheckbox,
    });
    crate::ui::accessibility::set_description(
        &option.button,
        Some(&crate::i18n::tr(
            "Group List entries under file-type headings",
        )),
    );
    preferences.bind_preference(
        &option.check,
        PreferenceManager::group_by_type,
        |widget, enabled| widget.set_visible(enabled),
    );
    let preferences = preferences.clone();
    let popover = popover.clone();
    option.button.connect_clicked(move |_| {
        preferences.set_group_by_type(!preferences.group_by_type());
        popdown(&popover);
    });
    option.button
}

fn column_sizes_option(
    preferences: &Rc<PreferenceManager>,
    popover: &glib::WeakRef<gtk::Popover>,
    current_mode: BrowserMode,
) -> gtk::Button {
    let option = appearance_option(OptionSpec {
        icon: crate::assets::icons::COLUMNS,
        label: "Show sizes in Columns view",
        checked: preferences.columns_show_sizes(),
        sensitive: true,
        role: gtk::AccessibleRole::MenuItemCheckbox,
    });
    option
        .button
        .set_visible(current_mode == BrowserMode::Columns);
    preferences.bind_preference(
        &option.check,
        PreferenceManager::columns_show_sizes,
        |widget, enabled| widget.set_visible(enabled),
    );
    let preferences = preferences.clone();
    let popover = popover.clone();
    option.button.connect_clicked(move |_| {
        preferences.set_columns_show_sizes(!preferences.columns_show_sizes());
        popdown(&popover);
    });
    option.button
}

fn customize_columns_option(
    target: &PaneTarget,
    popover: &glib::WeakRef<gtk::Popover>,
    current_mode: BrowserMode,
) -> gtk::Button {
    let option = appearance_option(OptionSpec {
        icon: crate::assets::icons::COLUMNS,
        label: "Customize columns…",
        checked: false,
        sensitive: true,
        role: gtk::AccessibleRole::MenuItem,
    });
    option.button.set_visible(current_mode == BrowserMode::List);
    // Buttons keep the target alive; a chooser's menu is its only owner.
    let target = target.clone();
    let popover = popover.clone();
    option.button.connect_clicked(move |_| {
        popdown(&popover);
        target.view().open_list_columns_popover();
    });
    option.button
}

fn connect_mode_buttons(
    modes: &[ModeOption],
    target: &PaneTarget,
    preferences: &Rc<PreferenceManager>,
    popover: &glib::WeakRef<gtk::Popover>,
) {
    for option in modes {
        let mode = option.mode;
        let target = target.clone();
        let preferences = preferences.clone();
        let popover = popover.clone();
        option.button.connect_clicked(move |_| {
            let view = target.view();
            apply_browser_mode(&view, &preferences, mode);
            popdown(&popover);
            let view = view.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(view) = view.upgrade() {
                    view.focus_switched_view();
                }
            });
        });
    }
}

/// The mode also changes from the keyboard, so track the active pane's view
/// rather than only the buttons in this menu.
fn follow_view_mode(
    target: &PaneTarget,
    modes: &[ModeOption],
    listing: &ListingOptions,
    button_icon: &gtk::Image,
) {
    // The target keeps its listeners, so they hold the menu's widgets weakly.
    let checks: Vec<_> = modes
        .iter()
        .map(|option| (option.mode, option.check.downgrade()))
        .collect();
    let group_by_type = listing.group_by_type.downgrade();
    let compact_full_names = listing.compact_full_names.downgrade();
    let column_sizes = listing.column_sizes.downgrade();
    let customize_columns = listing.customize_columns.downgrade();
    let button_icon = button_icon.downgrade();
    let show: Rc<dyn Fn(BrowserMode)> = Rc::new(move |mode| {
        for (checked_mode, check) in &checks {
            if let Some(check) = check.upgrade() {
                check.set_visible(mode == *checked_mode);
            }
        }
        if let Some(group_by_type) = group_by_type.upgrade() {
            group_by_type.set_sensitive(mode.supports_type_grouping());
        }
        if let Some(compact_full_names) = compact_full_names.upgrade() {
            compact_full_names.set_visible(mode == BrowserMode::Compact);
        }
        if let Some(column_sizes) = column_sizes.upgrade() {
            column_sizes.set_visible(mode == BrowserMode::Columns);
        }
        if let Some(customize_columns) = customize_columns.upgrade() {
            customize_columns.set_visible(mode == BrowserMode::List);
        }
        if let Some(button_icon) = button_icon.upgrade() {
            crate::assets::set_primary_icon(&button_icon, browser_mode_icon(mode));
        }
    });
    let follow = {
        let target = target.downgrade();
        let show = show.clone();
        Rc::new(move |view: &BrowserView| {
            let target = target.clone();
            let followed = view.downgrade();
            let show = show.clone();
            view.connect_view_mode_changed(move |mode| {
                let active = target.upgrade().zip(followed.upgrade());
                if active.is_some_and(|(target, view)| target.is_active(&view)) {
                    show(mode);
                }
            });
        })
    };
    for view in target.views() {
        follow(&view);
    }
    target.connect_added(move |view| follow(view));
    target.connect_retargeted(move |view| show(view.view_mode()));
}

fn preview_toggle(
    preview: &PreviewDrawer,
    preferences: &Rc<PreferenceManager>,
    popover: &glib::WeakRef<gtk::Popover>,
) -> gtk::ToggleButton {
    let (row, check, _) = appearance_row(
        crate::assets::icons::EYE,
        "Preview panel",
        preview.is_enabled(),
    );
    let preview_shortcut = gtk::Label::new(None);
    preview_shortcut.add_css_class("folder-context-shortcut");
    row.append(&preview_shortcut);
    row.reorder_child_after(&check, Some(&preview_shortcut));
    let preview_toggle = gtk::ToggleButton::builder()
        .child(&row)
        .has_frame(false)
        .build();
    preview_toggle.add_css_class("appearance-option");
    preview_toggle.add_css_class("preview-panel-option");
    crate::ui::accessibility::set_label(&preview_toggle, &crate::i18n::tr("Preview panel"));
    bind_preview_shortcut(&preview_shortcut, &preview_toggle, preferences);
    let actions = gio::SimpleActionGroup::new();
    actions.add_action(&preview.action());
    preview_toggle.insert_action_group("preview", Some(&actions));
    preview_toggle.set_action_name(Some("preview.preview-panel"));
    preview_toggle
        .bind_property("active", &check, "visible")
        .sync_create()
        .build();
    let popover = popover.clone();
    preview_toggle.connect_clicked(move |_| popdown(&popover));
    preview_toggle
}

/// The same command shows other keys in the other map or once rebound.
fn bind_preview_shortcut(
    shortcut: &gtk::Label,
    toggle: &gtk::ToggleButton,
    preferences: &Rc<PreferenceManager>,
) {
    let described = toggle.downgrade();
    preferences.bind_preference(
        shortcut,
        |preferences| {
            (
                crate::ui::shortcut_reference::context_hint_for(
                    crate::ui::shortcut_reference::ContextHint::Preview,
                    preferences,
                ),
                preferences.shown_keys(),
            )
        },
        move |widget, (hint, _)| {
            let shortcut = widget
                .downcast_ref::<gtk::Label>()
                .expect("preview shortcut label");
            let text = hint
                .map(|keys| crate::ui::keys::label(&keys))
                .unwrap_or_default();
            shortcut.set_text(&text);
            shortcut.set_visible(!text.is_empty());
            if let Some(toggle) = described.upgrade() {
                crate::ui::accessibility::set_description(
                    &toggle,
                    Some(&preview_toggle_description(&text)),
                );
            }
        },
    );
}

fn preview_toggle_description(shortcut: &str) -> String {
    if shortcut.is_empty() {
        crate::i18n::tr("Toggle preview panel while browsing")
    } else {
        rust_i18n::t!(
            "Toggle preview panel while browsing (%{text})",
            text = shortcut
        )
        .into_owned()
    }
}

fn append_density_section(shell: &MenuShell, preferences: &Rc<PreferenceManager>) {
    shell.append_heading("DENSITY");
    shell.content.append(&density_slider::density_slider(
        preferences,
        "Frame",
        PreferenceManager::frame_density,
        PreferenceManager::set_frame_density,
    ));
    shell.content.append(&density_slider::density_slider(
        preferences,
        "Content",
        PreferenceManager::browser_density,
        PreferenceManager::set_browser_density,
    ));
}

fn append_text_size_section(shell: &MenuShell, preferences: &Rc<PreferenceManager>) {
    shell.append_heading("TEXT SIZE");
    let text_controls = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    text_controls.add_css_class("appearance-text-size");
    let sample = gtk::Label::new(Some("Aa"));
    sample.add_css_class("appearance-text-sample");
    text_controls.append(&sample);
    let (stepper, [decrease, reset_size, increase]) = text_size_stepper();
    text_controls.append(&stepper);
    for (button, delta) in [(decrease, -1), (increase, 1)] {
        let manager = preferences.clone();
        button.connect_clicked(move |_| manager.set_text_size(manager.text_size().stepped(delta)));
    }
    preferences.bind_preference(&reset_size, PreferenceManager::text_size, |widget, size| {
        widget
            .downcast_ref::<gtk::Button>()
            .expect("text size reset")
            .set_label(&format!("{} px", size.root_font_px()));
    });
    let manager = preferences.clone();
    reset_size.connect_clicked(move |_| manager.set_text_size(TextSize::default()));
    shell.content.append(&text_controls);
}

fn text_size_stepper() -> (gtk::Box, [gtk::Button; 3]) {
    let labels = || {
        [
            with_keys(
                "Decrease text size (%{keys})",
                Keys::One(TEXT_SMALLER.shown()),
            ),
            with_keys("Reset text size (%{keys})", Keys::One(TEXT_RESET.shown())),
            with_keys(
                "Increase text size (%{keys})",
                Keys::One(TEXT_LARGER.alternate(0).shown()),
            ),
        ]
    };
    let (stepper, buttons) = crate::ui::controls::stepper(labels());
    let relabeled = buttons.each_ref().map(|button| button.downgrade());
    crate::ui::shortcut_reference::follow_keys(&stepper, move |_| {
        for (button, label) in relabeled.iter().zip(labels()) {
            if let Some(button) = button.upgrade() {
                crate::ui::controls::label_stepper_button(&button, &label);
            }
        }
    });
    (stepper, buttons)
}

fn hidden_files_icon(shown: bool) -> &'static str {
    if shown {
        crate::assets::icons::EYE
    } else {
        crate::assets::icons::EYE_OFF
    }
}

fn hidden_files_option(
    target: &PaneTarget,
    preferences: &Rc<PreferenceManager>,
    popover: &glib::WeakRef<gtk::Popover>,
) -> gtk::Button {
    let hidden_files_shown = preferences.sort_preferences().show_hidden;
    let option = appearance_option_with_shortcut(
        OptionSpec {
            icon: hidden_files_icon(hidden_files_shown),
            label: "Hidden files",
            checked: hidden_files_shown,
            sensitive: true,
            role: gtk::AccessibleRole::MenuItemCheckbox,
        },
        Keys::One(HIDDEN_FILES.alternate(0).shown()),
    );
    let observed_check = option.check.downgrade();
    let observed_icon = option.icon.downgrade();
    let show: Rc<dyn Fn(bool)> = Rc::new(move |shown| {
        if let Some(check) = observed_check.upgrade() {
            check.set_visible(shown);
        }
        if let Some(icon) = observed_icon.upgrade() {
            crate::assets::set_primary_icon(&icon, hidden_files_icon(shown));
        }
    });
    let follow = {
        let target = target.downgrade();
        let show = show.clone();
        Rc::new(move |view: &BrowserView| {
            let target = target.clone();
            let followed = view.downgrade();
            let show = show.clone();
            view.browser().observe_preferences(move |preferences| {
                let active = target.upgrade().zip(followed.upgrade());
                if active.is_some_and(|(target, view)| target.is_active(&view)) {
                    show(preferences.show_hidden);
                }
            });
        })
    };
    for view in target.views() {
        follow(&view);
    }
    target.connect_added(move |view| follow(view));
    target.connect_retargeted(move |view| show(view.browser().preferences().show_hidden));
    let target = target.clone();
    let popover = popover.clone();
    option.button.connect_clicked(move |_| {
        target.browser().toggle_hidden();
        popdown(&popover);
    });
    option.button
}

fn browser_mode_icon(mode: BrowserMode) -> &'static str {
    match mode {
        BrowserMode::Columns => crate::assets::icons::COLUMNS,
        BrowserMode::Icons => crate::assets::icons::ICONS,
        BrowserMode::List => crate::assets::icons::LIST,
        BrowserMode::Compact => crate::assets::icons::COMPACT,
    }
}

struct OptionSpec<'a> {
    icon: &'a str,
    /// English message key, localized when the row is built.
    label: &'a str,
    checked: bool,
    sensitive: bool,
    role: gtk::AccessibleRole,
}

struct OptionWidgets {
    button: gtk::Button,
    check: gtk::Image,
    icon: gtk::Image,
}

fn appearance_option(spec: OptionSpec<'_>) -> OptionWidgets {
    let (row, check, icon) = appearance_row(spec.icon, spec.label, spec.checked);
    let button = gtk::Button::builder()
        .child(&row)
        .sensitive(spec.sensitive)
        .accessible_role(spec.role)
        .build();
    button.add_css_class("appearance-option");
    button.set_has_frame(false);
    crate::ui::accessibility::set_label(&button, &crate::i18n::tr(spec.label));
    crate::ui::accessibility::sync_checked_with_icon(&button, &check);
    OptionWidgets {
        button,
        check,
        icon,
    }
}

fn appearance_option_with_shortcut(spec: OptionSpec<'_>, keys: Keys) -> OptionWidgets {
    let (row, check, icon) = appearance_row(spec.icon, spec.label, spec.checked);
    let button = gtk::Button::builder()
        .child(&row)
        .sensitive(spec.sensitive)
        .accessible_role(spec.role)
        .build();
    button.add_css_class("appearance-option");
    button.set_has_frame(false);
    let name = crate::i18n::tr(spec.label);
    crate::ui::accessibility::set_label(&button, &name);
    let shown = gtk::Label::new(None);
    shown.add_css_class("folder-context-shortcut");
    row.append(&shown);
    row.reorder_child_after(&check, Some(&shown));
    let described = button.downgrade();
    crate::ui::shortcut_reference::follow_keys(&shown, move |shown| {
        let text = crate::ui::keys::label(&keys);
        shown.set_text(&text);
        shown.set_visible(!text.is_empty());
        if let Some(button) = described.upgrade() {
            if text.is_empty() {
                crate::ui::accessibility::set_label(&button, &name);
            } else {
                crate::ui::accessibility::describe_menu_item(&button, &name, &text);
            }
        }
    });
    crate::ui::accessibility::sync_checked_with_icon(&button, &check);
    OptionWidgets {
        button,
        check,
        icon,
    }
}

fn appearance_row(icon: &str, label: &str, checked: bool) -> (gtk::Box, gtk::Image, gtk::Image) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let check = crate::assets::primary_icon(crate::assets::icons::CHECK, 16);
    check.set_visible(checked);
    let option = crate::assets::primary_icon(icon, 17);
    let label = gtk::Label::new(Some(&crate::i18n::tr(label)));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&option);
    row.append(&label);
    row.append(&check);
    (row, check, option)
}

fn append_menu_heading(container: &gtk::Box, text: &str) {
    let heading = gtk::Label::new(Some(text));
    heading.set_xalign(0.0);
    heading.add_css_class("menu-heading");
    container.append(&heading);
}

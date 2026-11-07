// SPDX-License-Identifier: MIT

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::{gio, glib, prelude::*};

use crate::{
    app::Browser,
    model::{FolderSort, SortDirection, SortKey, ViewPreferences},
    ui::preferences::{ListColumnId, PreferenceManager},
};

const FIELDS: [(&str, SortKey); 9] = [
    ("Name", SortKey::Name),
    ("Size", SortKey::Size),
    ("Modified", SortKey::Modified),
    ("Type", SortKey::Type),
    ("Extension", SortKey::Extension),
    ("Created", SortKey::Created),
    ("Unsorted", SortKey::Unsorted),
    ("Device order", SortKey::DeviceOrder),
    ("Recency", SortKey::Recency),
];

fn field_id(key: SortKey) -> &'static str {
    match key {
        SortKey::DeviceOrder => "device",
        SortKey::Recency => "recency",
        _ => key.stored_name().unwrap_or("name"),
    }
}

fn field_label(key: SortKey) -> String {
    crate::i18n::tr(
        FIELDS
            .iter()
            .find(|(_, candidate)| *candidate == key)
            .map_or("Name", |(label, _)| *label),
    )
}

struct SortMenu {
    popover: glib::WeakRef<gtk::PopoverMenu>,
    group: gio::SimpleActionGroup,
    browser: std::rc::Weak<Browser>,
    depth: usize,
    model: gio::Menu,
}

impl SortMenu {
    fn build(browser: &Rc<Browser>, depth: usize, list: bool) -> (gtk::PopoverMenu, Self) {
        let root = menu_model(browser, depth, list);
        let popover = gtk::PopoverMenu::from_model_full(&root, gtk::PopoverMenuFlags::NESTED);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::End);
        popover.add_css_class("column-popover");
        popover.add_css_class("sort-menu-popover");
        let group = gio::SimpleActionGroup::new();
        popover.insert_action_group("sorting", Some(&group));
        crate::ui::scrolling::popover::dismiss_on_outside_scroll(popover.upcast_ref());
        super::install_menu_keys(popover.upcast_ref());
        crate::ui::browser::context_menu::prepare_native_menu(&popover);
        let menu = Self {
            popover: popover.downgrade(),
            group,
            model: root,
            browser: Rc::downgrade(browser),
            depth,
        };
        menu.install_actions();
        (popover, menu)
    }

    fn install_actions(&self) {
        let prefs = self
            .browser
            .upgrade()
            .and_then(|browser| browser.column_preferences(self.depth))
            .unwrap_or_default();
        let field = gio::SimpleAction::new_stateful(
            "field",
            Some(&String::static_variant_type()),
            &field_id(prefs.sort_key).to_variant(),
        );
        self.connect(&field, |browser, depth, value| {
            if let Some(value) = value.and_then(|value| value.str())
                && let Some((_, key)) = FIELDS.iter().find(|(_, key)| field_id(*key) == value)
            {
                let active = browser
                    .column_preferences(depth)
                    .is_some_and(|prefs| prefs.sort_key == *key);
                browser.set_sort_key(depth, if active { SortKey::Unsorted } else { *key });
            }
        });
        let names = gio::SimpleAction::new_stateful(
            "names",
            Some(&bool::static_variant_type()),
            &prefs.name_comparison.natural.to_variant(),
        );
        self.connect(&names, |browser, depth, value| {
            if let Some(natural) = value.and_then(|value| value.get::<bool>()) {
                let mut comparison = browser
                    .requested_column_preferences(depth)
                    .unwrap_or_default()
                    .name_comparison;
                comparison.natural = natural;
                browser.set_name_comparison(depth, comparison);
            }
        });
        self.install_direction(prefs);
        self.install_toggles(prefs);
        self.install_defaults();
    }

    fn install_direction(&self, prefs: ViewPreferences) {
        let action = gio::SimpleAction::new_stateful(
            "direction",
            Some(&bool::static_variant_type()),
            &(prefs.sort_direction == SortDirection::Ascending).to_variant(),
        );
        self.connect(&action, |browser, depth, value| {
            if let Some(ascending) = value.and_then(|value| value.get::<bool>()) {
                browser.set_sort_direction(
                    depth,
                    if ascending {
                        SortDirection::Ascending
                    } else {
                        SortDirection::Descending
                    },
                );
            }
        });
    }

    fn install_toggles(&self, prefs: ViewPreferences) {
        for (name, enabled) in [
            ("sensitive", prefs.name_comparison.case_sensitive),
            ("folders", prefs.folders_first),
        ] {
            let action = gio::SimpleAction::new_stateful(name, None, &enabled.to_variant());
            self.connect(&action, move |browser, depth, _| {
                if name == "folders" {
                    browser.toggle_folders_first(depth);
                    return;
                }
                let mut comparison = browser
                    .requested_column_preferences(depth)
                    .unwrap_or_default()
                    .name_comparison;
                comparison.case_sensitive = !comparison.case_sensitive;
                browser.set_name_comparison(depth, comparison);
            });
        }
    }

    fn install_defaults(&self) {
        for name in ["default", "reset"] {
            let action = gio::SimpleAction::new(name, None);
            self.connect(&action, move |browser, depth, _| {
                let manager = PreferenceManager::shared();
                if name == "default" {
                    if let Some(prefs) = browser.requested_column_preferences(depth) {
                        manager.set_default_sort_options(prefs.sort_options());
                    }
                } else if let Some(location) = browser.location_at(depth) {
                    manager.reset_folder_sort(&location);
                    browser.restore_default_sort(depth, manager.default_sort_options());
                }
            });
        }
    }

    fn connect(
        &self,
        action: &gio::SimpleAction,
        run: impl Fn(&Rc<Browser>, usize, Option<&glib::Variant>) + 'static,
    ) {
        let browser = self.browser.clone();
        let depth = self.depth;
        action.connect_activate(move |_, value| {
            if let Some(browser) = browser.upgrade() {
                run(&browser, depth, value);
            }
        });
        self.group.add_action(action);
    }

    fn sync(&self, browser: &Browser, depth: usize) {
        let prefs = browser.column_preferences(depth).unwrap_or_default();
        for (name, value) in [
            ("field", field_id(prefs.sort_key).to_variant()),
            (
                "direction",
                (prefs.sort_direction == SortDirection::Ascending).to_variant(),
            ),
            ("names", prefs.name_comparison.natural.to_variant()),
            (
                "sensitive",
                prefs.name_comparison.case_sensitive.to_variant(),
            ),
            ("folders", prefs.folders_first.to_variant()),
        ] {
            self.action(name).set_state(&value);
        }
        let editable = !matches!(
            prefs.sort_key,
            SortKey::Unsorted | SortKey::DeviceOrder | SortKey::Recency
        );
        self.action("direction").set_enabled(!matches!(
            prefs.sort_key,
            SortKey::Unsorted | SortKey::DeviceOrder
        ));
        self.action("names").set_enabled(editable);
        self.action("sensitive").set_enabled(editable);
        self.action("folders")
            .set_enabled(prefs.sort_key != SortKey::Recency);
        self.action("default").set_enabled(
            !matches!(browser.folder_sort_at(depth), FolderSort::Unremembered)
                && prefs.sort_key.stored_name().is_some()
                && prefs.sort_options() != PreferenceManager::shared().default_sort_options(),
        );
        self.action("reset").set_enabled(matches!(
            browser.folder_sort_at(depth),
            FolderSort::Saved(..)
        ));
        if let Some(popover) = self.popover.upgrade() {
            self.present(popover.upcast_ref(), prefs);
        }
    }

    fn action(&self, name: &str) -> gio::SimpleAction {
        self.group
            .lookup_action(name)
            .and_downcast::<gio::SimpleAction>()
            .expect("installed sort action")
    }

    fn present(&self, widget: &gtk::Widget, prefs: ViewPreferences) {
        let children = children(widget);
        if let Some(popover) = widget.downcast_ref::<gtk::PopoverMenu>() {
            popover.add_css_class("actions-context-popover");
            popover.add_css_class("folder-context-popover");
        }
        if widget.type_().name() == "GtkModelButton" {
            hide_native_indicators(&children);
            keep_sort_menu_open(widget, &self.group, self.model.upcast_ref());
        }
        present_check(widget, &children, prefs);
        for child in children {
            self.present(&child, prefs);
        }
    }
}

fn menu_model(browser: &Browser, depth: usize, list: bool) -> gio::Menu {
    let root = gio::Menu::new();
    let fields = gio::Menu::new();
    append_fields(&fields, browser.location_at(depth).as_ref());
    root.append_section(Some(&crate::i18n::tr("SORT BY")), &fields);
    if list {
        root.append_section(None, &direction_model());
    }
    root.append_section(Some(&crate::i18n::tr("NAME ORDER")), &names_model());
    let folders = gio::Menu::new();
    folders.append(
        Some(&crate::i18n::tr("Folders first")),
        Some("sorting.folders"),
    );
    root.append_section(None, &folders);
    root.append_section(None, &default_model());
    root
}

/// Unsorted has no row: unchecking the active field turns sorting off.
fn append_fields(fields: &gio::Menu, location: Option<&crate::model::Location>) {
    for (label, key) in FIELDS {
        let shown = match key {
            SortKey::Unsorted => false,
            SortKey::DeviceOrder => {
                location.is_some_and(|location| location.is_camera_photo_root())
            }
            SortKey::Recency => location.is_some_and(|location| location.is_recent_root()),
            _ => true,
        };
        if !shown {
            continue;
        }
        let item = gio::MenuItem::new(Some(&crate::i18n::tr(label)), None);
        item.set_action_and_target_value(Some("sorting.field"), Some(&field_id(key).to_variant()));
        fields.append_item(&item);
    }
}

fn names_model() -> gio::Menu {
    let names = gio::Menu::new();
    for (label, value) in [("Natural", true), ("Alphabetical", false)] {
        let item = gio::MenuItem::new(Some(&crate::i18n::tr(label)), None);
        item.set_action_and_target_value(Some("sorting.names"), Some(&value.to_variant()));
        names.append_item(&item);
    }
    names.append(
        Some(&crate::i18n::tr("Case-sensitive names")),
        Some("sorting.sensitive"),
    );
    names
}

fn direction_model() -> gio::Menu {
    let model = gio::Menu::new();
    for (label, ascending) in [("Ascending", true), ("Descending", false)] {
        let item = gio::MenuItem::new(Some(&crate::i18n::tr(label)), None);
        item.set_action_and_target_value(Some("sorting.direction"), Some(&ascending.to_variant()));
        model.append_item(&item);
    }
    model
}

fn default_model() -> gio::Menu {
    let defaults = gio::Menu::new();
    for (label, action) in [
        ("Make default", "default"),
        ("Reset to default sort", "reset"),
    ] {
        let item = gio::MenuItem::new(
            Some(&crate::i18n::tr(label)),
            Some(&format!("sorting.{action}")),
        );
        item.set_attribute_value("hidden-when", Some(&"action-disabled".to_variant()));
        defaults.append_item(&item);
    }
    defaults
}

fn children(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut children = Vec::new();
    let mut next = widget.first_child();
    while let Some(child) = next {
        next = child.next_sibling();
        children.push(child);
    }
    children
}

fn checked_choice(text: &str, prefs: ViewPreferences) -> Option<(bool, gtk::AccessibleRole)> {
    FIELDS
        .iter()
        .find(|(name, _)| crate::i18n::tr(name) == text)
        .map(|(_, key)| (prefs.sort_key == *key, gtk::AccessibleRole::MenuItemRadio))
        .or_else(|| {
            [
                (
                    "Ascending",
                    prefs.sort_direction == SortDirection::Ascending,
                    gtk::AccessibleRole::MenuItemRadio,
                ),
                (
                    "Descending",
                    prefs.sort_direction == SortDirection::Descending,
                    gtk::AccessibleRole::MenuItemRadio,
                ),
                (
                    "Natural",
                    prefs.name_comparison.natural,
                    gtk::AccessibleRole::MenuItemRadio,
                ),
                (
                    "Alphabetical",
                    !prefs.name_comparison.natural,
                    gtk::AccessibleRole::MenuItemRadio,
                ),
                (
                    "Case-sensitive names",
                    prefs.name_comparison.case_sensitive,
                    gtk::AccessibleRole::MenuItemCheckbox,
                ),
                (
                    "Folders first",
                    prefs.folders_first,
                    gtk::AccessibleRole::MenuItemCheckbox,
                ),
            ]
            .into_iter()
            .find(|(name, _, _)| crate::i18n::tr(name) == text)
            .map(|(_, checked, role)| (checked, role))
        })
}

fn present_check(widget: &gtk::Widget, children: &[gtk::Widget], prefs: ViewPreferences) {
    if ![
        gtk::AccessibleRole::MenuItem,
        gtk::AccessibleRole::MenuItemRadio,
        gtk::AccessibleRole::MenuItemCheckbox,
    ]
    .contains(&widget.accessible_role())
    {
        return;
    }
    let Some(label) = children
        .iter()
        .find_map(|child| child.downcast_ref::<gtk::Label>())
    else {
        return;
    };
    let text = label.text();
    let choice = checked_choice(&text, prefs);
    let Some((checked, role)) = choice else {
        return;
    };
    widget.set_accessible_role(role);
    if text == crate::i18n::tr("Device order") {
        crate::ui::accessibility::set_description(
            widget,
            Some(&crate::i18n::tr(
                "Append photos as the device lists them; not necessarily chronological. Selecting this reloads the library.",
            )),
        );
    }
    crate::ui::accessibility::set_checked(widget, checked);
    let check = children
        .iter()
        .find(|child| child.has_css_class("sort-selection-check"))
        .cloned()
        .unwrap_or_else(|| {
            let check = crate::assets::primary_icon(crate::assets::icons::CHECK, 16);
            check.add_css_class("sort-selection-check");
            check.set_margin_start(12);
            check.insert_before(widget, None::<&gtk::Widget>);
            unparent_on_destroy(widget, check.upcast_ref());
            check.upcast()
        });
    label.set_hexpand(true);
    check.set_visible(checked);
}

fn unparent_on_destroy(owner: &gtk::Widget, child: &gtk::Widget) {
    let child = child.downgrade();
    owner.connect_destroy(move |owner| {
        if let Some(child) = child.upgrade()
            && child.parent().as_ref() == Some(owner)
        {
            child.unparent();
        }
    });
}

fn sort_action(model: &gio::MenuModel, label: &str) -> Option<(String, Option<glib::Variant>)> {
    for index in 0..model.n_items() {
        let text = model
            .item_attribute_value(index, "label", None)
            .and_then(|value| value.get::<String>());
        let action = model
            .item_attribute_value(index, "action", None)
            .and_then(|value| value.get::<String>());
        if text.as_deref() == Some(label)
            && let Some(action) =
                action.and_then(|name| name.strip_prefix("sorting.").map(str::to_owned))
        {
            return Some((action, model.item_attribute_value(index, "target", None)));
        }
        if let Some(section) = model.item_link(index, "section")
            && let Some(action) = sort_action(&section, label)
        {
            return Some(action);
        }
    }
    None
}

fn keep_sort_menu_open(
    widget: &gtk::Widget,
    group: &gio::SimpleActionGroup,
    model: &gio::MenuModel,
) {
    if widget.has_css_class("sort-stay-open") {
        return;
    }
    let label = widget.property::<String>("text");
    let Some((action, target)) = sort_action(model, &label) else {
        return;
    };
    widget.add_css_class("sort-stay-open");
    let weak = widget.downgrade();
    let group = group.downgrade();
    let activate = Rc::new(move || {
        if let (Some(_widget), Some(group)) = (weak.upgrade(), group.upgrade()) {
            group.activate_action(&action, target.as_ref());
        }
    });
    let row_actions = gio::SimpleActionGroup::new();
    let row_action = gio::SimpleAction::new("activate", None);
    let keyboard_activate = activate.clone();
    row_action.connect_activate(move |_, _| keyboard_activate());
    row_actions.add_action(&row_action);
    widget.insert_action_group("sort-row", Some(&row_actions));
    remove_native_click_controllers(widget);
    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    click.connect_pressed(|gesture, _, _, _| {
        if let Some(widget) = gesture.widget() {
            widget.grab_focus();
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    let clicked = activate.clone();
    click.connect_released(move |gesture, _, x, y| {
        if gesture.widget().is_some_and(|widget| widget.contains(x, y)) {
            clicked();
        }
    });
    widget.add_controller(click);
}

fn remove_native_click_controllers(widget: &gtk::Widget) {
    // GTK closes native rows before activating them, including on an unpaired release.
    let controllers = widget.observe_controllers();
    let native_clicks: Vec<_> = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
        .collect();
    for controller in native_clicks {
        widget.remove_controller(&controller);
    }
}

fn hide_native_indicators(children: &[gtk::Widget]) {
    for child in children {
        if child.type_().name() == "GtkBuiltinIcon" || self_contained_indicator(child) {
            child.set_visible(false);
        }
    }
}

fn self_contained_indicator(widget: &gtk::Widget) -> bool {
    // Empty indicator boxes still reserve space through GTK's size group.
    widget.downcast_ref::<gtk::Box>().is_some()
        && children(widget)
            .iter()
            .all(|child| child.type_().name() == "GtkBuiltinIcon")
}

pub(in crate::ui) fn column_sort_menu(browser: &Rc<Browser>, depth: usize) -> gtk::MenuButton {
    sort_menu_button(browser, depth, false)
}

pub(in crate::ui) fn list_sort_menu(browser: &Rc<Browser>, depth: usize) -> gtk::MenuButton {
    sort_menu_button(browser, depth, true)
}

thread_local! {
    static OPEN_MENUS: RefCell<Vec<Weak<SortMenu>>> = const { RefCell::new(Vec::new()) };
}

pub(in crate::ui) fn sync_open_sort_menus(browser: &Browser, depth: usize) {
    let open: Vec<Rc<SortMenu>> =
        OPEN_MENUS.with_borrow(|menus| menus.iter().filter_map(Weak::upgrade).collect());
    for menu in open {
        if menu.depth == depth
            && menu
                .browser
                .upgrade()
                .is_some_and(|owner| std::ptr::eq(owner.as_ref(), browser))
        {
            menu.sync(browser, depth);
        }
    }
}

fn track_open_menu(popover: &gtk::PopoverMenu, menu: &Rc<SortMenu>) {
    let weak_browser = menu.browser.clone();
    let opened = menu.clone();
    popover.connect_map(move |_| {
        OPEN_MENUS.with_borrow_mut(|menus| {
            menus.retain(|open| open.strong_count() > 0 && !open.ptr_eq(&Rc::downgrade(&opened)));
            menus.push(Rc::downgrade(&opened));
        });
        if let Some(browser) = weak_browser.upgrade() {
            opened.sync(&browser, opened.depth);
        }
    });
    let closed = Rc::downgrade(menu);
    popover.connect_unmap(move |_| {
        OPEN_MENUS.with_borrow_mut(|menus| {
            menus.retain(|open| open.strong_count() > 0 && !open.ptr_eq(&closed));
        });
    });
    let observed = Rc::downgrade(menu);
    PreferenceManager::shared().bind_preference(
        popover,
        |manager| {
            (
                manager.default_sort_options(),
                manager.folder_sorts_revision(),
            )
        },
        move |popover, _| {
            if popover.is_visible()
                && let Some(menu) = observed.upgrade()
                && let Some(browser) = menu.browser.upgrade()
            {
                menu.sync(&browser, menu.depth);
            }
        },
    );
}

fn sort_menu_button(browser: &Rc<Browser>, depth: usize, list: bool) -> gtk::MenuButton {
    let (popover, menu) = SortMenu::build(browser, depth, list);
    let menu = Rc::new(menu);
    let button = gtk::MenuButton::builder().popover(&popover).build();
    button.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::SETTINGS_2,
    )));
    crate::ui::accessibility::set_tooltip_and_label(&button, &crate::i18n::tr("Choose sort field"));
    crate::ui::controls::pane_header_action(&button);
    button.add_css_class("sort-menu-button");
    crate::ui::tenxer_mode::hide_while_enabled(&button);
    menu.sync(browser, depth);
    track_open_menu(&popover, &menu);
    button
}

pub(in crate::ui) fn sync_list_sort_menu(
    browser: &Browser,
    depth: usize,
    button: &gtk::MenuButton,
) {
    let prefs = browser.column_preferences(depth).unwrap_or_default();
    let hidden = PreferenceManager::shared().list_hidden_columns();
    let special_names = has_special_name_order(prefs);
    let labelled = needs_sort_label(prefs, &hidden);
    if !labelled {
        button.set_child(Some(&crate::assets::chrome_icon(
            crate::assets::icons::SETTINGS_2,
        )));
        button.set_tooltip_text(Some(&crate::i18n::tr("Choose sort field")));
        crate::ui::accessibility::set_label(button, &crate::i18n::tr("Choose sort field"));
        return;
    }
    let mut name = field_label(prefs.sort_key);
    if prefs.sort_key == SortKey::Name && special_names {
        name = rust_i18n::t!(
            "%{field} · %{order}",
            field = name,
            order = crate::i18n::tr(if prefs.name_comparison.natural {
                "Natural"
            } else {
                "Alphabetical"
            })
        )
        .to_string();
        if prefs.name_comparison.case_sensitive {
            name = rust_i18n::t!("%{field} · case-sensitive", field = name).to_string();
        }
    }
    let label = rust_i18n::t!(
        "Sort: %{field} %{direction}",
        field = name,
        direction = if prefs.sort_direction == SortDirection::Ascending {
            "↑"
        } else {
            "↓"
        }
    );
    button.set_child(Some(&gtk::Label::new(Some(&label))));
    button.set_tooltip_text(None);
    crate::ui::accessibility::set_label(button, &label);
}

fn has_special_name_order(prefs: ViewPreferences) -> bool {
    prefs.sort_key == SortKey::Name
        && (!prefs.name_comparison.natural || prefs.name_comparison.case_sensitive)
}

fn needs_sort_label(prefs: ViewPreferences, hidden: &[ListColumnId]) -> bool {
    let matching = match prefs.sort_key {
        SortKey::Name => Some(ListColumnId::Name),
        SortKey::Size => Some(ListColumnId::Size),
        SortKey::Type => Some(ListColumnId::Type),
        SortKey::Modified => Some(ListColumnId::Modified),
        _ => None,
    };
    !matches!(prefs.sort_key, SortKey::Unsorted | SortKey::DeviceOrder)
        && (matching.is_none_or(|column| hidden.contains(&column)) || has_special_name_order(prefs))
}

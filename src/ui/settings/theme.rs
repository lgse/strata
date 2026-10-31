// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk, gio, glib, prelude::*};

use crate::{
    assets::icons,
    ui::{
        controls::{number_stepper, segmented_control},
        preferences::{
            CornerRoundness, FileIconColors, InterfaceRenderer, OmarchyVariant, PreferenceManager,
            TextSize,
        },
        theme::{Theme, ThemeManager, ThemeTokens},
    },
};

use super::{
    append_heading,
    bindings::{bind_number, bind_switch, bind_theme_switch},
    page_content, scrollable_page,
};

mod editor;
#[cfg(test)]
mod tests;
use editor::theme_editor;

pub(super) struct ThemePage {
    pub(super) widget: gtk::Widget,
    pub(super) flows: Vec<(gtk::FlowBox, u32)>,
    /// Discards an unsaved theme preview when Settings closes.
    pub(super) dismiss: Rc<dyn Fn()>,
}

pub(super) fn theme_page(
    preferences: Rc<PreferenceManager>,
    themes: Rc<ThemeManager>,
) -> ThemePage {
    let content = page_content();
    content.add_css_class("theme-page");

    let system = super::settings_group(&content, "THEME");
    let follow = append_follow_omarchy_option(&system, &themes);
    append_omarchy_variant_option(&system, &preferences, &themes);
    let current = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    current.set_valign(gtk::Align::Center);
    current.set_halign(gtk::Align::Start);
    let current_row = super::control_row("Current theme", "", &current);
    let description = current_row
        .first_child()
        .and_downcast::<gtk::Box>()
        .expect("current theme row copy")
        .last_child()
        .and_downcast::<gtk::Label>()
        .expect("current theme description");
    let weak_description = description.downgrade();
    themes.bind_theme_preference(
        &current,
        |manager| (manager.follows_omarchy(), manager.appearance_tokens()),
        move |widget, (following, tokens)| {
            let current = widget
                .downcast_ref::<gtk::Box>()
                .expect("current theme preview");
            while let Some(child) = current.first_child() {
                current.remove(&child);
            }
            current.append(&theme_preview(&tokens));
            let name = gtk::Label::new(Some(&tokens.name));
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            name.set_max_width_chars(24);
            current.append(&name);
            if let Some(description) = weak_description.upgrade() {
                description.set_visible(true);
                description.set_text(&crate::i18n::tr(if following {
                    "Managed by Omarchy. Turn off Follow Omarchy to pick a theme manually."
                } else {
                    "Choose a theme from the library below."
                }));
            }
        },
    );
    system.append(&current_row);

    let library = gtk::Box::new(gtk::Orientation::Vertical, 0);
    library.add_css_class("theme-library");
    super::search::tag(&library, "Theme library");
    content.append(&library);
    let catalog = append_theme_catalog(&library);
    let custom = theme_grid();
    catalog.rows.append(&custom);
    fill_theme_grids(&catalog.packaged, &custom, &themes);
    bind_catalog_filter(
        [&catalog.packaged, &custom],
        catalog.search,
        catalog.clear,
        catalog.appearance_buttons,
    );
    let (editor_fields, dismiss) = append_custom_theme_editor(&catalog.container, &custom, &themes);
    themes.bind_theme_preference(
        &library,
        ThemeManager::follows_omarchy,
        |widget, following| widget.set_sensitive(!following),
    );
    let rendering = super::settings_group(&content, "RENDERING");
    let renderer = super::bindings::choice_menu(
        &preferences,
        "Interface renderer",
        &[
            ("GTK default", InterfaceRenderer::System),
            ("Cairo", InterfaceRenderer::Cairo),
        ],
        PreferenceManager::interface_renderer,
        PreferenceManager::set_interface_renderer,
    );
    let renderer_controls = gtk::Box::new(gtk::Orientation::Vertical, 8);
    renderer_controls.set_hexpand(true);
    renderer.set_hexpand(true);
    renderer.set_halign(gtk::Align::Fill);
    renderer_controls.append(&renderer);
    let restart = gtk::Button::with_label(&crate::i18n::tr("Restart now"));
    restart.set_hexpand(true);
    restart.set_halign(gtk::Align::Fill);
    restart.add_css_class("settings-action-button");
    crate::ui::accessibility::set_label(
        &restart,
        &crate::i18n::tr("Restart to apply interface renderer"),
    );
    preferences.bind_preference(
        &restart,
        PreferenceManager::interface_renderer_restart_required,
        |widget, required| widget.set_visible(required),
    );
    restart.connect_clicked(|_| {
        let application = gio::Application::default().and_downcast::<gtk::Application>();
        super::restart(application.as_ref());
    });
    renderer_controls.append(&restart);
    let renderer_row = super::control_row(
        "Interface renderer",
        "Cairo avoids text artifacts on some displays. Restart to apply changes; GSK_RENDERER overrides this choice.",
        &renderer_controls,
    );
    renderer_row.add_css_class("settings-renderer-row");
    renderer_row.set_orientation(gtk::Orientation::Vertical);
    rendering.append(&renderer_row);

    append_text_size_option(&content, &preferences);
    append_corner_roundness_option(&content, &preferences);
    append_file_icon_colors_option(&content, &preferences);
    let effects = super::settings_group(&content, "EFFECTS");
    let (row, toggle) = super::settings_option(
        "Element glow",
        "Show accent glow around dialogs, menus, and other elements.",
        preferences.element_glow(),
    );
    bind_switch(
        &preferences,
        &toggle,
        PreferenceManager::element_glow,
        PreferenceManager::set_element_glow,
    );
    effects.append(&row);
    let motion = super::settings_group(&content, "MOTION");
    let (row, toggle) = super::settings_option(
        "Reduce motion",
        "Disable nonessential interface animations.",
        preferences.reduce_motion(),
    );
    bind_switch(
        &preferences,
        &toggle,
        PreferenceManager::reduce_motion,
        PreferenceManager::set_reduce_motion,
    );
    motion.append(&row);

    let scroller = scrollable_page(&content, None);
    bind_theme_switch(
        &themes,
        &follow,
        ThemeManager::follows_omarchy,
        ThemeManager::set_follow_omarchy,
    );
    ThemePage {
        widget: scroller,
        flows: vec![(catalog.packaged, 1), (custom, 1), (editor_fields, 4)],
        dismiss,
    }
}

struct ThemeCatalog {
    packaged: gtk::FlowBox,
    container: gtk::Box,
    rows: gtk::Box,
    search: gtk::Entry,
    clear: gtk::Button,
    appearance_buttons: Vec<gtk::ToggleButton>,
}

fn append_theme_catalog(content: &gtk::Box) -> ThemeCatalog {
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    toolbar.add_css_class("settings-library-toolbar");
    let heading = append_heading(&toolbar, "THEME LIBRARY");
    heading.set_hexpand(true);
    let packaged = theme_grid();
    let (search_overlay, theme_search, clear_search) = super::search_field("Search themes");
    theme_search.add_css_class("theme-search");
    toolbar.append(&search_overlay);
    let (control, appearance_buttons) = segmented_control(&["All", "Light", "Dark"], 0);
    let appearance_filter = super::wrap::WrapRow::new(0);
    appearance_filter.add_css_class("segmented-control");
    for button in &appearance_buttons {
        control.remove(button);
        button.set_hexpand(false);
        appearance_filter.append(button);
    }
    appearance_filter.add_css_class("theme-appearance-filter");
    for button in &appearance_buttons {
        if let Some(label) = button.child().and_downcast::<gtk::Label>() {
            label.add_css_class("settings-control-label");
        }
    }
    appearance_filter.set_hexpand(false);
    toolbar.append(&appearance_filter);
    content.append(&toolbar);
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
    rows.append(&packaged);
    let catalog_scroll = gtk::ScrolledWindow::builder()
        .child(&rows)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(336)
        .max_content_height(336)
        .propagate_natural_height(true)
        .build();
    catalog_scroll.add_css_class("theme-catalog-scroll");
    let catalog_container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    catalog_container.add_css_class("theme-catalog-container");
    catalog_container.append(&catalog_scroll);
    content.append(&catalog_container);
    ThemeCatalog {
        packaged,
        container: catalog_container,
        rows,
        search: theme_search,
        clear: clear_search,
        appearance_buttons,
    }
}

fn fill_theme_grids(packaged: &gtk::FlowBox, custom: &gtk::FlowBox, themes: &Rc<ThemeManager>) {
    for theme in themes.themes() {
        let flow = if theme.custom { custom } else { packaged };
        append_theme_card(flow, theme, themes);
    }
}

fn append_custom_theme_editor(
    content: &gtk::Box,
    custom: &gtk::FlowBox,
    themes: &Rc<ThemeManager>,
) -> (gtk::FlowBox, Rc<dyn Fn()>) {
    let add = add_theme_card_button();
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    footer.add_css_class("theme-library-footer");
    let location = gtk::Label::new(Some(&crate::i18n::tr(
        "Custom themes live in ~/.config/strata/themes",
    )));
    location.set_xalign(0.0);
    location.set_hexpand(true);
    location.set_wrap(true);
    footer.append(&location);
    footer.append(&add);
    content.append(&footer);
    bind_new_custom_themes(custom, themes);
    let editor = theme_editor(themes.clone());
    editor.revealer.set_reveal_child(false);
    content.append(&editor.revealer);
    let reveal = editor.reveal;
    add.connect_clicked(move |_| reveal());
    (editor.fields, editor.dismiss)
}

fn append_follow_omarchy_option(content: &gtk::Box, themes: &Rc<ThemeManager>) -> gtk::Switch {
    let (row, follow) = super::settings_option(
        "Follow Omarchy",
        "Use the active Omarchy Quattro theme and switch when the system theme changes.",
        themes.follows_omarchy(),
    );
    content.append(&row);
    themes.bind_theme_preference(
        &row,
        ThemeManager::is_omarchy_available,
        super::search::set_available,
    );
    follow
}

fn append_file_icon_colors_option(content: &gtk::Box, preferences: &Rc<PreferenceManager>) {
    let group = super::settings_group(content, "FILE ICONS");
    let choice = super::bindings::choice_menu(
        preferences,
        "File icon colors",
        &[
            ("Brand colors", FileIconColors::Brand),
            ("Theme colors", FileIconColors::Theme),
        ],
        PreferenceManager::file_icon_colors,
        PreferenceManager::set_file_icon_colors,
    );
    group.append(&super::control_row(
        "File icon colors",
        "Brand colors use each file type's own palette; theme colors use the accent.",
        &choice,
    ));
    let (row, toggle) = super::settings_option(
        "Show thumbnails",
        "Show thumbnails instead of file-type icons. The preview panel stays available.",
        preferences.show_thumbnails(),
    );
    bind_switch(
        preferences,
        &toggle,
        PreferenceManager::show_thumbnails,
        PreferenceManager::set_show_thumbnails,
    );
    group.append(&row);
    let (row, toggle) = super::settings_option(
        "STL model thumbnails",
        "Render local STL models as file thumbnails, one at a time. Requires Show thumbnails.",
        preferences.stl_thumbnails(),
    );
    bind_switch(
        preferences,
        &toggle,
        PreferenceManager::stl_thumbnails,
        PreferenceManager::set_stl_thumbnails,
    );
    group.append(&row);
}

fn append_omarchy_variant_option(
    content: &gtk::Box,
    preferences: &Rc<PreferenceManager>,
    themes: &Rc<ThemeManager>,
) {
    let choice = super::bindings::choice_menu(
        preferences,
        "Omarchy variant",
        &[
            ("Lighter", OmarchyVariant::Lighter),
            ("Darker", OmarchyVariant::Darker),
            ("Contrast", OmarchyVariant::Contrast),
        ],
        PreferenceManager::omarchy_variant,
        PreferenceManager::set_omarchy_variant,
    );
    let row = super::control_row(
        "Omarchy variant",
        "Lighter or darker file columns, or stronger contrast. Keeps following your system theme.",
        &choice,
    );
    super::indent_row(&row);
    themes.bind_theme_preference(
        &row,
        |manager| (manager.is_omarchy_available(), manager.follows_omarchy()),
        |widget, (available, following)| {
            super::search::set_available(widget, available);
            widget.set_sensitive(following);
        },
    );
    content.append(&row);
}

fn append_text_size_option(content: &gtk::Box, preferences: &Rc<PreferenceManager>) {
    let group = super::settings_group(content, "TEXT");
    group.append(&super::control_row(
        "Interface font",
        "The typeface for menus, file lists, and dialogs. File previews keep a monospace font.",
        &interface_font_choice(preferences),
    ));
    let text_size_control = number_stepper(TextSize::MIN, TextSize::MAX, 1);
    crate::ui::accessibility::set_label(
        &text_size_control,
        &crate::i18n::tr("Text size in pixels"),
    );
    bind_number(
        preferences,
        &text_size_control,
        |manager| f64::from(manager.text_size().root_font_px()),
        |manager, value| manager.set_text_size(TextSize::new(value as u32)),
    );
    text_size_control.set_halign(gtk::Align::Start);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    controls.set_valign(gtk::Align::Center);
    controls.append(&text_size_control);
    let text_size_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    text_size_row.add_css_class("settings-option");
    text_size_row.add_css_class("settings-text-size-row");
    super::search::tag(&text_size_row, "Text size");
    let text_size_copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text_size_copy.set_hexpand(true);
    let text_size_title = gtk::Label::new(Some(&crate::i18n::tr("Text size")));
    text_size_title.set_xalign(0.0);
    text_size_title.add_css_class("settings-option-title");
    let text_size_description = gtk::Label::new(Some(&crate::i18n::tr(
        "Logical pixels, 8–48. Display scaling applies on top.",
    )));
    text_size_description.set_xalign(0.0);
    text_size_description.set_wrap(true);
    text_size_description.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    text_size_description.add_css_class("settings-option-description");
    let description = gtk::Box::new(gtk::Orientation::Vertical, 8);
    description.append(&text_size_description);
    let keys = super::wrap::WrapRow::new(6);
    keys.set_halign(gtk::Align::Start);
    keys.set_hexpand(false);
    keys.set_valign(gtk::Align::Center);
    crate::ui::shortcut_reference::follow_keys(&keys, |keys| {
        use crate::ui::keys::{
            self as shortcuts, Binding, Keys,
            bindings::{TEXT_LARGER, TEXT_RESET, TEXT_SMALLER},
        };
        while let Some(child) = keys.first_child() {
            child.unparent();
        }
        let keycap = |text: &str, class| {
            let label = gtk::Label::new(Some(text));
            label.add_css_class("settings-nowrap");
            label.add_css_class(class);
            keys.append(&label);
        };
        let bindings: Vec<Binding> = [TEXT_LARGER, TEXT_SMALLER, TEXT_RESET]
            .into_iter()
            .filter_map(|command| command.primary())
            .collect();
        // Shared modifiers, as the shipped bindings have, are shown once.
        let shared = bindings
            .first()
            .map(|first| first.modifiers())
            .filter(|modifiers| {
                !modifiers.is_empty()
                    && bindings
                        .iter()
                        .all(|binding| binding.modifiers() == *modifiers)
            });
        if let Some(modifiers) = shared {
            keycap(&shortcuts::modifiers_label(modifiers), "settings-keycap");
        }
        for (index, binding) in bindings.into_iter().enumerate() {
            if index > 0 {
                keycap("/", "keycap-separator");
            }
            let shown = if shared.is_some() {
                Binding::plain(binding.key())
            } else {
                binding
            };
            keycap(
                &shortcuts::label(&Keys::One(shown.shown())),
                "settings-keycap",
            );
        }
    });
    description.append(&keys);
    text_size_copy.append(&text_size_title);
    text_size_copy.append(&description);
    text_size_row.append(&text_size_copy);
    text_size_row.append(&controls);
    group.append(&text_size_row);
}

/// Lists installed families when first opened. Names use the interface font:
/// loading each family to preview it stalls opening and scrolling.
fn interface_font_choice(preferences: &Rc<PreferenceManager>) -> gtk::MenuButton {
    // An empty string stands for Default.
    let families = gtk::StringList::new(&[]);
    let query: Rc<RefCell<String>> = Rc::default();
    let filter = {
        let query = query.clone();
        gtk::CustomFilter::new(move |item| {
            let query = query.borrow();
            item.downcast_ref::<gtk::StringObject>()
                .is_some_and(|family| {
                    font_label(&family.string())
                        .to_lowercase()
                        .contains(query.as_str())
                })
        })
    };
    let filtered = gtk::FilterListModel::new(Some(families.clone()), Some(filter.clone()));
    let rows: Rc<RefCell<Vec<glib::WeakRef<gtk::ListItem>>>> = Rc::default();
    let factory = gtk::SignalListItemFactory::new();
    {
        let rows = rows.clone();
        factory.connect_setup(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let label = gtk::Label::new(None);
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let check = crate::assets::primary_icon(icons::CHECK, 16);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.append(&label);
            row.append(&check);
            item.set_child(Some(&row));
            rows.borrow_mut().push(item.downgrade());
        });
    }
    {
        let preferences = preferences.clone();
        factory.connect_bind(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(family) = item.item().and_downcast::<gtk::StringObject>() else {
                return;
            };
            let family = family.string();
            let Some(row) = item.child() else {
                return;
            };
            let (Some(label), Some(check)) = (
                row.first_child().and_downcast::<gtk::Label>(),
                row.last_child(),
            ) else {
                return;
            };
            label.set_text(&font_label(&family));
            check.set_visible(is_chosen_font(&preferences, &family));
        });
    }
    let list = gtk::ListView::new(
        Some(gtk::NoSelection::new(Some(filtered.clone()))),
        Some(factory),
    );
    list.set_single_click_activate(true);
    list.add_css_class("column-menu");
    list.add_css_class("font-choice-list");
    crate::ui::accessibility::set_label(&list, &crate::i18n::tr("Interface font"));
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(320)
        .min_content_width(240)
        .build();
    scroll.add_css_class("context-menu-scroll");
    let search = crate::ui::controls::form_entry();
    search.set_placeholder_text(Some(&crate::i18n::tr("Search fonts")));
    crate::ui::accessibility::set_label(&search, &crate::i18n::tr("Search fonts"));
    let empty = gtk::Label::new(Some(&crate::i18n::tr("No fonts match")));
    empty.add_css_class("settings-option-description");
    empty.set_visible(false);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.append(&search);
    body.append(&scroll);
    body.append(&empty);
    let popover = gtk::Popover::builder()
        .child(&body)
        .has_arrow(false)
        .build();
    popover.add_css_class("column-popover");
    let button = gtk::MenuButton::builder()
        .popover(&popover)
        .always_show_arrow(true)
        .valign(gtk::Align::Center)
        .build();
    button.add_css_class("form-control");
    button.add_css_class("settings-choice");
    crate::ui::accessibility::set_label(&button, &crate::i18n::tr("Interface font"));
    preferences.bind_preference(
        &button,
        PreferenceManager::interface_font,
        |widget, family| {
            if let Some(button) = widget.downcast_ref::<gtk::MenuButton>() {
                super::bindings::show_choice_value(
                    button,
                    &family.unwrap_or_else(|| crate::i18n::tr("Default")),
                );
            }
        },
    );
    {
        let rows = rows.clone();
        let weak_preferences = Rc::downgrade(preferences);
        preferences.bind_preference(&list, PreferenceManager::interface_font, move |_, _| {
            let Some(preferences) = weak_preferences.upgrade() else {
                return;
            };
            rows.borrow_mut().retain(|item| {
                let Some(item) = item.upgrade() else {
                    return false;
                };
                let family = item
                    .item()
                    .and_downcast::<gtk::StringObject>()
                    .map(|family| family.string());
                if let Some(check) = item.child().and_then(|row| row.last_child()) {
                    check.set_visible(
                        family.is_some_and(|family| is_chosen_font(&preferences, &family)),
                    );
                }
                true
            });
        });
    }
    {
        let empty = empty.downgrade();
        let scroll = scroll.downgrade();
        let filtered = filtered.clone();
        search.connect_changed(move |search| {
            query.replace(search.text().trim().to_lowercase());
            filter.changed(gtk::FilterChange::Different);
            let any = filtered.n_items() > 0;
            if let (Some(empty), Some(scroll)) = (empty.upgrade(), scroll.upgrade()) {
                empty.set_visible(!any);
                scroll.set_visible(any);
            }
        });
    }
    {
        let preferences = preferences.clone();
        let weak_button = button.downgrade();
        list.connect_activate(move |list, position| {
            let Some(family) = list
                .model()
                .and_then(|model| model.item(position))
                .and_downcast::<gtk::StringObject>()
            else {
                return;
            };
            let family = family.string();
            preferences.set_interface_font((!family.is_empty()).then(|| family.to_string()));
            if let Some(button) = weak_button.upgrade() {
                button.popdown();
            }
        });
    }
    popover.connect_show(move |popover| {
        if families.n_items() == 0 {
            let names = std::iter::once(String::new())
                .chain(installed_font_families(popover))
                .collect::<Vec<_>>();
            families.splice(0, 0, &names.iter().map(String::as_str).collect::<Vec<_>>());
        }
        search.set_text("");
        search.grab_focus();
    });
    button
}

fn font_label(family: &str) -> String {
    if family.is_empty() {
        crate::i18n::tr("Default")
    } else {
        family.to_owned()
    }
}

fn is_chosen_font(preferences: &PreferenceManager, family: &str) -> bool {
    preferences.interface_font().as_deref().unwrap_or_default() == family
}

fn installed_font_families(widget: &impl IsA<gtk::Widget>) -> impl Iterator<Item = String> {
    let mut families = widget
        .pango_context()
        .list_families()
        .iter()
        .map(|family| family.name().to_string())
        .collect::<Vec<_>>();
    families.sort_by_cached_key(|family| family.to_lowercase());
    families.dedup();
    families.into_iter()
}

fn append_corner_roundness_option(content: &gtk::Box, preferences: &Rc<PreferenceManager>) {
    let group = super::settings_group(content, "SHAPE");
    let control = number_stepper(
        CornerRoundness::MIN,
        CornerRoundness::MAX,
        CornerRoundness::STEP,
    );
    crate::ui::accessibility::set_label(&control, &crate::i18n::tr("Corner roundness in percent"));
    bind_number(
        preferences,
        &control,
        |manager| f64::from(manager.corner_roundness().percent()),
        |manager, value| manager.set_corner_roundness(CornerRoundness::new(value as u32)),
    );
    control.set_halign(gtk::Align::Start);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    controls.set_valign(gtk::Align::Center);
    controls.append(&control);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("settings-option");
    row.add_css_class("settings-text-size-row");
    super::search::tag(&row, "Corner roundness");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    let title = gtk::Label::new(Some(&crate::i18n::tr("Corner roundness")));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let description = gtk::Label::new(Some(&crate::i18n::tr(
        "Percent of the default corner radius, 0–200. 0 squares every corner.",
    )));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    description.add_css_class("settings-option-description");
    copy.append(&title);
    copy.append(&description);
    row.append(&copy);
    row.append(&controls);
    group.append(&row);
}

fn theme_grid() -> gtk::FlowBox {
    let grid = gtk::FlowBox::builder()
        .column_spacing(0)
        .row_spacing(0)
        .max_children_per_line(1)
        .min_children_per_line(1)
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .build();
    grid.add_css_class("theme-grid");
    grid
}

fn catalog_card_visible(appearance: ThemeAppearance, light: bool, name: &str, query: &str) -> bool {
    let appearance_matches = match appearance {
        ThemeAppearance::All => true,
        ThemeAppearance::Dark => !light,
        ThemeAppearance::Light => light,
    };
    appearance_matches && theme_name_matches(name, query)
}

fn bind_catalog_filter(
    flows: [&gtk::FlowBox; 2],
    theme_search: gtk::Entry,
    clear_search: gtk::Button,
    appearance_buttons: Vec<gtk::ToggleButton>,
) {
    let query = Rc::new(RefCell::new(String::new()));
    let appearance = Rc::new(Cell::new(ThemeAppearance::All));
    for flow in flows {
        let query = query.clone();
        let appearance = appearance.clone();
        flow.set_filter_func(move |child| {
            let Some(card) = child.child() else {
                return false;
            };
            catalog_card_visible(
                appearance.get(),
                card.has_css_class("light"),
                card.widget_name().as_str(),
                &query.borrow(),
            )
        });
    }
    let weak_flows = flows.map(|flow| flow.downgrade());
    let search_flows = weak_flows.clone();
    theme_search.connect_changed(move |search| {
        *query.borrow_mut() = search.text().to_string();
        clear_search.set_visible(!search.text().is_empty());
        for flow in search_flows.iter().filter_map(glib::WeakRef::upgrade) {
            flow.invalidate_filter();
        }
    });
    for (button, value) in appearance_buttons.into_iter().zip([
        ThemeAppearance::All,
        ThemeAppearance::Light,
        ThemeAppearance::Dark,
    ]) {
        let appearance = appearance.clone();
        let weak_flows = weak_flows.clone();
        button.connect_toggled(move |button| {
            if button.is_active() {
                appearance.set(value);
                for flow in weak_flows.iter().filter_map(glib::WeakRef::upgrade) {
                    flow.invalidate_filter();
                }
            }
        });
    }
}

fn add_theme_card_button() -> gtk::Button {
    let add = gtk::Button::new();
    add.add_css_class("add-theme-card");
    add.set_has_frame(false);
    let add_content = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    add_content.set_halign(gtk::Align::Center);
    add_content.set_valign(gtk::Align::Center);
    let plus = crate::assets::primary_icon(icons::PLUS, 16);
    let add_label = gtk::Label::new(Some(&crate::i18n::tr("Add theme")));
    add_label.set_wrap(true);
    add_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    add_content.append(&plus);
    add_content.append(&add_label);
    add.set_child(Some(&add_content));
    add
}

fn bind_new_custom_themes(custom: &gtk::FlowBox, themes: &Rc<ThemeManager>) {
    let known_custom = RefCell::new(
        themes
            .themes()
            .into_iter()
            .filter(|theme| theme.custom)
            .map(|theme| theme.id)
            .collect::<std::collections::HashSet<_>>(),
    );
    let weak_manager = Rc::downgrade(themes);
    themes.bind_theme_preference(
        custom,
        |themes| {
            themes
                .themes()
                .into_iter()
                .filter(|theme| theme.custom)
                .map(|theme| theme.id)
                .collect::<Vec<_>>()
        },
        move |widget, _| {
            let Some(flow) = widget.downcast_ref::<gtk::FlowBox>() else {
                return;
            };
            if let Some(manager) = weak_manager.upgrade() {
                for theme in manager.themes().into_iter().filter(|theme| theme.custom) {
                    if known_custom.borrow_mut().insert(theme.id.clone()) {
                        append_theme_card(flow, theme, &manager);
                    }
                }
            }
        },
    );
}

#[derive(Clone, Copy)]
enum ThemeAppearance {
    All,
    Dark,
    Light,
}

pub(super) fn theme_name_matches(name: &str, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty() || name.to_lowercase().contains(&query)
}

fn theme_is_light(tokens: &ThemeTokens) -> bool {
    theme_background_is_light(&tokens.background)
}

pub(super) fn theme_background_is_light(background: &str) -> bool {
    let Some(color) = crate::ui::theme::parse_rgb_channels(background) else {
        return false;
    };
    let channel = |index: usize| {
        let value = f64::from(color[index]) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * channel(0) + 0.7152 * channel(1) + 0.0722 * channel(2);
    luminance > 0.4
}

fn append_theme_card(
    flow: &gtk::FlowBox,
    theme: Theme,
    themes: &Rc<ThemeManager>,
) -> gtk::FlowBoxChild {
    let card = gtk::Button::new();
    card.add_css_class("theme-card");
    card.set_widget_name(&theme.tokens.name);
    if theme_is_light(&theme.tokens) {
        card.add_css_class("light");
    }
    card.set_has_frame(false);
    card.set_overflow(gtk::Overflow::Visible);
    let content = super::wrap::WrapRow::new(16);
    content.append(&theme_preview(&theme.tokens));
    let label = gtk::Label::new(Some(&theme.tokens.name));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_lines(2);
    content.append(&label);
    let kind = gtk::Label::new(Some(&crate::i18n::tr(if theme_is_light(&theme.tokens) {
        "LIGHT"
    } else {
        "DARK"
    })));
    kind.add_css_class("theme-kind");
    let metadata = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    metadata.append(&kind);
    content.append(&metadata);
    let check = crate::assets::primary_icon(icons::CHECK, 14);
    let selected = !themes.follows_omarchy() && themes.selected_id() == theme.id;
    check.set_visible(selected);
    metadata.append(&check);
    if selected {
        card.add_css_class("selected");
    }
    card.set_child(Some(&content));
    let theme_id = theme.id;
    let selected_theme = theme_id.clone();
    let check = check.downgrade();
    themes.bind_theme_preference(
        &card,
        move |themes| !themes.follows_omarchy() && themes.selected_id() == selected_theme,
        move |card, selected| {
            if selected {
                card.add_css_class("selected");
            } else {
                card.remove_css_class("selected");
            }
            if let Some(check) = check.upgrade() {
                check.set_visible(selected);
            }
        },
    );
    let themes = themes.clone();
    card.connect_clicked(move |_| {
        themes.select_theme(&theme_id);
    });
    flow.insert(&card, -1);
    card.parent()
        .and_downcast::<gtk::FlowBoxChild>()
        .expect("FlowBox must wrap inserted theme cards")
}

fn theme_preview(tokens: &ThemeTokens) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.add_css_class("theme-preview");
    area.set_content_width(80);
    area.set_content_height(28);
    area.set_valign(gtk::Align::Center);
    let tokens = tokens.clone();
    area.set_draw_func(move |_, context, width, height| {
        let color = |value: &str| gdk::RGBA::parse(value).unwrap_or(gdk::RGBA::BLACK);
        let paint = |context: &gtk::cairo::Context, value: &str| {
            let value = color(value);
            context.set_source_rgba(
                f64::from(value.red()),
                f64::from(value.green()),
                f64::from(value.blue()),
                1.0,
            );
        };
        context.rounded_rectangle(RoundedRect {
            x: 0.0,
            y: 0.0,
            width: f64::from(width),
            height: f64::from(height),
            radius: 6.0,
        });
        context.clip();
        paint(context, &tokens.background);
        context.rectangle(0.0, 0.0, f64::from(width), f64::from(height));
        let _ = context.fill();
        for (index, value) in [
            &tokens.accent,
            &tokens.danger,
            &tokens.highlight,
            &tokens.dim_text,
        ]
        .into_iter()
        .enumerate()
        {
            paint(context, value);
            context.rounded_rectangle(RoundedRect {
                x: 8.0 + index as f64 * 18.0,
                y: (f64::from(height) - 10.0) / 2.0,
                width: if index == 3 { 6.0 } else { 14.0 },
                height: 10.0,
                radius: 5.0,
            });
            let _ = context.fill();
        }
    });
    area
}

struct RoundedRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
}

trait RoundedRectangle {
    fn rounded_rectangle(&self, rect: RoundedRect);
}
impl RoundedRectangle for gtk::cairo::Context {
    fn rounded_rectangle(&self, rect: RoundedRect) {
        let degrees = std::f64::consts::PI / 180.0;
        let x = rect.x;
        let y = rect.y;
        let width = rect.width;
        let height = rect.height;
        let radius = rect.radius;
        self.new_sub_path();
        self.arc(x + width - radius, y + radius, radius, -90.0 * degrees, 0.0);
        self.arc(
            x + width - radius,
            y + height - radius,
            radius,
            0.0,
            90.0 * degrees,
        );
        self.arc(
            x + radius,
            y + height - radius,
            radius,
            90.0 * degrees,
            180.0 * degrees,
        );
        self.arc(
            x + radius,
            y + radius,
            radius,
            180.0 * degrees,
            270.0 * degrees,
        );
        self.close_path();
    }
}

// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};

use crate::{
    assets::icons,
    ui::{
        controls::segmented_control,
        theme::{TextSize, Theme, ThemeManager, ThemeTokens},
    },
};

use super::{
    append_heading,
    bindings::{bind_number, bind_switch},
    page_content, scrollable_page,
};

mod editor;
use editor::theme_editor;

pub(super) struct ThemePage {
    pub(super) widget: gtk::Widget,
    pub(super) flows: Vec<(gtk::FlowBox, u32)>,
}

pub(super) fn theme_page(manager: Rc<ThemeManager>) -> ThemePage {
    let content = page_content();
    content.add_css_class("theme-page");

    let system = super::settings_group(&content, "THEME");
    let follow = append_follow_omarchy_option(&system, &manager);
    let current = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    current.set_valign(gtk::Align::Center);
    let current_row = super::control_row("Current theme", "", &current);
    let description = current_row
        .first_child()
        .and_downcast::<gtk::Box>()
        .expect("current theme row copy")
        .last_child()
        .and_downcast::<gtk::Label>()
        .expect("current theme description");
    let weak_description = description.downgrade();
    manager.bind_preference(
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
                description.set_text(if following {
                    "Managed by Omarchy. Turn off Follow Omarchy to pick a theme manually."
                } else {
                    "Choose a theme from the library below."
                });
            }
        },
    );
    system.append(&current_row);

    let library = gtk::Box::new(gtk::Orientation::Vertical, 0);
    library.add_css_class("theme-library");
    content.append(&library);
    let catalog = append_theme_catalog(&library);
    let custom = theme_grid();
    catalog.rows.append(&custom);
    fill_theme_grids(&catalog.packaged, &custom, &manager);
    bind_catalog_filter(
        [&catalog.packaged, &custom],
        catalog.search,
        catalog.clear,
        catalog.appearance_buttons,
    );
    let editor_fields = append_custom_theme_editor(&catalog.container, &custom, &manager);
    manager.bind_preference(
        &library,
        ThemeManager::follows_omarchy,
        |widget, following| widget.set_sensitive(!following),
    );
    append_text_size_option(&content, &manager);
    let motion = super::settings_group(&content, "MOTION");
    let (row, toggle) = super::settings_option(
        "Reduce motion",
        "Disable nonessential interface animations.",
        manager.reduce_motion(),
    );
    bind_switch(
        &manager,
        &toggle,
        ThemeManager::reduce_motion,
        ThemeManager::set_reduce_motion,
    );
    motion.append(&row);

    let scroller = scrollable_page(&content, None);
    bind_switch(
        &manager,
        &follow,
        ThemeManager::follows_omarchy,
        ThemeManager::set_follow_omarchy,
    );
    ThemePage {
        widget: scroller,
        flows: vec![(catalog.packaged, 1), (custom, 1), (editor_fields, 4)],
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
    let (appearance_filter, appearance_buttons) = segmented_control(&["All", "Light", "Dark"], 0);
    appearance_filter.add_css_class("theme-appearance-filter");
    appearance_filter.set_hexpand(false);
    appearance_filter.set_homogeneous(false);
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

fn fill_theme_grids(packaged: &gtk::FlowBox, custom: &gtk::FlowBox, manager: &Rc<ThemeManager>) {
    for theme in manager.themes() {
        let flow = if theme.custom { custom } else { packaged };
        append_theme_card(flow, theme, manager);
    }
}

fn append_custom_theme_editor(
    content: &gtk::Box,
    custom: &gtk::FlowBox,
    manager: &Rc<ThemeManager>,
) -> gtk::FlowBox {
    let add = add_theme_card_button();
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    footer.add_css_class("theme-library-footer");
    let location = gtk::Label::new(Some("Custom themes live in ~/.config/strata/themes"));
    location.set_xalign(0.0);
    location.set_hexpand(true);
    location.set_wrap(true);
    footer.append(&location);
    footer.append(&add);
    content.append(&footer);
    bind_new_custom_themes(custom, manager);
    let (editor, editor_fields) = theme_editor(manager.clone());
    editor.set_reveal_child(false);
    content.append(&editor);
    let shown_editor = editor.clone();
    add.connect_clicked(move |_| shown_editor.set_reveal_child(true));
    editor_fields
}

fn append_follow_omarchy_option(content: &gtk::Box, manager: &ThemeManager) -> gtk::Switch {
    let (row, follow) = super::settings_option(
        "Follow Omarchy",
        "Use the active Omarchy Quattro theme and switch when the system theme changes.",
        manager.follows_omarchy(),
    );
    if manager.is_omarchy_available() {
        content.append(&row);
    }
    follow
}

fn append_text_size_option(content: &gtk::Box, manager: &Rc<ThemeManager>) {
    let group = super::settings_group(content, "TEXT");
    let text_size_control =
        gtk::SpinButton::with_range(f64::from(TextSize::MIN), f64::from(TextSize::MAX), 1.0);
    text_size_control.set_numeric(true);
    text_size_control.set_width_chars(3);
    text_size_control.set_alignment(0.5);
    // Keep GTK's native spin actions, with decrement before the numeric entry.
    let mut child = text_size_control.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget.has_css_class("down") {
            widget.insert_before(&text_size_control, text_size_control.first_child().as_ref());
            break;
        }
    }
    text_size_control.set_update_policy(gtk::SpinButtonUpdatePolicy::IfValid);
    text_size_control.add_css_class("form-control");
    text_size_control.add_css_class("text-size-control");
    crate::ui::accessibility::set_label(&text_size_control, "Text size in pixels");
    bind_number(
        manager,
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
    let text_size_copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text_size_copy.set_hexpand(true);
    let text_size_title = gtk::Label::new(Some("Text size"));
    text_size_title.set_xalign(0.0);
    text_size_title.add_css_class("settings-option-title");
    let text_size_description = gtk::Label::new(Some(
        "Logical pixels, 8–48. Display scaling applies on top.",
    ));
    text_size_description.set_xalign(0.0);
    text_size_description.set_wrap(true);
    text_size_description.add_css_class("settings-option-description");
    let description = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    description.add_css_class("settings-inline-description");
    description.append(&text_size_description);
    let keys = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    keys.add_css_class("settings-inline-keys");
    keys.set_hexpand(false);
    keys.set_valign(gtk::Align::Center);
    for key in ["Ctrl", "+", "/", "−", "/", "0"] {
        let label = gtk::Label::new(Some(key));
        label.add_css_class("settings-nowrap");
        label.add_css_class(if key == "/" {
            "keycap-separator"
        } else {
            "settings-keycap"
        });
        keys.append(&label);
    }
    description.append(&keys);
    text_size_copy.append(&text_size_title);
    text_size_copy.append(&description);
    text_size_row.append(&text_size_copy);
    text_size_row.append(&controls);
    group.append(&text_size_row);
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
                card.tooltip_text().as_deref().unwrap_or_default(),
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
    let add_label = gtk::Label::new(Some("Add theme"));
    add_content.append(&plus);
    add_content.append(&add_label);
    add.set_child(Some(&add_content));
    add
}

fn bind_new_custom_themes(custom: &gtk::FlowBox, manager: &Rc<ThemeManager>) {
    let known_custom = RefCell::new(
        manager
            .themes()
            .into_iter()
            .filter(|theme| theme.custom)
            .map(|theme| theme.id)
            .collect::<std::collections::HashSet<_>>(),
    );
    let weak_manager = Rc::downgrade(manager);
    manager.bind_preference(
        custom,
        |manager| {
            manager
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
    manager: &Rc<ThemeManager>,
) -> gtk::FlowBoxChild {
    let card = gtk::Button::new();
    card.add_css_class("theme-card");
    card.set_tooltip_text(Some(&theme.tokens.name));
    if theme_is_light(&theme.tokens) {
        card.add_css_class("light");
    }
    card.set_has_frame(false);
    card.set_overflow(gtk::Overflow::Visible);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    content.append(&theme_preview(&theme.tokens));
    let label = gtk::Label::new(Some(&theme.tokens.name));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content.append(&label);
    let kind = gtk::Label::new(Some(if theme_is_light(&theme.tokens) {
        "LIGHT"
    } else {
        "DARK"
    }));
    kind.add_css_class("theme-kind");
    content.append(&kind);
    let check = crate::assets::primary_icon(icons::CHECK, 14);
    let selected = !manager.follows_omarchy() && manager.selected_id() == theme.id;
    check.set_visible(selected);
    content.append(&check);
    if selected {
        card.add_css_class("selected");
    }
    card.set_child(Some(&content));
    let theme_id = theme.id;
    let selected_theme = theme_id.clone();
    let check = check.downgrade();
    manager.bind_preference(
        &card,
        move |manager| !manager.follows_omarchy() && manager.selected_id() == selected_theme,
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
    let manager = manager.clone();
    card.connect_clicked(move |_| {
        manager.select_theme(&theme_id);
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

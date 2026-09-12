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
    pub(super) text_size_actions: (gtk::Box, gtk::Button),
}

pub(super) fn theme_page(manager: Rc<ThemeManager>) -> ThemePage {
    let content = page_content();
    content.add_css_class("theme-page");

    let follow = append_follow_omarchy_option(&content, &manager);
    let text_size_actions = append_text_size_option(&content, &manager);

    let catalog = append_theme_catalog(&content);
    append_heading(&content, "YOUR THEMES");
    let custom = theme_grid();
    content.append(&custom);
    bind_catalog_filter(
        fill_theme_grids(&catalog.packaged, &custom, &manager),
        catalog.search,
        catalog.clear,
        catalog.appearance_buttons,
    );
    let editor_fields = append_custom_theme_editor(&content, &custom, &manager);

    let scroller = scrollable_page(&content, None);
    bind_switch(
        &manager,
        &follow,
        ThemeManager::follows_omarchy,
        ThemeManager::set_follow_omarchy,
    );
    ThemePage {
        widget: scroller,
        flows: vec![(catalog.packaged, 3), (custom, 3), (editor_fields, 4)],
        text_size_actions,
    }
}

struct ThemeCatalog {
    packaged: gtk::FlowBox,
    search: gtk::Entry,
    clear: gtk::Button,
    appearance_buttons: Vec<gtk::ToggleButton>,
}

fn append_theme_catalog(content: &gtk::Box) -> ThemeCatalog {
    append_heading(content, "THEMES");
    let packaged = theme_grid();
    let (search_overlay, theme_search, clear_search) = theme_search_overlay();
    content.append(&search_overlay);
    let cleared_search = theme_search.clone();
    clear_search.connect_clicked(move |_| {
        cleared_search.set_text("");
        cleared_search.grab_focus();
    });
    let (appearance_filter, appearance_buttons) = segmented_control(&["All", "Light", "Dark"], 0);
    appearance_filter.add_css_class("theme-appearance-filter");
    content.append(&appearance_filter);
    let catalog_scroll = gtk::ScrolledWindow::builder()
        .child(&packaged)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .min_content_height(240)
        .max_content_height(330)
        .propagate_natural_height(true)
        .build();
    catalog_scroll.add_css_class("theme-catalog-scroll");
    let catalog_container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    catalog_container.add_css_class("theme-catalog-container");
    catalog_container.append(&catalog_scroll);
    content.append(&catalog_container);
    ThemeCatalog {
        packaged,
        search: theme_search,
        clear: clear_search,
        appearance_buttons,
    }
}

fn fill_theme_grids(
    packaged: &gtk::FlowBox,
    custom: &gtk::FlowBox,
    manager: &Rc<ThemeManager>,
) -> Vec<(gtk::FlowBoxChild, String, bool)> {
    let mut catalog_cards = Vec::new();
    for theme in manager.themes() {
        let custom_theme = theme.custom;
        let name = theme.tokens.name.clone();
        let light = theme_is_light(&theme.tokens);
        let flow = if custom_theme { custom } else { packaged };
        let child = append_theme_card(flow, theme, manager);
        if !custom_theme {
            catalog_cards.push((child, name, light));
        }
    }
    catalog_cards
}

fn append_custom_theme_editor(
    content: &gtk::Box,
    custom: &gtk::FlowBox,
    manager: &Rc<ThemeManager>,
) -> gtk::FlowBox {
    let add = add_theme_card_button();
    custom.insert(&add, -1);
    bind_new_custom_themes(custom, manager);
    let (editor, editor_fields) = theme_editor(manager.clone());
    editor.set_reveal_child(false);
    content.append(&editor);
    let shown_editor = editor.clone();
    add.connect_clicked(move |_| shown_editor.set_reveal_child(true));
    editor_fields
}

fn append_follow_omarchy_option(content: &gtk::Box, manager: &ThemeManager) -> gtk::Switch {
    let follow = gtk::Switch::builder()
        .active(manager.follows_omarchy())
        .valign(gtk::Align::Center)
        .build();
    if !manager.is_omarchy_available() {
        return follow;
    }
    append_heading(content, "SYSTEM");
    let system = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    system.add_css_class("settings-option");
    let icon = crate::assets::primary_icon(icons::MONITOR, 22);
    icon.add_css_class("system-theme-icon");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let system_title = gtk::Label::new(Some("Follow Omarchy"));
    system_title.set_xalign(0.0);
    system_title.add_css_class("settings-option-title");
    let system_description = gtk::Label::new(Some(
        "Use the active Omarchy Quattro theme and follow system theme changes.",
    ));
    system_description.set_xalign(0.0);
    system_description.set_wrap(true);
    system_description.add_css_class("settings-option-description");
    copy.append(&system_title);
    copy.append(&system_description);
    system.append(&icon);
    system.append(&copy);
    system.append(&follow);
    content.append(&system);
    follow
}

fn append_text_size_option(
    content: &gtk::Box,
    manager: &Rc<ThemeManager>,
) -> (gtk::Box, gtk::Button) {
    append_heading(content, "TYPOGRAPHY");
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
    let reset = gtk::Button::with_label("Reset");
    reset.add_css_class("action-dialog-cancel");
    reset.set_halign(gtk::Align::Start);
    reset.set_valign(gtk::Align::Center);
    reset.set_tooltip_text(Some("Reset text size to 13 px"));
    let reset_manager = manager.clone();
    reset.connect_clicked(move |_| reset_manager.set_text_size(TextSize::default()));
    let control_height = gtk::SizeGroup::new(gtk::SizeGroupMode::Vertical);
    control_height.add_widget(&text_size_control);
    control_height.add_widget(&reset);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.set_halign(gtk::Align::Start);
    controls.append(&text_size_control);
    controls.append(&reset);
    let text_size_row = gtk::Box::new(gtk::Orientation::Vertical, 8);
    text_size_row.add_css_class("settings-option");
    let text_size_copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text_size_copy.set_hexpand(true);
    let text_size_title = gtk::Label::new(Some("Text size"));
    text_size_title.set_xalign(0.0);
    text_size_title.add_css_class("settings-option-title");
    let text_size_description = gtk::Label::new(Some(
        "Choose 8–48 logical pixels for interface text. Your display scaling applies on top. Use Ctrl++ / Ctrl+− to adjust and Ctrl+0 to reset.",
    ));
    text_size_description.set_xalign(0.0);
    text_size_description.set_wrap(true);
    text_size_description.add_css_class("settings-option-description");
    text_size_copy.append(&text_size_title);
    text_size_copy.append(&text_size_description);
    text_size_row.append(&text_size_copy);
    text_size_row.append(&controls);
    content.append(&text_size_row);
    (controls, reset)
}

fn theme_grid() -> gtk::FlowBox {
    let grid = gtk::FlowBox::builder()
        .column_spacing(12)
        .row_spacing(12)
        .max_children_per_line(3)
        .min_children_per_line(1)
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .build();
    grid.add_css_class("theme-grid");
    grid
}

fn selects_all_in_theme_search(key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    modifiers.contains(gdk::ModifierType::CONTROL_MASK) && matches!(key, gdk::Key::a | gdk::Key::A)
}

fn theme_search_overlay() -> (gtk::Overlay, gtk::Entry, gtk::Button) {
    let theme_search = gtk::Entry::new();
    theme_search.add_css_class("form-control");
    theme_search.add_css_class("theme-search");
    theme_search.set_placeholder_text(Some("Search themes"));
    let search_keys = gtk::EventControllerKey::new();
    search_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let selected_search = theme_search.downgrade();
    search_keys.connect_key_pressed(move |_, key, _, modifiers| {
        if !selects_all_in_theme_search(key, modifiers) {
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
        .tooltip_text("Clear theme search")
        .halign(gtk::Align::End)
        .valign(gtk::Align::Center)
        .margin_end(6)
        .visible(false)
        .build();
    clear_search.add_css_class("theme-search-clear");
    clear_search.set_has_frame(false);
    let search_overlay = gtk::Overlay::new();
    search_overlay.set_child(Some(&theme_search));
    search_overlay.add_overlay(&clear_search);
    (search_overlay, theme_search, clear_search)
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
    catalog_cards: Vec<(gtk::FlowBoxChild, String, bool)>,
    theme_search: gtk::Entry,
    clear_search: gtk::Button,
    appearance_buttons: Vec<gtk::ToggleButton>,
) {
    let catalog_cards = Rc::new(catalog_cards);
    let appearance = Rc::new(Cell::new(ThemeAppearance::All));
    let filtered_cards = catalog_cards.clone();
    let filtered_appearance = appearance.clone();
    let filter_search = theme_search.clone();
    let apply_catalog_filter: Rc<dyn Fn()> = Rc::new(move || {
        let query = filter_search.text();
        let appearance = filtered_appearance.get();
        for (child, name, light) in filtered_cards.iter() {
            child.set_visible(catalog_card_visible(appearance, *light, name, &query));
        }
    });
    let search_filter = apply_catalog_filter.clone();
    theme_search.connect_changed(move |search| {
        clear_search.set_visible(!search.text().is_empty());
        search_filter();
    });
    for (button, value) in appearance_buttons.into_iter().zip([
        ThemeAppearance::All,
        ThemeAppearance::Light,
        ThemeAppearance::Dark,
    ]) {
        let appearance = appearance.clone();
        let apply_filter = apply_catalog_filter.clone();
        button.connect_toggled(move |button| {
            if button.is_active() {
                appearance.set(value);
                apply_filter();
            }
        });
    }
}

fn add_theme_card_button() -> gtk::Button {
    let add = gtk::Button::new();
    add.add_css_class("add-theme-card");
    add.set_has_frame(false);
    let add_content = gtk::Box::new(gtk::Orientation::Vertical, 7);
    add_content.set_halign(gtk::Align::Center);
    add_content.set_valign(gtk::Align::Center);
    let plus = crate::assets::primary_icon(icons::PLUS, 22);
    let add_label = gtk::Label::new(Some("Add a theme"));
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
    card.set_has_frame(false);
    card.set_overflow(gtk::Overflow::Visible);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let preview = gtk::Overlay::new();
    preview.set_child(Some(&theme_preview(&theme.tokens)));
    let check = gtk::Image::from_icon_name(icons::CHECK_ON_PRIMARY);
    check.add_css_class("theme-card-check");
    check.set_halign(gtk::Align::End);
    check.set_valign(gtk::Align::Start);
    check.set_margin_top(8);
    check.set_margin_end(8);
    check.set_pixel_size(10);
    preview.add_overlay(&check);
    content.append(&preview);
    let label_row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    let selected = !manager.follows_omarchy() && manager.selected_id() == theme.id;
    check.set_visible(selected);
    if selected {
        card.add_css_class("selected");
    }
    let label = gtk::Label::new(Some(&theme.tokens.name));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label_row.append(&label);
    content.append(&label_row);
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
    area.set_content_width(190);
    area.set_content_height(72);
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
        paint(context, &tokens.surface);
        context.rectangle(0.0, 0.0, f64::from(width) * 0.40, f64::from(height));
        let _ = context.fill();
        for (x, y, w, value) in [
            (10.0, 23.0, 45.0, &tokens.dim_text),
            (10.0, 36.0, 59.0, &tokens.accent),
            (10.0, 51.0, 39.0, &tokens.dim_text),
            (f64::from(width) * 0.45, 23.0, 47.0, &tokens.accent),
            (f64::from(width) * 0.45, 37.0, 83.0, &tokens.dim_text),
            (f64::from(width) * 0.45, 51.0, 66.0, &tokens.dim_text),
        ] {
            paint(context, value);
            context.rounded_rectangle(RoundedRect {
                x,
                y,
                width: w,
                height: 5.0,
                radius: 2.5,
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

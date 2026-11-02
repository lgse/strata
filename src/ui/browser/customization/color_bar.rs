// SPDX-License-Identifier: MIT

use super::custom_color::show_custom_color_modal;
use crate::model::{FolderColor, FolderColorValue};
use gtk::gdk;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// What the color previews depict: the item's icon and whether it is a folder.
#[derive(Clone, Copy)]
pub(super) struct ColorBarItem {
    pub(super) preview_icon: &'static str,
    pub(super) label: &'static str,
}

impl ColorBarItem {
    pub(super) fn is_folder(self) -> bool {
        self.label == "folder"
    }
}

pub(super) struct FolderColorBar {
    pub(super) container: gtk::Box,
    pub(super) selection: Rc<ColorSelection>,
}

/// The current color plus the dots that reflect it.
pub(super) struct ColorSelection {
    active_state: RefCell<Option<FolderColorValue>>,
    dots: ColorDots,
    on_select: Box<dyn Fn(Option<FolderColorValue>)>,
}

impl ColorSelection {
    pub(super) fn update_active(&self, new_color: Option<FolderColorValue>) {
        self.active_state.replace(new_color.clone());
        self.dots.show_active(&new_color);
    }

    fn select(&self, next_color: Option<FolderColorValue>) {
        self.update_active(next_color.clone());
        (self.on_select)(next_color);
    }

    fn active_hex(&self) -> Option<String> {
        self.active_state
            .borrow()
            .as_ref()
            .map(|value| value.hex().to_owned())
    }

    fn is_active_preset(&self, color: FolderColor) -> bool {
        matches!(
            self.active_state.borrow().as_ref(),
            Some(FolderColorValue::Preset(p)) if *p == color
        )
    }
}

struct ColorDots {
    theme_btn: gtk::Button,
    color_dots: Vec<(FolderColor, gtk::Button, gtk::Image)>,
    custom: CustomDot,
}

impl ColorDots {
    fn show_active(&self, new_color: &Option<FolderColorValue>) {
        match new_color {
            None | Some(FolderColorValue::Preset(_)) => {
                if new_color.is_none() {
                    self.theme_btn.add_css_class("active");
                } else {
                    self.theme_btn.remove_css_class("active");
                }
                self.custom.show_unset();
            }
            Some(FolderColorValue::Custom(hex)) => {
                self.theme_btn.remove_css_class("active");
                self.custom.show_hex(hex);
            }
        }
        self.show_preset_dots(new_color);
    }

    fn show_preset_dots(&self, new_color: &Option<FolderColorValue>) {
        for (color, dot, check) in &self.color_dots {
            let is_match = matches!(new_color, Some(FolderColorValue::Preset(p)) if p == color);
            if is_match {
                dot.add_css_class("active");
            } else {
                dot.remove_css_class("active");
            }
            check.set_visible(is_match);
        }
    }
}

struct CustomDot {
    button: gtk::Button,
    stack: gtk::Stack,
    dot: gtk::DrawingArea,
    hex_for_draw: Rc<RefCell<Option<String>>>,
}

impl CustomDot {
    fn show_unset(&self) {
        self.button.remove_css_class("active");
        self.stack.set_visible_child_name("plus");
        let label = crate::i18n::tr("Custom color…");
        self.button.set_tooltip_text(Some(&label));
        crate::ui::accessibility::set_label(&self.button, &label);
        self.hex_for_draw.replace(None);
    }

    fn show_hex(&self, hex: &str) {
        self.button.add_css_class("active");
        self.stack.set_visible_child_name("dot");
        let label = rust_i18n::t!("Custom (%{hex})", hex = hex);
        self.button.set_tooltip_text(Some(&label));
        crate::ui::accessibility::set_label(&self.button, &label);
        self.hex_for_draw.replace(Some(hex.to_owned()));
        self.dot.queue_draw();
    }
}

pub(super) fn build_folder_color_bar(
    initial_color: Option<FolderColorValue>,
    item: ColorBarItem,
    on_color_selected: impl Fn(Option<FolderColorValue>) + 'static,
) -> FolderColorBar {
    let container = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    container.add_css_class("folder-color-bar");

    let theme_btn = build_theme_button();
    container.append(&theme_btn);
    let color_dots = build_color_dots(&container);
    let custom = build_custom_dot();
    container.append(&custom.button);

    let selection = Rc::new(ColorSelection {
        active_state: RefCell::new(initial_color.clone()),
        dots: ColorDots {
            theme_btn,
            color_dots,
            custom,
        },
        on_select: Box::new(on_color_selected),
    });

    selection.update_active(initial_color);
    connect_color_clicks(&selection, item);

    FolderColorBar {
        container,
        selection,
    }
}

fn connect_color_clicks(selection: &Rc<ColorSelection>, item: ColorBarItem) {
    let dots = &selection.dots;
    let theme_selection = selection.clone();
    dots.theme_btn
        .connect_clicked(move |_| theme_selection.select(None));

    for (color, dot, _) in &dots.color_dots {
        let color = *color;
        let selection = selection.clone();
        dot.connect_clicked(move |_| {
            let next_color = if selection.is_active_preset(color) {
                None
            } else {
                Some(FolderColorValue::Preset(color))
            };
            selection.select(next_color);
        });
    }

    let selection = selection.clone();
    dots.custom.button.connect_clicked(move |btn| {
        let initial_hex = selection.active_hex();
        let selection = selection.clone();
        show_custom_color_modal(btn, initial_hex.as_deref(), item, move |value| {
            selection.select(Some(value));
        });
    });
}

fn build_theme_button() -> gtk::Button {
    let theme_btn = gtk::Button::new();
    theme_btn.set_has_frame(false);
    theme_btn.add_css_class("folder-color-dot");
    theme_btn.add_css_class("folder-color-theme");
    crate::ui::accessibility::set_tooltip_and_label(
        &theme_btn,
        &crate::i18n::tr("Default (Theme color)"),
    );
    let theme_icon = crate::assets::primary_icon(crate::assets::icons::PALETTE, 12);
    theme_btn.set_child(Some(&theme_icon));
    theme_btn
}

fn build_color_dots(container: &gtk::Box) -> Vec<(FolderColor, gtk::Button, gtk::Image)> {
    let mut color_dots = Vec::new();
    for &color in &FolderColor::ALL {
        let dot = gtk::Button::new();
        dot.set_has_frame(false);
        dot.add_css_class("folder-color-dot");
        dot.add_css_class(color.css_class());
        dot.set_tooltip_text(Some(&crate::i18n::tr(color.name())));
        crate::ui::accessibility::set_own_label(&dot, &crate::i18n::tr(color.name()));
        let check = gtk::Image::from_icon_name(crate::assets::icons::CHECK_ON_PRIMARY);
        check.set_pixel_size(10);
        check.set_visible(false);
        dot.set_child(Some(&check));
        container.append(&dot);
        color_dots.push((color, dot, check));
    }
    color_dots
}

fn build_custom_dot() -> CustomDot {
    let button = gtk::Button::new();
    button.set_has_frame(false);
    button.add_css_class("folder-color-dot");
    button.add_css_class("folder-color-custom");
    button.set_tooltip_text(Some(&crate::i18n::tr("Custom color…")));

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::None);
    let custom_plus = crate::assets::primary_icon(crate::assets::icons::PLUS, 10);
    stack.add_named(&custom_plus, Some("plus"));

    let hex_for_draw = Rc::new(RefCell::new(None::<String>));
    let dot = gtk::DrawingArea::new();
    dot.set_content_width(20);
    dot.set_content_height(20);
    let hex_draw = hex_for_draw.clone();
    dot.set_draw_func(move |_, cr, width, height| {
        let Some(hex) = hex_draw.borrow().clone() else {
            return;
        };
        let Ok(rgba) = gdk::RGBA::parse(&hex) else {
            return;
        };
        draw_checked_dot(cr, &rgba, f64::from(width), f64::from(height));
    });
    stack.add_named(&dot, Some("dot"));
    stack.set_visible_child_name("plus");
    button.set_child(Some(&stack));

    CustomDot {
        button,
        stack,
        dot,
        hex_for_draw,
    }
}

fn draw_checked_dot(cr: &gtk::cairo::Context, rgba: &gdk::RGBA, w: f64, h: f64) {
    let r = (w.min(h) / 2.0) - 1.0;
    let cx = w / 2.0;
    let cy = h / 2.0;

    cr.set_source_rgba(
        f64::from(rgba.red()),
        f64::from(rgba.green()),
        f64::from(rgba.blue()),
        1.0,
    );
    cr.arc(cx, cy, r, 0.0, 2.0 * std::f64::consts::PI);
    let _ = cr.fill();

    cr.set_source_rgba(0.0, 0.0, 0.0, 0.25);
    cr.set_line_width(1.0);
    cr.arc(cx, cy, r, 0.0, 2.0 * std::f64::consts::PI);
    let _ = cr.stroke();

    cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
    cr.set_line_width(1.8);
    cr.move_to(cx - 3.5, cy);
    cr.line_to(cx - 1.0, cy + 2.8);
    cr.line_to(cx + 3.8, cy - 2.8);
    let _ = cr.stroke();
}

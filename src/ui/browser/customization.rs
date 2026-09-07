// SPDX-License-Identifier: GPL-3.0-or-later

use crate::model::{FolderColor, FolderColorValue};
use crate::ui::controls::modal_layout;
use crate::ui::modal::{ModalHost, dismiss_modal_layer, modal_layer};
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

fn rgba_to_hex(rgba: &gdk::RGBA) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (rgba.red() * 255.0).round() as u8,
        (rgba.green() * 255.0).round() as u8,
        (rgba.blue() * 255.0).round() as u8,
    )
}

#[expect(
    deprecated,
    reason = "ColorChooserWidget is embedded directly inside in-app modal instead of external window"
)]
fn show_custom_color_modal(
    parent: &impl IsA<gtk::Widget>,
    initial_color: Option<&str>,
    preview_icon: &'static str,
    item_label: &'static str,
    on_confirm: impl Fn(FolderColorValue) + 'static,
) {
    let Some(ModalHost {
        overlay: window_overlay,
        blurred_root,
    }) = ModalHost::blurred_for(parent)
    else {
        return;
    };
    if let Some(popover) = parent
        .ancestor(gtk::Popover::static_type())
        .and_downcast::<gtk::Popover>()
    {
        popover.popdown();
    }

    let item_title = if item_label == "folder" {
        "Folder"
    } else {
        "File"
    };
    let title = format!("Custom {item_title} Color");
    let subtitle = format!("Choose a color for this {item_label}");
    let layout = modal_layout(preview_icon, &title, &subtitle, "Apply");
    layout.close.set_visible(false);

    let modal_icon = layout.icon.clone();
    let initial_val = initial_color
        .and_then(FolderColorValue::parse)
        .unwrap_or_else(|| FolderColorValue::Custom("#34d399".to_owned()));
    let initial_hex = initial_val.hex().to_owned();

    crate::assets::set_custom_colored_icon(&modal_icon, preview_icon, &initial_hex);

    let chooser = gtk::ColorChooserWidget::new();
    chooser.set_use_alpha(false);
    if let Ok(rgba) = gdk::RGBA::parse(&initial_hex) {
        chooser.set_rgba(&rgba);
    }

    let icon_for_notify = modal_icon.clone();
    chooser.connect_rgba_notify(move |c| {
        let hex = rgba_to_hex(&c.rgba());
        crate::assets::set_custom_colored_icon(&icon_for_notify, preview_icon, &hex);
    });

    layout.body.append(&chooser);

    let content = layout.content;
    let cancel = layout.cancel;
    let confirm = layout.confirm;

    let back = gtk::Button::new();
    back.add_css_class("action-dialog-cancel");
    let back_icon = crate::assets::primary_icon(crate::assets::icons::ARROW_LEFT, 14);
    back.set_child(Some(&back_icon));
    back.set_tooltip_text(Some("Back to palette"));
    back.set_visible(false);
    layout.actions.prepend(&back);

    let chooser_for_back = chooser.clone();
    back.connect_clicked(move |_| {
        chooser_for_back.set_property("show-editor", false);
    });

    let back_btn = back.clone();
    let subtitle_label = layout.subtitle.clone();
    chooser.connect_notify_local(Some("show-editor"), move |c, _| {
        let in_editor = c.property::<bool>("show-editor");
        back_btn.set_visible(in_editor);
        if in_editor {
            subtitle_label.set_text(&format!("Customize {item_label} color"));
        } else {
            subtitle_label.set_text(&format!("Choose a color for this {item_label}"));
        }
    });

    let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
    window_overlay.add_overlay(&layer);

    let on_confirm = Rc::new(on_confirm);
    let confirm_layer = layer.clone();
    let confirm_overlay = window_overlay.clone();
    let confirm_root = blurred_root.clone();
    let chooser_for_confirm = chooser.clone();
    let on_confirm_click = on_confirm.clone();
    confirm.connect_clicked(move |_| {
        let hex = rgba_to_hex(&chooser_for_confirm.rgba());
        dismiss_modal_layer(&confirm_layer, &confirm_overlay, confirm_root.as_ref());
        on_confirm_click(FolderColorValue::Custom(hex));
    });

    let cancel_layer = layer.clone();
    let cancel_overlay = window_overlay.clone();
    let cancel_root = blurred_root.clone();
    cancel.connect_clicked(move |_| {
        dismiss_modal_layer(&cancel_layer, &cancel_overlay, cancel_root.as_ref());
    });

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let escape_layer = layer.clone();
    let escape_overlay = window_overlay;
    let escape_root = blurred_root;
    let chooser_for_escape = chooser.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            if chooser_for_escape.property::<bool>("show-editor") {
                chooser_for_escape.set_property("show-editor", false);
                glib::Propagation::Stop
            } else {
                dismiss_modal_layer(&escape_layer, &escape_overlay, escape_root.as_ref());
                glib::Propagation::Stop
            }
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(keys);

    chooser.grab_focus();
}

pub(super) fn show_customize_modal(
    parent: &impl IsA<gtk::Widget>,
    path: PathBuf,
    is_directory: bool,
    fallback_icon: &'static str,
) {
    let Some(ModalHost {
        overlay: window_overlay,
        blurred_root,
    }) = ModalHost::blurred_for(parent)
    else {
        return;
    };
    if let Some(popover) = parent
        .ancestor(gtk::Popover::static_type())
        .and_downcast::<gtk::Popover>()
    {
        popover.popdown();
    }

    let item_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let item_kind = if is_directory {
        "Customize Folder"
    } else {
        "Customize File"
    };
    let layout = modal_layout(crate::assets::icons::PALETTE, item_kind, &item_name, "Done");
    layout.content.add_css_class("customize-dialog");
    layout
        .subtitle
        .set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    layout.subtitle.set_max_width_chars(36);
    layout.cancel.set_visible(false);

    let theme_manager = crate::ui::theme::ThemeManager::shared();
    let initial_color = theme_manager.folder_color(&path);
    let initial_icon = theme_manager.custom_icon(&path);

    let preview = gtk::Box::new(gtk::Orientation::Vertical, 7);
    preview.add_css_class("customize-preview");
    preview.set_halign(gtk::Align::Center);
    let preview_icon = gtk::Image::new();
    preview_icon.add_css_class("customize-preview-icon");
    crate::ui::thumbnail::show_customized_icon(&preview_icon, &path, fallback_icon, 56);
    let preview_name = gtk::Label::new(Some(&item_name));
    preview_name.add_css_class("customize-preview-name");
    preview_name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    preview_name.set_max_width_chars(30);
    preview.append(&preview_icon);
    preview.append(&preview_name);
    layout.body.append(&preview);

    let clear = gtk::Button::with_label("Clear");
    clear.add_css_class("action-dialog-cancel");
    clear.set_sensitive(initial_color.is_some() || initial_icon.is_some());
    layout
        .actions
        .insert_child_after(&clear, Some(&layout.cancel));

    let color_section = gtk::Box::new(gtk::Orientation::Vertical, 8);
    color_section.add_css_class("customize-section");
    let color_label = gtk::Label::new(Some("COLOR"));
    color_label.add_css_class("customize-section-label");
    color_label.set_xalign(0.0);
    color_section.append(&color_label);

    let color_path = path.clone();
    let color_preview = preview_icon.clone();
    let clear_for_color = clear.clone();
    let item_label = if is_directory { "folder" } else { "file" };
    let color_bar = build_folder_color_bar(
        initial_color,
        fallback_icon,
        item_label,
        move |selected_color| {
            crate::ui::theme::ThemeManager::shared().set_folder_color(&color_path, selected_color);
            crate::ui::thumbnail::show_customized_icon(
                &color_preview,
                &color_path,
                fallback_icon,
                56,
            );
            clear_for_color.set_sensitive(true);
        },
    );
    color_section.append(&color_bar.container);
    layout.body.append(&color_section);

    let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
    section.add_css_class("customize-section");
    section.add_css_class("separated");
    let label = gtk::Label::new(Some("ICON"));
    label.add_css_class("customize-section-label");
    label.set_xalign(0.0);
    section.append(&label);

    let icon_grid = gtk::FlowBox::new();
    icon_grid.add_css_class("customize-icon-grid");
    icon_grid.set_selection_mode(gtk::SelectionMode::None);
    icon_grid.set_homogeneous(true);
    icon_grid.set_min_children_per_line(4);
    icon_grid.set_max_children_per_line(8);
    icon_grid.set_row_spacing(6);
    icon_grid.set_column_spacing(6);

    let icon_buttons: Rc<Vec<_>> = Rc::new(
        crate::assets::icons::CUSTOMIZATION_CHOICES
            .iter()
            .map(|&(icon_name, label)| {
                let button = gtk::Button::new();
                button.add_css_class("customize-icon-button");
                button.set_tooltip_text(Some(label));
                button.update_property(&[gtk::accessible::Property::Label(label)]);
                button.set_child(Some(&crate::assets::primary_icon(icon_name, 20)));
                if initial_icon.as_deref() == Some(icon_name) {
                    button.add_css_class("active");
                }
                icon_grid.append(&button);
                (icon_name, button)
            })
            .collect(),
    );

    let selected_emoji = initial_icon
        .as_deref()
        .and_then(crate::assets::icons::custom_emoji);
    let emoji_button = gtk::Button::with_label(&selected_emoji.map_or_else(
        || "Choose Emoji…".to_owned(),
        |emoji| format!("Emoji  {emoji}"),
    ));
    emoji_button.add_css_class("customize-emoji-button");
    emoji_button.update_property(&[gtk::accessible::Property::Description(
        "Choose any emoji for this item",
    )]);
    let emoji_chooser = gtk::EmojiChooser::new();
    emoji_chooser.add_css_class("customize-emoji-chooser");
    emoji_chooser.set_parent(&emoji_button);
    let chooser_for_button = emoji_chooser.clone();
    emoji_button.connect_clicked(move |_| chooser_for_button.popup());

    for (icon_name, button) in icon_buttons.iter() {
        let selected_name = *icon_name;
        let buttons = icon_buttons.clone();
        let icon_path = path.clone();
        let preview = preview_icon.clone();
        let clear_for_icon = clear.clone();
        let emoji_for_icon = emoji_button.clone();
        button.connect_clicked(move |_| {
            for (name, button) in buttons.iter() {
                if *name == selected_name {
                    button.add_css_class("active");
                } else {
                    button.remove_css_class("active");
                }
            }
            emoji_for_icon.set_label("Choose Emoji…");
            crate::ui::theme::ThemeManager::shared()
                .set_custom_icon(&icon_path, Some(selected_name));
            crate::ui::thumbnail::show_customized_icon(&preview, &icon_path, fallback_icon, 56);
            clear_for_icon.set_sensitive(true);
        });
    }

    let buttons_for_emoji = icon_buttons.clone();
    let emoji_path = path.clone();
    let emoji_preview = preview_icon.clone();
    let clear_for_emoji = clear.clone();
    let emoji_label = emoji_button.clone();
    emoji_chooser.connect_emoji_picked(move |chooser, emoji| {
        let preference = format!("emoji:{emoji}");
        for (_, button) in buttons_for_emoji.iter() {
            button.remove_css_class("active");
        }
        emoji_label.set_label(&format!("Emoji  {emoji}"));
        crate::ui::theme::ThemeManager::shared().set_custom_icon(&emoji_path, Some(&preference));
        crate::ui::thumbnail::show_customized_icon(&emoji_preview, &emoji_path, fallback_icon, 56);
        clear_for_emoji.set_sensitive(true);
        chooser.popdown();
    });

    section.append(&icon_grid);
    section.append(&emoji_button);
    layout.body.append(&section);

    let reset_color_ui = color_bar.update_active;
    let clear_path = path.clone();
    let clear_preview = preview_icon;
    let buttons_for_clear = icon_buttons;
    let emoji_for_clear = emoji_button;
    clear.connect_clicked(move |button| {
        crate::ui::theme::ThemeManager::shared().clear_item_customization(&clear_path);
        reset_color_ui(None);
        for (_, icon_button) in buttons_for_clear.iter() {
            icon_button.remove_css_class("active");
        }
        emoji_for_clear.set_label("Choose Emoji…");
        crate::ui::thumbnail::show_customized_icon(&clear_preview, &clear_path, fallback_icon, 56);
        button.set_sensitive(false);
    });

    let content = layout.content;
    let confirm = layout.confirm;
    let layer = modal_layer(&content, &window_overlay, blurred_root.clone(), None);
    window_overlay.add_overlay(&layer);

    let dismiss = {
        let layer = layer.clone();
        let overlay = window_overlay.clone();
        let root = blurred_root.clone();
        move || dismiss_modal_layer(&layer, &overlay, root.as_ref())
    };
    let done_dismiss = dismiss.clone();
    confirm.connect_clicked(move |_| done_dismiss());
    layout.close.connect_clicked(move |_| dismiss());

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let escape_layer = layer.clone();
    let escape_overlay = window_overlay;
    let escape_root = blurred_root;
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            dismiss_modal_layer(&escape_layer, &escape_overlay, escape_root.as_ref());
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(keys);
}

struct FolderColorBar {
    container: gtk::Box,
    update_active: Rc<dyn Fn(Option<FolderColorValue>)>,
}

fn build_folder_color_bar(
    initial_color: Option<FolderColorValue>,
    preview_icon: &'static str,
    item_label: &'static str,
    on_color_selected: impl Fn(Option<FolderColorValue>) + 'static,
) -> FolderColorBar {
    let container = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    container.add_css_class("folder-color-bar");

    let active_state = Rc::new(RefCell::new(initial_color.clone()));
    let on_select = Rc::new(on_color_selected);

    let theme_btn = gtk::Button::new();
    theme_btn.set_has_frame(false);
    theme_btn.add_css_class("folder-color-dot");
    theme_btn.add_css_class("folder-color-theme");
    theme_btn.set_tooltip_text(Some("Default (Theme color)"));
    let theme_icon = crate::assets::primary_icon(crate::assets::icons::PALETTE, 12);
    theme_btn.set_child(Some(&theme_icon));
    container.append(&theme_btn);

    let mut color_dots = Vec::new();
    for &color in &FolderColor::ALL {
        let dot = gtk::Button::new();
        dot.set_has_frame(false);
        dot.add_css_class("folder-color-dot");
        dot.add_css_class(color.css_class());
        dot.set_tooltip_text(Some(color.name()));
        let check = gtk::Image::from_icon_name(crate::assets::icons::CHECK_ON_PRIMARY);
        check.set_pixel_size(10);
        check.set_visible(false);
        dot.set_child(Some(&check));
        container.append(&dot);
        color_dots.push((color, dot, check));
    }

    let custom_btn = gtk::Button::new();
    custom_btn.set_has_frame(false);
    custom_btn.add_css_class("folder-color-dot");
    custom_btn.add_css_class("folder-color-custom");
    custom_btn.set_tooltip_text(Some("Custom color…"));

    let custom_stack = gtk::Stack::new();
    custom_stack.set_transition_type(gtk::StackTransitionType::None);
    let custom_plus = crate::assets::primary_icon(crate::assets::icons::PLUS, 10);
    custom_stack.add_named(&custom_plus, Some("plus"));

    let hex_for_draw = Rc::new(RefCell::new(None::<String>));
    let custom_dot = gtk::DrawingArea::new();
    custom_dot.set_content_width(20);
    custom_dot.set_content_height(20);
    let hex_draw = hex_for_draw.clone();
    custom_dot.set_draw_func(move |_, cr, width, height| {
        let Some(hex) = hex_draw.borrow().clone() else {
            return;
        };
        let Ok(rgba) = gdk::RGBA::parse(&hex) else {
            return;
        };
        let w = f64::from(width);
        let h = f64::from(height);
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
    });
    custom_stack.add_named(&custom_dot, Some("dot"));
    custom_stack.set_visible_child_name("plus");
    custom_btn.set_child(Some(&custom_stack));
    container.append(&custom_btn);

    let update_active: Rc<dyn Fn(Option<FolderColorValue>)> = {
        let active_state = active_state.clone();
        let theme_btn = theme_btn.clone();
        let color_dots = color_dots.clone();
        let custom_btn = custom_btn.clone();
        let custom_stack = custom_stack.clone();
        let custom_dot = custom_dot.clone();
        let hex_for_draw = hex_for_draw.clone();
        Rc::new(move |new_color| {
            active_state.replace(new_color.clone());
            match &new_color {
                None | Some(FolderColorValue::Preset(_)) => {
                    if new_color.is_none() {
                        theme_btn.add_css_class("active");
                    } else {
                        theme_btn.remove_css_class("active");
                    }
                    custom_btn.remove_css_class("active");
                    custom_stack.set_visible_child_name("plus");
                    custom_btn.set_tooltip_text(Some("Custom color…"));
                    hex_for_draw.replace(None);
                }
                Some(FolderColorValue::Custom(hex)) => {
                    theme_btn.remove_css_class("active");
                    custom_btn.add_css_class("active");
                    custom_stack.set_visible_child_name("dot");
                    custom_btn.set_tooltip_text(Some(&format!("Custom ({hex})")));
                    hex_for_draw.replace(Some(hex.clone()));
                    custom_dot.queue_draw();
                }
            }
            for (color, dot, check) in &color_dots {
                let is_match =
                    matches!(&new_color, Some(FolderColorValue::Preset(p)) if p == color);
                if is_match {
                    dot.add_css_class("active");
                } else {
                    dot.remove_css_class("active");
                }
                check.set_visible(is_match);
            }
        })
    };

    update_active(initial_color);

    {
        let active_state = active_state.clone();
        let update_ui = update_active.clone();
        let on_select = on_select.clone();
        theme_btn.connect_clicked(move |_| {
            active_state.replace(None);
            update_ui(None);
            on_select(None);
        });
    }

    for (color, dot, _) in color_dots {
        let active_state = active_state.clone();
        let update_ui = update_active.clone();
        let on_select = on_select.clone();
        dot.connect_clicked(move |_| {
            let next_color = if matches!(
                active_state.borrow().as_ref(),
                Some(FolderColorValue::Preset(p)) if *p == color
            ) {
                None
            } else {
                Some(FolderColorValue::Preset(color))
            };
            active_state.replace(next_color.clone());
            update_ui(next_color.clone());
            on_select(next_color);
        });
    }

    {
        let active_state = active_state.clone();
        let update_ui = update_active.clone();
        let on_select = on_select.clone();
        custom_btn.connect_clicked(move |btn| {
            let initial_hex = active_state.borrow().as_ref().map(|v| v.hex().to_owned());
            let active_state = active_state.clone();
            let update_ui = update_ui.clone();
            let on_select = on_select.clone();
            show_custom_color_modal(
                btn,
                initial_hex.as_deref(),
                preview_icon,
                item_label,
                move |value| {
                    let val = Some(value);
                    active_state.replace(val.clone());
                    update_ui(val.clone());
                    on_select(val);
                },
            );
        });
    }

    FolderColorBar {
        container,
        update_active,
    }
}

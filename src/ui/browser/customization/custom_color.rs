// SPDX-License-Identifier: MIT

use super::color_bar::ColorBarItem;
use super::popdown_ancestor_popover;
use crate::model::FolderColorValue;
use crate::ui::controls::{ModalLayout, modal_layout};
use crate::ui::keys::{self, bindings::ESCAPE_ANY};
use crate::ui::modal::{
    ModalHost, dismiss_modal_layer_then, modal_layer_with_backdrop, remember_modal_focus,
};
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::rc::Rc;

fn rgba_to_hex(rgba: &gdk::RGBA) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (rgba.red() * 255.0).round() as u8,
        (rgba.green() * 255.0).round() as u8,
        (rgba.blue() * 255.0).round() as u8,
    )
}

struct CustomColorCopy {
    title: &'static str,
    choose_subtitle: &'static str,
    editor_subtitle: &'static str,
}

fn custom_color_copy(folder: bool) -> CustomColorCopy {
    if folder {
        CustomColorCopy {
            title: "Custom Folder Color",
            choose_subtitle: "Choose a color for this folder",
            editor_subtitle: "Customize folder color",
        }
    } else {
        CustomColorCopy {
            title: "Custom File Color",
            choose_subtitle: "Choose a color for this file",
            editor_subtitle: "Customize file color",
        }
    }
}

#[expect(
    deprecated,
    reason = "ColorChooserWidget is embedded directly inside in-app modal instead of external window"
)]
pub(super) fn show_custom_color_modal(
    parent: &impl IsA<gtk::Widget>,
    initial_color: Option<&str>,
    item: ColorBarItem,
    on_confirm: impl Fn(FolderColorValue) + 'static,
) {
    let Some(host) = ModalHost::blurred_for(parent) else {
        return;
    };
    popdown_ancestor_popover(parent);

    let copy = custom_color_copy(item.is_folder());
    let layout = modal_layout(
        item.preview_icon,
        &crate::i18n::tr(copy.title),
        &crate::i18n::tr(copy.choose_subtitle),
        &crate::i18n::tr("Apply"),
    );
    layout.close.set_visible(false);

    let initial_val = initial_color
        .and_then(FolderColorValue::parse)
        .unwrap_or_else(|| FolderColorValue::Custom("#34d399".to_owned()));
    let chooser = build_color_chooser(&layout.icon, item.preview_icon, initial_val.hex());
    layout.body.append(&chooser);

    let back = build_back_button(&layout, &chooser);
    connect_editor_toggle(&chooser, back, layout.subtitle.clone(), copy);

    let dismiss = present_custom_color_layer(&layout, parent, host, &chooser);

    let on_confirm = Rc::new(on_confirm);
    let confirm_dismiss = dismiss.clone();
    let chooser_for_confirm = chooser.clone();
    let on_confirm_click = on_confirm.clone();
    layout.confirm.connect_clicked(move |_| {
        let hex = rgba_to_hex(&chooser_for_confirm.rgba());
        confirm_dismiss();
        on_confirm_click(FolderColorValue::Custom(hex));
    });

    let cancel_dismiss = dismiss.clone();
    layout.cancel.connect_clicked(move |_| cancel_dismiss());

    chooser.grab_focus();
}

#[expect(deprecated, reason = "ColorChooserWidget is the in-app color editor")]
fn build_color_chooser(
    modal_icon: &gtk::Image,
    preview_icon: &'static str,
    initial_hex: &str,
) -> gtk::ColorChooserWidget {
    crate::assets::set_custom_colored_icon(modal_icon, preview_icon, initial_hex);

    let chooser = gtk::ColorChooserWidget::new();
    chooser.set_use_alpha(false);
    crate::assets::use_bundled_spin_icons(&chooser);
    if let Ok(rgba) = gdk::RGBA::parse(initial_hex) {
        chooser.set_rgba(&rgba);
    }

    let icon_for_notify = modal_icon.clone();
    chooser.connect_rgba_notify(move |c| {
        let hex = rgba_to_hex(&c.rgba());
        crate::assets::set_custom_colored_icon(&icon_for_notify, preview_icon, &hex);
    });
    chooser
}

#[expect(deprecated, reason = "ColorChooserWidget is the in-app color editor")]
fn build_back_button(layout: &ModalLayout, chooser: &gtk::ColorChooserWidget) -> gtk::Button {
    let back = gtk::Button::new();
    back.add_css_class("action-dialog-cancel");
    let back_icon = crate::assets::primary_icon(crate::assets::icons::ARROW_LEFT, 14);
    back.set_child(Some(&back_icon));
    crate::ui::accessibility::set_tooltip_and_label(&back, &crate::i18n::tr("Back to palette"));
    back.set_visible(false);
    layout.actions.prepend(&back);

    let chooser_for_back = chooser.clone();
    back.connect_clicked(move |_| {
        chooser_for_back.set_property("show-editor", false);
    });
    back
}

#[expect(deprecated, reason = "ColorChooserWidget is the in-app color editor")]
fn connect_editor_toggle(
    chooser: &gtk::ColorChooserWidget,
    back: gtk::Button,
    subtitle: gtk::Label,
    copy: CustomColorCopy,
) {
    chooser.connect_notify_local(Some("show-editor"), move |c, _| {
        let in_editor = c.property::<bool>("show-editor");
        back.set_visible(in_editor);
        subtitle.set_text(&crate::i18n::tr(if in_editor {
            copy.editor_subtitle
        } else {
            copy.choose_subtitle
        }));
    });
}

/// Adds the layer to the overlay and returns its dismiss action; Escape leaves the
/// editor page first and only then closes the modal.
#[expect(deprecated, reason = "ColorChooserWidget is the in-app color editor")]
fn present_custom_color_layer(
    layout: &ModalLayout,
    parent: &impl IsA<gtk::Widget>,
    host: ModalHost,
    chooser: &gtk::ColorChooserWidget,
) -> Rc<dyn Fn()> {
    let ModalHost {
        overlay: window_overlay,
        blurred_root,
    } = host;
    // The Customize dialog stays open underneath, so the shared restore (which waits
    // for the last modal) cannot return focus to the button that opened this one.
    let opener = parent.as_ref().downgrade();
    let close_layer: Rc<dyn Fn(&gtk::Box)> = {
        let overlay = window_overlay.clone();
        Rc::new(move |layer| {
            let opener = opener.clone();
            dismiss_modal_layer_then(layer, &overlay, blurred_root.as_ref(), move || {
                if let Some(opener) = opener.upgrade()
                    && opener.is_mapped()
                {
                    opener.grab_focus();
                }
            });
        })
    };
    let layer = modal_layer_with_backdrop(&layout.content, close_layer.clone());
    remember_modal_focus(&layer, &window_overlay);
    window_overlay.add_overlay(&layer);
    let dismiss: Rc<dyn Fn()> = {
        let layer = layer.clone();
        Rc::new(move || close_layer(&layer))
    };

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let chooser_for_escape = chooser.clone();
    let escape_dismiss = dismiss.clone();
    keys::on_press(&keys, move |_, press| {
        if press.is(ESCAPE_ANY) {
            if chooser_for_escape.property::<bool>("show-editor") {
                chooser_for_escape.set_property("show-editor", false);
            } else {
                escape_dismiss();
            }
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(keys);
    dismiss
}

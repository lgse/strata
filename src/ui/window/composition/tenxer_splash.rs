// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc, time::Duration};

use gtk::{glib, prelude::*};

use crate::ui::{motion, preferences::PreferenceManager};

const ART: &[u8] = include_bytes!("../../../../data/brand/tenxer-splash.png");

pub(super) fn install(
    window: &gtk::ApplicationWindow,
    overlay: &gtk::Overlay,
    preferences: &Rc<PreferenceManager>,
) -> gtk::Picture {
    let picture = gtk::Picture::new();
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_halign(gtk::Align::Center);
    picture.set_valign(gtk::Align::Center);
    picture.set_can_target(false);
    picture.set_focusable(false);
    picture.add_css_class("tenxer-splash");
    crate::ui::accessibility::set_label(&picture, "10xer mode enabled");
    picture.set_visible(false);
    overlay.add_overlay(&picture);
    overlay.set_measure_overlay(&picture, false);

    let weak_window = window.downgrade();
    let weak_picture = picture.downgrade();
    overlay.connect_get_child_position(move |overlay, child| {
        let window = weak_window.upgrade()?;
        let picture = weak_picture.upgrade()?;
        if child != picture.upcast_ref::<gtk::Widget>() {
            return None;
        }
        let paintable = picture.paintable()?;
        let max_height = (f64::from(window.height()) * 0.66)
            .floor()
            .min(f64::from(overlay.height()));
        let ratio = paintable.intrinsic_aspect_ratio();
        if ratio <= 0.0 {
            return None;
        }
        let height = max_height.min(f64::from(overlay.width()) / ratio).max(0.0);
        let width = (height * ratio).floor() as i32;
        let height = height.floor() as i32;
        Some(gtk::gdk::Rectangle::new(
            (overlay.width() - width) / 2,
            (overlay.height() - height) / 2,
            width,
            height,
        ))
    });

    let previous = Cell::new(preferences.tenxer_mode());
    let generation = Rc::new(Cell::new(0_u64));
    let weak_picture = picture.downgrade();
    preferences.bind_preference(
        window,
        PreferenceManager::tenxer_mode,
        move |window, enabled| {
            if previous.replace(enabled) == enabled {
                return;
            }
            let revision = generation.get().wrapping_add(1);
            generation.set(revision);
            let Some(picture) = weak_picture.upgrade() else {
                return;
            };
            picture.set_visible(false);
            picture.remove_css_class("tenxer-splash-visible");
            picture.remove_css_class("tenxer-splash-exiting");
            if !enabled || !window.is_mapped() {
                return;
            }
            // Settings and dialogs may have added overlays since installation.
            if let Some(overlay) = picture.parent().and_downcast::<gtk::Overlay>() {
                overlay.remove_overlay(&picture);
                overlay.add_overlay(&picture);
                overlay.set_measure_overlay(&picture, false);
            }
            if picture.paintable().is_none() {
                let Ok(texture) = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_static(ART))
                else {
                    return;
                };
                picture.set_paintable(Some(&texture));
            }
            let animated = motion::animations_enabled();
            if animated {
                picture.remove_css_class("tenxer-splash-static");
                let first_frame = Cell::new(true);
                let generation = generation.clone();
                picture.add_tick_callback(move |picture, _| {
                    if generation.get() != revision {
                        return glib::ControlFlow::Break;
                    }
                    if first_frame.replace(false) {
                        return glib::ControlFlow::Continue;
                    }
                    picture.add_css_class("tenxer-splash-visible");
                    glib::ControlFlow::Break
                });
            } else {
                picture.add_css_class("tenxer-splash-static");
                picture.add_css_class("tenxer-splash-visible");
            }
            picture.set_visible(true);
            let weak_picture = picture.downgrade();
            let fade_generation = generation.clone();
            glib::timeout_add_local_once(Duration::from_millis(1150), move || {
                if fade_generation.get() == revision
                    && let Some(picture) = weak_picture.upgrade()
                {
                    picture.add_css_class("tenxer-splash-exiting");
                }
            });
            let weak_picture = picture.downgrade();
            let hide_generation = generation.clone();
            glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                if hide_generation.get() == revision
                    && let Some(picture) = weak_picture.upgrade()
                {
                    picture.set_visible(false);
                }
            });
        },
    );
    picture
}

#[cfg(test)]
mod tests;

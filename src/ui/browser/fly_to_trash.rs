// SPDX-License-Identifier: GPL-3.0-or-later

use crate::model::FileEntry;
use crate::ui::browser::entry::entry_icon;
use crate::ui::modal::window_overlay;
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;
use std::time::Duration;

const DURATION_MS: u64 = 600;
const FRAME_MS: u64 = 8;
const BOUNCE_MS: u64 = 400;
const ARC_HEIGHT: f64 = 80.0;

pub(in crate::ui) fn fly_to_trash(
    source: &gtk::Widget,
    entries: &[FileEntry],
    trash_button: &gtk::Button,
    on_done: impl Fn() + 'static,
) {
    let Some(overlay) = window_overlay(source) else {
        on_done();
        return;
    };
    let Some(trash_pos) = button_center_in_overlay(trash_button, &overlay) else {
        on_done();
        return;
    };
    let row_positions = collect_row_positions(source, &overlay, entries);
    if row_positions.is_empty() {
        on_done();
        return;
    }
    let icons: Vec<gtk::Image> = entries
        .iter()
        .map(|entry| crate::assets::primary_icon(entry_icon(entry), 18))
        .collect();
    let mut flyers = Vec::with_capacity(icons.len());
    for (icon, (x, y)) in icons.into_iter().zip(row_positions.iter()) {
        let flyer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        flyer.add_css_class("fly-to-trash");
        flyer.set_halign(gtk::Align::Start);
        flyer.set_valign(gtk::Align::Start);
        flyer.set_margin_start(*x as i32);
        flyer.set_margin_top(*y as i32);
        icon.add_css_class("fly-to-trash-icon");
        flyer.append(&icon);
        overlay.add_overlay(&flyer);
        flyers.push((flyer, *x, *y, trash_pos.0, trash_pos.1));
    }
    let overlay_for_cleanup = overlay.clone();
    let flyers_for_cleanup = flyers.clone();
    let trash_button = trash_button.clone();
    let on_done: Rc<dyn Fn()> = Rc::new(move || {
        for (flyer, _, _, _, _) in &flyers_for_cleanup {
            overlay_for_cleanup.remove_overlay(flyer);
        }
        bounce_trash(&trash_button);
        on_done();
    });
    animate_frame(flyers, 0, on_done);
}

pub(in crate::ui) fn fly_from_trash(
    source: &gtk::Widget,
    entries: &[FileEntry],
    trash_button: &gtk::Button,
    on_done: impl Fn() + 'static,
) {
    let Some(overlay) = window_overlay(source) else {
        on_done();
        return;
    };
    let Some(trash_pos) = button_center_in_overlay(trash_button, &overlay) else {
        on_done();
        return;
    };
    let row_positions = collect_row_positions(source, &overlay, entries);
    if row_positions.is_empty() {
        on_done();
        return;
    }
    let icons: Vec<gtk::Image> = entries
        .iter()
        .map(|entry| crate::assets::primary_icon(entry_icon(entry), 18))
        .collect();
    let mut flyers = Vec::with_capacity(icons.len());
    for (icon, (x, y)) in icons.into_iter().zip(row_positions.iter()) {
        let flyer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        flyer.add_css_class("fly-to-trash");
        flyer.set_halign(gtk::Align::Start);
        flyer.set_valign(gtk::Align::Start);
        flyer.set_margin_start(trash_pos.0 as i32);
        flyer.set_margin_top(trash_pos.1 as i32);
        icon.add_css_class("fly-to-trash-icon");
        icon.set_opacity(0.0);
        flyer.append(&icon);
        overlay.add_overlay(&flyer);
        flyers.push((flyer, trash_pos.0, trash_pos.1, *x, *y));
    }
    let overlay_for_cleanup = overlay.clone();
    let flyers_for_cleanup = flyers.clone();
    let on_done: Rc<dyn Fn()> = Rc::new(move || {
        for (flyer, _, _, _, _) in &flyers_for_cleanup {
            overlay_for_cleanup.remove_overlay(flyer);
        }
        on_done();
    });
    animate_frame_reverse(flyers, 0, on_done);
}

fn animate_frame(
    flyers: Vec<(gtk::Box, f64, f64, f64, f64)>,
    elapsed: u64,
    on_done: Rc<dyn Fn()>,
) {
    let progress = (elapsed as f64 / DURATION_MS as f64).min(1.0);
    let eased = ease_in_out_cubic(progress);
    let arc = -ARC_HEIGHT * (progress * std::f64::consts::PI).sin();
    for (flyer, start_x, start_y, end_x, end_y) in &flyers {
        let x = start_x + (end_x - start_x) * eased;
        let y = start_y + (end_y - start_y) * eased + arc;
        flyer.set_margin_start(x as i32);
        flyer.set_margin_top(y as i32);
        let opacity = 1.0 - ease_in_cubic(progress);
        if let Some(icon) = flyer.first_child() {
            icon.set_opacity(opacity);
        }
    }
    if progress >= 1.0 {
        on_done();
        return;
    }
    let flyers_clone = flyers.clone();
    let on_done_clone = on_done.clone();
    glib::timeout_add_local_once(
        Duration::from_millis(FRAME_MS),
        move || {
            animate_frame(flyers_clone, elapsed + FRAME_MS, on_done_clone);
        },
    );
}

fn animate_frame_reverse(
    flyers: Vec<(gtk::Box, f64, f64, f64, f64)>,
    elapsed: u64,
    on_done: Rc<dyn Fn()>,
) {
    let progress = (elapsed as f64 / DURATION_MS as f64).min(1.0);
    let eased = ease_out_cubic(progress);
    let arc = -ARC_HEIGHT * ((1.0 - progress) * std::f64::consts::PI).sin();
    for (flyer, start_x, start_y, end_x, end_y) in &flyers {
        let x = start_x + (end_x - start_x) * eased;
        let y = start_y + (end_y - start_y) * eased + arc;
        flyer.set_margin_start(x as i32);
        flyer.set_margin_top(y as i32);
        let opacity = ease_out_cubic(progress);
        if let Some(icon) = flyer.first_child() {
            icon.set_opacity(opacity);
        }
    }
    if progress >= 1.0 {
        on_done();
        return;
    }
    let flyers_clone = flyers.clone();
    let on_done_clone = on_done.clone();
    glib::timeout_add_local_once(
        Duration::from_millis(FRAME_MS),
        move || {
            animate_frame_reverse(flyers_clone, elapsed + FRAME_MS, on_done_clone);
        },
    );
}

fn bounce_trash(button: &gtk::Button) {
    button.remove_css_class("trash-bounce");
    button.add_css_class("trash-bounce");
    let button = button.clone();
    glib::timeout_add_local_once(
        Duration::from_millis(BOUNCE_MS),
        move || {
            button.remove_css_class("trash-bounce");
        },
    );
}

fn ease_in_out_cubic(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

fn ease_in_cubic(t: f64) -> f64 {
    t * t * t
}

fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

#[expect(deprecated, reason = "allocation is fine for position math")]
fn button_center_in_overlay(
    button: &gtk::Button,
    overlay: &gtk::Overlay,
) -> Option<(f64, f64)> {
    let alloc = button.allocation();
    let point = gtk::graphene::Point::new(
        alloc.width() as f32 / 2.0,
        alloc.height() as f32 / 2.0,
    );
    let translated = button.compute_point(overlay, &point)?;
    Some((f64::from(translated.x()), f64::from(translated.y())))
}

fn collect_row_positions(
    source: &gtk::Widget,
    overlay: &gtk::Overlay,
    entries: &[FileEntry],
) -> Vec<(f64, f64)> {
    let mut positions = Vec::new();
    for entry in entries {
        if let Some((x, y)) = find_row_position(source, overlay, entry) {
            positions.push((x, y));
        }
    }
    positions
}

fn find_row_position(
    source: &gtk::Widget,
    overlay: &gtk::Overlay,
    entry: &FileEntry,
) -> Option<(f64, f64)> {
    let mut found = None;
    walk_widgets(source, &mut |w: &gtk::Widget| {
        if found.is_some() {
            return;
        }
        if let Some(label) = find_label_with_text(w, &entry.display_name) {
            if let Some(row) = label.parent().and_then(|p| p.parent()) {
                if let Some(point) =
                    row.compute_point(overlay, &gtk::graphene::Point::new(8.0, 4.0))
                {
                    found = Some((f64::from(point.x()), f64::from(point.y())));
                }
            }
        }
    });
    found
}

fn walk_widgets(widget: &gtk::Widget, f: &mut dyn FnMut(&gtk::Widget)) {
    f(widget);
    let children = widget.observe_children();
    for i in 0..children.n_items() {
        if let Some(child) = children.item(i)
            && let Some(child) = child.downcast_ref::<gtk::Widget>()
        {
            walk_widgets(child, f);
        }
    }
}

fn find_label_with_text(widget: &gtk::Widget, text: &str) -> Option<gtk::Label> {
    if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
        if label.text() == text {
            return Some(label);
        }
    }
    let children = widget.observe_children();
    for i in 0..children.n_items() {
        if let Some(child) = children.item(i)
            && let Some(child) = child.downcast_ref::<gtk::Widget>()
            && let Some(label) = find_label_with_text(child, text)
        {
            return Some(label);
        }
    }
    None
}

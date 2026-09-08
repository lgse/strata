// SPDX-License-Identifier: GPL-3.0-or-later

use super::entry_animation::{
    EntryAnimationTarget, bounds_in_overlay, collect_entry_targets, completion,
    icon_center_in_overlay, sampled_indices,
};
use crate::model::FileEntry;
use crate::ui::browser::entry::entry_icon;
use crate::ui::modal::window_overlay;
use gtk::glib;
use gtk::prelude::*;
use std::time::{Duration, Instant};

const TRAVEL: Duration = Duration::from_millis(420);
const BOUNCE: Duration = Duration::from_millis(280);
const STAGGER_MS: u64 = 14;
const MAX_STAGGER_MS: u64 = 42;
const MAX_FLYERS: usize = 7;
const FLYER_SIZE: f64 = 26.0;

#[derive(Clone)]
struct Flyer {
    widget: gtk::Overlay,
    icon: gtk::Image,
    start: (f64, f64),
    end: (f64, f64),
    arc_height: f64,
    delay: Duration,
}

pub(in crate::ui) fn fly_to_trash(
    source: &gtk::Widget,
    entries: &[FileEntry],
    trash_button: &gtk::Button,
    on_done: impl FnOnce() + 'static,
) {
    if !crate::ui::motion::animations_enabled() {
        on_done();
        return;
    }
    let Some(overlay) = window_overlay(source) else {
        on_done();
        return;
    };
    let Some(trash_center) = widget_center_in_overlay(trash_button.upcast_ref(), &overlay) else {
        on_done();
        return;
    };
    let targets = collect_entry_targets(source, entries);
    let flyers = create_flyers(&overlay, &targets, entries.len(), trash_center, false);
    if flyers.is_empty() {
        on_done();
        return;
    }

    trash_button.add_css_class("trash-receiving");
    let button = trash_button.clone();
    animate_flyers(&overlay, flyers, false, move || {
        button.remove_css_class("trash-receiving");
        impact_trash(&button);
        on_done();
    });
}

pub(in crate::ui) fn fly_from_trash(
    source: &gtk::Widget,
    entries: &[FileEntry],
    trash_button: &gtk::Button,
    on_done: impl FnOnce() + 'static,
) {
    if !crate::ui::motion::animations_enabled() {
        on_done();
        return;
    }
    let Some(overlay) = window_overlay(source) else {
        on_done();
        return;
    };
    let Some(trash_center) = widget_center_in_overlay(trash_button.upcast_ref(), &overlay) else {
        on_done();
        return;
    };
    let targets = collect_entry_targets(source, entries);
    let flyers = create_flyers(&overlay, &targets, entries.len(), trash_center, true);
    if flyers.is_empty() {
        on_done();
        return;
    }

    release_trash(trash_button);
    animate_flyers(&overlay, flyers, true, on_done);
}

fn create_flyers(
    overlay: &gtk::Overlay,
    targets: &[EntryAnimationTarget],
    entry_count: usize,
    trash_center: (f64, f64),
    reversed: bool,
) -> Vec<Flyer> {
    sampled_indices(targets.len(), MAX_FLYERS)
        .into_iter()
        .enumerate()
        .filter_map(|(index, target_index)| {
            let target = &targets[target_index];
            let row_center = icon_center_in_overlay(&target.row, overlay).or_else(|| {
                let bounds = bounds_in_overlay(&target.row, overlay)?;
                Some((
                    f64::from(bounds.x()) + 13.0,
                    f64::from(bounds.y() + bounds.height() / 2.0),
                ))
            })?;
            let row_position = centered_position(row_center);
            let trash_position = centered_position(trash_center);
            let (start, end) = if reversed {
                (trash_position, row_position)
            } else {
                (row_position, trash_position)
            };
            let distance = (end.0 - start.0).hypot(end.1 - start.1);
            let available_above = start.1.min(end.1).max(18.0);
            let arc_variation = 1.0 + (index as f64 % 3.0 - 1.0) * 0.12;
            let arc_height =
                ((distance * 0.16).clamp(28.0, 92.0) * arc_variation).min(available_above);
            let widget = gtk::Overlay::new();
            widget.add_css_class("fly-to-trash");
            if reversed {
                widget.add_css_class("fly-to-trash-contracted");
            }
            widget.set_halign(gtk::Align::Start);
            widget.set_valign(gtk::Align::Start);
            widget.set_can_target(false);
            widget.set_margin_start(start.0.round() as i32);
            widget.set_margin_top(start.1.round() as i32);
            let icon = crate::assets::primary_icon(entry_icon(&target.entry), 18);
            icon.add_css_class("fly-to-trash-icon");
            icon.set_opacity(if reversed { 0.0 } else { 1.0 });
            widget.set_child(Some(&icon));
            if index == 0 && entry_count > MAX_FLYERS {
                let count = gtk::Label::new(Some(&entry_count.to_string()));
                count.add_css_class("fly-to-trash-count");
                count.set_halign(gtk::Align::End);
                count.set_valign(gtk::Align::Start);
                widget.add_overlay(&count);
            }
            overlay.add_overlay(&widget);
            Some(Flyer {
                widget,
                icon,
                start,
                end,
                arc_height,
                delay: Duration::from_millis((index as u64 * STAGGER_MS).min(MAX_STAGGER_MS)),
            })
        })
        .collect()
}

fn animate_flyers(
    overlay: &gtk::Overlay,
    flyers: Vec<Flyer>,
    reversed: bool,
    on_done: impl FnOnce() + 'static,
) {
    let started = Instant::now();
    let total = TRAVEL
        + flyers
            .iter()
            .map(|flyer| flyer.delay)
            .max()
            .unwrap_or_default();
    let overlay_for_cleanup = overlay.clone();
    let callback = completion(on_done);
    let _tick = overlay.clone().add_tick_callback(move |_, _| {
        let elapsed = started.elapsed();
        for flyer in &flyers {
            let progress = elapsed.checked_sub(flyer.delay).map_or(0.0, |elapsed| {
                (elapsed.as_secs_f64() / TRAVEL.as_secs_f64()).clamp(0.0, 1.0)
            });
            let position_progress = ease_in_out_cubic(progress);
            let x = flyer.start.0 + (flyer.end.0 - flyer.start.0) * position_progress;
            let y = flyer.start.1 + (flyer.end.1 - flyer.start.1) * position_progress
                - flyer.arc_height * (std::f64::consts::PI * position_progress).sin();
            flyer.widget.set_margin_start(x.round() as i32);
            flyer.widget.set_margin_top(y.round() as i32);
            flyer.icon.set_opacity(if reversed {
                ease_out_cubic(progress)
            } else {
                1.0 - ease_in_cubic(progress)
            });
            if reversed && progress >= 0.08 {
                flyer.widget.remove_css_class("fly-to-trash-contracted");
            } else if !reversed && progress >= 0.58 {
                flyer.widget.add_css_class("fly-to-trash-contracted");
            }
        }

        if elapsed < total {
            return glib::ControlFlow::Continue;
        }
        for flyer in &flyers {
            overlay_for_cleanup.remove_overlay(&flyer.widget);
        }
        if let Some(callback) = callback.borrow_mut().take() {
            callback();
        }
        glib::ControlFlow::Break
    });
}

fn impact_trash(button: &gtk::Button) {
    animate_trash_class(button, "trash-impact");
}

fn release_trash(button: &gtk::Button) {
    animate_trash_class(button, "trash-release");
}

fn animate_trash_class(button: &gtk::Button, class: &'static str) {
    button.remove_css_class(class);
    button.add_css_class(class);
    let button = button.clone();
    glib::timeout_add_local_once(BOUNCE, move || {
        button.remove_css_class(class);
    });
}

fn centered_position(center: (f64, f64)) -> (f64, f64) {
    (center.0 - FLYER_SIZE / 2.0, center.1 - FLYER_SIZE / 2.0)
}

fn ease_in_out_cubic(progress: f64) -> f64 {
    if progress < 0.5 {
        4.0 * progress * progress * progress
    } else {
        1.0 - (-2.0 * progress + 2.0).powi(3) / 2.0
    }
}

fn ease_in_cubic(progress: f64) -> f64 {
    progress * progress * progress
}

fn ease_out_cubic(progress: f64) -> f64 {
    1.0 - (1.0 - progress).powi(3)
}

fn widget_center_in_overlay(widget: &gtk::Widget, overlay: &gtk::Overlay) -> Option<(f64, f64)> {
    let bounds = bounds_in_overlay(widget, overlay)?;
    Some((
        f64::from(bounds.x() + bounds.width() / 2.0),
        f64::from(bounds.y() + bounds.height() / 2.0),
    ))
}

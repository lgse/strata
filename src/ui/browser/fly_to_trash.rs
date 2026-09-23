// SPDX-License-Identifier: GPL-3.0-or-later

use super::entry_animation::{
    EntryAnimationTarget, animate, bounds_in_overlay, collect_entry_targets, find_row_by_name,
    icon_center_in_overlay, sampled_indices,
};
use crate::model::FileEntry;
use crate::ui::browser::entry::entry_icon;
use crate::ui::modal::window_overlay;
use gtk::glib;
use gtk::prelude::*;
use std::time::Duration;

const TRAVEL: Duration = Duration::from_millis(360);
const RESTORE_TRAVEL: Duration = Duration::from_millis(240);
const RELEASE_TRAVEL: Duration = Duration::from_millis(220);
const BOUNCE: Duration = Duration::from_millis(280);
const WISP: Duration = Duration::from_millis(260);
const STAGGER_MS: u64 = 12;
const MAX_STAGGER_MS: u64 = 36;
const MAX_FLYERS: usize = 7;
const FLYER_SIZE: f64 = 26.0;

#[derive(Clone)]
struct Flyer {
    widget: gtk::Overlay,
    icon: gtk::Image,
    start: (f64, f64),
    end: (f64, f64),
    arc_height: f64,
    bank_class: &'static str,
    delay: Duration,
    target_name: Option<String>,
    index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flight {
    Inbound,
    Outbound,
    Release,
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
    let flyers = create_flyers(&overlay, &targets, trash_center, Flight::Inbound);
    if flyers.is_empty() {
        on_done();
        return;
    }

    trash_button.add_css_class("trash-receiving");
    let button = trash_button.clone();
    animate_flyers(&overlay, source, flyers, Flight::Inbound, move || {
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
    let mode = restore_flight(entries);
    let flyers = match mode {
        Flight::Release => {
            let targets = collect_entry_targets(source, entries);
            create_flyers(&overlay, &targets, trash_center, Flight::Release)
        }
        Flight::Outbound => create_outbound_flyers(&overlay, source, entries, trash_center),
        Flight::Inbound => unreachable!(),
    };
    if flyers.is_empty() {
        on_done();
        return;
    }

    if mode == Flight::Release {
        animate_trash_class(trash_button, "trash-rebel-shudder");
    } else {
        release_trash(trash_button);
    }
    animate_flyers(&overlay, source, flyers, mode, on_done);
}

fn restore_flight(entries: &[FileEntry]) -> Flight {
    if entries
        .iter()
        .all(|entry| super::paths::is_trash_location(&entry.location))
    {
        Flight::Release
    } else {
        Flight::Outbound
    }
}

// The destination is outside the Trash view; fly out rather than toward disappearing rows.
fn release_end(row_center: (f64, f64), index: usize, count: usize) -> (f64, f64) {
    let drift = (index as f64 - (count.saturating_sub(1)) as f64 / 2.0) * 44.0;
    let rise = (row_center.1 - 28.0).clamp(90.0, 240.0);
    (row_center.0 + drift, row_center.1 - rise)
}

fn create_flyers(
    overlay: &gtk::Overlay,
    targets: &[EntryAnimationTarget],
    trash_center: (f64, f64),
    mode: Flight,
) -> Vec<Flyer> {
    let indices = sampled_indices(targets.len(), MAX_FLYERS);
    let count = indices.len();
    indices
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
            let (start, end, arc_height) = match mode {
                Flight::Inbound => (
                    row_position,
                    trash_position,
                    arc_height(row_position, trash_position, index),
                ),
                Flight::Outbound => (
                    trash_position,
                    row_position,
                    arc_height(trash_position, row_position, index),
                ),
                Flight::Release => (row_position, release_end(row_center, index, count), 0.0),
            };
            let widget = gtk::Overlay::new();
            widget.add_css_class("fly-to-trash");
            if mode == Flight::Outbound {
                widget.add_css_class("fly-to-trash-contracted");
            } else if mode == Flight::Release {
                widget.add_css_class("fly-to-trash-release");
            }
            widget.set_halign(gtk::Align::Start);
            widget.set_valign(gtk::Align::Start);
            widget.set_can_target(false);
            widget.set_margin_start(start.0.round() as i32);
            widget.set_margin_top(start.1.round() as i32);
            let icon = crate::assets::primary_icon(entry_icon(&target.entry), 18);
            icon.add_css_class("fly-to-trash-icon");
            icon.set_opacity(if mode == Flight::Outbound { 0.0 } else { 1.0 });
            widget.set_child(Some(&icon));
            overlay.add_overlay(&widget);
            let delay = Duration::from_millis((index as u64 * STAGGER_MS).min(MAX_STAGGER_MS));
            if mode == Flight::Release {
                resurrect_burst(overlay, row_center, delay);
            }
            let drift = end.0 - start.0;
            let bank_class = if drift < -10.0 {
                "fly-launch-left"
            } else if drift > 10.0 {
                "fly-launch-right"
            } else {
                "fly-launch-straight"
            };
            Some(Flyer {
                widget,
                icon,
                start,
                end,
                arc_height,
                bank_class,
                delay,
                target_name: None,
                index,
            })
        })
        .collect()
}

fn create_outbound_flyers(
    overlay: &gtk::Overlay,
    source: &gtk::Widget,
    entries: &[FileEntry],
    trash_center: (f64, f64),
) -> Vec<Flyer> {
    let indices = sampled_indices(entries.len(), MAX_FLYERS);
    let trash_position = centered_position(trash_center);
    let default_end = bounds_in_overlay(source, overlay)
        .map(|bounds| {
            (
                (f64::from(bounds.x()) + 140.0).max(trash_position.0 + 100.0),
                f64::from(bounds.y()) + 80.0,
            )
        })
        .unwrap_or((trash_position.0 + 200.0, trash_position.1 - 60.0));

    indices
        .into_iter()
        .enumerate()
        .map(|(index, entry_index)| {
            let entry = &entries[entry_index];
            let existing_row = find_row_by_name(source, &entry.display_name);
            let existing_pos = existing_row
                .as_ref()
                .and_then(|row| icon_center_in_overlay(row, overlay))
                .map(centered_position);
            let end =
                existing_pos.unwrap_or((default_end.0, default_end.1 + (index as f64 * 32.0)));
            let arc = arc_height(trash_position, end, index);

            let widget = gtk::Overlay::new();
            widget.add_css_class("fly-to-trash");
            widget.add_css_class("fly-to-trash-contracted");
            widget.set_halign(gtk::Align::Start);
            widget.set_valign(gtk::Align::Start);
            widget.set_can_target(false);
            widget.set_margin_start(trash_position.0.round() as i32);
            widget.set_margin_top(trash_position.1.round() as i32);

            let icon = crate::assets::primary_icon(entry_icon(entry), 18);
            icon.add_css_class("fly-to-trash-icon");
            icon.set_opacity(0.0);
            widget.set_child(Some(&icon));
            overlay.add_overlay(&widget);

            let delay = Duration::from_millis((index as u64 * STAGGER_MS).min(MAX_STAGGER_MS));
            Flyer {
                widget,
                icon,
                start: trash_position,
                end,
                arc_height: arc,
                bank_class: "fly-launch-straight",
                delay,
                target_name: Some(entry.display_name.clone()),
                index,
            }
        })
        .collect()
}

fn resurrect_burst(overlay: &gtk::Overlay, center: (f64, f64), delay: Duration) {
    let overlay = overlay.clone();
    let spawn = move || {
        let burst = gtk::Overlay::new();
        burst.set_halign(gtk::Align::Start);
        burst.set_valign(gtk::Align::Start);
        burst.set_can_target(false);
        const BURST_SIZE: f64 = 48.0;
        burst.set_size_request(BURST_SIZE as i32, BURST_SIZE as i32);
        burst.set_margin_start((center.0 - BURST_SIZE / 2.0).round() as i32);
        burst.set_margin_top((center.1 - BURST_SIZE / 2.0).round() as i32);

        let shockwave = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        shockwave.add_css_class("trash-resurrect-shockwave");
        shockwave.set_halign(gtk::Align::Center);
        shockwave.set_valign(gtk::Align::Center);

        let beam = gtk::Box::new(gtk::Orientation::Vertical, 0);
        beam.add_css_class("trash-resurrect-beam");
        beam.set_halign(gtk::Align::Center);
        beam.set_valign(gtk::Align::Center);

        let core = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        core.add_css_class("trash-resurrect-core");
        core.set_halign(gtk::Align::Center);
        core.set_valign(gtk::Align::Center);

        let spark_left = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spark_left.add_css_class("trash-resurrect-spark");
        spark_left.set_halign(gtk::Align::Center);
        spark_left.set_valign(gtk::Align::Center);

        let spark_right = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spark_right.add_css_class("trash-resurrect-spark-alt");
        spark_right.set_halign(gtk::Align::Center);
        spark_right.set_valign(gtk::Align::Center);

        burst.set_child(Some(&shockwave));
        burst.add_overlay(&beam);
        burst.add_overlay(&core);
        burst.add_overlay(&spark_left);
        burst.add_overlay(&spark_right);

        overlay.add_overlay(&burst);
        let overlay_cleanup = overlay.clone();
        glib::timeout_add_local_once(WISP, move || {
            overlay_cleanup.remove_overlay(&burst);
        });
    };

    if delay.is_zero() {
        spawn();
    } else {
        glib::timeout_add_local_once(delay, spawn);
    }
}

fn arc_height(start: (f64, f64), end: (f64, f64), index: usize) -> f64 {
    let distance = (end.0 - start.0).hypot(end.1 - start.1);
    let available_above = start.1.min(end.1).max(18.0);
    let arc_variation = 1.0 + (index as f64 % 3.0 - 1.0) * 0.12;
    ((distance * 0.16).clamp(28.0, 92.0) * arc_variation).min(available_above)
}

fn animate_flyers(
    overlay: &gtk::Overlay,
    source: &gtk::Widget,
    flyers: Vec<Flyer>,
    mode: Flight,
    on_done: impl FnOnce() + 'static,
) {
    let travel = match mode {
        Flight::Release => RELEASE_TRAVEL,
        Flight::Outbound => RESTORE_TRAVEL,
        Flight::Inbound => TRAVEL,
    };
    let total = travel
        + flyers
            .iter()
            .map(|flyer| flyer.delay)
            .max()
            .unwrap_or_default();
    let overlay_for_cleanup = overlay.clone();
    let source_for_frame = source.clone();
    let overlay_for_frame = overlay.clone();
    let flyers = std::rc::Rc::new(std::cell::RefCell::new(flyers));
    let flyers_for_cleanup = flyers.clone();
    animate(
        overlay,
        total,
        move |elapsed| {
            let mut flyers = flyers.borrow_mut();
            for flyer in flyers.iter_mut() {
                if mode == Flight::Outbound
                    && let Some(target_name) = &flyer.target_name
                    && let Some(row) = find_row_by_name(&source_for_frame, target_name)
                    && let Some(center) = icon_center_in_overlay(&row, &overlay_for_frame)
                {
                    let row_pos = centered_position(center);
                    flyer.end = row_pos;
                    flyer.arc_height = arc_height(flyer.start, row_pos, flyer.index);
                }
                let progress = elapsed.checked_sub(flyer.delay).map_or(0.0, |elapsed| {
                    (elapsed.as_secs_f64() / travel.as_secs_f64()).clamp(0.0, 1.0)
                });
                if mode == Flight::Release {
                    const COIL_END: f64 = 0.12;
                    let (x, y) = if progress < COIL_END {
                        let coil_t = progress / COIL_END;
                        let dip = (coil_t * std::f64::consts::PI).sin() * 5.0;
                        (flyer.start.0, flyer.start.1 + dip)
                    } else {
                        let launch_t = (progress - COIL_END) / (1.0 - COIL_END);
                        let launch_progress = launch_curve(launch_t);
                        let x = flyer.start.0 + (flyer.end.0 - flyer.start.0) * launch_progress;
                        let y = flyer.start.1 + (flyer.end.1 - flyer.start.1) * launch_progress;
                        (x, y)
                    };
                    flyer.widget.set_margin_start(x.round() as i32);
                    flyer.widget.set_margin_top(y.round() as i32);
                    let opacity = if progress < 0.65 {
                        1.0
                    } else {
                        let fade_t = (progress - 0.65) / 0.35;
                        1.0 - fade_t * fade_t
                    };
                    flyer.icon.set_opacity(opacity);
                    if (COIL_END..0.82).contains(&progress) {
                        flyer.widget.add_css_class(flyer.bank_class);
                    } else if progress >= 0.82 {
                        flyer.widget.add_css_class("fly-insurrection-dispersal");
                    }
                } else {
                    let position_progress = ease_in_out_cubic(progress);
                    let x = flyer.start.0 + (flyer.end.0 - flyer.start.0) * position_progress;
                    let y = flyer.start.1 + (flyer.end.1 - flyer.start.1) * position_progress
                        - flyer.arc_height * (std::f64::consts::PI * position_progress).sin();
                    flyer.widget.set_margin_start(x.round() as i32);
                    flyer.widget.set_margin_top(y.round() as i32);
                    flyer.icon.set_opacity(match mode {
                        Flight::Outbound => ease_out_cubic(progress),
                        _ => 1.0 - ease_in_cubic(progress),
                    });
                    if mode == Flight::Outbound && progress >= 0.08 {
                        flyer.widget.remove_css_class("fly-to-trash-contracted");
                    } else if mode == Flight::Inbound && progress >= 0.58 {
                        flyer.widget.add_css_class("fly-to-trash-contracted");
                    }
                }
            }
        },
        move || {
            for flyer in flyers_for_cleanup.borrow().iter() {
                overlay_for_cleanup.remove_overlay(&flyer.widget);
            }
            on_done();
        },
    );
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

fn launch_curve(progress: f64) -> f64 {
    1.0 - (1.0 - progress).powi(3)
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

#[cfg(test)]
mod tests;

// SPDX-License-Identifier: GPL-3.0-or-later

use super::entry_animation::{
    EntryAnimationTarget, animate, bounds_in_overlay, collect_entry_targets,
    icon_center_in_overlay, sampled_indices,
};
use crate::model::FileEntry;
use crate::ui::browser::entry::entry_icon;
use crate::ui::modal::window_overlay;
use gtk::glib;
use gtk::prelude::*;
use std::time::Duration;

const TRAVEL: Duration = Duration::from_millis(420);
const RELEASE_TRAVEL: Duration = Duration::from_millis(350);
const BOUNCE: Duration = Duration::from_millis(280);
const WISP: Duration = Duration::from_millis(420);
const STAGGER_MS: u64 = 18;
const MAX_STAGGER_MS: u64 = 54;
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flight {
    /// Rows arc down into the trash icon.
    Inbound,
    /// Icons emerge from the trash icon onto restored rows.
    Outbound,
    /// Rows viewed inside Trash release their icons upward out of view.
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
    let lid = open_trash_lid(trash_button);
    let button = trash_button.clone();
    animate_flyers(&overlay, flyers, Flight::Inbound, move || {
        button.remove_css_class("trash-receiving");
        close_trash_lid(lid);
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
    let mode = restore_flight(entries);
    let flyers = create_flyers(&overlay, &targets, trash_center, mode);
    if flyers.is_empty() {
        on_done();
        return;
    }

    if mode == Flight::Release {
        animate_trash_class(trash_button, "trash-rebel-shudder");
    } else {
        release_trash(trash_button);
    }
    let lid = open_trash_lid(trash_button);
    animate_flyers(&overlay, flyers, mode, move || {
        close_trash_lid(lid);
        on_done();
    });
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

/// Restore from the Trash view has no visible destination, so icons lift off
/// their rows and fan out upward as they fade — items heading home.
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
            })
        })
        .collect()
}

/// An explosive breakout burst at the row: shockwave blast, rocket plume,
/// radiant core, and side shrapnel sparks.
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
    flyers: Vec<Flyer>,
    mode: Flight,
    on_done: impl FnOnce() + 'static,
) {
    let travel = if mode == Flight::Release {
        RELEASE_TRAVEL
    } else {
        TRAVEL
    };
    let total = travel
        + flyers
            .iter()
            .map(|flyer| flyer.delay)
            .max()
            .unwrap_or_default();
    let overlay_for_cleanup = overlay.clone();
    let flyers_for_cleanup = flyers.clone();
    animate(
        overlay,
        total,
        move |elapsed| {
            for flyer in &flyers {
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
            for flyer in &flyers_for_cleanup {
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

/// Opens the sidebar trash lid and returns the icon plus its current name so
/// the matching closed state can be restored after the flyers land.
fn open_trash_lid(trash_button: &gtk::Button) -> Option<(gtk::Image, String)> {
    // sidebar_button puts the icon image first in the row's content box.
    let image = trash_button
        .child()
        .and_then(|content| content.first_child())
        .and_then(|widget| widget.downcast::<gtk::Image>().ok())?;
    let name = crate::assets::primary_icon_name(&image)?;
    let open = name
        .strip_prefix("strata-trash")
        .map(|suffix| {
            format!(
                "strata-trash{}-open",
                suffix.strip_suffix("-open").unwrap_or(suffix)
            )
        })
        .unwrap_or_else(|| crate::assets::icons::TRASH_OPEN.to_owned());
    crate::assets::set_primary_icon(&image, &open);
    Some((image, name))
}

fn close_trash_lid(state: Option<(gtk::Image, String)>) {
    if let Some((image, name)) = state {
        crate::assets::set_primary_icon(&image, &name);
    }
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

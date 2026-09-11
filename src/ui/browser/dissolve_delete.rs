// SPDX-License-Identifier: GPL-3.0-or-later

use super::entry_animation::{animate, bounds_in_overlay, collect_entry_targets};
use crate::model::FileEntry;
use crate::ui::modal::window_overlay;
use gtk::glib;
use gtk::gsk::prelude::IsRenderNode;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::time::Duration;

const DURATION: Duration = Duration::from_millis(560);
const MIN_FRAGMENT_BUDGET: usize = 80;
const MAX_FRAGMENT_BUDGET: usize = 192;
const FRAGMENTS_PER_ROW: usize = 48;

pub(super) type DissolveCleanup = Box<dyn FnOnce()>;

#[derive(Clone)]
struct FragmentMotion {
    source: gtk::graphene::Rect,
    offset: (f64, f64),
    delay: f64,
}

#[derive(Clone)]
struct Fragment {
    node: gtk::gsk::RenderNode,
    motion: FragmentMotion,
}

#[derive(Clone)]
struct DissolveRow {
    origin: gtk::graphene::Point,
    fragments: Vec<Fragment>,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub(super) struct DissolveCanvas {
        pub(super) rows: RefCell<Vec<DissolveRow>>,
        pub(super) progress: Cell<f64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DissolveCanvas {
        const NAME: &'static str = "StrataDissolveCanvas";
        type Type = super::DissolveCanvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for DissolveCanvas {}

    impl WidgetImpl for DissolveCanvas {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let progress = self.progress.get();
            for row in self.rows.borrow().iter() {
                for fragment in &row.fragments {
                    snapshot_fragment(snapshot, row, fragment, progress);
                }
            }
        }
    }
}

glib::wrapper! {
    struct DissolveCanvas(ObjectSubclass<imp::DissolveCanvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl DissolveCanvas {
    fn new(rows: Vec<DissolveRow>) -> Self {
        let canvas: Self = glib::Object::new();
        canvas.imp().rows.replace(rows);
        canvas.set_halign(gtk::Align::Fill);
        canvas.set_valign(gtk::Align::Fill);
        canvas.set_hexpand(true);
        canvas.set_vexpand(true);
        canvas.set_can_target(false);
        canvas
    }

    fn set_progress(&self, progress: f64) {
        self.imp().progress.set(progress);
        self.queue_draw();
    }
}

pub(in crate::ui) fn dissolve_delete(
    source: &gtk::Widget,
    entries: &[FileEntry],
    on_done: impl FnOnce(DissolveCleanup) + 'static,
) {
    if !crate::ui::motion::animations_enabled() {
        on_done(Box::new(|| {}));
        return;
    }
    let Some(overlay) = window_overlay(source) else {
        on_done(Box::new(|| {}));
        return;
    };
    let targets = collect_entry_targets(source, entries);
    let measured: Vec<_> = targets
        .into_iter()
        .filter_map(|target| {
            let bounds = bounds_in_overlay(&target.row, &overlay)?;
            bounds_intersect_viewport(bounds, overlay.width(), overlay.height())
                .then_some((target.row, bounds))
        })
        .collect();
    if measured.is_empty() {
        on_done(Box::new(|| {}));
        return;
    }

    let fragment_budget = fragment_budget(measured.len());
    let tile_size = tile_size_for_budget(
        measured
            .iter()
            .map(|(_, bounds)| f64::from(bounds.width() * bounds.height()))
            .sum(),
        fragment_budget,
    );
    let mut random = Random::new(0x9E37_79B9_7F4A_7C15);
    let mut source_rows = Vec::with_capacity(measured.len());
    let mut rendered_rows = Vec::with_capacity(measured.len());
    for (index, (row, bounds)) in measured.iter().enumerate() {
        let Some(node) = snapshot_row(row, bounds.width(), bounds.height()) else {
            continue;
        };
        let fragments = fragments_for_size(
            bounds.width(),
            bounds.height(),
            tile_size,
            index,
            &mut random,
        )
        .into_iter()
        .map(|motion| Fragment {
            node: gtk::gsk::ClipNode::new(&node, &motion.source).upcast(),
            motion,
        })
        .collect();
        rendered_rows.push(DissolveRow {
            origin: gtk::graphene::Point::new(bounds.x(), bounds.y()),
            fragments,
        });
        source_rows.push((row.clone(), row.opacity()));
    }
    if rendered_rows.is_empty() {
        on_done(Box::new(|| {}));
        return;
    }

    let canvas = DissolveCanvas::new(rendered_rows);
    overlay.add_overlay(&canvas);
    for (row, _) in &source_rows {
        row.set_opacity(0.0);
    }

    let canvas_for_tick = canvas.clone();
    let overlay_for_cleanup = overlay.clone();
    let rows_for_cleanup = source_rows.clone();
    let canvas_for_cleanup = canvas.clone();
    animate(
        &canvas,
        DURATION,
        move |elapsed| {
            let progress = (elapsed.as_secs_f64() / DURATION.as_secs_f64()).clamp(0.0, 1.0);
            canvas_for_tick.set_progress(progress);
        },
        move || {
            overlay_for_cleanup.remove_overlay(&canvas_for_cleanup);
            on_done(Box::new(move || {
                for (row, opacity) in rows_for_cleanup {
                    row.set_opacity(opacity);
                }
            }));
        },
    );
}

fn snapshot_row(row: &gtk::Widget, width: f32, height: f32) -> Option<gtk::gsk::RenderNode> {
    let paintable = gtk::WidgetPaintable::new(Some(row));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
    snapshot.to_node()
}

fn snapshot_fragment(
    snapshot: &gtk::Snapshot,
    row: &DissolveRow,
    fragment: &Fragment,
    progress: f64,
) {
    let motion = &fragment.motion;
    let local = ((progress - motion.delay) / (1.0 - motion.delay)).clamp(0.0, 1.0);
    let movement = ease_out_cubic(local);
    let opacity = 1.0 - ease_in_cubic(local);
    if opacity <= 0.0 {
        return;
    }

    let lift = motion.offset.1 * movement + 14.0 * local * local;
    snapshot.save();
    snapshot.translate(&gtk::graphene::Point::new(
        row.origin.x() + (motion.offset.0 * movement) as f32,
        row.origin.y() + lift as f32,
    ));
    snapshot.push_opacity(opacity);
    snapshot.append_node(&fragment.node);
    snapshot.pop();
    snapshot.restore();
}

fn bounds_intersect_viewport(bounds: gtk::graphene::Rect, width: i32, height: i32) -> bool {
    bounds.width() > 0.0
        && bounds.height() > 0.0
        && bounds.x() < width as f32
        && bounds.y() < height as f32
        && bounds.x() + bounds.width() > 0.0
        && bounds.y() + bounds.height() > 0.0
}

fn fragment_budget(row_count: usize) -> usize {
    (row_count * FRAGMENTS_PER_ROW).clamp(MIN_FRAGMENT_BUDGET, MAX_FRAGMENT_BUDGET)
}

fn tile_size_for_budget(area: f64, budget: usize) -> f32 {
    (area / budget.max(1) as f64).sqrt().max(4.0) as f32
}

fn fragments_for_size(
    width: f32,
    height: f32,
    tile_size: f32,
    row_index: usize,
    random: &mut Random,
) -> Vec<FragmentMotion> {
    let columns = (width / tile_size).ceil().max(1.0) as usize;
    let rows = (height / tile_size).ceil().max(1.0) as usize;
    let row_delay = (row_index as f64 * 0.018).min(0.06);
    let mut fragments = Vec::with_capacity(rows * columns);
    for row in 0..rows {
        for column in 0..columns {
            let x = column as f32 * tile_size;
            let y = row as f32 * tile_size;
            let fragment_width = tile_size.min(width - x);
            let fragment_height = tile_size.min(height - y);
            let horizontal = (f64::from(x) + f64::from(fragment_width) / 2.0) / f64::from(width);
            let outward = (horizontal - 0.5) * 32.0;
            let offset_x = outward + random.centered(34.0);
            let offset_y = if random.next() < 0.18 {
                3.0 + random.next() * 13.0
            } else {
                -10.0 - random.next() * 38.0
            };
            fragments.push(FragmentMotion {
                source: gtk::graphene::Rect::new(x, y, fragment_width, fragment_height),
                offset: (offset_x, offset_y),
                delay: (row_delay + horizontal * 0.36 + random.next() * 0.13).min(0.55),
            });
        }
    }
    fragments
}

fn ease_in_cubic(progress: f64) -> f64 {
    progress * progress * progress
}

fn ease_out_cubic(progress: f64) -> f64 {
    1.0 - (1.0 - progress).powi(3)
}

struct Random(u64);

impl Random {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 32) as f64 / u32::MAX as f64
    }

    fn centered(&mut self, spread: f64) -> f64 {
        (self.next() - 0.5) * spread
    }
}

#[cfg(test)]
mod tests;

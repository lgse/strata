// SPDX-License-Identifier: MIT

//! Variable-width horizontal tab reorder, tear-out, incoming drop gap, and drag auto-scroll.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use rune::{Drag, El, Live, ScrollPos, Tx};
use rune_kit::shell::TearOutSpec;

/// Pointer travel in points before a press becomes a reorder drag.
const DRAG_SLOP: f32 = 4.0;

/// Maximum scroll step per frame while dragging a tab near a strip edge.
const AUTO_SCROLL_SPEED: f32 = 8.0;

/// Fraction of strip viewport width at each end that triggers auto-scrolling.
const AUTO_SCROLL_BAND: f32 = 0.05;

/// Vertical tolerance outside the strip band where edge auto-scroll remains active.
const AUTO_SCROLL_TOLERANCE: f32 = 10.0;

/// 60 Hz auto-scroll repeat cadence.
const AUTO_SCROLL_FRAME: Duration = Duration::from_micros(16_667);

/// Live scroll viewport geometry and offset, kept across views for drag auto-scroll and incoming
/// drop-slot hit testing.
#[derive(Clone)]
pub(super) struct ScrollView {
    pos: Rc<Cell<ScrollPos>>,
    rect: Live<[f32; 4]>,
}

impl Default for ScrollView {
    fn default() -> Self {
        Self {
            pos: Rc::new(Cell::new(ScrollPos::default())),
            rect: Live::new([0.0; 4]),
        }
    }
}

impl ScrollView {
    /// Attaches layout and scroll-offset observers to the strip's horizontal scroll viewport.
    pub(super) fn watch(&self, scroll: El) -> El {
        let pos = self.pos.clone();
        scroll
            .drawn_rect_into(&self.rect)
            .chain_scroll_offset(move |at| pos.set(at))
    }

    pub(super) fn rect(&self) -> [f32; 4] {
        self.rect.get()
    }

    pub(super) fn offset_x(&self) -> f32 {
        self.pos.get().offset[0]
    }

    /// Scrolls the viewport horizontally when a reorder drag nears an overflowing edge.
    pub(super) fn auto_scroll(&self, scroll_key: &str, d: &Drag, tx: &mut Tx) {
        let (at, [x, y, w, h]) = (self.pos.get(), self.rect.get());
        let [px, py] = d.window_point();
        let within_y = (y - AUTO_SCROLL_TOLERANCE..=y + h + AUTO_SCROLL_TOLERANCE).contains(&py);
        let band = w * AUTO_SCROLL_BAND;
        if !within_y || band <= 0.0 {
            return;
        }
        let pos = px - x;
        let speed = if pos <= band && at.offset[0] > 0.0 {
            -AUTO_SCROLL_SPEED * ((band - pos) / band)
        } else if pos >= w - band && at.offset[0] < at.max[0] {
            AUTO_SCROLL_SPEED * ((pos - (w - band)) / band)
        } else {
            return;
        };
        let to = (at.offset[0] + speed).clamp(0.0, at.max[0]);
        let mut next = at;
        next.offset[0] = to;
        self.pos.set(next);
        tx.scroll_to_instant(scroll_key, to, 0.0);
        tx.drag_repeat_after(AUTO_SCROLL_FRAME);
    }
}

/// Parameters for beginning a tab press/drag gesture.
pub(super) struct PressSpec {
    pub(super) from: usize,
    pub(super) grab: [f32; 2],
    pub(super) widths: Vec<f32>,
    pub(super) gap: f32,
    pub(super) tear: Option<TearOutSpec>,
}

/// What a drag release produced.
pub(super) enum Release {
    None,
    Click,
    Reorder { from: usize, to: usize },
    TearOut { from: usize, offset: [f32; 2] },
}

#[derive(Clone, Debug)]
struct Grab {
    at: [f32; 2],
    scrolled: f32,
    widths: Vec<f32>,
    gap: f32,
    tear: Option<TearOutSpec>,
}

#[derive(Clone, Debug)]
enum Gesture {
    Idle {
        swallow_click: bool,
    },
    Pressed {
        from: usize,
        grab: Grab,
    },
    Reorder {
        from: usize,
        to: usize,
        delta: f32,
        grab: Grab,
    },
    TearOut {
        from: usize,
        offset: [f32; 2],
        grab: Grab,
    },
}

impl Default for Gesture {
    fn default() -> Self {
        Self::Idle {
            swallow_click: false,
        }
    }
}

/// Incoming sidebar/external tab drag hovering over the strip.
#[derive(Clone, Debug, Default)]
enum Incoming {
    #[default]
    None,
    Landing {
        point: [f32; 2],
        slot: Rc<Cell<Option<usize>>>,
    },
}

/// Variable-width strip drag and incoming-gap state owned by [`super::BrowserTabsState`].
#[derive(Clone, Default)]
pub(super) struct ReorderState {
    gesture: Gesture,
    incoming: Incoming,
    pub(super) width_lock: Option<f32>,
    pub(super) scroll: ScrollView,
}

impl ReorderState {
    pub(super) fn is_idle(&self) -> bool {
        matches!(self.gesture, Gesture::Idle { .. })
    }

    pub(super) fn dragging(&self) -> Option<usize> {
        match self.gesture {
            Gesture::Reorder { from, .. } | Gesture::TearOut { from, .. } => Some(from),
            _ => None,
        }
    }

    pub(super) fn reordering(&self) -> bool {
        matches!(self.gesture, Gesture::Reorder { .. })
    }

    pub(super) fn torn_out(&self) -> bool {
        matches!(self.gesture, Gesture::TearOut { .. })
    }

    pub(super) fn has_active_gesture(&self) -> bool {
        !matches!(self.gesture, Gesture::Idle { .. })
    }

    pub(super) fn take_swallowed_click(&mut self) -> bool {
        match &mut self.gesture {
            Gesture::Idle { swallow_click } => std::mem::take(swallow_click),
            _ => false,
        }
    }

    pub(super) fn leave_strip(&mut self) -> bool {
        self.width_lock.take().is_some()
    }

    pub(super) fn set_incoming(&mut self, point: Option<[f32; 2]>) {
        match (point, &mut self.incoming) {
            (Some(point), Incoming::Landing { point: at, .. }) => *at = point,
            (Some(point), Incoming::None) => {
                self.incoming = Incoming::Landing {
                    point,
                    slot: Rc::new(Cell::new(None)),
                };
            }
            (None, incoming) => {
                if let Incoming::Landing { slot, .. } = incoming {
                    slot.set(None);
                }
                *incoming = Incoming::None;
            }
        }
    }

    pub(super) fn incoming_slot(&self) -> Option<usize> {
        match &self.incoming {
            Incoming::Landing { slot, .. } => slot.get(),
            Incoming::None => None,
        }
    }

    /// Resolves and stores the incoming drop slot from the laid-out tab widths when an incoming
    /// drag is hovering over the strip.
    pub(super) fn resolve_incoming(&self, widths: &[f32], gap: f32) -> Option<usize> {
        if !self.is_idle() {
            return None;
        }
        let Incoming::Landing { point, slot } = &self.incoming else {
            return None;
        };
        let viewport_x = self.scroll.rect()[0] - self.scroll.offset_x();
        let found = landing_slot(widths, gap, *point, viewport_x);
        slot.set(Some(found));
        Some(found)
    }

    pub(super) fn press(&mut self, spec: PressSpec) {
        let scrolled = self.scroll.offset_x();
        self.gesture = Gesture::Pressed {
            from: spec.from,
            grab: Grab {
                at: spec.grab,
                scrolled,
                widths: spec.widths,
                gap: spec.gap,
                tear: spec.tear,
            },
        };
    }

    pub(super) fn grab_point(&self) -> [f32; 2] {
        match &self.gesture {
            Gesture::Pressed { grab, .. }
            | Gesture::Reorder { grab, .. }
            | Gesture::TearOut { grab, .. } => grab.at,
            Gesture::Idle { .. } => [0.0; 2],
        }
    }

    /// Updates the active gesture from pointer motion `[dx, dy]`. Returns true when visual state
    /// changed.
    pub(super) fn follow(&mut self, dx: f32, dy: f32) -> bool {
        let (from, grab, was_torn, was_reordering) = match &self.gesture {
            Gesture::Pressed { from, grab } => (*from, grab.clone(), false, false),
            Gesture::Reorder { from, grab, .. } => (*from, grab.clone(), false, true),
            Gesture::TearOut { from, grab, .. } => (*from, grab.clone(), true, false),
            Gesture::Idle { .. } => return false,
        };
        if from >= grab.widths.len() {
            self.gesture = Gesture::default();
            return true;
        }
        let scrolled = self.scroll.offset_x() - grab.scrolled;
        let along = dx + scrolled;
        let across = dy;
        let (min_delta, max_delta) = drag_bounds(&grab.widths, grab.gap, from);
        let pitch = (grab.widths[from] + grab.gap).max(1.0);
        let over_strip = ((min_delta - pitch)..=(max_delta + pitch)).contains(&along);
        let detach = grab.tear.is_some_and(|spec| {
            if was_torn {
                across.abs() > spec.reattach_distance || !over_strip
            } else {
                across.abs() >= spec.detach_distance
            }
        });
        if detach {
            self.gesture = Gesture::TearOut {
                from,
                offset: [dx, dy],
                grab,
            };
            return true;
        }
        if !was_reordering && !was_torn && along.hypot(across) < DRAG_SLOP {
            return false;
        }
        let delta = along.clamp(min_delta, max_delta);
        let to = target_index(&grab.widths, grab.gap, from, delta);
        self.gesture = Gesture::Reorder {
            from,
            to,
            delta,
            grab,
        };
        true
    }

    pub(super) fn release(&mut self, cancelled: bool, on_tab: bool) -> Release {
        let prev = std::mem::take(&mut self.gesture);
        match prev {
            Gesture::Pressed { .. } => {
                self.gesture = Gesture::Idle {
                    swallow_click: on_tab,
                };
                Release::Click
            }
            Gesture::Reorder { from, to, .. } if !cancelled && from != to => {
                self.gesture = Gesture::Idle {
                    swallow_click: on_tab,
                };
                Release::Reorder { from, to }
            }
            Gesture::TearOut { from, offset, .. } if !cancelled => {
                self.gesture = Gesture::Idle {
                    swallow_click: on_tab,
                };
                Release::TearOut { from, offset }
            }
            Gesture::Reorder { .. } | Gesture::TearOut { .. } => {
                self.gesture = Gesture::Idle {
                    swallow_click: on_tab,
                };
                Release::None
            }
            other => {
                self.gesture = other;
                Release::None
            }
        }
    }

    /// Visual `[dx, dy]` offset for tab `index` during an active reorder or tear-out gesture.
    pub(super) fn shift(&self, index: usize) -> [f32; 2] {
        match &self.gesture {
            Gesture::Reorder {
                from,
                to,
                delta,
                grab,
            } => {
                let step = grab.widths.get(*from).copied().unwrap_or(0.0) + grab.gap;
                let dx = if index == *from {
                    *delta
                } else if *from < index && index <= *to {
                    -step
                } else if *to <= index && index < *from {
                    step
                } else {
                    0.0
                };
                [dx, 0.0]
            }
            Gesture::TearOut { from, offset, grab } => {
                let step = grab.widths.get(*from).copied().unwrap_or(0.0) + grab.gap;
                if index == *from {
                    *offset
                } else if index > *from {
                    [-step, 0.0]
                } else {
                    [0.0, 0.0]
                }
            }
            _ => [0.0, 0.0],
        }
    }
}

/// Clamped horizontal travel range `[min, max]` for dragging tab `from` within the strip.
pub(super) fn drag_bounds(widths: &[f32], gap: f32, from: usize) -> (f32, f32) {
    let min_delta: f32 = -widths[..from].iter().map(|&w| w + gap).sum::<f32>();
    let max_delta: f32 = widths
        .get(from + 1..)
        .unwrap_or(&[])
        .iter()
        .map(|&w| w + gap)
        .sum();
    (min_delta.min(0.0), max_delta.max(0.0))
}

/// Target index for dragging tab `from` by `delta` points across variable-width tabs.
pub(super) fn target_index(widths: &[f32], gap: f32, from: usize, delta: f32) -> usize {
    let dragged_w = widths[from];
    let orig_x: f32 = widths[..from].iter().map(|&w| w + gap).sum();
    let drag_center = orig_x + delta + dragged_w / 2.0;

    let mut slot_x = 0.0_f32;
    let mut best = from;
    let mut best_dist = f32::INFINITY;
    let mut others = widths
        .iter()
        .enumerate()
        .filter_map(|(i, &w)| (i != from).then_some(w));

    for candidate in 0..widths.len() {
        let center = slot_x + dragged_w / 2.0;
        let dist = (drag_center - center).abs();
        let tie_away = (dist - best_dist).abs() < 1e-3 && candidate != from && best == from;
        if dist < best_dist || tie_away {
            best_dist = dist;
            best = candidate;
        }
        if let Some(w) = others.next() {
            slot_x += w + gap;
        }
    }
    best
}

/// Insertion slot for an incoming drag at window point `point` over variable-width tabs.
pub(super) fn landing_slot(widths: &[f32], gap: f32, point: [f32; 2], viewport_x: f32) -> usize {
    let along = point[0] - viewport_x;
    let mut start = 0.0_f32;
    for (i, &w) in widths.iter().enumerate() {
        let pitch = w + gap;
        if along < start + pitch / 2.0 {
            return i;
        }
        start += pitch;
    }
    widths.len()
}

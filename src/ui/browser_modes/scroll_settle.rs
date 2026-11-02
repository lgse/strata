// SPDX-License-Identifier: MIT

//! Defers row refreshes until a pane's scrolling comes to rest.

use std::{cell::Cell, rc::Rc};

use gtk::{glib, prelude::*};

pub(super) fn install_scroll_refresh(
    scroll: &gtk::ScrolledWindow,
    scrolling: Rc<Cell<bool>>,
    on_refresh: impl Fn() + 'static,
) {
    let settle = Rc::new(ScrollSettle {
        scroll: scroll.downgrade(),
        scrolling,
        last_scroll: Cell::new(std::time::Instant::now()),
        started: Cell::new(0),
        armed: Cell::new(false),
        on_refresh: Box::new(on_refresh),
    });
    for adjustment in [scroll.vadjustment(), scroll.hadjustment()] {
        let settle = settle.clone();
        adjustment.connect_value_changed(move |_| {
            settle.scrolling.set(true);
            settle.last_scroll.set(std::time::Instant::now());
            if !settle.armed.replace(true) {
                settle.started.set(glib::monotonic_time());
                ScrollSettle::arm(&settle, settle.delay());
            }
        });
    }
}

pub(super) struct ScrollSettle {
    scroll: glib::WeakRef<gtk::ScrolledWindow>,
    scrolling: Rc<Cell<bool>>,
    last_scroll: Cell<std::time::Instant>,
    started: Cell<i64>,
    armed: Cell<bool>,
    on_refresh: Box<dyn Fn()>,
}

impl ScrollSettle {
    pub(super) const DELAY: std::time::Duration = std::time::Duration::from_millis(25);
    const MINIMUM_FRAMES: f64 = 2.0;

    fn delay(&self) -> std::time::Duration {
        let frames = self
            .scroll
            .upgrade()
            .and_then(|scroll| scroll.frame_clock())
            // fps() averages the whole frame history, so an idle gap before this scroll would
            // stretch the delay.
            .filter(|clock| {
                clock
                    .timings(clock.history_start())
                    .is_some_and(|timings| timings.frame_time() >= self.started.get())
            })
            .map(|clock| clock.fps())
            .filter(|fps| *fps > 0.0)
            .map_or(std::time::Duration::ZERO, |fps| {
                std::time::Duration::from_secs_f64(Self::MINIMUM_FRAMES / fps)
            });
        Self::DELAY.max(frames)
    }

    fn arm(settle: &Rc<Self>, delay: std::time::Duration) {
        let settle = Rc::downgrade(settle);
        glib::timeout_add_local_once(delay, move || {
            let Some(settle) = settle.upgrade() else {
                return;
            };
            let still = settle.last_scroll.get().elapsed();
            let delay = settle.delay();
            if still < delay {
                Self::arm(&settle, delay - still);
                return;
            }
            settle.armed.set(false);
            settle.scrolling.set(false);
            if settle.scroll.upgrade().is_some() {
                (settle.on_refresh)();
            }
        });
    }
}

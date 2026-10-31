// SPDX-License-Identifier: MIT

use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

pub(super) struct Widgets {
    pub root: glib::WeakRef<gtk::Box>,
    pub overlay: glib::WeakRef<gtk::Overlay>,
    pub dock: glib::WeakRef<gtk::ScrolledWindow>,
    pub list: glib::WeakRef<gtk::Box>,
    pub meta: glib::WeakRef<gtk::Label>,
    pub status: glib::WeakRef<gtk::Label>,
    pub progress: glib::WeakRef<gtk::ProgressBar>,
}

pub(super) struct Completion {
    widgets: Widgets,
    completed: Cell<bool>,
    hovered: Cell<bool>,
    focused: Cell<bool>,
    pinned: Cell<bool>,
    duration: Cell<Duration>,
    remaining: Cell<Duration>,
    started: Cell<Option<Instant>>,
    exiting: Cell<Option<Instant>>,
    source: RefCell<Option<glib::SourceId>>,
    cancel: RefCell<Option<Rc<dyn Fn()>>>,
}

impl Completion {
    pub(super) fn new(widgets: Widgets) -> Rc<Self> {
        Rc::new(Self {
            widgets,
            completed: Cell::new(false),
            hovered: Cell::new(false),
            focused: Cell::new(false),
            pinned: Cell::new(false),
            duration: Cell::new(Duration::ZERO),
            remaining: Cell::new(Duration::ZERO),
            started: Cell::new(None),
            exiting: Cell::new(None),
            source: RefCell::new(None),
            cancel: RefCell::new(None),
        })
    }

    pub(super) fn bind(
        self: &Rc<Self>,
        root: &gtk::Box,
        close: &gtk::Button,
        complete: &gtk::Button,
        pin: &gtk::ToggleButton,
    ) {
        let state = self.clone();
        close.connect_clicked(move |_| {
            if state.completed.get() {
                state.dismiss();
            } else {
                let action = state.cancel.borrow().clone();
                if let Some(action) = action {
                    action();
                }
            }
        });
        let state = self.clone();
        complete.connect_clicked(move |_| {
            if state.completed.get() {
                state.dismiss();
            }
        });
        let state = self.clone();
        pin.connect_toggled(move |pin| {
            state.pinned.set(pin.is_active());
            state.update_pause();
        });
        let motion = gtk::EventControllerMotion::new();
        let state = self.clone();
        motion.connect_enter(move |_, _, _| {
            state.hovered.set(true);
            state.update_pause();
        });
        let state = self.clone();
        motion.connect_leave(move |_| {
            state.hovered.set(false);
            state.update_pause();
        });
        root.add_controller(motion);
        let focus = gtk::EventControllerFocus::new();
        let state = self.clone();
        focus.connect_enter(move |_| {
            state.focused.set(true);
            state.update_pause();
        });
        let state = self.clone();
        focus.connect_leave(move |_| {
            state.focused.set(false);
            state.update_pause();
        });
        root.add_controller(focus);
        let state = self.clone();
        root.connect_parent_notify(move |root| {
            if root.parent().is_none() {
                state.stop();
            }
        });
        let state = self.clone();
        root.connect_realize(move |_| state.update_pause());
        let state = self.clone();
        root.connect_unrealize(move |_| {
            state.remaining.set(state.time_left(Instant::now()));
            state.started.set(None);
            state.stop();
        });
    }

    pub(super) fn set_cancel(&self, action: Rc<dyn Fn()>) {
        self.cancel.replace(Some(action));
    }

    pub(super) fn complete(self: &Rc<Self>, duration: Duration) {
        if self.completed.replace(true) {
            return;
        }
        self.cancel.take();
        self.duration.set(duration);
        self.remaining.set(duration);
        if let Some(status) = self.widgets.status.upgrade() {
            status.set_text(&crate::i18n::percent(100));
        }
        if let Some(progress) = self.widgets.progress.upgrade() {
            crate::ui::accessibility::set_description(
                &progress,
                Some(&crate::i18n::tr(
                    "Time remaining before notification closes",
                )),
            );
        }
        self.render(Instant::now());
        self.update_pause();
    }

    fn paused(&self) -> bool {
        self.hovered.get() || self.focused.get() || self.pinned.get()
    }

    fn time_left(&self, now: Instant) -> Duration {
        self.started.get().map_or(self.remaining.get(), |start| {
            self.remaining
                .get()
                .saturating_sub(now.saturating_duration_since(start))
        })
    }

    fn update_pause(self: &Rc<Self>) {
        if !self.completed.get() {
            return;
        }
        if !self
            .widgets
            .root
            .upgrade()
            .is_some_and(|root| root.parent().is_some() && root.is_realized())
        {
            return;
        }
        let now = Instant::now();
        if self.paused() {
            self.remaining.set(self.time_left(now));
            self.started.set(None);
            self.exiting.set(None);
            self.stop();
            if let Some(root) = self.widgets.root.upgrade() {
                root.set_opacity(1.0);
            }
        } else if self.started.get().is_none() {
            self.started.set(Some(now));
        }
        self.render(now);
        if !self.paused() && self.source.borrow().is_none() {
            let Some(root) = self.widgets.root.upgrade() else {
                return;
            };
            if root.parent().is_none() || !root.is_realized() {
                return;
            }
            let weak = Rc::downgrade(self);
            self.source.replace(Some(glib::timeout_add_local(
                Duration::from_millis(40),
                move || {
                    weak.upgrade()
                        .map_or(glib::ControlFlow::Break, |state| state.tick())
                },
            )));
        }
    }

    fn render(&self, now: Instant) {
        let left = self.time_left(now);
        if let Some(progress) = self.widgets.progress.upgrade() {
            progress.set_fraction(
                (left.as_secs_f64() / self.duration.get().as_secs_f64().max(f64::EPSILON))
                    .clamp(0.0, 1.0),
            );
        }
        if let Some(meta) = self.widgets.meta.upgrade() {
            let state = if self.pinned.get() {
                crate::i18n::tr("pinned")
            } else if self.paused() {
                crate::i18n::tr("paused")
            } else {
                rust_i18n::t!(
                    "closing in %{time}",
                    time = crate::i18n::duration(left.as_secs_f64().ceil() as u64)
                )
                .into_owned()
            };
            let text = format!(" · {state}");
            if meta.text() != text {
                meta.set_text(&text);
            }
        }
    }

    fn tick(&self) -> glib::ControlFlow {
        let Some(root) = self.widgets.root.upgrade() else {
            self.source.take();
            return glib::ControlFlow::Break;
        };
        if root.parent().is_none() || !root.is_realized() {
            self.source.take();
            return glib::ControlFlow::Break;
        }
        let now = Instant::now();
        self.render(now);
        if !self.time_left(now).is_zero() {
            return glib::ControlFlow::Continue;
        }
        let fade = if crate::ui::motion::animations_enabled() {
            Duration::from_millis(180)
        } else {
            Duration::ZERO
        };
        let start = self.exiting.get().unwrap_or(now);
        self.exiting.set(Some(start));
        let elapsed = now.saturating_duration_since(start);
        if elapsed < fade {
            root.set_opacity(1.0 - elapsed.as_secs_f64() / fade.as_secs_f64());
            return glib::ControlFlow::Continue;
        }
        // GLib removes the running source on Break; don't remove its ID twice.
        self.source.take();
        self.dismiss();
        glib::ControlFlow::Break
    }

    fn stop(&self) {
        if let Some(source) = self.source.borrow_mut().take() {
            source.remove();
        }
    }

    pub(super) fn dismiss(&self) {
        self.stop();
        self.cancel.take();
        if let Some(root) = self.widgets.root.upgrade()
            && let Some(list) = self.widgets.list.upgrade()
            && root.parent().as_ref() == Some(list.upcast_ref())
        {
            list.remove(&root);
        }
        if let Some(list) = self.widgets.list.upgrade()
            && list.first_child().is_none()
            && let Some(dock) = self.widgets.dock.upgrade()
            && dock.parent().is_some()
            && let Some(overlay) = self.widgets.overlay.upgrade()
        {
            overlay.remove_overlay(&dock);
        }
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        self.stop();
    }
}

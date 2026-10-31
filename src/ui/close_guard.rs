// SPDX-License-Identifier: MIT

use gtk::{glib, prelude::*};
use std::{cell::RefCell, rc::Rc};

pub(super) struct CloseBlocker {
    pub(super) title: String,
    pub(super) detail: String,
}

impl CloseBlocker {
    pub(super) fn show(&self, window: &gtk::Window) {
        super::modal::show_error_dialog(window, &self.title, &self.detail);
    }
}

/// Receives whether the application would exit once this window closes.
type Guard = Rc<dyn Fn(bool) -> Option<CloseBlocker>>;

thread_local! {
    static GUARDS: RefCell<Vec<(glib::WeakRef<gtk::Window>, Guard)>> =
        const { RefCell::new(Vec::new()) };
}

/// Registers a close refusal that application-wide closes (restart) can query
/// before closing any window.
pub(super) fn install(
    window: &impl IsA<gtk::Window>,
    guard: impl Fn(bool) -> Option<CloseBlocker> + 'static,
) {
    let window = window.upcast_ref::<gtk::Window>();
    let guard: Guard = Rc::new(guard);
    GUARDS.with(|guards| {
        let mut guards = guards.borrow_mut();
        guards.retain(|(window, _)| window.upgrade().is_some());
        guards.push((window.downgrade(), guard.clone()));
    });
    window.connect_close_request(move |window| {
        let closing_application = window
            .application()
            .is_some_and(|application| application.windows().len() == 1);
        match guard(closing_application) {
            Some(blocker) => {
                blocker.show(window);
                glib::Propagation::Stop
            }
            None => glib::Propagation::Proceed,
        }
    });
}

pub(super) fn application_blocker(
    application: &gtk::Application,
) -> Option<(gtk::Window, CloseBlocker)> {
    let windows = application.windows();
    let guards = GUARDS.with(|guards| guards.borrow().clone());
    guards.into_iter().find_map(|(window, guard)| {
        let window = window.upgrade().filter(|window| windows.contains(window))?;
        guard(true).map(|blocker| (window, blocker))
    })
}

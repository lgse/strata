// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
};

use gtk::{glib, prelude::*};

/// Why preference changes are not reaching the settings file.
#[derive(Clone, Copy, Debug)]
pub(super) enum SaveProblem {
    /// Writing failed; each later change tries again.
    WriteFailed,
    /// The file could not be read at startup, so saving stays off to keep it (#721).
    UnreadableAtStartup,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Streak {
    #[default]
    Armed,
    /// A notice waits for the setter that failed to return.
    Pending,
    Shown,
}

/// Tells the user once per failure streak that changes are not being saved.
/// Only browser windows register, so portal chooser failures are only logged.
#[derive(Default)]
pub(super) struct SaveNotices {
    windows: RefCell<Vec<glib::WeakRef<gtk::Window>>>,
    streak: Rc<Cell<Streak>>,
}

impl SaveNotices {
    pub(super) fn register(&self, window: &gtk::Window) {
        let mut windows = self.windows.borrow_mut();
        windows.retain(|candidate| candidate.upgrade().is_some());
        windows.push(window.downgrade());
    }

    pub(super) fn saved(&self) {
        self.streak.set(Streak::Armed);
    }

    /// Shows the notice in the active browser window, where the change was made.
    /// With no active browser window the change did not come from the user there
    /// (a timer, or a chooser), so the streak stays armed for the next failure.
    pub(super) fn report(&self, problem: SaveProblem, path: &Path, reason: &str) {
        if self.streak.get() != Streak::Armed {
            return;
        }
        let Some(window) = self
            .windows
            .borrow()
            .iter()
            .filter_map(glib::WeakRef::upgrade)
            .find(|window| window.is_visible() && window.is_active())
        else {
            return;
        };
        self.streak.set(Streak::Pending);
        let (title, detail) = notice_text(problem, path, reason);
        let streak = self.streak.clone();
        let window = window.downgrade();
        // Opened after the failing setter returns rather than inside its call stack.
        glib::idle_add_local_once(move || {
            if streak.get() != Streak::Pending {
                return;
            }
            streak.set(Streak::Armed);
            let Some(window) = window.upgrade().filter(|window| window.is_visible()) else {
                return;
            };
            if crate::ui::modal::window_overlay(&window).is_none() {
                return;
            }
            streak.set(Streak::Shown);
            crate::ui::modal::show_error_dialog_with_summary(
                &window,
                &title,
                &crate::i18n::tr("Changes last only until Strata closes"),
                &detail,
                Rc::new(|| {}),
            );
        });
    }
}

/// `reason` is already localized for the slot after a colon.
fn notice_text(problem: SaveProblem, path: &Path, reason: &str) -> (String, String) {
    // The error text may carry a multi-line source excerpt; the log keeps it whole.
    let reason = reason
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end_matches('.');
    let path = path.display();
    match problem {
        SaveProblem::WriteFailed => (
            crate::i18n::tr("Settings can't be saved"),
            rust_i18n::t!(
                "Strata couldn't write “%{path}”: %{reason}. It tries again with each change.",
                path = path,
                reason = reason
            )
            .into_owned(),
        ),
        SaveProblem::UnreadableAtStartup => (
            crate::i18n::tr("Settings file can't be read"),
            rust_i18n::t!(
                "Strata couldn't read “%{path}” when it started: %{reason}. To keep the file as it is, Strata won't save over it. Fix or remove the file, then restart Strata.",
                path = path,
                reason = reason
            )
            .into_owned(),
        ),
    }
}

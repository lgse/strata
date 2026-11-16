// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    io,
    path::Path,
    rc::Rc,
};

use gtk::{glib, prelude::*};
use strata_desktop::settings::save_notice::{
    NoticeProblem as SaveProblem, SaveSubject, save_notice_summary, save_notice_text,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Streak {
    #[default]
    Armed,
    /// A notice waits for the setter that failed to return.
    Pending,
    Shown,
}

/// Tells the user once per failure streak that changes are not being saved.
#[derive(Default)]
pub(super) struct SaveNotices {
    subject: SaveSubject,
    windows: RefCell<Vec<glib::WeakRef<gtk::Window>>>,
    streak: Rc<Cell<Streak>>,
}

impl SaveNotices {
    pub(super) fn for_folder_views() -> Self {
        Self {
            subject: SaveSubject::FolderViews,
            ..Self::default()
        }
    }

    pub(super) fn register(&self, window: &gtk::Window) {
        let mut windows = self.windows.borrow_mut();
        windows.retain(|candidate| candidate.upgrade().is_some());
        windows.push(window.downgrade());
    }

    pub(super) fn saved(&self) {
        self.streak.set(Streak::Armed);
    }

    pub(super) fn write_failed(&self, path: &Path, error: &io::Error) {
        let subject = match self.subject {
            SaveSubject::Settings => "preference",
            SaveSubject::FolderViews => "folder settings",
        };
        tracing::warn!(%error, path = %path.display(), "unable to save {subject}");
        self.report(SaveProblem::WriteFailed, path, error);
    }

    pub(super) fn unreadable_at_startup(&self, path: &Path, error: &io::Error) {
        self.report(SaveProblem::UnreadableAtStartup, path, error);
    }

    /// Shows the notice in the active browser window, where the change was made.
    /// With no active browser window the change did not come from the user there
    /// (a timer, or a chooser), so the streak stays armed for the next failure.
    fn report(&self, problem: SaveProblem, path: &Path, error: &io::Error) {
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
        let (title, detail) = save_notice_text(
            self.subject,
            problem,
            path,
            &crate::services::io_error_detail(error),
        );
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
                &save_notice_summary(),
                &detail,
                Rc::new(|| {}),
            );
        });
    }
}

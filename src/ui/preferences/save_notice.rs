// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    fs, io,
    path::Path,
    rc::Rc,
};

use gtk::{glib, prelude::*};

#[derive(Clone, Copy, Debug)]
enum SaveProblem {
    /// Writing failed; each later change tries again.
    WriteFailed,
    /// The file could not be read at startup, so saving stays off to keep it.
    UnreadableAtStartup,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SaveSubject {
    #[default]
    Settings,
    FolderViews,
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

    /// Creates the file's directory and writes it with `write`, one of the
    /// atomic writers in `crate::storage`. Success ends a failure streak; a
    /// failure is returned for `write_failed`.
    pub(super) fn write(
        &self,
        path: &Path,
        contents: &str,
        write: fn(&Path, &[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write(path, contents.as_bytes())?;
        self.streak.set(Streak::Armed);
        Ok(())
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
        let (title, detail) = notice_text(
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
                &crate::i18n::tr("Changes last only until Strata closes"),
                &detail,
                Rc::new(|| {}),
            );
        });
    }
}

/// `reason` is already localized for the slot after a colon.
fn notice_text(
    subject: SaveSubject,
    problem: SaveProblem,
    path: &Path,
    reason: &str,
) -> (String, String) {
    // The error text may carry a multi-line source excerpt; the log keeps it whole.
    let reason = reason
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end_matches('.');
    let path = path.display();
    let title = crate::i18n::tr(match (subject, problem) {
        (SaveSubject::Settings, SaveProblem::WriteFailed) => "Settings can't be saved",
        (SaveSubject::Settings, SaveProblem::UnreadableAtStartup) => "Settings file can't be read",
        (SaveSubject::FolderViews, SaveProblem::WriteFailed) => "Folder settings can't be saved",
        (SaveSubject::FolderViews, SaveProblem::UnreadableAtStartup) => {
            "Folder settings file can't be read"
        }
    });
    match problem {
        SaveProblem::WriteFailed => (
            title,
            rust_i18n::t!(
                "Strata couldn't write “%{path}”: %{reason}. It tries again with each change.",
                path = path,
                reason = reason
            )
            .into_owned(),
        ),
        SaveProblem::UnreadableAtStartup => (
            title,
            rust_i18n::t!(
                "Strata couldn't read “%{path}” when it started: %{reason}. To keep the file as it is, Strata won't save over it. Fix or remove the file, then restart Strata.",
                path = path,
                reason = reason
            )
            .into_owned(),
        ),
    }
}

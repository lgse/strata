// SPDX-License-Identifier: MIT

//! The Compress dialog and starting compression: GTK's `show_compress_dialog` and
//! `start_compression` (`src/ui/browser/archive.rs`), built on C15's [`StrataDialog`].
//!
//! - **What opens it.** The `compress` command: the entry menu's "Compress…" row, acting on
//!   the selection when the row's entry is in it or on the entry alone, and a key bound to
//!   [`ACTIONS_COMPRESS`] in the default map at `direct_commands` (`src/ui/window/keyboard/direct.rs`).
//!   Every entry needs a local path; otherwise nothing opens.
//! - **The dialog** ([`view`]). "Compress 1 item" (or the item count) with the entries' kind
//!   summary under it; **Archive name**, starting on the single entry's name or `"archive"`
//!   selected; **Format** (`ZIP`, `7Z`, `TAR.GZ`, `TAR` toggle buttons); and **Protection**
//!   (`No password`, `Password protected` toggle buttons, shown while the format supports a
//!   password, with **Password** and **Confirm password** fields under `Password protected`).
//!   Once the name or a password field differs from its starting value, backdrop clicks leave
//!   the dialog open; Escape, Cancel and the close button always close it.
//! - **Validation.** Editing **Archive name** updates its basename error live
//!   (`name_field_error`). Confirming strips a trailing extension matching the chosen format
//!   (`normalized_archive_name`), checks `validate_basename` (keeping the dialog open and
//!   focusing **Archive name** on refusal), and checks that a requested password is non-empty
//!   and matches **Confirm password** (stacking a danger error dialog over Compress when not).
//! - **Starting compression.** `archive_name_taken` probes the destination off the UI thread
//!   (`strata_fs::adapters::probes::presence`). When the name is free, `Browser::compress`
//!   starts with `TransferConflict::FailIfExists`, and `BrowserEvent::ArchiveCompleted` selects
//!   the created archive in its destination folder.
//!
//! Left out for [A13]:
//!
//! - **Compression conflicts** (`ask_to_replace_archive` when `archive_name_taken` finds an
//!   existing file), **progress / cancelling** (`on_archive_started`), and **undoing** a
//!   replaced archive (`docs/rune/parity/packages/A6.md`, item 7). When the destination name is
//!   already taken, compression stops without overwriting, as `transfer.rs` did before A12's
//!   conflict dialog.
//! - **10xer's `;` -> `c` chord** and its shortcut feedback banner (`Nothing to compress`,
//!   `Can't compress these items`), [A11]'s.
//!
//! [A11]: ../../../docs/rune/parity/packages/A11.md
//! [A13]: ../../../docs/rune/parity/packages/A13.md

#[cfg(test)]
mod tests;
mod view;

use std::ffi::OsStr;

use rune::{TextEdit, Tx};
use strata_core::{
    i18n::{count, tr},
    model::{ArchiveFormat, EntryKind, FileEntry, Location},
};
use strata_desktop::keys::{Map, bindings::ACTIONS_COMPRESS};
use strata_fs::{
    adapters::probes::{self, Presence},
    app::BrowserEvent,
    services::{Naming, TransferConflict, name_field_error, validate_basename},
};

use crate::{
    App,
    commands::{Action, Call, Holds, Keys, On, Outcome, Stage},
    filter, launch, menus, shell,
    slots::{Later, Listener, Part, Place, Setup},
};

/// One open or closing Compress dialog.
struct Form {
    entries: Vec<FileEntry>,
    destination: Location,
    default_name: String,
    title: String,
    subtitle: String,
    name: TextEdit,
    name_error: Option<&'static str>,
    format: ArchiveFormat,
    password_protected: bool,
    password: TextEdit,
    password_revealed: bool,
    confirm: TextEdit,
    confirm_revealed: bool,
    generation: u64,
}

impl Form {
    fn new(entries: Vec<FileEntry>, destination: Location, generation: u64) -> Self {
        let default_name = if entries.len() == 1 {
            entries[0].display_name.clone()
        } else {
            "archive".to_owned()
        };
        let title = rust_i18n::t!("Compress %{value1}", value1 = count("items", entries.len()))
            .into_owned();
        let subtitle = entry_kind_summary(&entries);
        let mut name = TextEdit::new(&default_name);
        name.set_selection(0, default_name.len());
        name.focused = true;
        Self {
            entries,
            destination,
            default_name,
            title,
            subtitle,
            name,
            name_error: None,
            format: ArchiveFormat::Zip,
            password_protected: false,
            password: TextEdit::new("").secure(),
            password_revealed: false,
            confirm: TextEdit::new("").secure(),
            confirm_revealed: false,
            generation,
        }
    }

    fn is_dirty(&self) -> bool {
        self.name.text != self.default_name
            || !self.password.text.is_empty()
            || !self.confirm.text.is_empty()
    }

    fn shows_password(&self) -> bool {
        self.format.supports_password() && self.password_protected
    }

    fn field_focused(&self) -> bool {
        self.name.focused
            || (self.shows_password() && (self.password.focused || self.confirm.focused))
    }

    fn wipe_secrets(&mut self) {
        self.password = TextEdit::new("").secure();
        self.confirm = TextEdit::new("").secure();
        self.password_revealed = false;
        self.confirm_revealed = false;
    }
}

/// A validation error stacked over the Compress dialog (`Password required`, `Passwords do not
/// match`).
struct Notice {
    title: String,
    message: String,
    generation: u64,
}

pub(crate) struct Compress {
    later: Later<App>,
    form: Option<Form>,
    open: bool,
    opened: u64,
    notice: Option<Notice>,
    notice_open: bool,
    notices: u64,
    focus_target: Option<String>,
    pending_destination: Option<Location>,
    pending_select: Option<String>,
}

impl Part for Compress {
    const PLACE: Place<App> = Place::Dialogs(view::build);

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            later: setup.later.clone(),
            form: None,
            open: false,
            opened: 0,
            notice: None,
            notice_open: false,
            notices: 0,
            focus_target: None,
            pending_destination: None,
            pending_select: None,
        }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, _, heard| {
            queue_archive_event(event, heard);
        }))
    }

    fn poll(app: &mut App, _: f64, tx: &mut Tx) -> bool {
        let Some(target) = app.compress.focus_target.take() else {
            return false;
        };
        if app.compress.open {
            tx.focus(target, false);
            return true;
        }
        false
    }

    fn keyboard(app: &App) -> Option<Holds> {
        let compress = &app.compress;
        if compress.notice_open {
            return Some(Holds::Modal { field: false });
        }
        let form = compress.form.as_ref().filter(|_| compress.open)?;
        Some(Holds::Modal {
            field: form.field_focused(),
        })
    }

    const COMMANDS: &'static [Action<App>] = &[Action::new("compress", compress)
        .command(ACTIONS_COMPRESS)
        .keys(&[Keys::at(
            Stage::DirectCommands,
            &[ACTIONS_COMPRESS.in_map(Map::Default)],
        )
        .when(|app, at| !shell::preferences(app).tenxer_mode && !at.text() && at.in_view())])];
}

/// Basename used when creating the archive, with `format`'s extension removed (`archive.rs`).
fn normalized_archive_name(name: &str, format: ArchiveFormat) -> String {
    name.strip_suffix(&format!(".{}", format.extension()))
        .unwrap_or(name)
        .to_owned()
}

/// `entry_kind_summary` (`src/ui/browser/entry.rs`).
fn entry_kind_summary(entries: &[FileEntry]) -> String {
    let directories = entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Directory)
        .count();
    let files = entries.iter().filter(|entry| entry.is_file()).count();
    if directories == entries.len() {
        count("folders", directories)
    } else if files == entries.len() {
        count("files", files)
    } else {
        count("items", entries.len())
    }
}

fn shows_columns(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)("columns"))
}

/// `BrowserView::command_targets` (`src/ui/browser/file_commands.rs`).
fn focus_entries(app: &mut App) -> Vec<FileEntry> {
    let tenxer = shell::preferences(app).tenxer_mode;
    if let Some(results) = filter::targets::selected(app) {
        if !results.is_empty() || !tenxer {
            return results;
        }
        return filter::targets::under_cursor(app)
            .map(|(_, entry)| entry)
            .into_iter()
            .collect();
    }
    shell::publish(app);
    if !tenxer {
        return app.browser.transfer_entries();
    }
    let depth = if shows_columns(app) {
        Some(app.columns.targeted())
    } else {
        app.browser.active_depth()
    };
    depth
        .map(|d| app.browser.command_entries(d))
        .unwrap_or_default()
}

fn target_entries(app: &mut App, on: &On) -> Vec<FileEntry> {
    match *on {
        On::Row((depth, id)) => menus::context_entries(app, depth, id)
            .map(|(_, entries)| entries)
            .unwrap_or_default(),
        On::Focus => focus_entries(app),
        On::Column(_) | On::Place(_) => Vec::new(),
    }
}

fn compress(app: &mut App, call: &Call<'_>, _: &mut Tx) -> Outcome {
    let entries = target_entries(app, &call.on);
    if !entries.is_empty()
        && entries
            .iter()
            .all(|entry| entry.location.native_path().is_some())
    {
        open(app, entries);
    }
    // `direct_commands` stops propagation on `ACTIONS_COMPRESS` even when nothing or a
    // non-local item is selected (`src/ui/window/keyboard/files.rs`).
    Outcome::Ran
}

fn open(app: &mut App, entries: Vec<FileEntry>) {
    let destination = entries[0]
        .location
        .parent()
        .or_else(|| app.browser.active_location())
        .unwrap_or_else(|| Location::local(launch::home_directory()));
    let compress = &mut app.compress;
    if let Some(previous) = &mut compress.form {
        previous.wipe_secrets();
    }
    compress.notice_open = false;
    compress.focus_target = None;
    compress.opened += 1;
    compress.form = Some(Form::new(entries, destination, compress.opened));
    compress.open = true;
}

fn cancel(app: &mut App) {
    let compress = &mut app.compress;
    compress.open = false;
    compress.notice_open = false;
    compress.focus_target = None;
    if let Some(form) = &mut compress.form {
        form.wipe_secrets();
    }
}

fn close_notice(app: &mut App) {
    app.compress.notice_open = false;
}

fn show_notice(app: &mut App, title: String, message: String) {
    let compress = &mut app.compress;
    compress.notices += 1;
    compress.notice = Some(Notice {
        title,
        message,
        generation: compress.notices,
    });
    compress.notice_open = true;
}

fn name_changed(app: &mut App) {
    if let Some(form) = &mut app.compress.form {
        form.name_error = name_field_error(&form.name.text, Naming::Single, false);
    }
}

fn choose_format(app: &mut App, format: ArchiveFormat) {
    let Some(form) = &mut app.compress.form else {
        return;
    };
    form.format = format;
    if !format.supports_password() {
        form.password_protected = false;
    }
}

fn set_password_protected(app: &mut App, protected: bool) {
    let Some(form) = &mut app.compress.form else {
        return;
    };
    if form.password_protected == protected {
        return;
    }
    form.password_protected = protected;
    if protected {
        form.password.focused = true;
        app.compress.focus_target = Some(view::password_key(form.generation));
    }
}

fn validated_password(form: &Form) -> Result<Option<String>, (&'static str, &'static str)> {
    if !form.shows_password() {
        return Ok(None);
    }
    if form.password.text.is_empty() {
        return Err((
            "Password required",
            "Enter a password or choose No password.",
        ));
    }
    if form.password.text != form.confirm.text {
        return Err((
            "Passwords do not match",
            "Please enter the same password in both fields.",
        ));
    }
    Ok(Some(form.password.text.clone()))
}

fn submit(app: &mut App) {
    let Some(form) = app.compress.form.as_mut() else {
        return;
    };
    if !app.compress.open {
        return;
    }
    let format = form.format;
    let archive_name = normalized_archive_name(&form.name.text, format);
    if let Err(message) = validate_basename(&archive_name) {
        form.name_error = Some(message);
        app.compress.focus_target = Some(view::name_key(form.generation));
        return;
    }
    let password = match validated_password(form) {
        Ok(password) => password,
        Err((title, message)) => {
            show_notice(app, tr(title), tr(message));
            return;
        }
    };
    let entries = form.entries.clone();
    let destination = form.destination.clone();
    form.wipe_secrets();
    app.compress.open = false;
    app.compress.notice_open = false;
    start_compression(app, entries, destination, (archive_name, format, password));
}

async fn archive_name_taken(destination: &Location, final_name: &str) -> bool {
    let Some(archive) = destination.child(OsStr::new(final_name)) else {
        return false;
    };
    matches!(
        probes::presence(&[archive], &gio::Cancellable::new())
            .await
            .as_slice(),
        [Presence::Present { .. }]
    )
}

fn start_compression(
    app: &mut App,
    entries: Vec<FileEntry>,
    destination: Location,
    (archive_name, format, password): (String, ArchiveFormat, Option<String>),
) {
    let later = app.compress.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let final_name = format!("{archive_name}.{}", format.extension());
        let taken = archive_name_taken(&destination, &final_name).await;
        later.then(move |app, tx| {
            // Compression conflict prompts (`ask_to_replace_archive`) belong to A13; stop safely
            // without overwriting when the target path already exists.
            if taken {
                return false;
            }
            app.compress.pending_destination = Some(destination.clone());
            app.compress.pending_select = None;
            shell::publish(app);
            app.browser.compress(
                entries,
                destination,
                archive_name,
                TransferConflict::FailIfExists,
                format,
                password,
            );
            shell::settle(app, tx);
            true
        });
    });
}

fn queue_archive_event(event: &BrowserEvent, heard: &mut crate::slots::Heard<'_, App>) {
    match event {
        BrowserEvent::ArchiveCompleted { select_name } => {
            let select_name = select_name.clone();
            heard.then(move |app, tx| on_archive_completed(app, select_name, tx));
        }
        BrowserEvent::EntriesInserted { depth, .. }
        | BrowserEvent::EntriesReplaced { depth, .. }
        | BrowserEvent::EntriesPublished { depth, .. }
        | BrowserEvent::EntriesSpliced { depth, .. }
        | BrowserEvent::LoadFinished { depth, .. }
        | BrowserEvent::ColumnReloaded { depth } => {
            let depth = *depth;
            heard.then(move |app, tx| reveal_pending_at(app, depth, tx));
        }
        BrowserEvent::OperationFailed { .. }
        | BrowserEvent::OperationCompletedWithErrors { .. }
        | BrowserEvent::OperationCancelled { .. }
        | BrowserEvent::Reset => {
            heard.then(|app, _| {
                app.compress.pending_destination = None;
                app.compress.pending_select = None;
                false
            });
        }
        _ => {}
    }
}

fn destination_depth(app: &App, destination: &Location) -> Option<usize> {
    if shows_columns(app) {
        app.browser.open_depth(destination)
    } else {
        app.browser
            .active_depth()
            .filter(|depth| app.browser.location_at(*depth).as_ref() == Some(destination))
    }
}

fn on_archive_completed(app: &mut App, select_name: String, tx: &mut Tx) -> bool {
    if select_name.is_empty() {
        app.compress.pending_destination = None;
        app.compress.pending_select = None;
        return false;
    }
    let Some(destination) = app.compress.pending_destination.clone() else {
        return false;
    };
    app.compress.pending_select = Some(select_name);
    match destination_depth(app, &destination) {
        Some(depth) => reveal_pending_at(app, depth, tx),
        None => {
            if app.browser.active_location().as_ref() == Some(&destination) {
                app.browser.reload_active();
            } else {
                app.browser.navigate(destination);
            }
            shell::settle(app, tx);
            true
        }
    }
}

fn reveal_pending_at(app: &mut App, depth: usize, tx: &mut Tx) -> bool {
    let Some(destination) = app.compress.pending_destination.as_ref() else {
        return false;
    };
    if app.browser.location_at(depth).as_ref() != Some(destination) {
        return false;
    }
    let Some(name) = app.compress.pending_select.clone() else {
        return false;
    };
    if !app
        .browser
        .select_entries_by_name_at(depth, std::slice::from_ref(&name))
    {
        return false;
    }
    if shows_columns(app) {
        app.browser.set_active_column(depth);
    }
    app.compress.pending_destination = None;
    app.compress.pending_select = None;
    shell::settle(app, tx);
    true
}

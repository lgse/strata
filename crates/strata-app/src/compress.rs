// SPDX-License-Identifier: MIT

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
    services::{Naming, TransferConflict, name_field_error, validate_basename},
};
use strata_kit::icons::IconName;

use crate::{
    App, archive,
    commands::{Action, Call, Holds, Keys, On, Outcome, Stage},
    conflict_dialog::{
        self, ConflictActions, ConflictChoice, ConflictFocus, ConflictPrompt, compact_display_path,
    },
    launch, menus, shell,
    slots::{Later, Part, Place, Setup},
    tenxer,
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
        }
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

fn target_entries(app: &mut App, on: &On) -> Vec<FileEntry> {
    match *on {
        On::Row((depth, id)) => menus::context_entries(app, depth, id)
            .map(|(_, entries)| entries)
            .unwrap_or_default(),
        On::Focus => tenxer::command_targets(app),
        On::Column(_) | On::Place(_) => Vec::new(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompressAttempt {
    Nothing,
    Refused,
    Started,
}

pub(crate) fn open_for_focus(app: &mut App) -> CompressAttempt {
    let entries = tenxer::command_targets(app);
    if entries.is_empty() {
        return CompressAttempt::Nothing;
    }
    if entries
        .iter()
        .any(|entry| entry.location.native_path().is_none())
    {
        return CompressAttempt::Refused;
    }
    open(app, entries);
    CompressAttempt::Started
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
    compress.opened += 1;
    compress.form = Some(Form::new(entries, destination, compress.opened));
    compress.open = true;
}

fn cancel(app: &mut App) {
    let compress = &mut app.compress;
    compress.open = false;
    compress.notice_open = false;
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

fn set_password_protected(app: &mut App, protected: bool, tx: &mut Tx) {
    let Some(form) = &mut app.compress.form else {
        return;
    };
    if form.password_protected == protected {
        return;
    }
    form.password_protected = protected;
    if protected {
        form.password.focused = true;
        tx.focus_when_shown(view::password_key(form.generation), false);
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

fn submit(app: &mut App, tx: &mut Tx) {
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
        let end = form.name.text.len();
        form.name.anchor = 0;
        form.name.focus = end;
        tx.focus(view::name_key(form.generation), false);
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

#[derive(Clone)]
struct PendingCompression {
    entries: Vec<FileEntry>,
    destination: Location,
    archive_name: String,
    format: ArchiveFormat,
    password: Option<String>,
}

fn start_compression(
    app: &mut App,
    entries: Vec<FileEntry>,
    destination: Location,
    (archive_name, format, password): (String, ArchiveFormat, Option<String>),
) {
    archive::clear_extract_destination(app);
    let pending = PendingCompression {
        entries,
        destination,
        archive_name,
        format,
        password,
    };
    let later = app.compress.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let final_name = format!("{}.{}", pending.archive_name, pending.format.extension());
        let taken = archive_name_taken(&pending.destination, &final_name).await;
        later.then(move |app, tx| {
            if taken {
                ask_to_replace_archive(app, pending, final_name);
                return true;
            }
            run_compression(app, pending, TransferConflict::FailIfExists);
            shell::settle(app, tx);
            true
        });
    });
}

fn run_compression(app: &mut App, pending: PendingCompression, conflict: TransferConflict) {
    archive::prepare_compression(app, pending.destination.clone());
    shell::publish(app);
    app.browser.compress(
        pending.entries,
        pending.destination,
        pending.archive_name,
        conflict,
        pending.format,
        pending.password,
    );
}

fn ask_to_replace_archive(app: &mut App, pending: PendingCompression, final_name: String) {
    let explanation = rust_i18n::t!(
        "An archive named “%{final_name}” already exists in %{value1}. Replace it to overwrite its contents, or keep both to create a numbered copy.",
        final_name = final_name,
        value1 = compact_display_path(&pending.destination)
    )
    .into_owned();
    let prompt = ConflictPrompt {
        name: final_name,
        explanation,
        apply_to_all_visible: false,
        skip_visible: false,
        actions: ConflictActions {
            replace: true,
            keep_both: true,
            merge: false,
            focus: ConflictFocus::Replace,
        },
    };
    conflict_dialog::prompt_with_icon(app, IconName::Archive, prompt, move |app, choice, _| {
        let conflict = match choice {
            ConflictChoice::Replace => TransferConflict::ReplaceExisting,
            ConflictChoice::KeepBoth => TransferConflict::KeepBoth,
            ConflictChoice::Merge | ConflictChoice::Skip => return,
        };
        run_compression(app, pending.clone(), conflict);
    });
}

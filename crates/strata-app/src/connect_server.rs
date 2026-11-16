// SPDX-License-Identifier: MIT

//! The Connect to server dialog (`src/ui/window/network_location.rs` and
//! `connect_typed_location` in `src/ui/browser/location.rs`), mounted on C15's
//! [`strata_components::dialog::StrataDialog`].
//!
//! - **Opening.** Runs as `"connect-to-server"` from the sidebar Network row's context menu
//!   (`crates/strata-app/src/sidebar/menu.rs`).
//! - **Connecting.** Validates and opens the typed address through
//!   `Browser::navigate_input_located`, sanitizing embedded URI credentials first
//!   (`sanitize_uri_credentials`). While an asynchronous check is in flight, the address field,
//!   **Pin to sidebar** checkbox and **Connect** button are disabled and the header spinner says
//!   **Connecting…** (`MIN_CONNECTING` = 1 s minimum on failures so an instant failure still
//!   reads as an attempt).
//! - **Completion.** When the target column opens (`BrowserEvent::ColumnAdded`), the dialog
//!   closes, focuses the active file view, and — when **Pin to sidebar** was checked — appends
//!   the normalized location to the shared bookmarks store (`bookmarks::update`). Asynchronous
//!   refusals (`BrowserEvent::LocationNavigationRejected`) are intercepted ahead of
//!   [`crate::open_failed`] via [`fail_pending`] so the error shows inline under **Server
//!   address** without opening a second modal.

mod view;

use std::path::PathBuf;
use std::time::Duration;

use gio::glib;
use rune::{El, TextEdit, Tx};
use strata_core::model::Location;
use strata_fs::{
    adapters::{bookmarks, gio_file_for_location, location_for_file},
    app::BrowserEvent,
    services::{LocationValidationError, sanitize_uri_credentials},
};

use crate::{
    App,
    commands::{Action, Call, Holds, Outcome},
    shell,
    slots::{Frame, Listener, Part, Place, Setup},
};

/// Minimum duration an asynchronous failure stays in **Connecting…** (`network_location.rs`).
pub(crate) const MIN_CONNECTING: Duration = Duration::from_secs(1);

enum Pending {
    Waiting {
        target: Location,
        pin: bool,
        started_at: Option<f64>,
    },
    Failed {
        message: String,
        started_at: Option<f64>,
    },
}

pub(super) struct Form {
    pub(super) generation: u64,
    pub(super) address: TextEdit,
    pub(super) pin: bool,
    pub(super) connecting: bool,
    pub(super) error: Option<String>,
    pub(super) attempt: u64,
    pending: Option<Pending>,
}

impl Form {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            address: TextEdit::new(""),
            pin: false,
            connecting: false,
            error: None,
            attempt: 0,
            pending: None,
        }
    }

    pub(super) fn blocks_backdrop(&self) -> bool {
        self.connecting || !self.address.text.trim().is_empty()
    }

    fn clear_address(&mut self) {
        self.address = TextEdit::new("");
    }
}

pub(crate) struct ConnectServer {
    opened: u64,
    open: bool,
    form: Option<Form>,
}

impl Part for ConnectServer {
    const PLACE: Place<App> = Place::Dialogs(build);

    fn new(_: &Setup<'_>) -> Self {
        Self {
            opened: 0,
            open: false,
            form: None,
        }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, _, heard| {
            if let BrowserEvent::ColumnAdded { location, .. } = event {
                let location = location.clone();
                heard.then(move |app, tx| settle_column(app, &location, tx));
            }
        }))
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        poll_pending(app, clock, tx)
    }

    fn keyboard(app: &App) -> Option<Holds> {
        let state = &app.connect_server;
        let form = state.form.as_ref().filter(|_| state.open)?;
        Some(Holds::Modal {
            field: form.address.focused && !form.connecting,
        })
    }

    const COMMANDS: &'static [Action<App>] = &[Action::new("connect-to-server", connect_to_server)];
}

fn poll_pending(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let Some(form) = app
        .connect_server
        .form
        .as_mut()
        .filter(|_| app.connect_server.open)
    else {
        return false;
    };
    match &mut form.pending {
        Some(Pending::Waiting { started_at, .. }) => {
            started_at.get_or_insert(clock);
            false
        }
        Some(Pending::Failed { started_at, .. }) => {
            let since = *started_at.get_or_insert(clock);
            let left = MIN_CONNECTING.as_secs_f64() - (clock - since);
            if left > 1e-6 {
                tx.wake_after(Duration::from_secs_f64(left));
                return false;
            }
            let Some(Pending::Failed { message, .. }) = form.pending.take() else {
                return false;
            };
            form.connecting = false;
            form.error = Some(message);
            form.attempt += 1;
            form.address.focused = true;
            tx.focus_when_shown(view::address_key(form.generation), false);
            true
        }
        None => false,
    }
}

fn connect_to_server(app: &mut App, _: &Call<'_>, _: &mut Tx) -> Outcome {
    if app
        .connect_server
        .form
        .as_ref()
        .is_some_and(|form| matches!(form.pending, Some(Pending::Waiting { .. })))
    {
        app.browser.bump_navigation_generation();
    }
    let state = &mut app.connect_server;
    state.opened += 1;
    state.form = Some(Form::new(state.opened));
    state.open = true;
    Outcome::Ran
}

/// Intercepts a `LocationNavigationRejected` raised by an in-flight Connect to server attempt
/// (`fail_pending_connect` in `src/ui/browser/location.rs`).
pub(crate) fn fail_pending(app: &mut App, error: &LocationValidationError) -> bool {
    let state = &mut app.connect_server;
    if !state.open {
        return false;
    }
    let Some(form) = state.form.as_mut() else {
        return false;
    };
    let Some(Pending::Waiting { started_at, .. }) = form.pending.take() else {
        return false;
    };
    form.pending = Some(Pending::Failed {
        message: error.message(),
        started_at,
    });
    true
}

fn same_location(a: &Location, b: &Location) -> bool {
    match (a.uri_value(), b.uri_value()) {
        (Some(a), Some(b)) => a.trim_end_matches('/') == b.trim_end_matches('/'),
        _ => a == b,
    }
}

fn target_reached(target: &Location, column: &Location) -> bool {
    same_location(target, column)
        || target
            .native_path()
            .zip(column.native_path())
            .is_some_and(|(target, column)| target.starts_with(column))
}

fn settle_column(app: &mut App, column: &Location, tx: &mut Tx) -> bool {
    let reached = app
        .connect_server
        .form
        .as_ref()
        .filter(|_| app.connect_server.open)
        .and_then(|form| match &form.pending {
            Some(Pending::Waiting { target, pin, .. }) if target_reached(target, column) => {
                Some(*pin)
            }
            _ => None,
        });
    let Some(pin) = reached else {
        return false;
    };
    complete_open(app, column.clone(), pin, tx);
    true
}

fn complete_open(app: &mut App, location: Location, pin: bool, tx: &mut Tx) {
    if pin {
        pin_location(location);
    }
    let view = shell::view_focus(app).map(str::to_owned);
    app.connect_server.open = false;
    if let Some(form) = app.connect_server.form.as_mut() {
        form.connecting = false;
        form.pending = None;
        form.clear_address();
    }
    if let Some(view) = view {
        tx.focus(view, true);
    }
}

fn sanitize_input(input: &str) -> Result<String, LocationValidationError> {
    if !input.contains("://") {
        return Ok(input.to_owned());
    }
    sanitize_uri_credentials(input).map(|(sanitized, _)| sanitized)
}

pub(super) fn clear_error(app: &mut App) {
    if let Some(form) = app.connect_server.form.as_mut() {
        form.error = None;
    }
}

pub(super) fn toggle_pin(app: &mut App) {
    if let Some(form) = app.connect_server.form.as_mut()
        && !form.connecting
    {
        form.pin = !form.pin;
    }
}

pub(super) fn cancel(app: &mut App) {
    let waiting = app
        .connect_server
        .form
        .as_ref()
        .is_some_and(|form| matches!(form.pending, Some(Pending::Waiting { .. })));
    if waiting {
        app.browser.bump_navigation_generation();
    }
    app.connect_server.open = false;
    if let Some(form) = app.connect_server.form.as_mut() {
        form.connecting = false;
        form.pending = None;
        form.clear_address();
    }
}

pub(super) fn submit(app: &mut App, tx: &mut Tx) {
    if !app.connect_server.open {
        return;
    }
    let Some((generation, input, pin)) = app.connect_server.form.as_mut().and_then(|form| {
        if form.connecting {
            return None;
        }
        form.error = None;
        Some((
            form.generation,
            form.address.text.trim().to_owned(),
            form.pin,
        ))
    }) else {
        return;
    };
    app.browser.bump_navigation_generation();
    shell::publish(app);
    let previous = app.browser.active_location();
    let browser = app.browser.clone();
    let result =
        sanitize_input(&input).and_then(|clean| browser.navigate_input_located(&clean, None));
    match result {
        Ok(target) => {
            let reopened = previous
                .as_ref()
                .is_some_and(|active| same_location(active, &target));
            if reopened {
                complete_open(app, target, pin, tx);
            } else if let Some(form) = app.connect_server.form.as_mut() {
                form.connecting = true;
                form.pending = Some(Pending::Waiting {
                    target,
                    pin,
                    started_at: None,
                });
            }
        }
        Err(error) => {
            if let Some(form) = app.connect_server.form.as_mut() {
                form.connecting = false;
                form.error = Some(error.message());
                form.attempt += 1;
                form.address.focused = true;
            }
            tx.focus_when_shown(view::address_key(generation), false);
        }
    }
}

fn pin_label(location: &Location) -> String {
    let name = location.display_name();
    if name.is_empty() {
        location.display_path()
    } else {
        name
    }
}

/// Canonicalizes a URI location through `gio::File` (`smb://host/share` -> `smb://host/share/`)
/// so Unpin matches the reloaded bookmark (`network_location.rs`).
fn normalized_location(location: Location) -> Location {
    location_for_file(&gio_file_for_location(&location)).unwrap_or(location)
}

fn is_standard_place(location: &Location) -> bool {
    let Some(path) = location.native_path() else {
        return false;
    };
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
    if path == home {
        return true;
    }
    [
        glib::UserDirectory::Desktop,
        glib::UserDirectory::Documents,
        glib::UserDirectory::Downloads,
        glib::UserDirectory::Music,
        glib::UserDirectory::Pictures,
        glib::UserDirectory::Videos,
    ]
    .into_iter()
    .filter_map(glib::user_special_dir)
    .any(|standard| standard == path)
}

fn pin_location(location: Location) {
    let location = normalized_location(location);
    if is_standard_place(&location) {
        return;
    }
    let label = pin_label(&location);
    let edit = move |contents: &[u8]| {
        let mut places = bookmarks::parse_pinned_places(contents);
        if places.iter().any(|(pinned, _)| *pinned == location) {
            return None;
        }
        places.push((location.clone(), label.clone()));
        Some(bookmarks::serialize_pinned_places(&places).into_bytes())
    };
    glib::MainContext::default().spawn_local(async move {
        if let Err(error) = bookmarks::update(edit).await {
            tracing::warn!(%error, "the pinned folders could not be saved");
        }
    });
}

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let state = &app.connect_server;
    if let Some(form) = &state.form {
        view::build_dialog(app, form, state.open, frame.layer);
    }
    None
}

#[cfg(test)]
mod tests;

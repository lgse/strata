// SPDX-License-Identifier: MIT

use crate::model::Location;
use crate::services::{LocationValidationError, UriCredentials, sanitize_uri_credentials};
use crate::ui::browser::ViewState;
use crate::ui::modal::{show_error_dialog, show_error_dialog_with_action};
use gtk::gio;
use gtk::prelude::*;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

mod breadcrumbs;
pub(super) mod completion;
mod devices;
mod dialogs;
mod mount_errors;
mod mounting;
mod unlock_progress;
mod volumes;
pub(super) use breadcrumbs::is_breadcrumb_button_target;
pub(super) use dialogs::MountCredentials;
pub(super) use mount_errors::MountStrategy;
pub(super) use unlock_progress::UnlockProgressSlot;
#[cfg(test)]
use {
    crate::ui::browser::BrowserView,
    devices::{
        DeviceKeys, UnlockViewFollowUp, identity_matches_volume, unlock_target_matches,
        unlock_view_follow_up, wait_for_mount_change,
    },
    dialogs::{
        MountDialogHandlers, MountPromptDetails, authentication_retry_message,
        dismiss_authentication_prompt, password_save_for_selection, show_authentication_dialog,
        show_trust_question_dialog,
    },
    futures_channel::oneshot,
    mount_errors::{
        device_volume_mount_is_ready, log_mount_finished, log_mount_started,
        mount_error_is_authentication_failure, mount_error_is_cancelled, mount_failure_message,
        trust_question_result, volume_error_is_authentication_failure,
        volume_error_is_in_flight_mount,
    },
    std::cell::Cell,
    unlock_progress::{UNLOCK_PROGRESS_DELAY, begin_unlock_slot, unlock_progress_dismissed_for},
};

fn credentials_from_location_input(
    input: &str,
) -> Result<(String, Option<MountCredentials>), LocationValidationError> {
    if !input.contains("://") {
        return Ok((input.to_owned(), None));
    }
    let (sanitized, credentials) = sanitize_uri_credentials(input)?;
    let credentials = credentials.map(|credentials: UriCredentials| MountCredentials {
        anonymous: false,
        username: credentials.username,
        domain: String::new(),
        password: credentials.password,
        save: gio::PasswordSave::Never,
    });
    Ok((sanitized, credentials))
}

/// Entry text to show while a typed location mounts: the sanitized URI in its decoded display
/// form, or `None` when the typed text needs no rewrite.
fn mounting_entry_text(input: &str, sanitized: &str) -> Option<String> {
    if sanitized == input {
        return None;
    }
    let displayed = Location::uri(sanitized).display_path();
    (displayed != input).then_some(displayed)
}

pub(super) enum TypedLocation {
    Navigating {
        location: Location,
    },
    Mounting {
        location: Location,
        sanitized: String,
    },
}

/// How a typed location a dialog waits on turned out.
pub(in crate::ui) enum ConnectOutcome {
    Opened(Location),
    /// The message to show, or `None` when the user cancelled authentication.
    Failed(Option<String>),
}

/// A typed location a dialog waits on until it opens or fails.
pub(in crate::ui) struct PendingConnect {
    /// Unknown until the typed input resolves; remote targets open asynchronously.
    target: RefCell<Option<Location>>,
    cancellable: gio::Cancellable,
    finished: Box<dyn Fn(ConnectOutcome)>,
}

/// Remote roots differ only by a trailing slash depending on who built the URI.
fn same_location(a: &Location, b: &Location) -> bool {
    match (a.uri_value(), b.uri_value()) {
        (Some(a), Some(b)) => a.trim_end_matches('/') == b.trim_end_matches('/'),
        _ => a == b,
    }
}

impl ViewState {
    /// The entry's focus sits on its inner text widget, so the root's focus
    /// widget is checked rather than the entry itself.
    pub(super) fn location_entry_has_focus(&self) -> bool {
        let entry = self.location_entry.upcast_ref::<gtk::Widget>();
        self.location_entry.has_focus()
            || self
                .overlay
                .root()
                .and_then(|root| root.focus())
                .as_ref()
                .is_some_and(|focused| focused == entry || focused.is_ancestor(entry))
    }

    pub(super) fn begin_location_edit(&self) {
        self.location_stack.set_visible_child_name("entry");
        self.location_entry.grab_focus();
        self.location_entry.select_region(0, -1);
        self.path_completion
            .refresh(&self.location_entry, &self.browser);
    }

    fn show_breadcrumbs(&self) {
        self.breadcrumb_reveal_pending.set(true);
        self.location_stack.set_visible_child_name("breadcrumbs");
    }

    pub(super) fn cancel_location_edit(&self) {
        self.path_completion.dismiss();
        // Resetting a visible entry emits changed and reopens its completion popover.
        self.show_breadcrumbs();
        self.restore_location_text();
        self.browser.focus_active();
    }

    pub(super) fn submit_location(self: &Rc<Self>) {
        self.path_completion.dismiss();
        let input = self.location_entry.text();
        match self.open_typed_location(input.as_str(), None) {
            Ok(TypedLocation::Navigating { .. }) => {
                self.show_breadcrumbs();
                self.browser.focus_active();
            }
            Ok(TypedLocation::Mounting { sanitized, .. }) => {
                if let Some(text) = mounting_entry_text(input.as_str(), &sanitized) {
                    self.location_entry.set_text(&text);
                }
            }
            Err(error) => {
                self.show_breadcrumbs();
                self.restore_location_text();
                show_error_dialog(
                    &self.overlay,
                    &crate::i18n::tr("Unable to open location"),
                    &error.message(),
                );
            }
        }
    }

    /// Navigates to typed `input`, mounting first when the location needs it.
    /// Credentials embedded in a URI move into the mount operation and are
    /// never kept with the text. A relative path resolves against `base`.
    pub(super) fn open_typed_location(
        self: &Rc<Self>,
        input: &str,
        base: Option<&Path>,
    ) -> Result<TypedLocation, LocationValidationError> {
        let (input, credentials) = credentials_from_location_input(input)?;
        self.pending_location_credentials.replace(credentials);
        match self.browser.navigate_input_located(&input, base) {
            Ok(location) => Ok(TypedLocation::Navigating { location }),
            Err(LocationValidationError::NotMounted(location)) => {
                let credentials = self.pending_location_credentials.take();
                self.mount_then_navigate_with_credentials(
                    location.clone(),
                    MountStrategy::EnclosingVolume,
                    credentials,
                );
                Ok(TypedLocation::Mounting {
                    location,
                    sanitized: input,
                })
            }
            Err(LocationValidationError::Mountable(location)) => {
                let credentials = self.pending_location_credentials.take();
                self.mount_then_navigate_with_credentials(
                    location.clone(),
                    MountStrategy::Mountable,
                    credentials,
                );
                Ok(TypedLocation::Mounting {
                    location,
                    sanitized: input,
                })
            }
            Err(error) => {
                self.pending_location_credentials.take();
                Err(error)
            }
        }
    }

    pub(super) fn connect_typed_location(
        self: &Rc<Self>,
        input: &str,
        base: Option<&Path>,
        finished: Box<dyn Fn(ConnectOutcome)>,
    ) -> Result<(), LocationValidationError> {
        self.cancel_connect();
        let connect = Rc::new(PendingConnect {
            target: RefCell::new(None),
            cancellable: gio::Cancellable::new(),
            finished,
        });
        // Installed first: a native volume starts mounting before this returns.
        self.pending_connect.replace(Some(connect.clone()));
        match self.open_typed_location(input, base) {
            Ok(TypedLocation::Navigating { location }) => {
                let reopened = self
                    .browser
                    .active_location()
                    .is_some_and(|active| same_location(&active, &location));
                if reopened {
                    self.settle_connect(&connect, ConnectOutcome::Opened(location));
                } else {
                    connect.target.replace(Some(location));
                }
                Ok(())
            }
            Ok(TypedLocation::Mounting { location, .. }) => {
                connect.target.replace(Some(location));
                Ok(())
            }
            Err(error) => {
                self.pending_connect.take();
                Err(error)
            }
        }
    }

    /// Abandons the pending connect so neither its validation nor its mount navigates.
    pub(super) fn cancel_connect(&self) {
        let Some(connect) = self.pending_connect.take() else {
            return;
        };
        connect.cancellable.cancel();
        self.pending_location_credentials.take();
        self.browser.bump_navigation_generation();
    }

    /// Reports `outcome` when `connect` is still the pending one; false otherwise.
    fn settle_connect(&self, connect: &Rc<PendingConnect>, outcome: ConnectOutcome) -> bool {
        if !self.is_pending_connect(connect) {
            return false;
        }
        self.pending_connect.take();
        (connect.finished)(outcome);
        true
    }

    fn is_pending_connect(&self, connect: &Rc<PendingConnect>) -> bool {
        self.pending_connect
            .borrow()
            .as_ref()
            .is_some_and(|pending| Rc::ptr_eq(pending, connect))
    }

    /// Reports a failure to the pending connect; false when none is waiting.
    pub(super) fn fail_pending_connect(&self, message: Option<String>) -> bool {
        let connect = self.pending_connect.borrow().clone();
        connect
            .is_some_and(|connect| self.settle_connect(&connect, ConnectOutcome::Failed(message)))
    }

    /// Opens the pending connect once its validated target becomes a column. A
    /// native target was validated before any of its columns appear, so the
    /// first ancestor column settles it too (a typed file reveals its folder).
    pub(super) fn settle_connect_column(&self, location: &Location) {
        let connect = self.pending_connect.borrow().clone();
        if let Some(connect) = connect {
            let reached = connect.target.borrow().as_ref().is_some_and(|target| {
                same_location(target, location)
                    || target
                        .native_path()
                        .zip(location.native_path())
                        .is_some_and(|(target, column)| target.starts_with(column))
            });
            if reached {
                self.settle_connect(&connect, ConnectOutcome::Opened(location.clone()));
            }
        }
    }

    pub(super) fn handle_navigation_rejected(
        self: &Rc<Self>,
        parent_depth: usize,
        rejected: &Location,
        error: LocationValidationError,
    ) {
        match error {
            LocationValidationError::NotMounted(location) => {
                self.mount_then_descend(parent_depth, location, MountStrategy::EnclosingVolume);
            }
            LocationValidationError::Mountable(location) => {
                self.mount_then_descend(parent_depth, location, MountStrategy::Mountable);
            }
            LocationValidationError::NotAuthorized => {}
            error => {
                self.show_open_failure(
                    &crate::i18n::tr("Unable to open directory"),
                    rejected,
                    &error,
                );
            }
        }
    }

    /// Reports why `location` can't be opened, offering `admin://` when only
    /// permissions stand in the way.
    pub(super) fn show_open_failure(
        self: &Rc<Self>,
        title: &str,
        location: &Location,
        error: &LocationValidationError,
    ) {
        let Some(administrator) = matches!(error, LocationValidationError::Inaccessible)
            .then(|| self.browser.administrator_alternative(location))
            .flatten()
        else {
            show_error_dialog(&self.overlay, title, &error.message());
            return;
        };
        let weak = Rc::downgrade(self);
        show_error_dialog_with_action(
            &self.overlay,
            title,
            &error.message(),
            (
                crate::assets::icons::LOCK,
                &crate::i18n::tr("Open as Administrator"),
            ),
            Rc::new(move || {
                if let Some(state) = weak.upgrade() {
                    state.open_as_administrator(administrator.clone());
                }
            }),
        );
    }

    /// Validation runs the first `admin://` operation, so polkit prompts before the
    /// current folder is replaced, and a dismissed prompt leaves it open.
    pub(super) fn open_as_administrator(self: &Rc<Self>, location: Location) {
        self.browser.navigate_location(location, true);
    }

    fn restore_location_text(&self) {
        if let Some(location) = self.browser.active_location() {
            self.location_entry.set_text(&location.display_path());
        }
    }
}

#[cfg(test)]
mod tests;

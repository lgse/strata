// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

use strata_components::mount_prompts::{MountPromptState, MountRequest, MountResponse};
use strata_core::i18n::tr;
use strata_kit::icons::IconName;

use crate::{
    App,
    commands::Holds,
    slots::{Part, Place, Scope, Setup},
};

pub(crate) mod operation;
mod view;

#[cfg(test)]
mod tests;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// One pending prompt request answered once through [`MountPromptsPart`].
pub(crate) struct PromptTicket {
    pub(crate) id: u64,
    withdrawn: Rc<Cell<bool>>,
    reply: Option<Box<dyn FnOnce(MountResponse)>>,
}

impl PromptTicket {
    pub(crate) fn new(
        id: u64,
        withdrawn: Rc<Cell<bool>>,
        reply: impl FnOnce(MountResponse) + 'static,
    ) -> Self {
        Self {
            id,
            withdrawn,
            reply: Some(Box::new(reply)),
        }
    }

    pub(crate) fn answer(mut self, response: MountResponse) {
        if self.withdrawn.get() {
            let _ = self.reply.take();
            return;
        }
        if let Some(reply) = self.reply.take() {
            reply(response);
        }
    }
}

impl Drop for PromptTicket {
    fn drop(&mut self) {
        if !self.withdrawn.get()
            && let Some(reply) = self.reply.take()
        {
            reply(MountResponse::Abort);
        }
    }
}

/// Confirmation before deleting a cached LUKS passphrase from Secret Service and locking.
pub(crate) struct ForgetPasswordDialog {
    pub(crate) volume_name: String,
    on_confirm: Option<Box<dyn FnOnce()>>,
    on_cancel: Option<Box<dyn FnOnce()>>,
}

impl Drop for ForgetPasswordDialog {
    fn drop(&mut self) {
        if let Some(cancel) = self.on_cancel.take() {
            cancel();
        }
    }
}

/// Compact progress modal shown while unlocking an encrypted volume or password drive.
pub(crate) struct UnlockProgressOverlay {
    pub(crate) id: u64,
    pub(crate) volume_name: String,
    on_hide: Option<Box<dyn FnOnce()>>,
}

pub(crate) struct ReleaseProgressOverlay {
    pub(crate) id: u64,
    pub(crate) title: String,
    pub(crate) subtitle: String,
    pub(crate) body: String,
    pub(crate) icon: IconName,
    pub(crate) safe_to_remove: bool,
    on_hide: Option<Box<dyn FnOnce()>>,
}

/// Window-scoped state for mount authentication prompts and device progress/confirmation modals.
#[derive(Default)]
pub(crate) struct MountPromptsPart {
    pub(super) prompt: MountPromptState<PromptTicket>,
    pub(super) forget_password: Option<ForgetPasswordDialog>,
    pub(super) unlock_progress: Option<UnlockProgressOverlay>,
    pub(super) release_overlay: Option<ReleaseProgressOverlay>,
}

impl Part for MountPromptsPart {
    const PLACE: Place<App> = Place::Dialogs(view::build);
    const SCOPE: Scope = Scope::Window;

    fn new(_: &Setup<'_>) -> Self {
        Self::default()
    }

    fn keyboard(app: &App) -> Option<Holds> {
        let state = &app.mount_prompts;
        if state.prompt.is_open() {
            return Some(Holds::Modal {
                field: state.prompt.password_form().is_some(),
            });
        }
        if state.forget_password.is_some()
            || state.unlock_progress.is_some()
            || state.release_overlay.is_some()
        {
            return Some(Holds::Modal { field: false });
        }
        None
    }
}

/// Opens a C26 mount prompt (`ask-password` or `ask-question`). Any replaced prompt aborts.
pub(crate) fn open_prompt(app: &mut App, request: MountRequest, ticket: PromptTicket) {
    if ticket.withdrawn.get() {
        return;
    }
    if let Some(old) = app.mount_prompts.prompt.open(request, ticket) {
        old.answer(MountResponse::Abort);
    }
}

/// Closes the active C26 mount prompt without replying when GIO aborts or finishes its operation.
pub(crate) fn withdraw_prompt(app: &mut App, id: u64) {
    let matches = app
        .mount_prompts
        .prompt
        .ticket()
        .is_some_and(|ticket| ticket.id == id);
    if matches && let Some(ticket) = app.mount_prompts.prompt.withdraw() {
        ticket.withdrawn.set(true);
    }
}

/// Shows the "Forget saved password?" confirmation dialog before locking an encrypted volume.
pub(crate) fn show_forget_password(
    app: &mut App,
    volume_name: String,
    on_confirm: impl FnOnce() + 'static,
    on_cancel: impl FnOnce() + 'static,
) {
    app.mount_prompts.forget_password = Some(ForgetPasswordDialog {
        volume_name,
        on_confirm: Some(Box::new(on_confirm)),
        on_cancel: Some(Box::new(on_cancel)),
    });
}

pub(super) fn confirm_forget_password(app: &mut App) {
    let Some(mut dialog) = app.mount_prompts.forget_password.take() else {
        return;
    };
    let _ = dialog.on_cancel.take();
    if let Some(confirm) = dialog.on_confirm.take() {
        confirm();
    }
}

pub(super) fn cancel_forget_password(app: &mut App) {
    // Dropping the taken dialog runs `on_cancel` once.
    let _ = app.mount_prompts.forget_password.take();
}

/// Presents the compact "Unlocking volume" progress modal.
pub(crate) fn show_unlock_progress(
    app: &mut App,
    id: u64,
    volume_name: String,
    on_hide: impl FnOnce() + 'static,
) {
    app.mount_prompts.unlock_progress = Some(UnlockProgressOverlay {
        id,
        volume_name,
        on_hide: Some(Box::new(on_hide)),
    });
}

/// Dismisses the "Unlocking volume" progress modal if it belongs to `id`.
pub(crate) fn dismiss_unlock_progress(app: &mut App, id: u64) {
    let matches = app
        .mount_prompts
        .unlock_progress
        .as_ref()
        .is_some_and(|overlay| overlay.id == id);
    if matches {
        app.mount_prompts.unlock_progress = None;
    }
}

pub(super) fn hide_unlock_progress(app: &mut App) {
    let Some(mut overlay) = app.mount_prompts.unlock_progress.take() else {
        return;
    };
    if let Some(on_hide) = overlay.on_hide.take() {
        on_hide();
    }
}

/// Presents the compact release progress modal ("Unmounting" / "Ejecting" / "Locking volume").
pub(crate) fn show_release_overlay(
    app: &mut App,
    overlay: ReleaseOverlaySpec,
    on_hide: impl FnOnce() + 'static,
) {
    app.mount_prompts.release_overlay = Some(ReleaseProgressOverlay {
        id: overlay.id,
        title: overlay.title,
        subtitle: overlay.subtitle,
        body: overlay.body,
        icon: overlay.icon,
        safe_to_remove: false,
        on_hide: Some(Box::new(on_hide)),
    });
}

/// What a newly presented release progress modal displays.
pub(crate) struct ReleaseOverlaySpec {
    pub(crate) id: u64,
    pub(crate) title: String,
    pub(crate) subtitle: String,
    pub(crate) body: String,
    pub(crate) icon: IconName,
}

/// Updates the body text of an open release progress modal when GIO reports unmount progress.
pub(crate) fn update_release_overlay_body(app: &mut App, id: u64, body: String) {
    if let Some(overlay) = app.mount_prompts.release_overlay.as_mut()
        && overlay.id == id
        && !overlay.safe_to_remove
    {
        overlay.body = body;
    }
}

pub(crate) fn show_release_safe_to_remove(app: &mut App, id: u64, display_name: &str) -> bool {
    let Some(overlay) = app.mount_prompts.release_overlay.as_mut() else {
        return false;
    };
    if overlay.id != id {
        return false;
    }
    overlay.safe_to_remove = true;
    overlay.title = tr("Safe to remove");
    overlay.body = rust_i18n::t!(
        "%{display_name} can be unplugged.",
        display_name = display_name
    )
    .into_owned();
    let _ = overlay.on_hide.take();
    true
}

/// Dismisses a release progress modal if it belongs to `id` (and has not turned into "Safe to remove").
pub(crate) fn dismiss_release_overlay(app: &mut App, id: u64) {
    let matches = app
        .mount_prompts
        .release_overlay
        .as_ref()
        .is_some_and(|overlay| overlay.id == id);
    if matches {
        app.mount_prompts.release_overlay = None;
    }
}

pub(super) fn dismiss_or_hide_release_overlay(app: &mut App) {
    let Some(mut overlay) = app.mount_prompts.release_overlay.take() else {
        return;
    };
    if !overlay.safe_to_remove
        && let Some(on_hide) = overlay.on_hide.take()
    {
        on_hide();
    }
}

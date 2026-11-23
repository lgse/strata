// SPDX-License-Identifier: MIT

use rune::El;
use strata_components::{
    dialog::{Focus, MESSAGE_WIDTH, StrataDialog, Tone},
    mount_prompts::{MountLabels, MountPrompts},
};
use strata_core::i18n::tr;
use strata_kit::icons::IconName;

use super::{
    ForgetPasswordDialog, PromptTicket, ReleaseProgressOverlay, UnlockProgressOverlay,
    cancel_forget_password, confirm_forget_password, dismiss_or_hide_release_overlay,
    hide_unlock_progress,
};
use crate::{App, slots::Frame};

/// Strata's `.action-dialog.compact` width (`strata_components::dialog` docs: 440 pt).
const COMPACT_WIDTH: f32 = 440.0;

pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let state = &app.mount_prompts;
    if let Some(overlay) = state.unlock_progress.as_ref() {
        build_unlock_progress(app, overlay, frame.layer);
    }
    if let Some(overlay) = state.release_overlay.as_ref() {
        build_release_overlay(app, overlay, frame.layer);
    }
    if let Some(dialog) = state.forget_password.as_ref() {
        build_forget_password(app, dialog, frame.layer);
    }
    let stacked = state.unlock_progress.is_some()
        || state.release_overlay.is_some()
        || state.forget_password.is_some();
    MountPrompts::new(
        "mount",
        |app: &App| &app.mount_prompts.prompt,
        |app: &mut App| &mut app.mount_prompts.prompt,
        |_app: &mut App, ticket: PromptTicket, answer| ticket.answer(answer),
    )
    .labels(translated_labels())
    .stacked(stacked)
    .build(&app.look, app, frame.layer);
    None
}

fn translated_labels() -> MountLabels {
    let defaults = MountLabels::default();
    MountLabels {
        sign_in_title: tr("Authentication required"),
        passphrase_title: tr("Passphrase required"),
        sign_in_subtitle: tr("Authenticate to access this volume or location"),
        connect: tr("Connect"),
        cancel: tr("Cancel"),
        username: tr("Username"),
        domain: tr("Domain"),
        password: tr("Password"),
        passphrase: tr("Passphrase"),
        connect_as: tr("Connect as"),
        registered: tr("Registered user"),
        anonymous: tr("Anonymous"),
        storage: tr("Password storage"),
        never: tr("Don't remember"),
        session: tr("Until logout"),
        forever: tr("Forever"),
        show_password: tr("Show password"),
        hide_password: tr("Hide password"),
        show_passphrase: tr("Show password"),
        hide_passphrase: tr("Hide password"),
        host_key_title: tr("Verify server identity"),
        host_key_subtitle: tr(
            "Only continue after verifying the SSH host key through a trusted source.",
        ),
        host_key_warning: tr(
            "If the server did not provide a fingerprint, verify it outside Strata before continuing.",
        ),
        close: tr("Close dialog"),
        ..defaults
    }
}

fn build_forget_password(app: &App, dialog: &ForgetPasswordDialog, layer: &mut Vec<El>) {
    StrataDialog::new(
        "forget-saved-password",
        tr("Forget saved password?"),
        cancel_forget_password,
    )
    .open(true)
    .subtitle(dialog.volume_name.clone())
    .icon(IconName::Lock)
    .tone(Tone::Danger)
    .width(MESSAGE_WIDTH)
    .blocking(true)
    .message(tr(
        "This volume’s password is saved. Locking will forget it, and you’ll need to enter it again to unlock.",
    ))
    .secondary(tr("Cancel"), cancel_forget_password)
    .primary(tr("Forget and lock"), confirm_forget_password)
    .initial_focus(Focus::Primary)
    .close_label(tr("Close dialog"))
    .build(&app.look, layer);
}

fn build_unlock_progress(app: &App, overlay: &UnlockProgressOverlay, layer: &mut Vec<El>) {
    let title = tr("Unlocking volume");
    StrataDialog::new("unlock-progress", title.clone(), hide_unlock_progress)
        .open(true)
        .subtitle(overlay.volume_name.clone())
        .icon(IconName::Lock)
        .width(COMPACT_WIDTH)
        .blocking(true)
        .loading(Some(title))
        .message(tr(
            "You can hide this and keep working. Unlocking will continue in the background.",
        ))
        .primary(tr("Hide"), hide_unlock_progress)
        .initial_focus(Focus::Primary)
        .close_label(tr("Close dialog"))
        .build(&app.look, layer);
}

fn build_release_overlay(app: &App, overlay: &ReleaseProgressOverlay, layer: &mut Vec<El>) {
    let primary_label = if overlay.safe_to_remove {
        tr("Close")
    } else {
        tr("Hide")
    };
    let loading = (!overlay.safe_to_remove).then(|| overlay.title.clone());
    StrataDialog::new(
        "device-release",
        overlay.title.clone(),
        dismiss_or_hide_release_overlay,
    )
    .open(true)
    .subtitle(overlay.subtitle.clone())
    .icon(overlay.icon)
    .width(COMPACT_WIDTH)
    .blocking(!overlay.safe_to_remove)
    .loading(loading)
    .message(overlay.body.clone())
    .primary(primary_label, dismiss_or_hide_release_overlay)
    .initial_focus(Focus::Primary)
    .close_label(tr("Close dialog"))
    .build(&app.look, layer);
}

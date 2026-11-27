// SPDX-License-Identifier: MIT

use rune::{El, Tx};
use strata_components::dialog::{Focus, MESSAGE_WIDTH, StrataDialog, Tone};
use strata_core::{
    i18n::{count, tr},
    model::ActionDefinition,
};
use strata_fs::services::JobRequest;

use super::{icons::action_icon, jobs};
use crate::{
    App,
    error_dialog::{self, Error},
    shell,
    slots::Frame,
};

pub(super) struct ConfirmSpec {
    pub(super) request: JobRequest,
    pub(super) generation: u64,
}

pub(crate) fn run_action(app: &mut App, request: JobRequest) {
    let registry = super::shared_registry(app);
    let current = registry.definition(request.action.id().to_owned());
    let later = app.custom_actions.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let checked = current.await.map_err(|error| error.user_message());
        later.then(move |app, tx| finish_verification(app, tx, request, checked));
    });
}

fn finish_verification(
    app: &mut App,
    tx: &mut Tx,
    request: JobRequest,
    checked: Result<ActionDefinition, String>,
) -> bool {
    let refused = match checked {
        Ok(definition) if definition == request.action.definition => None,
        Ok(_) => Some(tr("Actions changed")),
        Err(reason) => Some(reason),
    };
    if let Some(reason) = refused {
        error_dialog::show(app, Error::new(tr("Unable to run action"), reason));
        return true;
    }
    if request.action.definition.run.confirm {
        open_confirm(app, request);
        return true;
    }
    jobs::enqueue_and_present(app, request, tx);
    true
}

fn open_confirm(app: &mut App, request: JobRequest) {
    let state = &mut app.custom_actions;
    state.confirm_generation = state.confirm_generation.wrapping_add(1);
    state.confirm = Some(ConfirmSpec {
        request,
        generation: state.confirm_generation,
    });
    state.confirm_open = true;
}

fn cancelled(app: &mut App) {
    app.custom_actions.confirm_open = false;
}

fn confirmed(app: &mut App, tx: &mut Tx) {
    if !app.custom_actions.confirm_open {
        return;
    }
    app.custom_actions.confirm_open = false;
    let Some(request) = app
        .custom_actions
        .confirm
        .as_ref()
        .map(|spec| spec.request.clone())
    else {
        return;
    };
    jobs::enqueue_and_present(app, request, tx);
}

pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let spec = app.custom_actions.confirm.as_ref()?;
    let action = &spec.request.action;
    let summary = rust_i18n::t!(
        "This runs “%{value1}” on %{value2}.",
        value1 = action.name(),
        value2 = count("selected_items", spec.request.inputs.len())
    );
    let mut dialog = StrataDialog::new(
        format!("custom-action-confirm-{}", spec.generation),
        tr("Run this action?"),
        cancelled,
    )
    .open(app.custom_actions.confirm_open)
    .subtitle(action.name().to_owned())
    .icon(action_icon(action.definition.icon.as_deref()))
    .tone(Tone::Danger)
    .message(summary.into_owned())
    .secondary(tr("Cancel"), cancelled)
    .primary_tx(tr("Run"), confirmed)
    .initial_focus(Focus::Primary)
    .default_button(false)
    .close_label(tr("Close dialog"))
    .width(MESSAGE_WIDTH);
    if let Some(description) = action.definition.description.as_deref() {
        dialog = dialog.message(description.to_owned());
    }
    if let Some(view) = shell::view_focus(app) {
        dialog = dialog.return_focus(view);
    }
    dialog.build(&app.look, frame.layer);
    None
}

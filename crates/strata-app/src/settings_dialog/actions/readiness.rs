// SPDX-License-Identifier: MIT

//! Async runtime interpreter readiness check and status badge for the Script tab,
//! matching `src/ui/settings/actions/readiness.rs`.

use rune::{El, icon, row, text};
use strata_core::{
    i18n::tr,
    model::{ActionRuntime, ExecutionMode},
};
use strata_fs::adapters::resolve_action_executable;
use strata_kit::{icons::IconName, look::Look};

use super::editor::draft_definition;
use crate::{App, slots::WeakLater};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ReadinessStatus {
    Checking,
    Available { text: String, detail: String },
    Unavailable { text: String, detail: String },
}

pub(super) struct ReadinessState {
    key: Option<(ActionRuntime, Result<String, String>)>,
    request: u64,
    pub(super) status: ReadinessStatus,
}

impl Default for ReadinessState {
    fn default() -> Self {
        Self {
            key: None,
            request: 0,
            status: ReadinessStatus::Checking,
        }
    }
}

pub(super) struct RefreshRequest<'a> {
    pub(super) runtime: ActionRuntime,
    pub(super) source: &'a str,
    pub(super) later: WeakLater<App>,
    pub(super) editor_gen: u64,
    pub(super) force: bool,
}

impl ReadinessState {
    pub(super) fn refresh(&mut self, req: RefreshRequest<'_>) {
        if req.runtime == ActionRuntime::Command {
            return;
        }
        let target = executable(req.runtime, first_line_prefix(req.source));
        let key = (req.runtime, target.clone());
        // Editing the body must not repeatedly stat every directory on PATH.
        if !req.force && self.key.as_ref() == Some(&key) {
            return;
        }
        self.key = Some(key);
        self.request = self.request.wrapping_add(1);
        let request = self.request;
        let target = match target {
            Ok(target) => target,
            Err(error) => {
                self.status = ReadinessStatus::Unavailable {
                    text: tr("Invalid interpreter — cannot run"),
                    detail: error,
                };
                return;
            }
        };
        let runtime = req.runtime;
        let later = req.later;
        let editor_gen = req.editor_gen;
        glib::MainContext::default().spawn_local(async move {
            let lookup_target = target.clone();
            let Ok(found) =
                gio::spawn_blocking(move || resolve_action_executable(&lookup_target)).await
            else {
                return;
            };
            let status = status_from_lookup(runtime, &target, found);
            let Some(later) = later.upgrade() else {
                return;
            };
            later.then(move |app, _| {
                let Some(editor) = app.settings_dialog.actions.editor.as_mut() else {
                    return false;
                };
                if editor.generation != editor_gen || editor.readiness.request != request {
                    return false;
                }
                editor.readiness.status = status;
                true
            });
        });
    }
}

fn first_line_prefix(source: &str) -> &str {
    let line = source.split('\n').next().unwrap_or("");
    match line.char_indices().nth(4096) {
        Some((idx, _)) => &line[..idx],
        None => line,
    }
}

pub(super) fn executable(runtime: ActionRuntime, source: &str) -> Result<String, String> {
    let definition = draft_definition(runtime, ExecutionMode::WholeSelection);
    definition
        .interpreter_for_source(source)
        .map(|declared| {
            declared
                .map(|interpreter| interpreter.program)
                .unwrap_or_else(|| {
                    runtime
                        .default_interpreter()
                        .expect("script runtime")
                        .to_owned()
                })
        })
        .map_err(|error| error.user_message())
}

fn status_from_lookup(
    runtime: ActionRuntime,
    target: &str,
    found: Option<std::path::PathBuf>,
) -> ReadinessStatus {
    match found {
        Some(path) => ReadinessStatus::Available {
            text: rust_i18n::t!("%{runtime} available", runtime = runtime.label()).into_owned(),
            detail: rust_i18n::t!(
                "Executable: %{value1}\nInterpreter availability only. Script syntax and external dependencies are not checked. Nothing is executed by this check.",
                value1 = path.display()
            )
            .into_owned(),
        },
        None => ReadinessStatus::Unavailable {
            text: rust_i18n::t!(
                "%{runtime} not found — cannot run",
                runtime = runtime.label()
            )
            .into_owned(),
            detail: rust_i18n::t!(
                "Executable not found: %{target}. Install it and reopen this dialog, or choose an installed executable. You can still save this draft.",
                target = target
            )
            .into_owned(),
        },
    }
}

pub(super) fn indicator(look: &Look, state: &ReadinessState) -> Option<El> {
    let kit = &look.kit;
    let c = &kit.c;
    let (glyph, icon_color, text_color, label, detail) = match &state.status {
        ReadinessStatus::Checking => return None,
        ReadinessStatus::Available { text, detail } => (
            IconName::Success,
            c.icon_accent,
            c.text_accent,
            text.clone(),
            detail.clone(),
        ),
        ReadinessStatus::Unavailable { text, detail } => (
            IconName::Warning,
            c.danger.icon,
            c.danger.fg,
            text.clone(),
            detail.clone(),
        ),
    };
    Some(
        row()
            .key("settings-action-readiness")
            .gap(6.0)
            .items_center()
            .help(detail.clone())
            .child(icon(look.icon(glyph), 15.0).color(icon_color).a11y_hidden())
            .child(
                text(label.clone())
                    .label(label)
                    .description(detail)
                    .font(kit.ty.small)
                    .color(text_color),
            ),
    )
}

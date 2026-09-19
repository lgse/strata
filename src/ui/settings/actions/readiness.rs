// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use crate::{
    assets,
    model::{ActionRuntime, ExecutionMode},
};
use gtk::prelude::*;

pub(super) fn indicator(
    runtime: Rc<Cell<ActionRuntime>>,
    choices: &[gtk::ToggleButton],
    python: &sourceview5::Buffer,
    bash: &sourceview5::Buffer,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("action-runtime-status");
    row.set_valign(gtk::Align::Center);
    let icons = gtk::Stack::new();
    icons.add_named(
        &assets::primary_icon(assets::icons::CIRCLE_CHECK, 15),
        Some("available"),
    );
    icons.add_named(
        &assets::danger_icon(assets::icons::TRIANGLE_ALERT, 15),
        Some("unavailable"),
    );
    row.append(&icons);
    let label = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .build();
    row.append(&label);

    let update: Rc<dyn Fn(bool)> = Rc::new({
        let row = row.downgrade();
        let label = label.downgrade();
        let icons = icons.downgrade();
        let python = python.downgrade();
        let bash = bash.downgrade();
        let previous = RefCell::new(None);
        move |force| {
            let (Some(row), Some(label), Some(icons), Some(python), Some(bash)) = (
                row.upgrade(),
                label.upgrade(),
                icons.upgrade(),
                python.upgrade(),
                bash.upgrade(),
            ) else {
                return;
            };
            let runtime = runtime.get();
            if runtime == ActionRuntime::Command {
                return;
            }
            let buffer = if runtime == ActionRuntime::Bash {
                &bash
            } else {
                &python
            };
            let start = buffer.start_iter();
            let mut end = start;
            end.forward_to_line_end();
            if end.offset() > 4096 {
                end = buffer.iter_at_offset(4096);
            }
            let source = buffer.text(&start, &end, false);
            let target = executable(runtime, &source);
            let key = (runtime, target.clone());
            // Editing the body must not repeatedly stat every directory on PATH.
            if !force && previous.borrow().as_ref() == Some(&key) {
                return;
            }
            previous.replace(Some(key));
            let (state, text, detail) = match target {
                Ok(target) => match crate::adapters::resolve_action_executable(&target) {
                    Some(path) => (
                        "available",
                        format!("{} available", runtime.label()),
                        format!(
                            "Executable: {}\nInterpreter availability only. Script syntax and external dependencies are not checked. Nothing is executed by this check.",
                            path.display()
                        ),
                    ),
                    None => (
                        "unavailable",
                        format!("{} not found — cannot run", runtime.label()),
                        format!(
                            "Executable not found: {target}. Install it and reopen this dialog, or choose an installed executable. You can still save this draft."
                        ),
                    ),
                },
                Err(error) => (
                    "unavailable",
                    "Invalid interpreter — cannot run".to_owned(),
                    error,
                ),
            };
            for class in ["available", "unavailable"] {
                row.remove_css_class(class);
            }
            row.add_css_class(state);
            icons.set_visible_child_name(state);
            label.set_text(&text);
            label.set_tooltip_text(Some(&detail));
            label.update_property(&[gtk::accessible::Property::Description(&detail)]);
        }
    });
    update(true);
    for buffer in [python, bash] {
        let update = update.clone();
        buffer.connect_changed(move |_| update(false));
    }
    for choice in choices {
        let update = update.clone();
        choice.connect_toggled(move |choice| {
            if choice.is_active() {
                update(false);
            }
        });
    }
    row.connect_map(move |_| update(true));
    row
}

fn executable(runtime: ActionRuntime, source: &str) -> Result<String, String> {
    let definition = super::draft_definition(runtime, ExecutionMode::WholeSelection);
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
        .map_err(|error| error.to_string())
}

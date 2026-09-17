// SPDX-License-Identifier: MIT

//! Editor tests. These cover the form's validation and the small pure helpers;
//! the editor's appearance is reviewed with screenshots instead.

use super::*;
use crate::model::ActionRuntime;

fn form_for(definition: ActionDefinition, script: Option<ActionScript>) -> EditorForm {
    let action = ActionHandle {
        directory: std::path::PathBuf::from("/tmp/actions/test"),
        definition,
        availability: crate::services::ActionAvailability::Unavailable {
            reason: "not saved".to_owned(),
        },
    };
    EditorForm::new(EditorMode::Create, &action, script)
}

fn python_draft() -> ActionDefinition {
    draft_definition(ActionRuntime::Python, ExecutionMode::WholeSelection)
}

#[test]
fn a_new_action_suggests_an_id_from_its_name() {
    crate::test_support::gtk_test(
        "ui::settings::actions::tests::a_new_action_suggests_an_id_from_its_name",
        || {
            let form = form_for(python_draft(), None);
            form.name.set_text("Convert to WebP");
            let (definition, script) = form.read().expect("a named action is valid");
            assert_eq!(definition.id, "convert-to-webp");
            assert_eq!(definition.run.runtime, ActionRuntime::Python);
            assert_eq!(
                script.map(|script| script.file_name),
                Some("main.py".to_owned())
            );
        },
    );
}

#[test]
fn the_editor_requires_a_name_and_reports_invalid_fields() {
    crate::test_support::gtk_test(
        "ui::settings::actions::tests::the_editor_requires_a_name_and_reports_invalid_fields",
        || {
            let form = form_for(python_draft(), None);
            assert_eq!(form.read().err(), Some("Enter a name".to_owned()));

            form.name.set_text("Action");
            form.max_items.set_text("lots");
            assert_eq!(
                form.read().err(),
                Some("The maximum item count must be a number".to_owned())
            );

            // The script file name is validated by the model, not the form.
            form.max_items.set_text("");
            form.entrypoint.set_text("../escape.py");
            assert!(
                form.read()
                    .err()
                    .is_some_and(|message| message.contains("plain file name")),
                "path traversal in the script field is refused"
            );
            form.entrypoint.set_text("main.py");
            assert!(form.read().is_ok());
        },
    );
}

#[test]
fn a_command_action_without_a_program_is_refused() {
    crate::test_support::gtk_test(
        "ui::settings::actions::tests::a_command_action_without_a_program_is_refused",
        || {
            let definition =
                draft_definition(ActionRuntime::Command, ExecutionMode::WholeSelection);
            let form = form_for(definition, None);
            form.name.set_text("Build project");
            let message = form.read().expect_err("a program is required");
            assert!(message.contains("Enter the program"), "{message}");

            form.program.set_text("make");
            form.arguments.set_text("-C\n{paths}");
            let (definition, script) = form.read().expect("a command action saves");
            assert_eq!(definition.run.program.as_deref(), Some("make"));
            assert_eq!(definition.run.args, vec!["-C", "{paths}"]);
            assert!(script.is_none(), "command actions carry no script");
        },
    );
}

#[test]
fn per_item_and_whole_selection_tokens_are_validated() {
    crate::test_support::gtk_test(
        "ui::settings::actions::tests::per_item_and_whole_selection_tokens_are_validated",
        || {
            let whole = draft_definition(ActionRuntime::Command, ExecutionMode::WholeSelection);
            let form = form_for(whole, None);
            form.name.set_text("Checksums");
            form.program.set_text("sha256sum");
            form.arguments.set_text("{path}");
            assert!(
                form.read()
                    .err()
                    .is_some_and(|message| message.contains("per-item")),
                "{{path}} is refused for a whole-selection action"
            );

            let per_item = draft_definition(ActionRuntime::Command, ExecutionMode::PerItem);
            let form = form_for(per_item, None);
            form.name.set_text("Convert");
            form.program.set_text("convert");
            form.arguments.set_text("{paths}");
            assert!(
                form.read()
                    .err()
                    .is_some_and(|message| message.contains("whole-selection")),
                "{{paths}} is refused for a per-item action"
            );
            form.arguments.set_text("{path}\n{parent}");
            assert!(form.read().is_ok());
        },
    );
}

#[test]
fn selection_rules_and_placement_round_trip_through_the_form() {
    crate::test_support::gtk_test(
        "ui::settings::actions::tests::selection_rules_and_placement_round_trip_through_the_form",
        || {
            let mut definition = python_draft();
            definition.when.kinds = vec![InputKind::File];
            definition.when.extensions = vec!["png".to_owned(), "jpg".to_owned()];
            definition.when.max_items = Some(12);
            definition.menu = MenuPlacement::Top;
            definition.description = Some("Creates smaller copies".to_owned());
            definition.run.mode = ExecutionMode::PerItem;
            definition.run.on_error = ErrorPolicy::Stop;
            definition.run.confirm = true;
            let form = form_for(definition, None);
            form.name.set_text("Resize images");
            let (read, _) = form.read().expect("a complete action saves");
            assert_eq!(read.when.kinds, vec![InputKind::File]);
            assert_eq!(read.when.extensions, vec!["png", "jpg"]);
            assert_eq!(read.when.max_items, Some(12));
            assert_eq!(read.menu, MenuPlacement::Top);
            assert_eq!(read.run.mode, ExecutionMode::PerItem);
            assert_eq!(read.run.on_error, ErrorPolicy::Stop);
            assert!(read.run.confirm);
            assert_eq!(read.description.as_deref(), Some("Creates smaller copies"));
        },
    );
}

#[test]
fn extension_fields_accept_dots_caps_and_spaces() {
    crate::test_support::gtk_test(
        "ui::settings::actions::tests::extension_fields_accept_dots_caps_and_spaces",
        || {
            let form = form_for(python_draft(), None);
            form.name.set_text("Images");
            form.extensions.set_text(" .PNG , jpg ,, ");
            let (definition, _) = form.read().expect("extensions normalize");
            assert_eq!(definition.when.extensions, vec!["png", "jpg"]);
        },
    );
}

#[test]
fn unknown_icons_are_dropped_rather_than_rendered_as_missing() {
    crate::test_support::gtk_test(
        "ui::settings::actions::tests::unknown_icons_are_dropped_rather_than_rendered_as_missing",
        || {
            let mut definition = python_draft();
            definition.icon = Some("not-bundled".to_owned());
            let action = ActionHandle {
                directory: std::path::PathBuf::from("/tmp/actions/test"),
                definition,
                availability: crate::services::ActionAvailability::Unavailable {
                    reason: "not saved".to_owned(),
                },
            };
            let form = EditorForm::new(EditorMode::Create, &action, None);
            form.name.set_text("Action");
            let (read, _) = form.read().expect("the action still saves");
            assert_eq!(
                read.icon, None,
                "an icon that is not bundled falls back to the default"
            );
        },
    );
}

#[test]
fn summary_lines_describe_runtime_scope_and_problems() {
    let mut definition = python_draft();
    definition.when.kinds = vec![InputKind::File];
    definition.when.extensions = vec!["png".to_owned()];
    definition.run.mode = ExecutionMode::PerItem;
    let action = ActionHandle {
        directory: std::path::PathBuf::from("/tmp/actions/test"),
        definition,
        availability: crate::services::ActionAvailability::Unavailable {
            reason: "The interpreter “python3” was not found".to_owned(),
        },
    };
    let text = summary(&action);
    assert!(text.contains("Python"), "{text}");
    assert!(text.contains("per item"), "{text}");
    assert!(text.contains("files"), "{text}");
    assert!(text.contains("png"), "{text}");
    assert!(text.contains("not found"), "{text}");
}

#[test]
fn duplicate_ids_avoid_collisions_and_stay_within_the_limit() {
    let store = crate::adapters::LocalActionStore::at(std::path::PathBuf::from("/tmp/unused"));
    let registry = std::rc::Rc::new(ActionRegistry::new(store));
    assert_eq!(unique_copy_id(&registry, "resize"), "resize-copy");
    assert_eq!(
        unique_copy_id(&registry, &"x".repeat(crate::model::MAX_ACTION_ID_CHARS)),
        "x".repeat(crate::model::MAX_ACTION_ID_CHARS),
        "an over-long id is truncated rather than rejected"
    );
    assert_eq!(copy_name("Resize"), "Resize copy");
    assert!(copy_name(&"n".repeat(200)).chars().count() <= crate::model::MAX_ACTION_NAME_CHARS);
}

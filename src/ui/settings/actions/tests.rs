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
            form.tabs.set_current_page(Some(SCRIPT_TAB));
            assert_eq!(form.read().err(), Some("Enter a name".to_owned()));
            assert_eq!(form.tabs.current_page(), Some(GENERAL_TAB));

            form.name.set_text("Action");
            form.max_items.set_text("lots");
            assert_eq!(
                form.read().err(),
                Some("The maximum item count must be a number".to_owned())
            );
            assert_eq!(form.tabs.current_page(), Some(BEHAVIOR_TAB));

            // The script file name is validated by the model, not the form.
            form.max_items.set_text("");
            form.entrypoint.set_text("../escape.py");
            assert!(
                form.read()
                    .err()
                    .is_some_and(|message| message.contains("plain file name")),
                "path traversal in the script field is refused"
            );
            assert_eq!(form.tabs.current_page(), Some(SCRIPT_TAB));
            form.entrypoint.set_text("main.py");
            form.files.set_active(false);
            form.folders.set_active(false);
            assert_eq!(
                form.read().err().as_deref(),
                Some("Choose Files, Folders, or both")
            );
            assert_eq!(form.tabs.current_page(), Some(BEHAVIOR_TAB));
            form.files.set_active(true);
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
            form.arguments.buffer().set_text("-C\n{paths}");
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
            form.arguments.buffer().set_text("{path}");
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
            form.arguments.buffer().set_text("{paths}");
            assert!(
                form.read()
                    .err()
                    .is_some_and(|message| message.contains("whole-selection")),
                "{{paths}} is refused for a per-item action"
            );
            form.arguments.buffer().set_text("{path}\n{parent}");
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
fn icon_choices_save_bundled_icons_and_fall_back_for_unknown_ones() {
    crate::test_support::gtk_test(
        "ui::settings::actions::tests::icon_choices_save_bundled_icons_and_fall_back_for_unknown_ones",
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
            choose(&form, "printer");
            assert_eq!(
                form.read().expect("popover icon saves").0.icon.as_deref(),
                Some("printer")
            );
            choose(&form, "copy");
            assert_eq!(
                form.read().expect("inline icon saves").0.icon.as_deref(),
                Some("copy")
            );
        },
    );
}

fn choice(form: &EditorForm, label: &str) -> gtk::ToggleButton {
    fn find(widget: &gtk::Widget, label: &str) -> Option<gtk::ToggleButton> {
        if let Some(button) = widget.downcast_ref::<gtk::ToggleButton>()
            && (button.label().as_deref() == Some(label)
                || button.tooltip_text().as_deref() == Some(label))
        {
            return Some(button.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Some(button) = find(&widget, label) {
                return Some(button);
            }
        }
        None
    }
    find(form.root.upcast_ref(), label).expect("an editor choice")
}

fn choose(form: &EditorForm, label: &str) {
    choice(form, label).set_active(true);
}

#[test]
fn runtime_changes_preserve_drafts_and_save_only_the_active_runtime() {
    crate::test_support::gtk_test(
        "ui::settings::actions::tests::runtime_changes_preserve_drafts_and_save_only_the_active_runtime",
        || {
            let form = form_for(python_draft(), None);
            form.name.set_text("Process files");
            form.script.buffer().set_text("print('python draft')\n");
            choose(&form, "Bash");
            let (definition, script) = form.read().expect("Bash starts with a usable template");
            let script = script.expect("Bash script");
            assert_eq!(script.file_name, "run.sh");
            assert!(definition.interpreter_for_source(&script.contents).is_ok());
            assert!(script.contents.contains("STRATA_ACTION_PATHS"));
            form.script.buffer().set_text("printf 'bash draft\\n'\n");
            choose(&form, "Command");
            form.program.set_text("printf");
            form.arguments.buffer().set_text("%s\\n\n{paths}");
            let (command, script) = form.read().expect("command accepts separate arguments");
            assert_eq!(command.run.args, ["%s\\n", "{paths}"]);
            assert!(script.is_none());
            choose(&form, "Python");
            let (python, script) = form.read().expect("hidden command arguments are not saved");
            assert!(python.run.args.is_empty());
            assert_eq!(
                script.expect("Python script").contents,
                "print('python draft')\n"
            );
            choose(&form, "Bash");
            assert_eq!(
                form.read()
                    .expect("Bash draft saves")
                    .1
                    .expect("Bash script")
                    .contents,
                "printf 'bash draft\\n'\n"
            );
            choose(&form, "Command");
            assert_eq!(form.read().expect("command draft saves").0.run, command.run);
        },
    );
}

#[test]
fn tab_switches_retain_edits_and_execution_choices_control_failure_policy() {
    crate::test_support::gtk_test(
        "ui::settings::actions::tests::tab_switches_retain_edits_and_execution_choices_control_failure_policy",
        || {
            let form = form_for(python_draft(), None);
            form.name.set_text("Batch rename");
            form.tabs.set_current_page(Some(SCRIPT_TAB));
            form.script.buffer().set_text("print('retained')\n");
            form.tabs.set_current_page(Some(BEHAVIOR_TAB));
            assert!(!choice(&form, "Stop").is_sensitive());
            choose(&form, "Per item");
            assert!(choice(&form, "Stop").is_sensitive());
            choose(&form, "Stop");
            choose(&form, "Home");
            choose(&form, "Menu item");
            form.confirm.set_active(true);
            form.tabs.set_current_page(Some(GENERAL_TAB));
            form.description.set_text("Rename selected files");
            let (definition, script) = form.read().expect("all tabs save together");
            assert_eq!(definition.name, "Batch rename");
            assert_eq!(
                definition.description.as_deref(),
                Some("Rename selected files")
            );
            assert_eq!(
                script.expect("script remains after tab switches").contents,
                "print('retained')\n"
            );
            assert_eq!(definition.run.mode, ExecutionMode::PerItem);
            assert_eq!(definition.run.on_error, ErrorPolicy::Stop);
            assert_eq!(definition.run.working_directory, WorkingDirectory::Home);
            assert_eq!(definition.menu, MenuPlacement::Top);
            assert!(definition.run.confirm);
            choose(&form, "Whole selection");
            assert!(!choice(&form, "Stop").is_sensitive());
            assert_eq!(
                form.read().expect("whole selection saves").0.run.mode,
                ExecutionMode::WholeSelection
            );
            choose(&form, "Per item");
            assert_eq!(
                form.read()
                    .expect("per-item policy is retained")
                    .0
                    .run
                    .on_error,
                ErrorPolicy::Stop
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

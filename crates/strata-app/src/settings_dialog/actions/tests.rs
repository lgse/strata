// SPDX-License-Identifier: MIT

use std::{collections::HashMap, fs, path::PathBuf, rc::Rc};

use glib::variant::ToVariant;
use rune::{Key, Mods};
use serde_json::Value;
use strata_core::{
    i18n::tr,
    model::{
        ActionDefinition, ActionRuntime, ErrorPolicy, ExecutionMode, InputKind, MenuPlacement,
        WorkingDirectory, action::examples::ACTION_EXAMPLES,
    },
    storage::config_directory,
};
use strata_fs::services::{ActionAvailability, ActionHandle};

use super::{
    editor::{
        BEHAVIOR_TAB, EditorDraft, EditorMode, EditorSession, GENERAL_TAB, SCRIPT_TAB,
        draft_definition,
    },
    examples, folder_picker, icon_chooser, readiness,
    view::{IMPORT_ACTION_KEY, NEW_ACTION_KEY},
};
use crate::{
    custom_actions, error_dialog,
    host::offscreen::Offscreen,
    preview,
    settings_dialog::tests::{click, has_state, press},
    test_support::{
        PATIENCE, hosted,
        view::{self, idle},
    },
};

const MODULE: &str = "settings_dialog::actions::tests";

fn open_actions(host: &mut Offscreen) {
    click(host, "header-settings");
    click(host, "settings-nav-actions");
}

fn find_by_key<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    view::all(tree, &|node| node["key"] == key)
        .into_iter()
        .next()
}

fn find_by_role_and_name<'a>(tree: &'a Value, role: &str, name: &str) -> Option<&'a Value> {
    view::all(tree, &|node| {
        node["role"] == role && view::text(node, "name") == name
    })
    .into_iter()
    .next()
}

fn write_sample_action(actions_root: &std::path::Path, id: &str, name: &str, enabled: bool) {
    let dir = actions_root.join(id);
    fs::create_dir_all(&dir).expect("create action dir");
    fs::write(
        dir.join("action.toml"),
        format!(
            "schema_version = 1\n\
             id = \"{id}\"\n\
             name = \"{name}\"\n\
             enabled = {enabled}\n\
             menu = \"submenu\"\n\n\
             [when]\n\n\
             [run]\n\
             runtime = \"command\"\n\
             program = \"/bin/sh\"\n\
             args = [\"-c\", \"true\"]\n"
        ),
    )
    .expect("write action.toml");
}

#[test]
fn empty_state_live_observer_toggle_enabled_and_problems_section() {
    hosted(
        MODULE,
        "empty_state_live_observer_toggle_enabled_and_problems_section",
        |mut host| {
            let actions_dir = config_directory().join("actions");
            open_actions(&mut host);

            assert!(host.app().settings_dialog.actions.is_observing());
            let tree = host.tree();
            assert!(
                find_by_key(&tree, "settings-actions-empty").is_some(),
                "empty message renders when no actions exist"
            );

            // Add a valid action and a broken action on disk, then trigger a registry reload.
            write_sample_action(&actions_dir, "demo", "Demo Action", true);
            let broken_dir = actions_dir.join("broken");
            fs::create_dir_all(&broken_dir).expect("broken dir");
            fs::write(broken_dir.join("action.toml"), "not valid toml = [").expect("broken toml");

            host.runtime().update(|app, _| {
                let registry = custom_actions::shared_registry(app);
                glib::MainContext::default().spawn_local(registry.reload());
            });
            assert!(
                host.run_until(PATIENCE, |h| {
                    let tree = h.tree();
                    find_by_key(&tree, "settings-action-row-demo").is_some()
                        && find_by_key(&tree, "settings-action-problem-broken").is_some()
                }),
                "live observer updates Actions list and PROBLEMS section"
            );

            let tree = host.tree();
            let toggle =
                find_by_key(&tree, "settings-action-enabled-demo").expect("enabled switch");
            assert!(has_state(toggle, "checked"));

            // Toggle enabled off and confirm disk + UI reflect disabled state.
            click(&mut host, "settings-action-enabled-demo");
            assert!(
                host.run_until(PATIENCE, |h| {
                    let tree = h.tree();
                    find_by_key(&tree, "settings-action-enabled-demo")
                        .is_some_and(|node| !has_state(node, "checked"))
                }),
                "toggling enabled switch updates UI state"
            );
            let saved = fs::read_to_string(actions_dir.join("demo/action.toml")).expect("read");
            assert!(saved.contains("enabled = false"), "saved toml:\n{saved}");

            // Closing Settings releases the live observer.
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().settings_dialog.is_open());
            assert!(!host.app().settings_dialog.actions.is_observing());
        },
    );
}

#[test]
fn create_edit_duplicate_and_delete_action_through_ui() {
    hosted(
        MODULE,
        "create_edit_duplicate_and_delete_action_through_ui",
        |mut host| {
            open_actions(&mut host);

            // Open New action modal.
            click(&mut host, NEW_ACTION_KEY);
            let editor_gen = host
                .app()
                .settings_dialog
                .actions
                .editor
                .as_ref()
                .expect("editor open")
                .generation;
            let primary_key = format!("settings-action-editor-{editor_gen}-primary");

            // Submitting an empty name surfaces validation error on General tab.
            click(&mut host, &primary_key);
            let tree = host.tree();
            let err = find_by_key(&tree, "settings-action-editor-error").expect("validation error");
            assert_eq!(view::text(err, "name"), tr("Enter a name"));

            // Fill name, choose an icon from the More icons popover, and configure Behavior.
            host.runtime().update(|app, _| {
                let editor = app
                    .settings_dialog
                    .actions
                    .editor
                    .as_mut()
                    .expect("editor open");
                editor.name.replace_text("Convert Images");
                editor.description.replace_text("Converts images");
            });
            idle(&mut host);

            click(&mut host, icon_chooser::MORE_ICONS_BTN_KEY);
            click(&mut host, "settings-action-icon-printer");
            assert_eq!(
                host.app()
                    .settings_dialog
                    .actions
                    .editor
                    .as_ref()
                    .and_then(|e| e.selected_icon.as_deref()),
                Some("printer")
            );

            // Switch to Behavior tab: On failure is disabled for Whole selection, enabled for Per item.
            click(&mut host, "settings-action-editor-tabs-tab-2");
            let tree = host.tree();
            let stop_btn = find_by_key(&tree, "settings-action-editor-policy-1").expect("stop btn");
            assert!(has_state(stop_btn, "disabled"));

            click(&mut host, "settings-action-editor-mode-1");
            let tree = host.tree();
            let stop_btn = find_by_key(&tree, "settings-action-editor-policy-1").expect("stop btn");
            assert!(!has_state(stop_btn, "disabled"));
            click(&mut host, "settings-action-editor-policy-1");

            // Create action and wait for it to appear in the list.
            click(&mut host, &primary_key);
            assert!(
                host.run_until(PATIENCE, |h| {
                    h.app().settings_dialog.actions.editor.is_none()
                        && find_by_key(&h.tree(), "settings-action-row-convert-images").is_some()
                }),
                "created action appears in list"
            );

            // Edit the created action: id is disabled, description saves.
            click(&mut host, "settings-action-edit-convert-images");
            assert!(
                host.run_until(PATIENCE, |h| {
                    h.app().settings_dialog.actions.editor.is_some()
                }),
                "edit modal opens after async read"
            );
            let tree = host.tree();
            let id_field = find_by_key(&tree, "settings-action-editor-id").expect("id field");
            assert!(has_state(id_field, "disabled"));
            let edit_gen = host
                .app()
                .settings_dialog
                .actions
                .editor
                .as_ref()
                .expect("editor open")
                .generation;
            host.runtime().update(|app, _| {
                app.settings_dialog
                    .actions
                    .editor
                    .as_mut()
                    .expect("editor open")
                    .description
                    .replace_text("Updated description");
            });
            idle(&mut host);
            click(
                &mut host,
                &format!("settings-action-editor-{edit_gen}-primary"),
            );
            assert!(
                host.run_until(PATIENCE, |h| h
                    .app()
                    .settings_dialog
                    .actions
                    .editor
                    .is_none()),
                "edit modal closes after save"
            );
            let saved =
                fs::read_to_string(config_directory().join("actions/convert-images/action.toml"))
                    .expect("saved manifest");
            assert!(saved.contains("Updated description"), "{saved}");

            // Duplicate creates convert-images-copy.
            click(&mut host, "settings-action-duplicate-convert-images");
            assert!(
                host.run_until(PATIENCE, |h| {
                    find_by_key(&h.tree(), "settings-action-row-convert-images-copy").is_some()
                }),
                "duplicated action appears in list"
            );

            // Delete with confirmation: Cancel keeps action, Delete removes it.
            click(&mut host, "settings-action-delete-convert-images-copy");
            let tree = host.tree();
            assert!(
                find_by_role_and_name(&tree, "dialog", &tr("Delete this action?")).is_some(),
                "delete confirmation dialog opens"
            );
            let del_gen = host
                .app()
                .settings_dialog
                .actions
                .delete_confirm
                .as_ref()
                .expect("confirm open")
                .generation;
            click(
                &mut host,
                &format!("settings-action-delete-confirm-{del_gen}-secondary"),
            );
            assert!(host.app().settings_dialog.actions.delete_confirm.is_none());
            assert!(
                find_by_key(&host.tree(), "settings-action-row-convert-images-copy").is_some(),
                "Cancel preserves the action"
            );

            click(&mut host, "settings-action-delete-convert-images-copy");
            let del_gen = host
                .app()
                .settings_dialog
                .actions
                .delete_confirm
                .as_ref()
                .expect("confirm open")
                .generation;
            click(
                &mut host,
                &format!("settings-action-delete-confirm-{del_gen}-primary"),
            );
            assert!(
                host.run_until(PATIENCE, |h| {
                    find_by_key(&h.tree(), "settings-action-row-convert-images-copy").is_none()
                }),
                "confirmed delete removes action from list"
            );
        },
    );
}

#[test]
fn examples_library_applies_templates_and_confirms_before_replacing_draft() {
    hosted(
        MODULE,
        "examples_library_applies_templates_and_confirms_before_replacing_draft",
        |mut host| {
            open_actions(&mut host);
            click(&mut host, NEW_ACTION_KEY);
            click(&mut host, "settings-action-editor-tabs-tab-1");
            click(&mut host, examples::EXAMPLES_BTN_KEY);

            let tree = host.tree();
            assert!(
                find_by_key(&tree, "settings-action-library-popover").is_some(),
                "library popover opens"
            );

            // Search narrows the library so Convert images to WebP is at the top.
            host.runtime().update(|app, _| {
                app.settings_dialog
                    .actions
                    .editor
                    .as_mut()
                    .expect("editor open")
                    .examples_query
                    .replace_text("webp");
            });
            idle(&mut host);

            // Untouched draft applies directly without confirmation.
            click(
                &mut host,
                "settings-action-library-item-convert-images-to-webp",
            );
            let editor = host
                .app()
                .settings_dialog
                .actions
                .editor
                .as_ref()
                .expect("editor open");
            assert!(!editor.examples_open);
            assert_eq!(editor.name.text, tr("Convert images to WebP"));
            assert_eq!(editor.mode_choice, ExecutionMode::PerItem);
            assert_eq!(
                editor.extensions.text,
                "png, jpg, jpeg, webp, bmp, tif, tiff"
            );

            // Modify draft so applying another Python template asks for confirmation.
            host.runtime().update(|app, _| {
                app.settings_dialog
                    .actions
                    .editor
                    .as_mut()
                    .expect("editor open")
                    .python_editor
                    .set_text("print('custom draft')\n");
            });
            idle(&mut host);

            click(&mut host, examples::EXAMPLES_BTN_KEY);
            click(
                &mut host,
                "settings-action-library-item-convert-images-to-webp",
            );
            assert!(
                find_by_key(&host.tree(), "settings-action-library-confirm").is_some(),
                "replace confirmation shows for modified draft"
            );
            click(&mut host, "settings-action-library-keep");
            assert_eq!(
                host.app()
                    .settings_dialog
                    .actions
                    .editor
                    .as_ref()
                    .expect("editor")
                    .python_editor
                    .text(),
                "print('custom draft')\n"
            );

            click(
                &mut host,
                "settings-action-library-item-convert-images-to-webp",
            );
            click(&mut host, "settings-action-library-replace");
            let editor = host
                .app()
                .settings_dialog
                .actions
                .editor
                .as_ref()
                .expect("editor");
            assert!(!editor.examples_open);
            assert_ne!(editor.python_editor.text(), "print('custom draft')\n");
            // Replacement is a single undo step in CodeEditor.
            host.runtime().update(|app, _| {
                app.settings_dialog
                    .actions
                    .editor
                    .as_mut()
                    .expect("editor")
                    .python_editor
                    .edit
                    .undo();
            });
            assert_eq!(
                host.app()
                    .settings_dialog
                    .actions
                    .editor
                    .as_ref()
                    .expect("editor")
                    .python_editor
                    .text(),
                "print('custom draft')\n"
            );
        },
    );
}

#[test]
fn import_and_export_show_notices_and_invalid_import_shows_error_dialog() {
    hosted(
        MODULE,
        "import_and_export_show_notices_and_invalid_import_shows_error_dialog",
        |mut host| {
            open_actions(&mut host);

            let source_root = tempfile::tempdir().expect("source tempdir");
            write_sample_action(source_root.path(), "shared-tool", "Shared Tool", true);
            let import_src = source_root.path().join("shared-tool");

            // Import valid folder -> imported disabled + Action imported notice.
            let src_clone = import_src.clone();
            host.app_mut().settings_dialog.actions.folder_picker =
                Some(Rc::new(move |_| Ok(Some(src_clone.clone()))));
            click(&mut host, IMPORT_ACTION_KEY);
            assert!(
                host.run_until(PATIENCE, |h| {
                    h.app().settings_dialog.actions.notice.is_some()
                }),
                "import notice opens"
            );
            let notice = host
                .app()
                .settings_dialog
                .actions
                .notice
                .as_ref()
                .expect("notice");
            assert_eq!(notice.title, tr("Action imported"));
            assert!(notice.detail.contains("shared-tool"), "{}", notice.detail);
            let notice_gen = notice.generation;
            click(
                &mut host,
                &format!("settings-action-notice-{notice_gen}-primary"),
            );
            assert!(host.app().settings_dialog.actions.notice.is_none());
            let tree = host.tree();
            let toggle = find_by_key(&tree, "settings-action-enabled-shared-tool")
                .expect("imported action row visible");
            assert!(
                !has_state(toggle, "checked"),
                "imported actions start disabled"
            );

            // Export the imported action to a fresh directory -> Action exported notice.
            let export_dest = tempfile::tempdir().expect("export tempdir");
            let dest_path = export_dest.path().to_path_buf();
            host.app_mut().settings_dialog.actions.folder_picker =
                Some(Rc::new(move |_| Ok(Some(dest_path.clone()))));
            click(&mut host, "settings-action-export-shared-tool");
            assert!(
                host.run_until(PATIENCE, |h| {
                    h.app().settings_dialog.actions.notice.is_some()
                }),
                "export notice opens"
            );
            let notice = host
                .app()
                .settings_dialog
                .actions
                .notice
                .as_ref()
                .expect("notice");
            assert_eq!(notice.title, tr("Action exported"));
            assert!(export_dest.path().join("shared-tool/action.toml").exists());
            let notice_gen = notice.generation;
            click(
                &mut host,
                &format!("settings-action-notice-{notice_gen}-primary"),
            );

            // Importing a directory without action.toml surfaces error_dialog.
            let invalid_dir = tempfile::tempdir().expect("invalid tempdir");
            let invalid_path = invalid_dir.path().to_path_buf();
            host.app_mut().settings_dialog.actions.folder_picker =
                Some(Rc::new(move |_| Ok(Some(invalid_path.clone()))));
            click(&mut host, IMPORT_ACTION_KEY);
            assert!(
                host.run_until(PATIENCE, |h| error_dialog::shown_details(h.app()).is_some()),
                "error dialog opens for invalid action folder"
            );
            let (_, _, title, _, _) =
                error_dialog::shown_details(host.app()).expect("error details");
            assert_eq!(title, tr("Unable to import the action"));
        },
    );
}

#[test]
fn settings_search_finds_custom_actions_new_and_import_targets() {
    hosted(
        MODULE,
        "settings_search_finds_custom_actions_new_and_import_targets",
        |mut host| {
            click(&mut host, "header-settings");
            for (query, expected_key) in [
                ("new custom action", NEW_ACTION_KEY),
                ("import custom action", IMPORT_ACTION_KEY),
            ] {
                host.runtime().update(|app, _| {
                    app.settings_dialog.search.edit_mut().replace_text("");
                });
                idle(&mut host);
                click(&mut host, "settings-search-field");
                for ch in query.chars() {
                    press(&mut host, Key::Text(ch.to_string()), Mods::default());
                }
                host.runtime().frame(0.2);
                idle(&mut host);
                assert!(
                    find_by_key(&host.tree(), expected_key).is_some(),
                    "query {query:?} reveals {expected_key}"
                );
            }
        },
    );
}

#[test]
fn editor_validation_runtime_preservation_examples_and_portal_parser() {
    hosted(
        MODULE,
        "editor_validation_runtime_preservation_examples_and_portal_parser",
        |mut host| {
            let later = host.app().settings_dialog.actions.later.downgrade();
            let highlighter = preview::highlighter(host.app_mut());
            let handle = |definition: ActionDefinition| ActionHandle {
                directory: PathBuf::from("/tmp/actions/test"),
                definition,
                availability: ActionAvailability::Unavailable {
                    reason: "not saved".to_owned(),
                },
            };

            // Validation routes each invalid field to the right tab.
            let initial_handle = handle(draft_definition(
                ActionRuntime::Python,
                ExecutionMode::WholeSelection,
            ));
            let mut form = EditorSession::new(
                EditorDraft {
                    mode: EditorMode::Create,
                    generation: 1,
                    action: &initial_handle,
                    script: None,
                },
                &highlighter,
                later.clone(),
            );
            assert_eq!(form.read().unwrap_err().tab, GENERAL_TAB);
            form.name.replace_text("Convert to WebP");
            form.max_items.replace_text("lots");
            assert_eq!(form.read().unwrap_err().tab, BEHAVIOR_TAB);
            form.max_items.replace_text("");
            form.entrypoint.replace_text("../escape.py");
            assert_eq!(form.read().unwrap_err().tab, SCRIPT_TAB);
            form.entrypoint.replace_text("main.py");
            form.files = false;
            form.folders = false;
            assert_eq!(form.read().unwrap_err().tab, BEHAVIOR_TAB);
            form.files = true;
            form.extensions.replace_text(" .PNG , jpg ,, ");
            form.selected_icon = Some("not-bundled".to_owned());
            let (def, script) = form.read().expect("valid draft");
            assert_eq!(def.id, "convert-to-webp");
            assert_eq!(def.when.extensions, vec!["png", "jpg"]);
            assert_eq!(def.icon, None);
            assert_eq!(script.expect("script").file_name, "main.py");

            // Runtime switches preserve Python/Bash/Command drafts and swap default entrypoints.
            form.python_editor.set_text("print('py')\n");
            form.set_runtime(ActionRuntime::Bash, later.clone());
            assert_eq!(form.entrypoint.text, "run.sh");
            form.bash_editor.set_text("echo sh\n");
            form.set_runtime(ActionRuntime::Command, later.clone());
            assert_eq!(form.read().unwrap_err().tab, SCRIPT_TAB);
            form.program.replace_text("sha256sum");
            form.arguments.replace_text("{path}");
            assert_eq!(form.read().unwrap_err().tab, SCRIPT_TAB);
            form.arguments.replace_text("{paths}");
            let (cmd_def, cmd_script) = form.read().expect("command saves");
            assert_eq!(cmd_def.run.args, vec!["{paths}"]);
            assert!(cmd_script.is_none());
            form.set_runtime(ActionRuntime::Python, later.clone());
            assert_eq!(form.python_editor.text(), "print('py')\n");
            assert!(form.read().expect("py").0.run.args.is_empty());

            // Every bundled example yields a valid action definition & script.
            for example in ACTION_EXAMPLES {
                form.apply_example(example, later.clone());
                let (ex_def, ex_script) = form.read().expect("example valid");
                assert_eq!(ex_def.run.runtime, example.runtime());
                assert_eq!(ex_def.run.mode, example.mode);
                assert_eq!(ex_script.expect("script").contents, example.script());
            }

            // Unedited multiline argv and hidden conditions round-trip unchanged.
            let mut preserved = draft_definition(ActionRuntime::Command, ExecutionMode::PerItem);
            preserved.id = "multi".to_owned();
            preserved.name = "Multi".to_owned();
            preserved.run.program = Some("printf".to_owned());
            preserved.run.args = vec!["a\nb".to_owned(), "{path}".to_owned()];
            preserved.when.min_items = 2;
            preserved.when.mime_types = vec!["image/*".to_owned()];
            preserved.run.working_directory = WorkingDirectory::Home;
            preserved.menu = MenuPlacement::Top;
            preserved.run.on_error = ErrorPolicy::Stop;
            let preserved_handle = handle(preserved.clone());
            let form2 = EditorSession::new(
                EditorDraft {
                    mode: EditorMode::Edit,
                    generation: 2,
                    action: &preserved_handle,
                    script: None,
                },
                &highlighter,
                later,
            );
            let (out, _) = form2.read().expect("preserved");
            assert_eq!(out.run.args, preserved.run.args);
            assert_eq!(out.when.min_items, 2);
            assert_eq!(out.when.mime_types, vec!["image/*"]);
            assert_eq!(out.run.working_directory, WorkingDirectory::Home);
            assert_eq!(out.menu, MenuPlacement::Top);
            assert_eq!(out.run.on_error, ErrorPolicy::Stop);
            assert_eq!(out.when.kinds, vec![InputKind::File, InputKind::Folder]);

            // Readiness executable extraction & example matching.
            assert_eq!(
                readiness::executable(ActionRuntime::Python, "#!/usr/bin/env python3\n"),
                Ok("python3".to_owned())
            );
            assert!(readiness::executable(ActionRuntime::Bash, "#!/bin/sh\n").is_ok());
            assert!(examples::matches(&ACTION_EXAMPLES[0], "python", "All"));
            assert!(!examples::matches(
                &ACTION_EXAMPLES[0],
                "nonexistent-xyz",
                "All"
            ));

            // Portal FileChooser Response variant parsing.
            let mut ok_map: HashMap<String, glib::Variant> = HashMap::new();
            ok_map.insert(
                "uris".to_owned(),
                vec!["file:///tmp/my-action"].to_variant(),
            );
            let accepted = (0u32, ok_map).to_variant();
            assert_eq!(
                folder_picker::parse_portal_response(&accepted),
                Some(PathBuf::from("/tmp/my-action"))
            );
            let mut encoded_map: HashMap<String, glib::Variant> = HashMap::new();
            encoded_map.insert(
                "uris".to_owned(),
                vec!["file:///tmp/my%20action"].to_variant(),
            );
            let encoded = (0u32, encoded_map).to_variant();
            assert_eq!(
                folder_picker::parse_portal_response(&encoded),
                Some(PathBuf::from("/tmp/my action"))
            );
            let cancelled = (1u32, HashMap::<String, glib::Variant>::new()).to_variant();
            assert_eq!(folder_picker::parse_portal_response(&cancelled), None);
        },
    );
}

#[test]
fn escape_unwinds_popover_then_editor_then_settings_and_apply_clears_query_focus() {
    hosted(
        MODULE,
        "escape_unwinds_popover_then_editor_then_settings_and_apply_clears_query_focus",
        |mut host| {
            open_actions(&mut host);
            click(&mut host, NEW_ACTION_KEY);
            click(&mut host, "settings-action-editor-tabs-tab-1");
            click(&mut host, examples::EXAMPLES_BTN_KEY);

            assert!(host.app().settings_dialog.is_open());
            assert!(
                host.app()
                    .settings_dialog
                    .actions
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.examples_open)
            );

            // First Escape closes only the Library popover.
            press(&mut host, Key::Escape, Mods::default());
            assert!(host.app().settings_dialog.is_open());
            assert!(
                host.app()
                    .settings_dialog
                    .actions
                    .editor
                    .as_ref()
                    .is_some_and(|e| !e.examples_open)
            );

            // Applying a template while the library search field is focused clears its focus flag.
            click(&mut host, examples::EXAMPLES_BTN_KEY);
            host.runtime().update(|app, _| {
                let editor = app
                    .settings_dialog
                    .actions
                    .editor
                    .as_mut()
                    .expect("editor open");
                editor.examples_query.focused = true;
                editor.examples_query.replace_text("webp");
            });
            idle(&mut host);
            click(
                &mut host,
                "settings-action-library-item-convert-images-to-webp",
            );
            let editor = host
                .app()
                .settings_dialog
                .actions
                .editor
                .as_ref()
                .expect("editor open");
            assert!(!editor.examples_open);
            assert!(!editor.examples_query.focused);

            // Second Escape closes only the editor modal; Settings stays open.
            press(&mut host, Key::Escape, Mods::default());
            assert!(host.app().settings_dialog.actions.editor.is_none());
            assert!(host.app().settings_dialog.is_open());

            // Third Escape closes Settings.
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().settings_dialog.is_open());
        },
    );
}

#[test]
fn closing_settings_discards_in_flight_edit_and_notices() {
    hosted(
        MODULE,
        "closing_settings_discards_in_flight_edit_and_notices",
        |mut host| {
            let actions_dir = config_directory().join("actions");
            write_sample_action(&actions_dir, "demo", "Demo Action", true);
            open_actions(&mut host);
            assert!(
                host.run_until(PATIENCE, |h| {
                    find_by_key(&h.tree(), "settings-action-row-demo").is_some()
                }),
                "demo action loads"
            );

            // Start async edit and immediately close Settings before the read completes.
            host.runtime().update(|app, _| {
                super::start_edit(app, "demo".to_owned());
                crate::settings_dialog::close_settings(app);
            });
            host.wait(std::time::Duration::from_millis(120));
            idle(&mut host);
            assert!(!host.app().settings_dialog.is_open());
            assert!(
                host.app().settings_dialog.actions.editor.is_none(),
                "closing Settings discards an in-flight edit open"
            );

            // Start async import and immediately close Settings before completion.
            let source_root = tempfile::tempdir().expect("source tempdir");
            write_sample_action(source_root.path(), "bg-import", "BG Import", true);
            let import_src = source_root.path().join("bg-import");
            open_actions(&mut host);
            host.runtime().update(move |app, tx| {
                app.settings_dialog.actions.folder_picker =
                    Some(Rc::new(move |_| Ok(Some(import_src.clone()))));
                super::start_import(app, tx);
                crate::settings_dialog::close_settings(app);
            });
            assert!(
                host.run_until(PATIENCE, |_| {
                    actions_dir.join("bg-import/action.toml").exists()
                }),
                "import still writes to disk"
            );
            idle(&mut host);
            assert!(
                host.app().settings_dialog.actions.notice.is_none(),
                "closing Settings suppresses stranded import notice"
            );

            // Start async export and immediately close Settings before completion.
            let export_dest = tempfile::tempdir().expect("export tempdir");
            let dest_path = export_dest.path().to_path_buf();
            let exported_manifest = export_dest.path().join("demo/action.toml");
            open_actions(&mut host);
            host.runtime().update(move |app, _| {
                app.settings_dialog.actions.folder_picker =
                    Some(Rc::new(move |_| Ok(Some(dest_path.clone()))));
                super::start_export(app, "demo".to_owned());
                crate::settings_dialog::close_settings(app);
            });
            assert!(
                host.run_until(PATIENCE, |_| exported_manifest.exists()),
                "export still writes to disk"
            );
            idle(&mut host);
            assert!(
                host.app().settings_dialog.actions.notice.is_none(),
                "closing Settings suppresses stranded export notice"
            );
        },
    );
}

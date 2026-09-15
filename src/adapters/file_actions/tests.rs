// SPDX-License-Identifier: MIT

use super::{ActionKind, FileAction, build_argv, parse_actions, uses_single_path_token};
use std::path::Path;

fn action_from(toml: &str) -> FileAction {
    let actions = parse_actions(toml, Path::new("actions.toml"));
    assert_eq!(actions.len(), 1);
    actions.into_iter().next().expect("one action")
}

#[test]
fn missing_file_yields_no_actions() {
    let actions = super::load_actions_from(Path::new("/no/such/strata-actions.toml"));
    assert!(actions.is_empty());
}

#[test]
fn invalid_toml_yields_no_actions() {
    let actions = parse_actions("this is not toml {", Path::new("actions.toml"));
    assert!(actions.is_empty());
}

#[test]
fn skips_incomplete_and_duplicate_ids() {
    let actions = parse_actions(
        r#"
[[actions]]
id = "ok"
label = "Send"
command = ["localsend"]

[[actions]]
id = ""
label = "Bad"
command = ["true"]

[[actions]]
id = "empty-label"
label = "   "
command = ["true"]

[[actions]]
id = "no-command"
label = "Nope"
command = []

[[actions]]
id = "ok"
label = "Duplicate"
command = ["other"]
"#,
        Path::new("actions.toml"),
    );
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].id, "ok");
    assert_eq!(actions[0].label, "Send");
}

#[test]
fn unknown_icon_falls_back_to_external_link() {
    let action = action_from(
        r#"
[[actions]]
id = "send"
label = "Send"
icon = "not-a-real-icon"
command = ["localsend"]
"#,
    );
    assert_eq!(action.icon, crate::assets::icons::EXTERNAL_LINK);
}

#[test]
fn appends_paths_when_command_has_no_tokens() {
    let argv = build_argv(
        &["localsend".into(), "--headless".into(), "send".into()],
        &[Path::new("/tmp/a"), Path::new("/tmp/b")],
    )
    .expect("argv");
    assert_eq!(
        argv.iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        ["localsend", "--headless", "send", "/tmp/a", "/tmp/b"]
    );
}

#[test]
fn expands_paths_token_in_place() {
    let argv = build_argv(
        &[
            "tool".into(),
            "--files".into(),
            "{paths}".into(),
            "--go".into(),
        ],
        &[Path::new("/a"), Path::new("/b")],
    )
    .expect("argv");
    assert_eq!(
        argv.iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        ["tool", "--files", "/a", "/b", "--go"]
    );
}

#[test]
fn single_path_token_rejects_multiple_paths() {
    assert!(uses_single_path_token(&["sync".into(), "{path}".into()]));
    assert!(
        build_argv(
            &["sync".into(), "{path}".into()],
            &[Path::new("/a"), Path::new("/b")]
        )
        .is_none()
    );
    let argv = build_argv(&["sync".into(), "{path}".into()], &[Path::new("/a")]).expect("argv");
    assert_eq!(
        argv.iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        ["sync", "/a"]
    );
}

#[test]
fn target_filters_files_and_folders() {
    let files_only = action_from(
        r#"
[[actions]]
id = "files"
label = "Files"
command = ["true"]
targets = "files"
"#,
    );
    let folders_only = action_from(
        r#"
[[actions]]
id = "folders"
label = "Folders"
command = ["true"]
targets = "folders"
"#,
    );
    assert!(files_only.matches(&[ActionKind::File], 1));
    assert!(!files_only.matches(&[ActionKind::Folder], 1));
    assert!(!files_only.matches(&[ActionKind::File, ActionKind::Folder], 2));
    assert!(folders_only.matches(&[ActionKind::Folder], 1));
    assert!(!folders_only.matches(&[ActionKind::File], 1));
}

#[test]
fn single_path_token_hides_multi_selection() {
    let action = action_from(
        r#"
[[actions]]
id = "one"
label = "One"
command = ["echo", "{path}"]
"#,
    );
    assert!(action.matches(&[ActionKind::File], 1));
    assert!(!action.matches(&[ActionKind::File], 2));
}

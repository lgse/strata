// SPDX-License-Identifier: MIT

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::*;

const PYTHON_ACTION: &str = r#"
schema_version = 1
id = "resize-images"
name = "Resize images"
description = "Create smaller copies of the selected images."
icon = "image"
menu = "submenu"

[when]
kinds = ["file"]
extensions = ["png", "jpg"]

[run]
runtime = "python"
entrypoint = "main.py"
mode = "per-item"
on_error = "continue"
"#;

const COMMAND_ACTION: &str = r#"
schema_version = 1
id = "build-project"
name = "Build project"
menu = "top"

[when]
kinds = ["folder"]
min_items = 1
max_items = 1

[run]
runtime = "command"
program = "make"
args = ["-C", "{path}"]
mode = "per-item"
working_directory = "parent"
confirm = true
"#;

const BASH_ACTION: &str = r#"
schema_version = 1
id = "checksums"
name = "Generate checksums"

[when]

[run]
runtime = "bash"
entrypoint = "run.sh"
"#;

fn python_definition() -> ActionDefinition {
    ActionDefinition::parse(PYTHON_ACTION).expect("python action parses")
}

fn command_definition() -> ActionDefinition {
    ActionDefinition::parse(COMMAND_ACTION).expect("command action parses")
}

fn run_manifest(run: &str) -> String {
    format!(
        "schema_version = 1\nid = \"a1\"\nname = \"Action\"\n\n[when]\nkinds = [\"file\"]\n\n[run]\n{run}\n"
    )
}

#[test]
fn parses_the_agreed_python_manifest() {
    let definition = python_definition();
    assert_eq!(definition.id, "resize-images");
    assert_eq!(definition.name, "Resize images");
    assert_eq!(definition.menu, MenuPlacement::Submenu);
    assert_eq!(definition.icon.as_deref(), Some("image"));
    assert!(definition.enabled);
    assert_eq!(definition.when.kinds, vec![InputKind::File]);
    assert_eq!(definition.when.extensions, vec!["png", "jpg"]);
    assert_eq!(definition.when.min_items, 1);
    assert_eq!(definition.when.max_items, None);
    assert_eq!(definition.run.runtime, ActionRuntime::Python);
    assert_eq!(definition.run.script_entrypoint(), Some("main.py"));
    assert_eq!(definition.run.mode, ExecutionMode::PerItem);
    assert_eq!(definition.run.on_error, ErrorPolicy::Continue);
    assert!(!definition.run.confirm);
}

#[test]
fn defaults_keep_a_minimal_manifest_usable() {
    let definition = ActionDefinition::parse(BASH_ACTION).expect("minimal manifest parses");
    assert!(definition.enabled);
    assert_eq!(definition.menu, MenuPlacement::Submenu);
    assert_eq!(definition.when.kinds, Vec::new());
    assert_eq!(definition.when.min_items, 1);
    assert_eq!(
        definition.run.mode,
        ExecutionMode::WholeSelection,
        "a single invocation is the safer default"
    );
    assert_eq!(definition.run.on_error, ErrorPolicy::Continue);
    assert_eq!(definition.run.working_directory, WorkingDirectory::Parent);
}

#[test]
fn a_generated_manifest_round_trips() {
    for definition in [python_definition(), command_definition()] {
        let manifest = definition.to_manifest().expect("manifest renders");
        assert!(manifest.starts_with(MANIFEST_HEADER));
        let reparsed = ActionDefinition::parse(&manifest).expect("generated manifest parses");
        assert_eq!(reparsed, definition);
        assert!(
            manifest.contains("[when]") && manifest.contains("[run]"),
            "generated manifest keeps readable sections: {manifest}"
        );
    }
}

#[test]
fn generated_manifests_omit_empty_sections() {
    let definition = ActionDefinition::parse(BASH_ACTION).expect("minimal manifest parses");
    let manifest = definition.to_manifest().expect("manifest renders");
    assert!(!manifest.contains("extensions"));
    assert!(!manifest.contains("mime_types"));
    assert!(!manifest.contains("max_items"));
    assert!(!manifest.contains("description"));
}

#[test]
fn rejects_unsupported_schema_and_unknown_fields() {
    assert_eq!(
        ActionDefinition::parse(
            "schema_version = 99\nid = \"a1\"\nname = \"Action\"\n[when]\n[run]\nruntime = \"bash\"\nentrypoint = \"run.sh\"\n"
        ),
        Err(ActionError::UnsupportedSchema { found: 99 })
    );

    let typo = ActionDefinition::parse(
        "schema_version = 1\nid = \"a1\"\nname = \"Action\"\nshell = true\n[when]\n[run]\nruntime = \"bash\"\nentrypoint = \"run.sh\"\n",
    );
    assert!(
        matches!(typo, Err(ActionError::Toml(_))),
        "typos in the manifest must not be silently ignored"
    );

    let unknown_section = ActionDefinition::parse(
        "schema_version = 1\nid = \"a1\"\nname = \"Action\"\n[when]\n[run]\nruntime = \"bash\"\nentrypoint = \"run.sh\"\n[env]\nFOO = \"bar\"\n",
    );
    assert!(matches!(unknown_section, Err(ActionError::Toml(_))));
}

#[test]
fn rejects_invalid_identity_and_text() {
    for id in [
        "",
        "-leading",
        "Uppercase",
        "with space",
        "with/slash",
        "..",
        "with:colon",
    ] {
        let source = format!(
            "schema_version = 1\nid = \"{id}\"\nname = \"Action\"\n[when]\n[run]\nruntime = \"bash\"\nentrypoint = \"run.sh\"\n"
        );
        assert_eq!(
            ActionDefinition::parse(&source),
            Err(ActionError::InvalidId(id.to_owned())),
            "id {id:?} must be rejected"
        );
    }
    let long_id = "a".repeat(MAX_ACTION_ID_CHARS + 1);
    assert!(
        ActionDefinition::parse(&format!(
            "schema_version = 1\nid = \"{long_id}\"\nname = \"Action\"\n[when]\n[run]\nruntime = \"bash\"\nentrypoint = \"run.sh\"\n"
        ))
        .is_err()
    );

    for name in ["", " ", "padded ", " padded"] {
        let source = format!(
            "schema_version = 1\nid = \"a1\"\nname = \"{name}\"\n[when]\n[run]\nruntime = \"bash\"\nentrypoint = \"run.sh\"\n"
        );
        assert_eq!(
            ActionDefinition::parse(&source),
            Err(ActionError::InvalidName)
        );
    }
    assert_eq!(
        ActionDefinition::parse(
            "schema_version = 1\nid = \"a1\"\nname = \"nested\\nline\"\n[when]\n[run]\nruntime = \"bash\"\nentrypoint = \"run.sh\"\n"
        ),
        Err(ActionError::InvalidName)
    );
    assert!(matches!(
        ActionDefinition::parse(
            "schema_version = 1\nid = \"a1\"\nname = \"nested\nline\"\n[when]\n[run]\nruntime = \"bash\"\nentrypoint = \"run.sh\"\n"
        ),
        Err(ActionError::Toml(_))
    ));
    let long_name = "n".repeat(MAX_ACTION_NAME_CHARS + 1);
    assert_eq!(
        ActionDefinition::parse(&format!(
            "schema_version = 1\nid = \"a1\"\nname = \"{long_name}\"\n[when]\n[run]\nruntime = \"bash\"\nentrypoint = \"run.sh\"\n"
        )),
        Err(ActionError::InvalidName)
    );

    assert_eq!(
        ActionDefinition::parse(
            "schema_version = 1\nid = \"a1\"\nname = \"Action\"\nicon = \"../escape\"\n[when]\n[run]\nruntime = \"bash\"\nentrypoint = \"run.sh\"\n"
        ),
        Err(ActionError::InvalidIcon)
    );
}

#[test]
fn rejects_invalid_entrypoints_and_programs() {
    for entrypoint in [".", "..", "sub/main.py", "/abs.py", "-dashed.py"] {
        let source = format!(
            "schema_version = 1\nid = \"a1\"\nname = \"Action\"\n[when]\n[run]\nruntime = \"python\"\nentrypoint = \"{entrypoint}\"\n"
        );
        assert_eq!(
            ActionDefinition::parse(&source),
            Err(ActionError::InvalidEntrypoint(entrypoint.to_owned())),
            "entrypoint {entrypoint:?} must be rejected"
        );
    }
    for program in ["", "-rf"] {
        let source = format!(
            "schema_version = 1\nid = \"a1\"\nname = \"Action\"\n[when]\n[run]\nruntime = \"command\"\nprogram = \"{program}\"\n"
        );
        assert!(
            matches!(
                ActionDefinition::parse(&source),
                Err(ActionError::InvalidProgram(_))
            ),
            "program {program:?} must be rejected"
        );
    }
    assert!(matches!(
        ActionDefinition::parse(
            "schema_version = 1\nid = \"a1\"\nname = \"Action\"\n[when]\n[run]\nruntime = \"command\"\nprogram = \"nul\\u0000name\"\n"
        ),
        Err(ActionError::InvalidProgram(_))
    ));
    assert_eq!(
        ActionDefinition::parse(
            "schema_version = 1\nid = \"a1\"\nname = \"Action\"\n[when]\n[run]\nruntime = \"command\"\nprogram = \"make\"\nentrypoint = \"main.py\"\n"
        ),
        Err(ActionError::UnexpectedEntrypoint)
    );
    assert_eq!(
        ActionDefinition::parse(
            "schema_version = 1\nid = \"a1\"\nname = \"Action\"\n[when]\n[run]\nruntime = \"python\"\n"
        ),
        Err(ActionError::MissingEntrypoint)
    );
    assert_eq!(
        ActionDefinition::parse(
            "schema_version = 1\nid = \"a1\"\nname = \"Action\"\n[when]\n[run]\nruntime = \"command\"\n"
        ),
        Err(ActionError::MissingProgram)
    );
    assert_eq!(
        ActionDefinition::parse(
            "schema_version = 1\nid = \"a1\"\nname = \"Action\"\n[when]\n[run]\nruntime = \"python\"\nentrypoint = \"main.py\"\nprogram = \"python3\"\n"
        ),
        Err(ActionError::UnexpectedProgram)
    );
    assert!(
        ActionDefinition::parse(&run_manifest(
            "runtime = \"bash\"\nentrypoint = \"run.sh\"\nargs = [\"--flag\"]\n"
        ))
        .is_err()
    );
}

#[test]
fn validates_conditions() {
    assert_eq!(
        ActionDefinition::parse(
            "schema_version = 1\nid = \"a1\"\nname = \"Action\"\n[when]\nextensions = [\".png\"]\n[run]\nruntime = \"bash\"\nentrypoint = \"run.sh\"\n"
        ),
        Err(ActionError::InvalidExtension(".png".to_owned()))
    );
    assert_eq!(
        ActionDefinition::parse(
            "schema_version = 1\nid = \"a1\"\nname = \"Action\"\n[when]\nmime_types = [\"image\"]\n[run]\nruntime = \"bash\"\nentrypoint = \"run.sh\"\n"
        ),
        Err(ActionError::InvalidMimeType("image".to_owned()))
    );
    assert_eq!(
        ActionDefinition::parse(
            "schema_version = 1\nid = \"a1\"\nname = \"Action\"\n[when]\nmin_items = 0\n[run]\nruntime = \"bash\"\nentrypoint = \"run.sh\"\n"
        ),
        Err(ActionError::InvalidItemRange)
    );
    assert_eq!(
        ActionDefinition::parse(
            "schema_version = 1\nid = \"a1\"\nname = \"Action\"\n[when]\nmin_items = 3\nmax_items = 2\n[run]\nruntime = \"bash\"\nentrypoint = \"run.sh\"\n"
        ),
        Err(ActionError::InvalidItemRange)
    );
    let long_extensions = (0..MAX_EXTENSIONS + 1)
        .map(|index| format!("\"e{index}\""))
        .collect::<Vec<_>>()
        .join(", ");
    assert_eq!(
        ActionDefinition::parse(&format!(
            "schema_version = 1\nid = \"a1\"\nname = \"Action\"\n[when]\nextensions = [{long_extensions}]\n[run]\nruntime = \"bash\"\nentrypoint = \"run.sh\"\n"
        )),
        Err(ActionError::TooManyConditions("extensions"))
    );
}

#[test]
fn validates_argument_tokens_against_the_mode() {
    assert_eq!(
        ActionDefinition::parse(&run_manifest(
            "runtime = \"command\"\nprogram = \"cat\"\nargs = [\"{path}\"]\nmode = \"whole-selection\"\n"
        )),
        Err(ActionError::PathTokenInPerItem)
    );
    assert_eq!(
        ActionDefinition::parse(&run_manifest(
            "runtime = \"command\"\nprogram = \"cat\"\nargs = [\"{paths}\"]\nmode = \"per-item\"\n"
        )),
        Err(ActionError::PathsTokenInWholeSelection)
    );
    assert_eq!(
        ActionDefinition::parse(&run_manifest(
            "runtime = \"command\"\nprogram = \"cat\"\nargs = [\"prefix-{path}\"]\nmode = \"per-item\"\n"
        )),
        Err(ActionError::InvalidArgument("prefix-{path}".to_owned()))
    );
    assert_eq!(
        ActionDefinition::parse(&run_manifest(
            "runtime = \"command\"\nprogram = \"cat\"\nargs = [\"{unknown}\"]\n"
        )),
        Err(ActionError::InvalidArgument("{unknown}".to_owned()))
    );
    assert_eq!(
        ActionDefinition::parse(&run_manifest(
            "runtime = \"command\"\nprogram = \"cat\"\nargs = [\"{}\"]\n"
        )),
        Err(ActionError::InvalidArgument("{}".to_owned()))
    );

    let definition = ActionDefinition::parse(&run_manifest(
        "runtime = \"command\"\nprogram = \"cat\"\nargs = [\"--flag\", \"{parent}\", \"\"]\n",
    ))
    .expect("literal arguments and {parent} are allowed");
    assert_eq!(definition.run.args.len(), 3);
    assert_eq!(
        definition.run.argument_tokens(),
        Ok(vec![
            ArgumentToken::Literal("--flag".to_owned()),
            ArgumentToken::Parent,
            ArgumentToken::Literal(String::new()),
        ])
    );

    let too_many = (0..MAX_ARGUMENTS + 1)
        .map(|_| "\"x\"")
        .collect::<Vec<_>>()
        .join(", ");
    assert_eq!(
        ActionDefinition::parse(&run_manifest(&format!(
            "runtime = \"command\"\nprogram = \"cat\"\nargs = [{too_many}]\n"
        ))),
        Err(ActionError::TooManyArguments)
    );
    let long_argument = "x".repeat(MAX_ARGUMENT_CHARS + 1);
    assert_eq!(
        ActionDefinition::parse(&run_manifest(&format!(
            "runtime = \"command\"\nprogram = \"cat\"\nargs = [\"{long_argument}\"]\n"
        ))),
        Err(ActionError::ArgumentTooLong)
    );
}

#[test]
fn parses_shebangs_including_env_indirection() {
    assert_eq!(
        interpreter_from_source("#!/bin/bash\necho hi\n"),
        Some(Interpreter {
            program: "/bin/bash".to_owned(),
            arguments: Vec::new()
        })
    );
    assert_eq!(
        interpreter_from_source("#!/usr/bin/env python3\nprint(1)\n"),
        Some(Interpreter {
            program: "python3".to_owned(),
            arguments: Vec::new()
        })
    );
    assert_eq!(
        interpreter_from_source("#!/usr/bin/env -S bash -e\nset -e\n"),
        Some(Interpreter {
            program: "bash".to_owned(),
            arguments: vec!["-e".to_owned()]
        })
    );
    assert_eq!(
        interpreter_from_source("#!/usr/bin/env PYTHONPATH=/tmp python3 -u\n"),
        Some(Interpreter {
            program: "python3".to_owned(),
            arguments: vec!["-u".to_owned()]
        })
    );
    assert_eq!(
        interpreter_from_source("#!/usr/bin/python3.12 -B\n"),
        Some(Interpreter {
            program: "/usr/bin/python3.12".to_owned(),
            arguments: vec!["-B".to_owned()]
        })
    );
    assert_eq!(
        interpreter_from_source("#!/usr/bin/env\n"),
        None,
        "env without a program names no interpreter"
    );
    assert_eq!(interpreter_from_source("#!/\n"), None);
    assert_eq!(interpreter_from_source("print(1)\n"), None);
    assert_eq!(
        interpreter_from_source("\n#!/bin/bash\n"),
        None,
        "a shebang is only line one"
    );
    assert!(!has_shebang("print(1)\n"));
    assert!(has_shebang("#!/bin/sh\r\necho hi\r\n"));
    assert_eq!(
        interpreter_from_source("#!/bin/sh\r\necho hi\r\n"),
        Some(Interpreter {
            program: "/bin/sh".to_owned(),
            arguments: Vec::new()
        })
    );
}

#[test]
fn classifies_interpreter_families() {
    assert_eq!(interpreter_family("python3"), InterpreterFamily::Python);
    assert_eq!(
        interpreter_family("/usr/bin/python"),
        InterpreterFamily::Python
    );
    assert_eq!(interpreter_family("python3.12t"), InterpreterFamily::Python);
    assert_eq!(interpreter_family("/bin/bash"), InterpreterFamily::Shell);
    assert_eq!(interpreter_family("sh"), InterpreterFamily::Shell);
    assert_eq!(interpreter_family("/usr/bin/env"), InterpreterFamily::Other);
    assert_eq!(
        interpreter_family("/usr/bin/perl"),
        InterpreterFamily::Other
    );
    assert_eq!(interpreter_family("node"), InterpreterFamily::Other);
}

#[test]
fn accepts_a_matching_shebang_and_rejects_a_conflicting_one() {
    let python = python_definition();
    assert_eq!(
        python.interpreter_for_source("#!/usr/bin/env python3\nprint(1)\n"),
        Ok(Some(Interpreter {
            program: "python3".to_owned(),
            arguments: Vec::new()
        }))
    );
    assert_eq!(
        python.interpreter_for_source("print(1)\n"),
        Ok(None),
        "a script without a shebang uses the runtime default"
    );
    assert_eq!(
        python.interpreter_for_source("#!/bin/bash\necho hi\n"),
        Err(ActionError::ShebangMismatch {
            declared: "#!/bin/bash".to_owned(),
            runtime: ActionRuntime::Python
        })
    );
    assert_eq!(
        python.interpreter_for_source("#!/usr/bin/perl\n"),
        Err(ActionError::ShebangMismatch {
            declared: "#!/usr/bin/perl".to_owned(),
            runtime: ActionRuntime::Python
        })
    );
    assert_eq!(
        python.interpreter_for_source("#!/\n"),
        Err(ActionError::InvalidShebang)
    );

    let bash = ActionDefinition::parse(BASH_ACTION).expect("bash action parses");
    assert!(bash.interpreter_for_source("#!/bin/sh\n").is_ok());
    assert_eq!(
        bash.interpreter_for_source("#!/usr/bin/env python3\n"),
        Err(ActionError::ShebangMismatch {
            declared: "#!python3".to_owned(),
            runtime: ActionRuntime::Bash
        })
    );

    assert_eq!(
        command_definition().interpreter_for_source("#!/usr/bin/perl\n"),
        Ok(None),
        "command actions use their declared program, not a shebang"
    );
}

#[test]
fn matching_requires_every_input_to_satisfy_the_conditions() {
    let images = ActionDefinition::parse(&run_manifest(
        "runtime = \"command\"\nprogram = \"convert\"\nargs = [\"{paths}\"]\n",
    ))
    .expect("command action parses");

    let png = ActionInput::file("a.png", Some("image/png"));
    let jpg = ActionInput::file("b.jpg", Some("image/jpeg"));
    let folder = ActionInput::folder("folder");
    assert!(images.when.matches(std::slice::from_ref(&png)));
    assert!(images.when.matches(&[png.clone(), jpg.clone()]));
    assert!(
        !images.when.matches(&[png.clone(), folder]),
        "a mixed selection must not silently drop entries"
    );
    assert!(!images.when.matches(&[]));
}

#[test]
fn matching_covers_kinds_extensions_mime_types_and_counts() {
    let conditions = ActionConditions {
        kinds: vec![InputKind::File],
        extensions: vec!["png".to_owned()],
        mime_types: vec!["image/*".to_owned()],
        min_items: 1,
        max_items: Some(2),
    };
    assert!(conditions.matches(&[ActionInput::file("Photo.PNG", Some("image/png"))]));
    assert!(
        !conditions.matches(&[ActionInput::file("photo.png", Some("application/pdf"))]),
        "a content type that contradicts the rules must not match"
    );
    assert!(
        !conditions.matches(&[ActionInput::file("photo.png", None)]),
        "unknown content types cannot satisfy a mime rule"
    );
    let three = vec![
        ActionInput::file("a.png", Some("image/png")),
        ActionInput::file("b.png", Some("image/png")),
        ActionInput::file("c.png", Some("image/png")),
    ];
    assert!(
        !conditions.matches(&three),
        "max_items must bound the selection"
    );
    assert!(!conditions.matches(&[ActionInput::file("photo.jpeg", Some("image/jpeg"))]));
    assert!(!conditions.matches(&[ActionInput::folder("photo.png")]));
    assert_eq!(
        ActionInput::file(".bashrc", None).extension(),
        None,
        "a dotfile has no extension"
    );
    assert_eq!(
        ActionInput::file("archive.tar.gz", None)
            .extension()
            .as_deref(),
        Some("gz")
    );
    assert_eq!(
        ActionInput::file("archive.TAR.GZ", None)
            .extension()
            .as_deref(),
        Some("gz")
    );

    let folders_only = ActionConditions {
        kinds: vec![InputKind::Folder],
        ..ActionConditions::default()
    };
    assert!(folders_only.matches(&[ActionInput::folder("src")]));
    assert!(!folders_only.matches(&[ActionInput::file("src", None)]));
}

#[test]
fn matching_accepts_native_names_that_are_not_utf8() {
    use std::os::unix::ffi::OsStringExt;
    let input = ActionInput {
        kind: InputKind::File,
        name: OsString::from_vec(b"pho\xffto.png".to_vec()),
        content_type: Some("image/png".to_owned()),
    };
    assert_eq!(input.extension().as_deref(), Some("png"));
    let conditions = ActionConditions {
        extensions: vec!["png".to_owned()],
        ..ActionConditions::default()
    };
    assert!(conditions.matches(&[input]));
}

#[test]
fn expands_command_arguments_without_shell_interpolation() {
    let tokens = vec![
        ArgumentToken::Literal("--flag".to_owned()),
        ArgumentToken::Parent,
        ArgumentToken::Paths,
    ];
    let parent = Path::new("/home/user/Pictures");
    let inputs = [
        PathBuf::from("/home/user/Pictures/a b.png"),
        PathBuf::from("/home/user/Pictures/-rf"),
    ];
    assert_eq!(
        expand_arguments(&tokens, &inputs, parent),
        Ok(vec![
            OsString::from("--flag"),
            OsString::from("/home/user/Pictures"),
            OsString::from("/home/user/Pictures/a b.png"),
            OsString::from("/home/user/Pictures/-rf"),
        ])
    );

    let single = [ArgumentToken::Path];
    assert_eq!(
        expand_arguments(&single, &inputs, parent),
        Err(ArgumentTokenError::PathCount(2))
    );
    assert_eq!(
        expand_arguments(&single, &[], parent),
        Err(ArgumentTokenError::PathCount(0))
    );
    assert_eq!(
        expand_arguments(
            &[ArgumentToken::Path],
            &[PathBuf::from("relative.txt")],
            parent
        ),
        Err(ArgumentTokenError::RelativePath(PathBuf::from(
            "relative.txt"
        )))
    );
}

#[test]
fn suggests_readable_ids() {
    assert_eq!(suggest_id("Resize images"), "resize-images");
    assert_eq!(
        suggest_id("Convert to WebP  (fast)"),
        "convert-to-webp-fast"
    );
    assert_eq!(suggest_id("   "), "action");
    assert_eq!(suggest_id("///"), "action");
    assert_eq!(suggest_id("Build"), "build");
    assert!(suggest_id(&"n".repeat(200)).len() <= MAX_ACTION_ID_CHARS);
    assert!(suggest_id("2x resize").starts_with('2'));
}

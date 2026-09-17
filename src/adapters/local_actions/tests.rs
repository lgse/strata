// SPDX-License-Identifier: MIT

//! Store tests: everything here treats the actions directory as untrusted
//! input, because it is editable by hand and by import.

use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};

use crate::model::ExecutionMode;
use crate::services::actions::{ActionScript, ActionStore, ActionStoreError, ActionWriteRequest};

use super::*;

const PYTHON_MANIFEST: &str = r#"
schema_version = 1
id = "resize-images"
name = "Resize images"

[when]
extensions = ["png"]

[run]
runtime = "python"
entrypoint = "main.py"
mode = "per-item"
"#;

fn store(directory: &tempfile::TempDir) -> Rc<LocalActionStore> {
    LocalActionStore::at(directory.path().join("actions"))
}

fn write_action(
    store: &LocalActionStore,
    manifest: &str,
    script: Option<(&str, &str)>,
) -> Result<(), ActionStoreError> {
    let definition = crate::model::ActionDefinition::parse(manifest)?;
    store.write(&ActionWriteRequest {
        definition,
        script: script.map(|(file_name, contents)| ActionScript {
            file_name: file_name.to_owned(),
            contents: contents.to_owned(),
        }),
    })
}

fn modes(path: &Path) -> u32 {
    fs::symlink_metadata(path)
        .expect("path exists")
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn writes_and_loads_a_python_action() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = store(&fixture);
    write_action(
        &store,
        PYTHON_MANIFEST,
        Some(("main.py", "#!/usr/bin/env python3\nprint('hi')\n")),
    )
    .expect("action writes");

    let catalog = store.load();
    assert!(catalog.failures().is_empty(), "{:?}", catalog.failures());
    let handle = catalog.get("resize-images").expect("action loads");
    assert_eq!(handle.name(), "Resize images");
    assert_eq!(handle.directory, store.root().join("resize-images"));
    assert_eq!(handle.definition.run.mode, ExecutionMode::PerItem);
    let program = handle.program().expect("python3 is available");
    match program {
        ActionProgram::Script {
            interpreter,
            script,
            family,
            ..
        } => {
            assert!(
                Path::new(interpreter).is_absolute(),
                "the interpreter is resolved once, at load time"
            );
            assert!(script.ends_with("resize-images/main.py"));
            assert_eq!(*family, InterpreterFamily::Python);
        }
        ActionProgram::Command { .. } => panic!("expected a script program"),
    }

    assert_eq!(
        modes(store.root()),
        0o700,
        "the actions directory is private"
    );
    assert_eq!(
        modes(&store.root().join("resize-images")),
        0o700,
        "each action directory is private"
    );
    assert_eq!(
        modes(&store.root().join("resize-images/action.toml")),
        0o600,
        "manifests are owner-only"
    );
    assert_eq!(
        modes(&store.root().join("resize-images/main.py")),
        0o600,
        "generated scripts are owner-only"
    );
}

#[test]
fn a_missing_actions_directory_is_an_empty_catalog() {
    let fixture = tempfile::tempdir().expect("fixture");
    let catalog = store(&fixture).load();
    assert!(catalog.actions().is_empty());
    assert!(catalog.failures().is_empty());
}

#[test]
fn refuses_a_manifest_whose_id_disagrees_with_its_directory() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = store(&fixture);
    write_action(
        &store,
        PYTHON_MANIFEST,
        Some(("main.py", "#!/usr/bin/env python3\n")),
    )
    .expect("action writes");
    fs::rename(
        store.root().join("resize-images"),
        store.root().join("renamed"),
    )
    .expect("rename");

    let catalog = store.load();
    assert!(catalog.actions().is_empty());
    assert!(
        matches!(
            catalog.failures()[0].error,
            ActionStoreError::IdMismatch { .. }
        ),
        "{:?}",
        catalog.failures()
    );
}

#[test]
fn reports_actions_with_problems_instead_of_hiding_them() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = store(&fixture);
    // A directory without a manifest.
    fs::create_dir_all(store.root().join("empty")).expect("directory");
    // A manifest whose entrypoint is missing. The store refuses to create this
    // through `write`, so it is reproduced the way a hand edit would.
    fs::create_dir_all(store.root().join("resize-images")).expect("directory");
    fs::write(
        store.root().join("resize-images/action.toml"),
        PYTHON_MANIFEST,
    )
    .expect("manifest");
    // A directory whose manifest is invalid TOML.
    fs::create_dir_all(store.root().join("broken")).expect("directory");
    fs::write(
        store.root().join("broken/action.toml"),
        "schema_version = nope\n",
    )
    .expect("manifest");
    // Files and hidden entries are ignored entirely.
    fs::write(store.root().join("notes.txt"), "hello").expect("file");
    fs::write(store.root().join(".swap"), "hello").expect("file");

    let catalog = store.load();
    let mut failures: Vec<_> = catalog
        .failures()
        .iter()
        .map(|failure| failure.directory.clone())
        .collect();
    failures.sort();
    assert_eq!(failures, vec!["broken", "empty", "resize-images"]);
    assert!(catalog.actions().is_empty());
    let missing = catalog
        .failures()
        .iter()
        .find(|failure| failure.directory == "resize-images")
        .expect("entrypoint failure");
    assert!(matches!(
        missing.error,
        ActionStoreError::MissingEntrypoint(_)
    ));
    assert!(
        catalog
            .failures()
            .iter()
            .any(|failure| failure.directory == "empty"
                && matches!(failure.error, ActionStoreError::NotAnActionDirectory(_))),
    );
}

#[test]
fn refuses_links_and_non_regular_files() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = store(&fixture);
    write_action(
        &store,
        PYTHON_MANIFEST,
        Some(("main.py", "#!/usr/bin/env python3\n")),
    )
    .expect("action writes");

    // A symlinked action directory is not followed.
    let outside = fixture.path().join("outside");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(outside.join("action.toml"), PYTHON_MANIFEST).expect("manifest");
    fs::write(outside.join("main.py"), "#!/usr/bin/env python3\n").expect("script");
    symlink(&outside, store.root().join("linked-dir")).expect("symlink");
    let catalog = store.load();
    assert!(
        catalog
            .failures()
            .iter()
            .any(|failure| failure.directory == "linked-dir"),
        "a symlinked action directory must not load from outside: {:?}",
        catalog.failures()
    );

    // A symlinked entrypoint is refused rather than executed.
    let action = store.root().join("resize-images");
    fs::remove_file(action.join("main.py")).expect("remove");
    symlink("/bin/echo", action.join("main.py")).expect("symlink");
    let error = store
        .load()
        .failures()
        .iter()
        .find(|failure| failure.directory == "resize-images")
        .map(|failure| failure.error.clone())
        .expect("failure");
    assert!(
        matches!(error, ActionStoreError::NotARegularFile(_)),
        "{error:?}"
    );

    // A symlinked manifest is refused too.
    fs::remove_file(action.join("main.py")).expect("remove");
    fs::write(action.join("main.py"), "#!/usr/bin/env python3\n").expect("script");
    fs::remove_file(action.join("action.toml")).expect("remove");
    symlink(outside.join("action.toml"), action.join("action.toml")).expect("symlink");
    assert!(
        store
            .load()
            .failures()
            .iter()
            .any(|failure| matches!(failure.error, ActionStoreError::NotARegularFile(_))),
    );
}

#[test]
fn reports_an_unavailable_interpreter_without_hiding_the_action() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = store(&fixture);
    write_action(
        &store,
        PYTHON_MANIFEST,
        Some((
            "main.py",
            "#!/nonexistent/python-really-not-here\nprint('hi')\n",
        )),
    )
    .expect("write accepts a shebang whose interpreter may be absent");

    let catalog = store.load();
    assert!(catalog.failures().is_empty(), "{:?}", catalog.failures());
    let handle = catalog.get("resize-images").expect("action loads");
    assert!(!handle.is_available());
    let reason = handle.unavailable_reason().expect("reason");
    assert!(
        reason.contains("python-really-not-here"),
        "the reason names the missing interpreter: {reason}"
    );
}

#[test]
fn rejects_a_shebang_that_contradicts_the_runtime() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = store(&fixture);
    let error = write_action(
        &store,
        PYTHON_MANIFEST,
        Some(("main.py", "#!/bin/bash\necho not python\n")),
    )
    .expect_err("mismatched shebang is refused");
    assert!(
        matches!(
            error,
            ActionStoreError::Invalid(crate::model::ActionError::ShebangMismatch { .. })
        ),
        "{error:?}"
    );
    assert!(
        !store.root().join("resize-images/action.toml").exists(),
        "a refused write must not leave a manifest behind"
    );
}

#[test]
fn rejects_a_script_name_that_disagrees_with_the_entrypoint() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = store(&fixture);
    let definition = crate::model::ActionDefinition::parse(PYTHON_MANIFEST).expect("definition");
    let error = store
        .write(&ActionWriteRequest {
            definition,
            script: Some(ActionScript {
                file_name: "../escape.py".to_owned(),
                contents: "#!/usr/bin/env python3\n".to_owned(),
            }),
        })
        .expect_err("path traversal is refused");
    assert!(
        matches!(
            error,
            ActionStoreError::Invalid(crate::model::ActionError::InvalidEntrypoint(_))
        ),
        "{error:?}"
    );
    assert!(!fixture.path().join("escape.py").exists());
}

#[test]
fn metadata_only_edits_keep_the_existing_script_and_get_validated() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = store(&fixture);
    write_action(
        &store,
        PYTHON_MANIFEST,
        Some(("main.py", "#!/usr/bin/env python3\n")),
    )
    .expect("action writes");

    let renamed = PYTHON_MANIFEST.replace("Resize images", "Resized images");
    write_action(&store, &renamed, None).expect("metadata-only edit succeeds");
    let handle = store.load();
    assert_eq!(
        handle.get("resize-images").map(|handle| handle.name()),
        Some("Resized images")
    );
    assert!(
        store.root().join("resize-images/main.py").is_file(),
        "the script is untouched"
    );

    // The same edit with the script removed fails instead of pointing at nothing.
    fs::remove_file(store.root().join("resize-images/main.py")).expect("remove");
    let error = write_action(&store, &renamed, None).expect_err("missing script");
    assert!(
        matches!(error, ActionStoreError::MissingEntrypoint(_)),
        "{error:?}"
    );
}

#[test]
fn delete_removes_one_action_and_refuses_the_rest() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = store(&fixture);
    write_action(
        &store,
        PYTHON_MANIFEST,
        Some(("main.py", "#!/usr/bin/env python3\n")),
    )
    .expect("action writes");
    fs::create_dir_all(store.root().join("unrelated")).expect("directory");

    assert!(matches!(
        store.delete("../escape"),
        Err(ActionStoreError::Invalid(_))
    ));
    assert!(matches!(
        store.delete("missing"),
        Err(ActionStoreError::NotFound(_))
    ));
    assert!(
        matches!(store.delete("unrelated"), Ok(())),
        "any action directory can be removed"
    );
    store.delete("resize-images").expect("action deletes");
    assert!(!store.root().join("resize-images").exists());

    // A symlinked directory is never followed for deletion.
    let outside = fixture.path().join("outside");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(outside.join("keep.txt"), "keep").expect("file");
    symlink(&outside, store.root().join("linked")).expect("symlink");
    assert!(store.delete("linked").is_err());
    assert!(outside.join("keep.txt").is_file(), "the target survives");
}

#[test]
fn import_copies_only_the_manifest_and_its_entrypoint_and_disables_it() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = store(&fixture);
    let source = fixture.path().join("incoming");
    fs::create_dir_all(source.join("extra")).expect("source");
    fs::write(source.join("action.toml"), PYTHON_MANIFEST).expect("manifest");
    fs::write(
        source.join("main.py"),
        "#!/usr/bin/env python3\nprint('hi')\n",
    )
    .expect("script");
    fs::write(source.join("extra/payload.bin"), "not copied").expect("extra");
    fs::write(source.join("notes.md"), "not copied").expect("notes");

    let id = store.import(&source).expect("import succeeds");
    assert_eq!(id, "resize-images");
    let action = store.root().join("resize-images");
    assert!(action.join("action.toml").is_file());
    assert!(action.join("main.py").is_file());
    assert!(
        !action.join("extra").exists() && !action.join("notes.md").exists(),
        "import must not carry unrelated files along"
    );
    let catalog = store.load();
    let handle = catalog.get(&id).expect("imported action loads");
    assert!(
        !handle.definition.enabled,
        "imported actions stay disabled until reviewed"
    );

    // A second import of the same action gets a fresh id instead of clobbering.
    let second = store.import(&source).expect("second import succeeds");
    assert_eq!(
        second, "resize-images-1",
        "a taken id gains a numeric suffix instead of overwriting"
    );
    assert!(
        !store
            .load()
            .get(&second)
            .expect("action")
            .definition
            .enabled
    );

    // Importing a directory without a manifest is refused.
    let empty = fixture.path().join("not-an-action");
    fs::create_dir_all(&empty).expect("directory");
    assert!(matches!(
        store.import(&empty),
        Err(ActionStoreError::NotAnActionDirectory(_))
    ));

    // A symlinked source is refused.
    let link = fixture.path().join("link");
    symlink(&source, &link).expect("symlink");
    assert!(store.import(&link).is_err());
}

#[test]
fn export_writes_a_portable_copy_and_refuses_to_overwrite() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = store(&fixture);
    write_action(
        &store,
        PYTHON_MANIFEST,
        Some(("main.py", "#!/usr/bin/env python3\nprint('hi')\n")),
    )
    .expect("action writes");
    let destination = fixture.path().join("exported");
    fs::create_dir_all(&destination).expect("destination");

    let target = store
        .export("resize-images", &destination)
        .expect("export succeeds");
    assert_eq!(target, destination.join("resize-images"));
    assert!(target.join("action.toml").is_file());
    assert!(target.join("main.py").is_file());
    let copied = fs::read_to_string(target.join("main.py")).expect("script");
    assert_eq!(copied, "#!/usr/bin/env python3\nprint('hi')\n");

    // The exported manifest is loadable on its own.
    let parsed = crate::model::ActionDefinition::parse(
        &fs::read_to_string(target.join("action.toml")).expect("manifest"),
    )
    .expect("exported manifest parses");
    assert_eq!(parsed.id, "resize-images");

    assert!(matches!(
        store.export("resize-images", &destination),
        Err(ActionStoreError::AlreadyExists(_))
    ));
    assert!(matches!(
        store.export("missing", &destination),
        Err(ActionStoreError::NotFound(_))
    ));
}

#[test]
fn a_command_action_resolves_its_program_and_reports_a_missing_one() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = store(&fixture);
    let manifest = r#"
schema_version = 1
id = "build"
name = "Build"

[when]

[run]
runtime = "command"
program = "definitely-not-a-real-program-xyz"
"#;
    write_action(&store, manifest, None).expect("command action writes");
    let catalog = store.load();
    let handle = catalog.get("build").expect("action loads");
    assert!(!handle.is_available());
    assert!(
        handle
            .unavailable_reason()
            .is_some_and(|reason| reason.contains("definitely-not-a-real-program-xyz"))
    );
}

#[test]
fn a_relative_program_path_is_treated_as_unavailable() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = store(&fixture);
    fs::create_dir_all(store.root().join("tools")).expect("directory");
    fs::write(store.root().join("tools/run"), "#!/bin/sh\n").expect("file");
    let manifest = r#"
schema_version = 1
id = "relative"
name = "Relative"

[when]

[run]
runtime = "command"
program = "tools/run"
"#;
    write_action(&store, manifest, None).expect("writes");
    let catalog = store.load();
    let handle = catalog.get("relative").expect("loads");
    assert!(
        !handle.is_available(),
        "a relative program path is never resolved against the working directory"
    );
}

#[test]
fn serialized_manifests_keep_the_documented_shape() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = store(&fixture);
    write_action(
        &store,
        PYTHON_MANIFEST,
        Some(("main.py", "#!/usr/bin/env python3\n")),
    )
    .expect("action writes");
    let text =
        fs::read_to_string(store.root().join("resize-images/action.toml")).expect("manifest");
    assert!(text.starts_with("# Strata custom action"));
    assert!(text.contains("schema_version = 1"));
    assert!(text.contains("[when]") && text.contains("[run]"));
    assert!(text.contains("runtime = \"python\""));
}

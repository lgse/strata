// SPDX-License-Identifier: MIT

use super::*;
use std::os::unix::{
    ffi::OsStringExt,
    fs::{MetadataExt, symlink},
};

fn recipe() -> String {
    ACTION_EXAMPLES
        .iter()
        .find(|example| example.name == "Batch rename")
        .expect("rename recipe")
        .script()
}

fn with_pattern(pattern: &str) -> String {
    recipe().replace(
        "PATTERN = \"{index:03d}_{filename}\"",
        &format!(
            "PATTERN = {}",
            serde_json::to_string(pattern).expect("pattern")
        ),
    )
}

fn rename_action(fixture: &Fixture, source: &str, mode: ExecutionMode) -> Rc<ActionHandle> {
    fixture.action(
        "rename",
        &script(ActionRuntime::Python, "main.py", source),
        mode,
    )
}

#[test]
fn lowercase_recipe_renames_spaces_without_clobbering_collisions() {
    let source = ACTION_EXAMPLES
        .iter()
        .find(|example| example.name == "Lowercase file names")
        .expect("lowercase recipe")
        .script();
    for collision in [false, true] {
        let fixture = Fixture::new();
        let first = fixture.path().join("Photo One.TXT");
        let target = fixture.path().join("photo-one.txt");
        let second = if collision {
            target.clone()
        } else {
            fixture.path().join("already-lower.txt")
        };
        fs::write(&first, b"first").expect("first");
        fs::write(&second, b"second").expect("second");
        let action = rename_action(&fixture, &source, ExecutionMode::WholeSelection);
        let outcome = run(
            &fixture.runner,
            request(action, &[first.clone(), second.clone()], fixture.path()),
        );
        if collision {
            assert_ne!(outcome.ended().0, Some(0));
            assert_eq!(fs::read(first).expect("first unchanged"), b"first");
        } else {
            assert_eq!(outcome.ended().0, Some(0), "{}", outcome.ended().2);
            assert_eq!(fs::read(target).expect("lowercase name"), b"first");
            assert!(!first.exists());
        }
        assert_eq!(fs::read(second).expect("second unchanged"), b"second");
    }
}

#[test]
fn batch_rename_preserves_contents_and_native_names_in_supplied_order() {
    let fixture = Fixture::new();
    let names = [
        OsString::from("-z notes\n.txt"),
        OsString::from_vec(b"a-\xff.md".to_vec()),
    ];
    let inputs: Vec<_> = names.iter().map(|name| fixture.path().join(name)).collect();
    for input in &inputs {
        fs::write(input, b"original content").expect("input");
    }
    let inodes: Vec<_> = inputs
        .iter()
        .map(|path| fs::metadata(path).expect("metadata").ino())
        .collect();
    let outcome = run(
        &fixture.runner,
        request(
            rename_action(&fixture, &recipe(), ExecutionMode::WholeSelection),
            &inputs,
            fixture.path(),
        ),
    );
    let (code, _, log) = outcome.ended();
    assert_eq!(code, Some(0), "{log}");
    for (index, input) in inputs.iter().enumerate() {
        let mut name = OsString::from(format!("{:03}_", index + 1));
        name.push(&names[index]);
        let output = fixture.path().join(name);
        assert!(!input.exists());
        assert_eq!(
            fs::read(&output).expect("renamed file"),
            b"original content"
        );
        assert_eq!(fs::metadata(output).expect("metadata").ino(), inodes[index]);
    }
    assert!(
        outcome
            .progress()
            .iter()
            .any(|progress| progress.completed == 2 && progress.total == Some(2))
    );
}

#[test]
fn rename_patterns_use_the_global_per_item_index_and_support_unchanged_names() {
    let fixture = Fixture::new();
    let input = fixture.path().join("photo.jpeg");
    fs::write(&input, b"photo").expect("input");
    let action = rename_action(
        &fixture,
        &with_pattern("{stem}-{index:02d}-of-{total}{suffix}"),
        ExecutionMode::PerItem,
    );
    let mut invocation = request(action, std::slice::from_ref(&input), fixture.path());
    invocation.position = Some((7, 12));
    let outcome = run(&fixture.runner, invocation);
    assert_eq!(outcome.ended().0, Some(0), "{}", outcome.ended().2);
    let output = fixture.path().join("photo-07-of-12.jpeg");
    assert_eq!(fs::read(&output).expect("pattern output"), b"photo");
    let unchanged = rename_action(
        &fixture,
        &with_pattern("{filename}"),
        ExecutionMode::WholeSelection,
    );
    let outcome = run(
        &fixture.runner,
        request(unchanged, std::slice::from_ref(&output), fixture.path()),
    );
    assert_eq!(outcome.ended().0, Some(0), "{}", outcome.ended().2);
    assert_eq!(fs::read(output).expect("unchanged"), b"photo");
    assert!(outcome.created().is_empty());
    assert!(
        outcome
            .progress()
            .iter()
            .any(|progress| progress.completed == 1)
    );
}

#[test]
fn custom_naming_function_receives_the_file_and_strata_context() {
    let fixture = Fixture::new();
    let input = fixture.path().join("Photo.jpeg");
    fs::write(&input, b"photo").expect("input");
    let source = recipe().replace(
        "\nctx = context()\nwith ExitStack()",
        r#"
def new_name(context):
    context.batch.log(f"Renaming {context.path.name}")
    return f"{context.index:02d}-{context.filename.lower()}"

ctx = context()
with ExitStack()"#,
    );
    let action = rename_action(&fixture, &source, ExecutionMode::WholeSelection);
    let outcome = run(
        &fixture.runner,
        request(action, std::slice::from_ref(&input), fixture.path()),
    );
    assert_eq!(outcome.ended().0, Some(0), "{}", outcome.ended().2);
    assert!(outcome.ended().2.contains("Renaming Photo.jpeg"));
    assert_eq!(
        fs::read(fixture.path().join("01-photo.jpeg")).expect("custom name"),
        b"photo"
    );
    assert!(!input.exists());
}

#[test]
fn rename_preflight_rejects_invalid_and_duplicate_names_before_changing_any_input() {
    for pattern in [
        "",
        ".",
        "..",
        "../escape",
        "/absolute",
        "bad\0name",
        "same.txt",
        "second.txt",
    ] {
        let fixture = Fixture::new();
        let inputs = [
            fixture.path().join("first.txt"),
            fixture.path().join("second.txt"),
        ];
        for input in &inputs {
            fs::write(input, b"keep").expect("input");
        }
        let action = rename_action(
            &fixture,
            &with_pattern(pattern),
            ExecutionMode::WholeSelection,
        );
        let outcome = run(&fixture.runner, request(action, &inputs, fixture.path()));
        assert_ne!(outcome.ended().0, Some(0), "{pattern:?}");
        for input in &inputs {
            assert_eq!(fs::read(input).expect("input untouched"), b"keep");
        }
        assert!(outcome.created().is_empty());
    }
}

#[test]
fn rename_preflight_refuses_existing_files_links_and_unsupported_inputs() {
    for conflict in [
        "file",
        "link",
        "dangling",
        "directory-input",
        "symlink-input",
    ] {
        let fixture = Fixture::new();
        let first = fixture.path().join("first.txt");
        let second = fixture.path().join("second.txt");
        fs::write(&first, b"first").expect("first");
        fs::write(&second, b"second").expect("second");
        let target = fixture.path().join("2.txt");
        let missing = fixture.path().join("missing");
        let other_input = match conflict {
            "file" => {
                fs::write(&target, b"existing").expect("target");
                second.clone()
            }
            "link" => {
                symlink(&first, &target).expect("link");
                second.clone()
            }
            "dangling" => {
                symlink(&missing, &target).expect("link");
                second.clone()
            }
            "directory-input" => {
                fs::create_dir(&target).expect("directory");
                target.clone()
            }
            _ => {
                symlink(&second, &target).expect("input link");
                target.clone()
            }
        };
        let action = rename_action(
            &fixture,
            &with_pattern("{index}.txt"),
            ExecutionMode::WholeSelection,
        );
        let outcome = run(
            &fixture.runner,
            request(action, &[first.clone(), other_input], fixture.path()),
        );
        assert_ne!(outcome.ended().0, Some(0), "{conflict}");
        assert_eq!(fs::read(&first).expect("first unchanged"), b"first");
        assert_eq!(fs::read(&second).expect("second unchanged"), b"second");
        assert!(!fixture.path().join("1.txt").exists());
        if conflict == "file" {
            assert_eq!(fs::read(target).expect("target unchanged"), b"existing");
        }
        assert!(!missing.exists());
    }
}

#[test]
fn rename_does_not_clobber_a_destination_created_after_preflight() {
    let fixture = Fixture::new();
    let input = fixture.path().join("file.txt");
    fs::write(&input, b"original").expect("input");
    let source = format!(
        "{}\n{}",
        r#"#!/usr/bin/env python3
from pathlib import Path
import strata_actions
original_progress = strata_actions.Context.progress
def race(self, processed, total=None, message=None):
    if processed == 0:
        (Path(self.parent) / "001_file.txt").write_bytes(b"other writer")
    original_progress(self, processed, total, message)
strata_actions.Context.progress = race
"#,
        recipe()
    );
    let action = rename_action(&fixture, &source, ExecutionMode::WholeSelection);
    let outcome = run(
        &fixture.runner,
        request(action, std::slice::from_ref(&input), fixture.path()),
    );
    assert_ne!(outcome.ended().0, Some(0));
    assert_eq!(fs::read(input).expect("input unchanged"), b"original");
    assert_eq!(
        fs::read(fixture.path().join("001_file.txt")).expect("racing target unchanged"),
        b"other writer"
    );
}

#[test]
fn rename_detects_duplicate_inputs_and_targets_through_directory_aliases() {
    for duplicate_source in [true, false] {
        let fixture = Fixture::new();
        let folder = fixture.path().join("files");
        fs::create_dir(&folder).expect("folder");
        let alias = fixture.path().join("alias");
        symlink(&folder, &alias).expect("directory alias");
        let first = folder.join("first.txt");
        let second = folder.join("second.txt");
        fs::write(&first, b"first").expect("first");
        fs::write(&second, b"second").expect("second");
        let (aliased_input, source) = if duplicate_source {
            (alias.join("first.txt"), recipe())
        } else {
            (alias.join("second.txt"), with_pattern("same.txt"))
        };
        let action = rename_action(&fixture, &source, ExecutionMode::WholeSelection);
        let outcome = run(
            &fixture.runner,
            request(action, &[first.clone(), aliased_input], fixture.path()),
        );
        assert_ne!(outcome.ended().0, Some(0));
        assert_eq!(fs::read(first).expect("first unchanged"), b"first");
        assert_eq!(fs::read(second).expect("second unchanged"), b"second");
        assert!(outcome.created().is_empty());
    }
}

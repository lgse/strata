// SPDX-License-Identifier: MIT

use super::*;
use std::os::unix::{ffi::OsStringExt, fs::symlink};

fn recipe(name: &str) -> &'static crate::model::action::examples::ActionExample {
    ACTION_EXAMPLES
        .iter()
        .find(|example| example.name == name)
        .expect("recipe")
}

fn shell_action(fixture: &Fixture, name: &str, prelude: &str) -> Rc<ActionHandle> {
    let example = recipe(name);
    let source = format!("#!/usr/bin/env bash\n{prelude}\n{}", example.script());
    fixture.action(
        "shell-example",
        &script(example.runtime(), "run.sh", &source),
        example.mode,
    )
}

#[test]
fn count_lines_totals_the_selection_and_preserves_native_file_names() {
    let fixture = Fixture::new();
    let inputs = [
        fixture.path().join("a file\n.txt"),
        fixture
            .path()
            .join(OsString::from_vec(b"-file\xff.txt".to_vec())),
    ];
    fs::write(&inputs[0], b"one\ntwo\n").expect("first");
    fs::write(&inputs[1], b"three\nfour\nfive\n").expect("second");
    let action = shell_action(&fixture, "Count lines", "");
    let outcome = run(&fixture.runner, request(action, &inputs, fixture.path()));
    let (code, _, log) = outcome.ended();
    assert_eq!(code, Some(0), "{log}");
    assert!(log.contains("Total: 5 lines across 2 files"), "{log}");
    assert_eq!(
        fs::read(&inputs[0]).expect("first untouched"),
        b"one\ntwo\n"
    );
    assert_eq!(
        fs::read(&inputs[1]).expect("second untouched"),
        b"three\nfour\nfive\n"
    );
}

#[test]
fn exif_recipe_backs_up_before_invocation_and_passes_literal_paths() {
    let fixture = Fixture::new();
    let input = fixture.path().join("-photo [1]; $(touch injected)\n.jpg");
    // A stand-in verifies invocation and backup safety independently of ExifTool's parser.
    let action = shell_action(
        &fixture,
        "Strip EXIF metadata",
        r#"
exiftool() {
    [[ $# == 4 && $1 == -overwrite_original && $2 == -all= && $3 == -- ]] || return 9
    printf 'cleaned' > "$4"
}
"#,
    );
    for content in [b"first metadata".as_slice(), b"second metadata".as_slice()] {
        fs::write(&input, content).expect("photo");
        let outcome = run(
            &fixture.runner,
            request(action.clone(), std::slice::from_ref(&input), fixture.path()),
        );
        let (code, _, log) = outcome.ended();
        assert_eq!(code, Some(0), "{log}");
        assert_eq!(fs::read(&input).expect("working photo"), b"cleaned");
    }
    let mut originals = Vec::new();
    for entry in fs::read_dir(fixture.path()).expect("backups") {
        let entry = entry.expect("entry");
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("strata-original-")
        {
            assert_eq!(
                entry.metadata().expect("directory").permissions().mode() & 0o077,
                0
            );
            let backup = entry.path().join(input.file_name().expect("name"));
            originals.push(fs::read(&backup).expect("original"));
            assert_eq!(
                fs::metadata(backup).expect("backup").permissions().mode() & 0o077,
                0
            );
        }
    }
    originals.sort();
    assert_eq!(
        originals,
        [b"first metadata".to_vec(), b"second metadata".to_vec()]
    );
    assert!(!fixture.path().join("injected").exists());
    let linked = fixture.path().join("link.jpg");
    symlink(&input, &linked).expect("symlink");
    let outcome = run(&fixture.runner, request(action, &[linked], fixture.path()));
    assert_ne!(outcome.ended().0, Some(0));
    assert_eq!(
        fs::read(input).expect("symlink target untouched"),
        b"cleaned"
    );
}

#[test]
fn failed_exif_backup_does_not_start_metadata_removal() {
    let fixture = Fixture::new();
    let input = fixture.path().join("photo.jpg");
    fs::write(&input, b"original photo").expect("photo");
    let action = shell_action(
        &fixture,
        "Strip EXIF metadata",
        r#"
cat() { printf 'partial copy'; return 19; }
exiftool() { printf 'modified' > "$4"; }
"#,
    );
    let outcome = run(
        &fixture.runner,
        request(action, std::slice::from_ref(&input), fixture.path()),
    );
    assert_eq!(outcome.ended().0, Some(19));
    assert_eq!(fs::read(input).expect("untouched photo"), b"original photo");
}

#[test]
fn shell_recipes_report_missing_tools_before_modifying_files() {
    let fixture = Fixture::new();
    let input = fixture.path().join("file.txt");
    fs::write(&input, b"original").expect("input");
    for (name, tool) in [("Strip EXIF metadata", "exiftool"), ("Count lines", "wc")] {
        let action = shell_action(&fixture, name, "PATH=\"\"");
        let outcome = run(
            &fixture.runner,
            request(action, std::slice::from_ref(&input), fixture.path()),
        );
        let (code, _, log) = outcome.ended();
        assert_ne!(code, Some(0));
        assert!(log.contains(&format!("{tool} was not found")), "{log}");
        assert_eq!(fs::read(&input).expect("original"), b"original");
    }
    assert!(
        !fs::read_dir(fixture.path())
            .expect("fixture")
            .any(|entry| entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .starts_with("strata-original-"))
    );
}

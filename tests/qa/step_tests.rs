// SPDX-License-Identifier: MIT

use std::{
    fs::{self, File},
    path::Path,
    process::Command,
    time::{Duration, Instant, SystemTime},
};

use serde_json::{Value, json};

use crate::{
    CONTROLLER_PANIC, REPOSITORY,
    baseline::{DIFFERENT_PIXEL_BUDGET, compare_in, different_fraction, differing_pixels},
    drive::file_failure,
    fixture::{OrderedMap, expand_tree, fixed_fixture_name, manifest},
    fs_check::{FsState, FsStep, contained},
    fs_helpers::{glob_matches, pdf_image, raw_photo},
    peer_steps::Relaunch,
    peers,
    process::wait_for_exit,
    render_toml,
    run::Outcome,
    runner_steps::{EachStep, ProcStep},
    scenario::{Header, Image, Scenario, load_scenario},
    steps::{
        check_step, flag_names, increment, run_steps, slice, stem, substitute_notes,
        unknown_variable,
    },
    variables::{placeholders, quote_path, runs, substitute_text, unescape, variables},
};

pub(crate) fn a_numbered_tree_expands_like_the_python_comprehension() {
    let tree =
        json!({ "nested": { "{000..002}.txt": "{}\n" }, "{8..10}.md": "n{}", "a.txt": "{}" });
    assert_eq!(
        expand_tree(&tree, &manifest()),
        Ok(json!({
            "nested": { "000.txt": "0\n", "001.txt": "1\n", "002.txt": "2\n" },
            "8.md": "n8", "9.md": "n9", "10.md": "n10",
            "a.txt": "{}",
        }))
    );
    assert!(expand_tree(&json!("missing"), &manifest()).is_err());
}

pub(crate) fn a_run_keeps_its_own_mode_steps_and_rejects_a_malformed_one() {
    let header = || -> Header {
        serde_json::from_value(json!({
            "scenario": "example", "modes": ["columns", "icons"],
            "mutation": null, "known_bug": null,
        }))
        .expect("the header should parse")
    };
    let scenario = |steps: Vec<Value>| Scenario {
        header: header(),
        steps: steps.into_iter().enumerate().collect(),
    };
    let steps = vec![
        json!({ "select": { "entry": "archive" }, "modes": ["columns"] }),
        json!({ "click": { "entry": "archive" }, "count": 2, "modes": ["icons"] }),
        json!({ "wait": { "view_mode": "$view" } }),
        json!({ "key": "Escape", "when": { "dialog": true } }),
    ];
    let runs = runs(&header());
    let icons = runs
        .iter()
        .find(|run| run.mode == "icons")
        .expect("an icons run");
    assert_eq!(
        run_steps(&scenario(steps), icons, &variables(icons, &placeholders())),
        Ok(vec![
            (1, json!({ "click": { "entry": "archive" }, "count": 2 })),
            (2, json!({ "wait": { "view_mode": "Icons" } })),
            (3, json!({ "key": "Escape", "when": { "dialog": true } })),
        ])
    );
    for typo in [
        json!({ "click": { "entry": "a", "nickname": "b" } }),
        json!({ "fs": { "path": "a", "exists": true, "text": "b" } }),
        json!({ "fs": { "path": "a", "write": "b" }, "timeout_ms": 0 }),
        json!({ "fs": { "path": "a", "is": "socket" } }),
        json!({ "key": "Escape", "when": { "modal": true } }),
        json!({ "holds": { "dialog": true } }),
        json!({ "layout": "klingon" }),
        json!({ "expect": { "dialog": true }, "as": "open", "when": { "dialog": true } }),
        json!({ "wait": { "all": [] } }),
        json!({ "fs": { "path": "a", "volume": "/dev/shm", "with": { "../b": "c" } } }),
        json!({ "fs": { "path": "a", "write": "b", "with": { "c": "d" } } }),
    ] {
        assert!(
            run_steps(
                &scenario(vec![typo.clone()]),
                icons,
                &variables(icons, &placeholders())
            )
            .is_err(),
            "{typo} should be rejected before the run starts"
        );
    }
}

pub(crate) fn notes_stand_in_for_what_a_run_reads_and_unknown_variables_are_rejected() {
    let header = || -> Header {
        serde_json::from_value(json!({
            "scenario": "example", "modes": ["columns"], "mutation": null,
            "known_bug": null, "variants": [{ "id": "name" }, { "id": "row-space" }],
        }))
        .expect("the header should parse")
    };
    let runs = runs(&header());
    let row_space = runs
        .iter()
        .find(|run| run.variant.id == "row-space")
        .expect("a row-space run");
    let check = |steps: Vec<Value>| {
        let scenario = Scenario {
            header: header(),
            steps: steps.into_iter().enumerate().collect(),
        };
        run_steps(&scenario, row_space, &variables(row_space, &placeholders()))
    };
    let kept = check(vec![
        json!({ "note": "entries", "dir": "$root", "as": "names" }),
        json!({ "wait": { "entries": "$names" } }),
        json!({ "note": "middle_visible", "unless_first": "a", "as": "first" }),
        json!({ "increment": "first", "as": "first_next" }),
        json!({ "fs": { "path": "archive/$first/x", "mkdir": true } }),
        json!({ "wait": { "focused": "$first_next" } }),
        json!({ "key": "Escape", "variants": ["name"] }),
    ])
    .expect("notes should stand in for their values");
    assert_eq!(
        kept.len(),
        6,
        "the name variant's step stays out of the row-space run"
    );
    assert_eq!(kept[1].1, json!({ "wait": { "entries": "$names" } }));
    for broken in [
        vec![json!({ "wait": { "focused": "$cursor" } })],
        vec![
            json!({ "wait": { "focused": "$cursor" } }),
            json!({ "note": "focused", "as": "cursor" }),
        ],
        vec![json!({ "note": "focused", "as": "root" })],
        vec![json!({ "note": "focused", "as": "cursor", "when": { "dialog": null } })],
        vec![json!({ "increment": "names", "as": "next" })],
        vec![
            json!({ "note": "entries", "as": "names" }),
            json!({ "increment": "names", "as": "next" }),
        ],
    ] {
        assert!(
            check(broken.clone()).is_err(),
            "{broken:?} should be rejected"
        );
    }

    let directory = tempfile::tempdir().expect("a scratch directory");
    let path = directory.path().join("example.jsonl");
    fs::write(
        &path,
        "{\"scenario\":\"example\",\"modes\":[\"columns\"],\"mutation\":null,\
         \"variants\":[{\"id\":\"name\"}]}\n{\"key\":\"Escape\",\"variants\":[\"missing\"]}\n",
    )
    .expect("the scenario should be written");
    assert!(
        load_scenario(&path).is_err(),
        "an unknown step variant is a typo"
    );

    let notes = vec![
        ("names".to_owned(), json!(["a.txt", "b.txt"])),
        ("first".to_owned(), json!("folder-087")),
    ];
    assert_eq!(
        substitute_notes(
            &json!({ "wait": { "selected": "$names" }, "message": "to become $names in $first" }),
            &notes
        ),
        json!({
            "wait": { "selected": ["a.txt", "b.txt"] },
            "message": "to become ['a.txt', 'b.txt'] in folder-087",
        })
    );
    assert_eq!(increment("folder-087"), Ok("folder-088".to_owned()));
    assert_eq!(increment("folder-099"), Ok("folder-100".to_owned()));
    assert!(increment("readme").is_err());
}

pub(crate) fn fs_actions_make_numbered_directories_and_pin_a_populated_tree() {
    let root = tempfile::tempdir().expect("a scratch directory");
    let act = |step: Value| {
        let step: FsStep = serde_json::from_value(step).expect("the step should parse");
        step.fs.observe(
            root.path(),
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_000),
            &mut FsState::default(),
        )
    };
    fs::create_dir(root.path().join("archive")).expect("the directory should be created");
    act(json!({ "fs": { "path": "archive/folder-{08..10}", "mkdir": true } }))
        .expect("numbered directories should be made");
    let mut made: Vec<String> = fs::read_dir(root.path().join("archive"))
        .expect("the directory should list")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    made.sort();
    assert_eq!(made, ["folder-08", "folder-09", "folder-10"]);
    assert!(
        act(json!({ "fs": { "path": "archive/folder-09", "mkdir": true } })).is_err(),
        "Path.mkdir() refuses an existing directory"
    );
    act(json!({ "fs": { "path": ".", "populate": { "docs": { "{1..2}.txt": "n{}" } } } }))
        .expect("the tree should be populated");
    assert_eq!(
        fs::read_to_string(root.path().join("docs/2.txt")).expect("the file should exist"),
        "n2"
    );
    let pinned = |path: &str| {
        fs::metadata(root.path().join(path))
            .and_then(|metadata| metadata.modified())
            .expect("the time should be readable")
    };
    let expected = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    assert_eq!(pinned("docs/1.txt"), expected);
    assert_eq!(
        pinned("archive/folder-08"),
        expected,
        "populate pins everything below it"
    );
}

pub(crate) fn a_variable_replaces_only_its_whole_token() {
    let variables = [
        ("view".to_owned(), "Icons".to_owned()),
        ("root".to_owned(), "fixture".to_owned()),
    ];
    let replaced = substitute_text("$view, $viewport, $root/x, $$view, $", &variables);
    assert_eq!(replaced, "Icons, $viewport, fixture/x, $$view, $");
    assert_eq!(
        unescape(&json!(replaced)),
        json!("Icons, $viewport, fixture/x, $view, $"),
        "$$ is a literal $ once nothing is left to replace"
    );
    assert_eq!(unknown_variable(&json!("Energy $$E=mc^2$$")), None);
}

pub(crate) fn a_slice_keeps_the_middle_band_or_one_name_of_a_noted_list() {
    let rows: Vec<Value> = (0..7).map(|row| json!(format!("{row:03}.txt"))).collect();
    assert_eq!(
        slice(&rows, Some(3), None),
        Ok(json!(["003.txt", "004.txt", "005.txt"]))
    );
    assert_eq!(slice(&rows, Some(3), Some(-1)), Ok(json!("005.txt")));
    assert_eq!(slice(&rows, None, Some(0)), Ok(json!("000.txt")));
    assert!(slice(&rows, None, Some(7)).is_err());
}

pub(crate) fn an_empty_check_counts_a_missing_directory_as_empty() {
    let root = tempfile::tempdir().expect("a scratch directory");
    fs::create_dir(root.path().join("full")).expect("the directory should be created");
    fs::write(root.path().join("full/a.txt"), "a").expect("the file should be written");
    let check = |path: &str, empty: bool| {
        let step: FsStep =
            serde_json::from_value(json!({ "fs": { "path": path, "empty": empty } }))
                .expect("the step should parse");
        step.fs
            .observe(root.path(), SystemTime::UNIX_EPOCH, &mut FsState::default())
    };
    assert!(check("missing", true).is_ok());
    assert!(check("missing", false).is_err());
    assert!(check("full", false).is_ok());
    assert!(check("full", true).is_err());
}

pub(crate) fn only_a_controller_panic_is_filed_as_a_harness_error() {
    let mut outcome = Outcome::default();
    file_failure(&mut outcome, 3, "line 3", "timed out");
    file_failure(
        &mut outcome,
        4,
        "line 4",
        &format!("{CONTROLLER_PANIC}: boom"),
    );
    assert_eq!(
        outcome.failures,
        [(Some(3), "line 3: timed out".to_owned())]
    );
    assert_eq!(outcome.errors.len(), 1);
}

pub(crate) fn remove_and_rename_act_only_inside_the_run_directories() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let fixture = scratch.path().join("fixture");
    let outside = scratch.path().join("outside");
    fs::create_dir_all(fixture.join("documents")).expect("the fixture");
    fs::create_dir(&outside).expect("a directory outside the roots");
    std::os::unix::fs::symlink(&outside, fixture.join("away")).expect("a link outside");
    let roots = [fixture.clone()];
    assert!(contained(&fixture.join("documents"), &roots).is_ok());
    assert!(contained(&fixture.join("documents/../todo.txt"), &roots).is_ok());
    assert!(
        contained(&fixture.join("away"), &roots).is_ok(),
        "the link itself is inside"
    );
    assert!(contained(&fixture.join("away/escaped.txt"), &roots).is_err());
    assert!(contained(&fixture.join("../outside/x"), &roots).is_err());
    assert!(contained(&fixture, &[scratch.path().join("fixture/documents")]).is_err());
    assert!(contained(Path::new("/etc/passwd"), &roots).is_err());
}

pub(crate) fn file_parts_join_repeats_and_fs_actions_write_archives_and_sparse_files() {
    let manifest = manifest();
    let tree = expand_tree(
        &json!({ "table.csv": ["name,value\n", { "repeat": "record-{},{}\n", "from": 0, "to": 2 }] }),
        &manifest,
    )
    .expect("parts should expand");
    assert_eq!(
        tree["table.csv"],
        json!("name,value\nrecord-0,0\nrecord-1,1\nrecord-2,2\n")
    );
    assert!(expand_tree(&json!({ "a": [{ "repeat": "x" }] }), &manifest).is_err());
    assert_eq!(stem("042.txt").as_deref(), Ok("042"));

    let root = tempfile::tempdir().expect("a scratch directory");
    let act = |step: Value| {
        let step: FsStep = serde_json::from_value(step).expect("the step should parse");
        assert!(
            step.fs.acts(),
            "an archive, a truncation, and a copy act at once"
        );
        step.fs
            .observe(root.path(), SystemTime::UNIX_EPOCH, &mut FsState::default())
    };
    act(json!({ "fs": { "path": "a.zip", "zip": [
        ["3D/3dmodel.model", "geometry"],
        ["Metadata/thumbnail.png", { "image": { "width": 2, "height": 2, "rgb": [255, 0, 0] } }],
    ] } }))
    .expect("the archive should be written");
    let mut archive =
        zip::ZipArchive::new(File::open(root.path().join("a.zip")).expect("the archive"))
            .expect("a readable zip");
    assert_eq!(
        archive
            .file_names()
            .collect::<Result<Vec<_>, _>>()
            .expect("readable names"),
        ["3D/3dmodel.model", "Metadata/thumbnail.png"]
    );
    assert_eq!(
        archive.by_index(0).expect("an entry").compression(),
        zip::CompressionMethod::Stored,
        "zipfile writes stored entries by default"
    );
    act(json!({ "fs": { "path": "big.stl", "truncate": 4097 } }))
        .expect("the file should be sized");
    assert_eq!(
        fs::metadata(root.path().join("big.stl"))
            .map(|m| m.len())
            .ok(),
        Some(4097)
    );
    act(json!({ "fs": { "path": "copy.json", "from_repository": "tests/fixtures/harness.json" } }))
        .expect("the repository file should be copied");
    assert_eq!(
        fs::read(root.path().join("copy.json")).ok(),
        fs::read(Path::new(REPOSITORY).join("tests/fixtures/harness.json")).ok()
    );
    let escape: FsStep = serde_json::from_value(
        json!({ "fs": { "path": "x", "from_repository": "../etc/passwd" } }),
    )
    .expect("the step should parse");
    assert!(
        escape.fs.kind().is_err(),
        "from_repository stays inside the repository"
    );
}

pub(crate) fn a_flag_keeps_whether_a_predicate_held_and_always_runs() {
    assert!(check_step(&json!({ "flag": { "dialog": null }, "as": "enabled" })).is_ok());
    assert!(check_step(&json!({ "flag": { "dialog": null } })).is_err());
    assert!(check_step(&json!({ "flag": { "nope": 1 }, "as": "enabled" })).is_err());
    assert!(
        check_step(
            &json!({ "flag": { "dialog": null }, "as": "enabled", "when": { "dialog": null } })
        )
        .is_err(),
        "a flag always runs"
    );
}

pub(crate) fn fs_actions_write_archives_thumbnails_and_records() {
    let root = tempfile::tempdir().expect("a scratch directory");
    let mut state = FsState {
        roots: vec![root.path().to_owned()],
        ..FsState::default()
    };
    let mut step = |value: Value| {
        let step: FsStep = serde_json::from_value(value).expect("the step should parse");
        step.fs
            .observe(root.path(), SystemTime::UNIX_EPOCH, &mut state)
    };
    fs::write(root.path().join("todo.txt"), "todo\n").expect("the fixture file");
    step(json!({ "fs": { "path": "a.tar.gz", "tar_gz": {
        "mode": "755", "mtime": 1_000_000_000, "members": [["todo.txt", "todo\n"]] } } }))
    .expect("the tarball should be written");
    assert!(
        step(json!({ "fs": { "path": "a.tar.gz", "tar_members": [["todo.txt", "todo.txt"]] } }))
            .is_ok()
    );
    assert!(
        step(json!({ "fs": { "path": "a.tar.gz", "tar_members": [["other", "todo.txt"]] } }))
            .is_err()
    );
    step(json!({ "fs": { "path": "a.zip", "zip": [["todo.txt", {
        "text": "todo\n", "date_time": [2001, 9, 9, 1, 46, 40], "mode": "100755" }]] } }))
    .expect("the zip should be written");
    assert!(
        step(json!({ "fs": { "path": "a.zip", "zip_members": [["todo.txt", "todo.txt"]] } }))
            .is_ok()
    );
    assert!(step(json!({ "fs": { "path": "a.zip", "is_zip": true } })).is_ok());
    assert!(step(json!({ "fs": { "path": "todo.txt", "is_zip": false } })).is_ok());

    let cache = root.path().join("cache").display().to_string();
    step(json!({ "fs": { "path": "todo.txt", "thumbnail": {
        "cache": cache, "width": 4, "height": 2, "rgb": [0, 0, 255], "mode": "700" } } }))
    .expect("the thumbnail should be written");
    assert!(
        step(
            json!({ "fs": { "path": "todo.txt", "cached_in": cache, "pixel": {
        "at": [0, 0], "rgb": [0, 0, 255] } } })
        )
        .is_ok()
    );
    assert!(
        step(
            json!({ "fs": { "path": "todo.txt", "cached_in": cache, "alpha_range": [255, 255] } })
        )
        .is_ok()
    );
    assert!(
        step(json!({ "fs": { "path": "todo.txt", "cached_in": cache, "alpha_range": [0, 255] } }))
            .is_err(),
        "a solid thumbnail has no transparent pixel"
    );
    assert!(
        step(json!({ "fs": { "path": ".", "all_cached_in": cache, "except_suffix": ".zip" } }))
            .is_err(),
        "the tarball has no thumbnail"
    );

    fs::write(
        root.path().join("record"),
        r#"{"paths": ["b", "a"], "action": "keep"}"#,
    )
    .expect("a record");
    assert!(step(json!({ "fs": { "path": "record", "json": { "action": "keep" } } })).is_ok());
    assert!(step(json!({ "fs": { "path": "record", "json": { "paths": ["a", "b"] } } })).is_err());
    assert!(
        step(json!({ "fs": { "path": "record", "json_set": { "paths": ["a", "b"] } } })).is_ok()
    );

    fs::write(
        root.path().join("action.toml"),
        "name = \"x\"\n[run]\nargs = [\"a\"]\n",
    )
    .expect("a manifest");
    assert!(
        step(
            json!({ "fs": { "path": "action.toml", "toml": { "name": "x", "run.args": ["a"] } } })
        )
        .is_ok()
    );
    assert!(
        step(json!({ "fs": { "path": "action.toml", "toml": { "run.args": ["b"] } } })).is_err()
    );

    step(json!({ "fs": { "path": "bad%E8name.txt", "write": "x", "percent_path": true } }))
        .expect("a name that is not UTF-8 should be written");
    assert!(
        root.path()
            .join(
                <std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(
                    b"bad\xe8name.txt".to_vec()
                )
            )
            .is_file()
    );
    assert!(
        step(json!({ "fs": { "path": "/etc/strata-qa-outside", "hex": "00" } })).is_err(),
        "a write outside the run's directories is refused"
    );
    assert_eq!(quote_path(b"/tmp/a b/bad\xe8.txt"), "/tmp/a%20b/bad%E8.txt");
    assert_eq!(
        &raw_photo()[..4],
        b"II\x2a\0",
        "the DNG starts with a little-endian TIFF header"
    );
    assert!(
        pdf_image(&Image {
            width: 4,
            height: 2,
            rgb: [0, 0, 255],
            width_step: 0,
            thumbnail: None
        })
        .is_ok_and(|pdf| pdf.starts_with(b"%PDF-"))
    );
}

pub(crate) fn log_proc_and_each_steps_check_their_shape_before_the_run() {
    for valid in [
        json!({ "log": { "count": "started" }, "as": "initial" }),
        json!({ "log": { "count": "started", "at_most": 2 }, "timeout_ms": 0 }),
        json!({ "log": { "matches": "pid=(\\d+)", "pick": -1 }, "as": "pid" }),
        json!({ "log": { "contains": "Landlock" }, "as": "landlocked" }),
        json!({ "proc": { "gone": [1, 2] } }),
        json!({ "proc": { "kill": 7 } }),
        json!({ "proc": { "app_cwd": "/" }, "timeout_ms": 0 }),
        json!({ "restart": true }),
        json!({ "dbus_send": ["--print-reply"] }),
        json!({ "each": "names", "as": "name", "step": { "wait": { "entry": "$name" } } }),
    ] {
        assert!(check_step(&valid).is_ok(), "{valid} should be accepted");
    }
    for invalid in [
        json!({ "log": { "count": "started" } }),
        json!({ "log": { "count": "a", "contains": "b" }, "as": "x" }),
        json!({ "log": { "contains": "a", "at_least": 1 } }),
        json!({ "log": { "contains": "a" }, "as": "x", "if": "y" }),
        json!({ "proc": { "kill": 7 }, "timeout_ms": 0 }),
        json!({ "proc": { "app_cwd": "/", "kill": 7 } }),
        json!({ "restart": true, "message": "x" }),
        json!({ "dbus_send": [] }),
        json!({ "each": "names", "as": "name", "step": { "fs": { "path": "a", "exists": true } } }),
    ] {
        assert!(
            check_step(&invalid).is_err(),
            "{invalid} should be rejected"
        );
    }
    let kill: ProcStep =
        serde_json::from_value(json!({ "proc": { "kill": 1 } })).expect("a kill should parse");
    assert!(
        kill.run(std::process::id())
            .is_err_and(|error| error.starts_with("refusing to kill")),
        "a pid that is not the application's child is never signalled"
    );
    let own_cwd = std::env::current_dir().expect("the runner's working folder");
    for (cwd, holds) in [
        (own_cwd.to_str().expect("a UTF-8 folder"), true),
        ("/nowhere", false),
    ] {
        let step: ProcStep =
            serde_json::from_value(json!({ "proc": { "app_cwd": cwd }, "timeout_ms": 0 }))
                .expect("an app_cwd query should parse");
        assert_eq!(step.run(std::process::id()).is_ok(), holds, "app_cwd {cwd}");
    }
    let directory = tempfile::tempdir().expect("a scratch directory");
    let path = directory.path().join("example.jsonl");
    for (name, allowed) in [
        ("PATH", true),
        ("DISPLAY", false),
        ("HOME", false),
        ("XDG_CACHE_HOME", false),
    ] {
        fs::write(
            &path,
            format!(
                "{{\"scenario\":\"example\",\"modes\":[\"columns\"],\"mutation\":null,\
                 \"known_bug\":null,\"env\":{{\"{name}\":\"x\"}}}}\n{{\"key\":\"Escape\"}}\n"
            ),
        )
        .expect("the scenario should be written");
        assert_eq!(load_scenario(&path).is_ok(), allowed, "env {name}");
    }
    let each: EachStep = serde_json::from_value(json!({
        "each": "names", "as": "name", "derive": { "caption": { "number_plus": 320, "suffix": "×180" } },
        "step": { "wait": { "present": { "role": "label", "name": "$caption" } }, "message": "for $name" },
    }))
    .expect("the each step should parse");
    assert_eq!(
        each.expand(&json!("photo-0012.png")),
        Ok(
            json!({ "wait": { "present": { "role": "label", "name": "332×180" } }, "message": "for photo-0012.png" })
        )
    );
}

pub(crate) fn peer_restart_and_guard_steps_check_their_shape_before_the_run() {
    for valid in [
        json!({ "launch": ["$fixture"] }),
        json!({ "launch": ["a-%FF"], "percent_path": true }),
        json!({ "restart": { "directory": "/tmp/x", "env": { "RUST_LOG": "trace" }, "stdout": "closed", "stderr": "null" } }),
        json!({ "portal_open": "Title", "options": { "current_folder": "/tmp", "multiple": true } }),
        json!({ "portal_response": { "done": false } }),
        json!({ "portal_response": { "done": true, "status": 0, "names": ["a"] }, "timeout_ms": 20000 }),
        json!({ "http": "aGk=", "as": "url" }),
        json!({ "http_requests": 1 }),
        json!({ "foreign": { "paths": ["/tmp/a"], "move_only": true }, "as": "foreign" }),
        json!({ "foreign_label": "done" }),
        json!({ "foreign_stop": true }),
        json!({ "zipcrypto": { "path": "a.zip", "password": "p", "member": "m", "contents": { "repeat": "x", "count": 3 } }, "as": "c" }),
        json!({ "fail": "message" }),
        json!({ "flag": { "focused": null }, "as": "f", "timeout_ms": 5000, "unless": ["a", "b"] }),
        json!({ "expect": { "focused": null }, "as": "f", "if": "a" }),
    ] {
        assert!(check_step(&valid).is_ok(), "{valid} should be accepted");
    }
    for invalid in [
        json!({ "launch": [] }),
        json!({ "restart": { "stdout": "closed" } }),
        json!({ "restart": { "stdout": "piped", "stderr": "null" } }),
        json!({ "restart": { "env": { "DISPLAY": ":1" } } }),
        json!({ "portal_response": { "done": false, "status": 0 } }),
        json!({ "foreign": { "paths": [] }, "as": "foreign" }),
        json!({ "foreign_stop": false }),
        json!({ "fail": "" }),
        json!({ "expect": { "focused": null }, "if": [] }),
        json!({ "expect": { "focused": null }, "as": "f", "when": { "focused": null } }),
    ] {
        assert!(
            check_step(&invalid).is_err(),
            "{invalid} should be rejected"
        );
    }
    assert_eq!(flag_names(Some(&json!(["a", "b"]))), ["a", "b"]);
    assert_eq!(flag_names(Some(&json!("a"))), ["a"]);
    let relaunch: Relaunch =
        serde_json::from_value(json!({ "stdout": "closed", "stderr": "null" }))
            .expect("a relaunch");
    assert_eq!(
        relaunch.streams().as_deref(),
        Some("stdout=closed,stderr=null")
    );
}

pub(crate) fn glob_and_zipcrypto_helpers_match_names_and_write_archives() {
    assert!(glob_matches(
        ".strata-compression-*",
        ".strata-compression-x1"
    ));
    assert!(!glob_matches(
        ".strata-compression-*",
        "strata-compression-x1"
    ));
    assert!(glob_matches("*.tar.gz", "a.tar.gz"));
    assert!(!glob_matches("*.7z", "a.7z.part"));
    assert!(glob_matches("a*b*c", "aXbYc"));
    let directory = tempfile::tempdir().expect("a scratch directory");
    let path = directory.path().join("password.zip");
    for deflated in [false, true] {
        peers::write_zipcrypto(
            &path,
            "zipsecret",
            "some.txt",
            b"hello from zipcrypto",
            deflated,
        )
        .expect("the archive should be written");
        let collision = peers::zipcrypto_collision(&path).expect("a colliding password");
        assert!(collision.parse::<u32>().is_ok_and(|number| number < 4096));
        let archive = zip::ZipArchive::new(File::open(&path).expect("the archive")).expect("a zip");
        assert_eq!(
            archive
                .file_names()
                .collect::<Result<Vec<_>, _>>()
                .expect("readable names"),
            ["some.txt"]
        );
    }
}

pub(crate) fn baseline_comparison_thresholds_pixels_and_budget() {
    let grey = image::Rgb([100, 100, 100]);
    let baseline = image::RgbImage::from_pixel(200, 1, grey);
    for (channel, shift, differs) in [(0, 24, false), (1, 25, true), (2, -25, true)] {
        let mut candidate = baseline.clone();
        let mut moved = grey;
        moved.0[channel] = moved.0[channel].saturating_add_signed(shift);
        candidate.put_pixel(7, 0, moved);
        let mask = differing_pixels(&baseline, &candidate);
        assert_eq!(
            mask.iter().filter(|value| **value).count(),
            usize::from(differs)
        );
    }
    for (different, matched) in [(1, true), (2, false)] {
        let mask: Vec<bool> = (0..200).map(|index| index < different).collect();
        assert_eq!(different_fraction(&mask) <= DIFFERENT_PIXEL_BUDGET, matched);
    }
}

pub(crate) fn baseline_mismatches_fail_and_only_an_update_writes_the_baseline() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let (images, artifacts) = (
        scratch.path().join("gtk-4.22"),
        scratch.path().join("artifacts"),
    );
    fs::create_dir_all(&images).expect("the baseline directory");
    fs::create_dir_all(&artifacts).expect("the artifact directory");
    let save = |path: &Path, width| {
        image::RgbImage::from_pixel(width, 2, image::Rgb([9, 9, 9]))
            .save_with_format(path, image::ImageFormat::Png)
            .expect("an image");
    };
    let (expected, actual) = (images.join("view.png"), scratch.path().join("capture.png"));
    save(&expected, 2);
    save(&actual, 3);
    let committed = fs::read(&expected).expect("the baseline");

    let resized = compare_in(&images, "view", &actual, &artifacts, false).expect("a comparison");
    assert!(!resized.matched);
    assert_eq!(resized.detail, "size changed from (2, 2) to (3, 2)");
    let missing = compare_in(&images, "absent", &actual, &artifacts, false).expect("a comparison");
    assert!(!missing.matched);
    assert_eq!(missing.detail, "no baseline at gtk-4.22/absent.png");
    assert_eq!(missing.different_fraction, 1.0);

    assert_eq!(fs::read(&expected).expect("the baseline"), committed);
    assert!(!images.join("absent.png").exists());
    assert!(artifacts.join("view.actual.png").is_file());

    let captured = fs::read(&actual).expect("the capture");
    for name in ["view", "absent"] {
        let updated = compare_in(&images, name, &actual, &artifacts, true).expect("an update");
        assert!(updated.matched, "{name}");
        assert_eq!(
            fs::read(images.join(format!("{name}.png"))).expect("the baseline"),
            captured
        );
    }
}

pub(crate) fn fixture_roots_and_baseline_steps_check_their_shape_before_the_run() {
    let names = [
        ("/tmp/strata-e2e-baseline", Some("strata-e2e-baseline")),
        ("/tmp/a/b", None),
        ("/tmp/", None),
        ("/tmp/..", None),
        ("/var/tmp/x", None),
    ];
    for (root, name) in names {
        assert_eq!(fixed_fixture_name(root), name, "{root}");
    }
    assert!(check_step(&json!({ "baseline": "columns-view" })).is_ok());
    for step in [
        json!({ "baseline": "columns-view.png" }),
        json!({ "baseline": "" }),
        json!({ "baseline": "a", "timeout_ms": 5 }),
    ] {
        assert!(check_step(&step).is_err(), "{step}");
    }
}

pub(crate) fn waiting_for_exit_wakes_on_the_exit_and_gives_up_at_the_timeout() {
    let mut quick = Command::new("true").spawn().expect("true should start");
    let started = Instant::now();
    let status = wait_for_exit(&mut quick, Duration::from_secs(10));
    assert!(
        status.is_some_and(|status| status.success()),
        "true should be reaped with its success, not {status:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the exit should wake the wait, which took {:?}",
        started.elapsed()
    );

    let mut slow = Command::new("sleep")
        .arg("10")
        .spawn()
        .expect("sleep should start");
    let started = Instant::now();
    let status = wait_for_exit(&mut slow, Duration::from_millis(100));
    let waited = started.elapsed();
    let _ = slow.kill();
    let reaped = slow.wait();
    assert!(
        status.is_none(),
        "sleep 10 should still run, not {status:?}"
    );
    assert!(
        waited >= Duration::from_millis(100) && waited < Duration::from_secs(2),
        "the wait should end at its timeout, not after {waited:?}"
    );
    assert!(
        reaped.is_ok_and(|status| !status.success()),
        "the killed sleep should be reaped"
    );
}

pub(crate) fn prefs_write_nested_tables_into_settings_toml() {
    let keymap = json!({ "default": { "rename": ["<Control>e"], "escape": ["Escape", "\"q\\"] } });
    let preferences = OrderedMap(vec![
        ("type_to_search".to_owned(), json!(false)),
        ("keymap".to_owned(), keymap.clone()),
    ]);
    let written: toml::Table = render_toml(&preferences)
        .parse()
        .expect("the rendered settings.toml should parse");
    assert_eq!(
        serde_json::to_value(written).expect("TOML converts to JSON"),
        json!({ "type_to_search": false, "keymap": keymap })
    );
}

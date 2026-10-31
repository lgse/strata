// SPDX-License-Identifier: MIT

use std::os::unix::fs::PermissionsExt;

use super::*;

const MANIFEST: &str = r#"
schema_version = 1
id = "example"
name = "Example Cloud"
description = "Share links and pins for Example Cloud."
category = "cloud"
maintainer = "octocat"
runtime = "sh"
entry = "provider.sh"
icon = "logo"

[icons]
logo = "logo.png"

[[requirements]]
command = "example-daemon"
packages = { pacman = "example", apt = "example-client" }

[[requirements]]
module = "gi"
"#;

fn png() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    bytes.extend(16u32.to_be_bytes());
    bytes.extend(16u32.to_be_bytes());
    bytes.extend([8, 6, 0, 0, 0]);
    bytes
}

fn files(manifest: &str) -> BTreeMap<String, Vec<u8>> {
    [
        (CATALOG_FILE, manifest.as_bytes().to_vec()),
        ("provider.sh", b"#!/bin/sh\ncat\n".to_vec()),
        ("messages.json", b"{}".to_vec()),
        ("logo.png", png()),
    ]
    .into_iter()
    .map(|(name, bytes)| (name.to_owned(), bytes))
    .collect()
}

fn example() -> Entry {
    parse("example", files(MANIFEST)).expect("valid entry")
}

#[test]
fn parse_accepts_a_complete_entry_and_hashes_its_files() {
    let entry = example();
    assert_eq!(entry.name, "Example Cloud");
    assert_eq!(entry.category, Category::Cloud);
    assert_eq!(entry.icon_bytes(), Some(png().as_slice()));
    assert_eq!(
        entry
            .requirements()
            .iter()
            .map(Requirement::label)
            .collect::<Vec<_>>(),
        ["sh", "example-daemon", "gi"],
        "the runtime is an implicit first requirement"
    );
    assert_eq!(
        entry.requirements[0].install_command(Some(PackageManager::Apt)),
        Some("sudo apt install example-client".to_owned())
    );
    assert_eq!(
        entry.requirements[0].install_command(Some(PackageManager::Dnf)),
        None,
        "no hint for a manager the entry does not list"
    );
    let mut changed = files(MANIFEST);
    changed.insert("messages.json".into(), b"{\"en\": {}}".to_vec());
    let changed = parse("example", changed).expect("valid entry");
    assert_ne!(changed.hash, entry.hash, "any file change is an update");
    assert_eq!(entry.hash, example().hash, "the hash is stable");
}

#[test]
fn parse_rejects_invalid_manifests() {
    type Case = (&'static str, BTreeMap<String, Vec<u8>>, &'static str);
    let cases: Vec<Case> = vec![
        ("example", BTreeMap::new(), "catalog.toml is missing"),
        ("other", files(MANIFEST), "does not match the directory"),
        ("Bad Id", files(MANIFEST), "slug"),
        (
            "example",
            files(&MANIFEST.replace("maintainer = \"octocat\"", "maintainer = \"\"")),
            "GitHub username",
        ),
        (
            "example",
            files(&MANIFEST.replace("category = \"cloud\"", "category = \"games\"")),
            "catalog.toml:",
        ),
        (
            "example",
            files(&format!("{MANIFEST}\nversion = \"2\"\n")),
            "catalog.toml:",
        ),
        (
            "example",
            files(&MANIFEST.replace("entry = \"provider.sh\"", "entry = \"missing.sh\"")),
            "entry",
        ),
        (
            "example",
            files(&MANIFEST.replace("icon = \"logo\"", "icon = \"badge\"")),
            "not declared",
        ),
        (
            "example",
            files(&MANIFEST.replace("module = \"gi\"", "module = \"gi\"\ncommand = \"x\"")),
            "exactly one command or module",
        ),
        (
            "example",
            files(&MANIFEST.replace("module = \"gi\"", "module = \"gi.repository\"")),
            "top-level module name",
        ),
        (
            "example",
            files(&MANIFEST.replace("module = \"gi\"", "module = \"gi\"\nalternatives = [\"x\"]")),
            "belong to a command requirement",
        ),
        (
            "example",
            files(&MANIFEST.replace(
                "command = \"example-daemon\"",
                "command = \"example-daemon\"\nalternatives = [\"bin/example\"]",
            )),
            "executable basename",
        ),
        (
            "example",
            files(&MANIFEST.replace(
                "command = \"example-daemon\"",
                "command = \"example-daemon\"\nalternatives = [\"a\", \"b\", \"c\", \"d\", \"e\"]",
            )),
            "at most 4 alternatives",
        ),
        (
            "example",
            files(&MANIFEST.replace("apt = \"example-client\"", "brew = \"example\"")),
            "unknown package manager",
        ),
        (
            "example",
            files(&MANIFEST.replace("pacman = \"example\"", "pacman = \"-rf\"")),
            "invalid package name",
        ),
        (
            "example",
            files(&MANIFEST.replace("runtime = \"sh\"", "runtime = \"/usr/bin/sh\"")),
            "interpreter basename",
        ),
    ];
    for (id, files, expected) in cases {
        let error = parse(id, files).expect_err(expected);
        assert!(error.contains(expected), "{expected:?} in {error:?}");
    }
    for name in ["provider.json", "catalog-install.json", "disabled"] {
        let mut reserved = files(MANIFEST);
        reserved.insert(name.into(), b"{}".to_vec());
        assert!(
            parse("example", reserved)
                .expect_err(name)
                .contains("written by Strata"),
            "{name} is reserved"
        );
    }
    let mut bad_icon = files(MANIFEST);
    bad_icon.insert("logo.png".into(), b"not a png".to_vec());
    assert!(
        parse("example", bad_icon)
            .expect_err("icon")
            .contains("PNG")
    );
}

#[test]
fn install_registers_a_resolved_command_and_tracks_updates_and_edits() {
    let config = tempfile::tempdir().expect("config");
    let root = config.path().join("strata/providers");
    let entry = example();
    assert_eq!(status(&entry, &root), Status::NotInstalled);

    let directory = install(&entry, &root).expect("install");
    assert_eq!(directory, root.join("example"));
    assert_eq!(
        fs::metadata(&root).expect("root").permissions().mode() & 0o777,
        0o700
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("provider.json")).expect("manifest"))
            .expect("json");
    let sh = crate::trusted_command::resolve("sh").expect("sh");
    assert_eq!(
        manifest["command"],
        serde_json::json!([sh, directory.join("provider.sh")])
    );
    assert_eq!(manifest["name"], "Example Cloud");
    assert_eq!(manifest["icons"]["logo"], "logo.png");
    assert_eq!(
        file_providers::load(&directory)
            .and_then(|registration| registration.manifest.category)
            .as_deref()
            .and_then(Category::from_id),
        Some(entry.category),
        "the registration carries the category for its fallback icon"
    );
    assert_eq!(
        fs::read(directory.join("logo.png")).expect("icon"),
        png(),
        "artwork is copied beside the manifest"
    );
    assert_eq!(
        status(&entry, &root),
        Status::Installed {
            update: false,
            modified: false
        }
    );
    assert!(
        file_providers::discover(&root)
            .iter()
            .any(|registration| registration.manifest.id == "example"),
        "the installed copy is a trusted registration"
    );

    fs::write(directory.join("messages.json"), b"{\"edited\": true}").expect("edit");
    assert_eq!(
        status(&entry, &root),
        Status::Installed {
            update: false,
            modified: true
        }
    );

    let mut next = files(MANIFEST);
    next.remove("messages.json");
    next.insert("strings.json".into(), b"{}".to_vec());
    let next = parse("example", next).expect("valid entry");
    assert_eq!(
        status(&next, &root),
        Status::Installed {
            update: true,
            modified: true
        }
    );
    install(&next, &root).expect("update");
    assert_eq!(
        status(&next, &root),
        Status::Installed {
            update: false,
            modified: false
        }
    );
    assert!(
        !directory.join("messages.json").exists(),
        "files the previous installation recorded are removed"
    );
    assert!(directory.join("strings.json").exists());

    fs::remove_file(directory.join(RECORD_FILE)).expect("drop record");
    assert_eq!(status(&next, &root), Status::Manual);
    assert!(
        manual_installations(&root, std::slice::from_ref(&next)).is_empty(),
        "a catalog id stays with its catalog row"
    );
    assert_eq!(
        manual_installations(&root, &[]),
        vec![Installation {
            id: "example".into(),
            name: "Example Cloud".into(),
            icon: next.icon_bytes().map(<[u8]>::to_vec),
            enabled: true,
        }]
    );

    set_enabled(&root, "example", false).expect("disable");
    assert!(!enabled(&root, "example"));
    assert!(!manual_installations(&root, &[])[0].enabled);
    install(&next, &root).expect("reinstall");
    assert!(
        !enabled(&root, "example"),
        "an update keeps the registration disabled"
    );
    set_enabled(&root, "example", true).expect("enable");
    assert!(enabled(&root, "example"));
    assert!(set_enabled(&root, "missing", false).is_err());
    assert!(set_enabled(&root, "../example", false).is_err());

    remove(&root, "example").expect("remove");
    assert_eq!(status(&next, &root), Status::NotInstalled);
    assert!(remove(&root, "example").is_err());
    assert!(remove(&root, "../example").is_err());
}

#[test]
fn install_refuses_links_and_shared_directories() {
    let config = tempfile::tempdir().expect("config");
    let root = config.path().join("strata/providers");
    fs::create_dir_all(&root).expect("root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).expect("perms");
    assert!(
        install(&example(), &root)
            .expect_err("shared root")
            .contains("writable by other users")
    );
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("perms");
    let elsewhere = config.path().join("elsewhere");
    fs::create_dir(&elsewhere).expect("target");
    std::os::unix::fs::symlink(&elsewhere, root.join("example")).expect("link");
    assert!(
        install(&example(), &root)
            .expect_err("linked directory")
            .contains("symbolic link")
    );
    assert_eq!(status(&example(), &root), Status::Manual);
    remove(&root, "example").expect("remove the link");
    assert!(
        elsewhere.exists(),
        "removing a link leaves its target alone"
    );
    let missing = Entry {
        runtime: "no-such-interpreter-xyz".into(),
        ..example()
    };
    assert!(
        install(&missing, &root)
            .expect_err("missing runtime")
            .contains("not installed")
    );
}

#[test]
fn requirement_checks_cover_the_runtime_commands_and_modules() {
    let mut entry = example();
    entry.runtime = "python3".into();
    entry.requirements.push(Requirement {
        command: None,
        alternatives: Vec::new(),
        module: Some("no_such_module_xyz".into()),
        packages: BTreeMap::new(),
    });
    entry.requirements.push(Requirement {
        command: None,
        alternatives: Vec::new(),
        module: Some("json".into()),
        packages: BTreeMap::new(),
    });
    entry.requirements.push(Requirement {
        command: Some("strata-missing-command".into()),
        alternatives: vec!["strata-also-missing".into(), "sh".into()],
        module: None,
        packages: BTreeMap::new(),
    });
    let results = check_requirements(&entry);
    let satisfied: Vec<(&str, bool)> = results
        .iter()
        .map(|result| (result.requirement.label(), result.satisfied))
        .collect();
    assert_eq!(
        satisfied,
        [
            ("python3", true),
            ("example-daemon", false),
            (
                "gi",
                crate::trusted_command::resolve("python3").is_ok() && results[2].satisfied
            ),
            ("no_such_module_xyz", false),
            ("json", true),
            (
                "strata-missing-command",
                crate::trusted_command::resolve("sh").is_ok()
            ),
        ]
    );
}

#[test]
fn directories_load_like_the_bundle_and_skip_templates_and_documentation() {
    let catalog = tempfile::tempdir().expect("catalog");
    for name in ["_template", ".git", "broken"] {
        fs::create_dir(catalog.path().join(name)).expect("dir");
        fs::write(catalog.path().join(name).join(CATALOG_FILE), "junk").expect("file");
    }
    let directory = catalog.path().join("example");
    fs::create_dir(&directory).expect("dir");
    for (name, bytes) in files(MANIFEST) {
        fs::write(directory.join(name), bytes).expect("file");
    }
    fs::write(directory.join("README.md"), "docs").expect("readme");
    fs::write(directory.join(".hidden"), "x").expect("hidden");
    fs::create_dir(directory.join("tests")).expect("tests");
    let entries = load_directory(catalog.path());
    assert_eq!(
        entries,
        vec![example()],
        "documentation and tests are not installed"
    );
    assert!(load_directory(&catalog.path().join("missing")).is_empty());
}

#[test]
fn bundled_entries_parse_and_are_translated() {
    assert_eq!(
        bundled().len(),
        bundle::BUNDLED.len(),
        "every bundled integration must parse"
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/locales");
    for entry in bundled() {
        for locale in rust_i18n::available_locales!() {
            let catalog: serde_json::Value = serde_json::from_str(
                &fs::read_to_string(root.join(format!("{locale}.json"))).expect("catalog"),
            )
            .expect("catalog JSON");
            for text in [&entry.name, &entry.description] {
                assert!(
                    catalog.get(text).is_some(),
                    "{}: {text:?} is not translated in {locale}",
                    entry.id
                );
            }
        }
    }
}

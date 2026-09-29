// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    time::Duration,
};

use serde_norway::{Mapping, Value};

use super::{
    ProcessSignal, RestartMode, RestartOps, UdiskieInstallState, config_json, config_yml,
    discover_udiskie_at, install_at, managed_event_hook, read_state, restart_udiskie_at,
    state_directory, status_at, uninstall_at,
};
use crate::portal_setup::SetupContext;

#[test]
fn install_empty_file_writes_managed_mapping() {
    let fixture = fixture();
    let context = context(fixture.path());
    let executable = executable(fixture.path());
    let yml = config_yml(&context);
    fs::create_dir_all(yml.parent().expect("udiskie config directory"))
        .expect("udiskie config directory");
    fs::write(&yml, b"").expect("empty config");

    install_at(&context, &executable).expect("install");

    let contents = fs::read_to_string(&yml).expect("installed config");
    assert!(
        contents.starts_with(super::MANAGED_HEADER),
        "installed config should start with the managed header"
    );
    let root = mapping(&yml);
    assert_event_hook(&root, &executable);
    assert_eq!(
        root.get("program_options")
            .and_then(Value::as_mapping)
            .and_then(|options| options.get("password_prompt")),
        Some(&Value::Bool(false)),
        "password_prompt should be boolean false"
    );
    assert!(
        root.get("device_config")
            .and_then(Value::as_sequence)
            .and_then(|devices| devices.first())
            .is_some_and(super::is_luks_automount_off),
        "LUKS automount-off rule should be first"
    );
    let state = load_state(&context);
    assert!(
        state.added_luks_automount_rule,
        "empty file should record that the LUKS rule was inserted"
    );
    assert_eq!(state.previous_event_hook, None);
    assert_eq!(state.previous_password_prompt, None);
}

#[test]
fn install_preserves_unrelated_program_options() {
    let fixture = fixture();
    let context = context(fixture.path());
    let executable = executable(fixture.path());
    write_yml(
        &context,
        "program_options:\n  file_manager: thunar\n  notify: true\necho: stay\n",
    );

    install_at(&context, &executable).expect("install");

    let root = mapping(&config_yml(&context));
    let options = root
        .get("program_options")
        .and_then(Value::as_mapping)
        .expect("program_options mapping");
    assert_eq!(
        options.get("file_manager").and_then(Value::as_str),
        Some("thunar"),
        "unrelated program_options keys should be preserved"
    );
    assert_eq!(
        options.get("notify").and_then(Value::as_bool),
        Some(true),
        "unrelated program_options keys should be preserved"
    );
    assert_eq!(
        root.get("echo").and_then(Value::as_str),
        Some("stay"),
        "unrelated top-level keys should be preserved"
    );
    assert_event_hook(&root, &executable);
}

#[test]
fn uninstall_restores_previous_hook_and_prompt_types() {
    let cases = [
        (
            "program_options:\n  event_hook:\n    - /usr/bin/notify-send\n    - '{event}'\n",
            "event_hook",
            Value::Sequence(vec![
                Value::String("/usr/bin/notify-send".into()),
                Value::String("{event}".into()),
            ]),
        ),
        (
            "program_options:\n  event_hook: echo plugged\n",
            "event_hook",
            Value::String("echo plugged".into()),
        ),
        (
            "program_options:\n  password_prompt: false\n",
            "password_prompt",
            Value::Bool(false),
        ),
    ];
    for (original, key, expected) in cases {
        let fixture = fixture();
        let context = context(fixture.path());
        write_yml(&context, original);
        install_at(&context, &executable(fixture.path())).expect("install");
        uninstall_at(&context).expect("uninstall");
        let restored = mapping(&config_yml(&context))
            .get("program_options")
            .and_then(Value::as_mapping)
            .and_then(|options| options.get(key))
            .cloned()
            .unwrap_or_else(|| panic!("restored {key}"));
        assert_eq!(
            restored, expected,
            "{key} should restore the original YAML type from {original}"
        );
    }
}

#[test]
fn uninstall_deletes_previously_unset_keys() {
    let fixture = fixture();
    let context = context(fixture.path());
    let executable = executable(fixture.path());
    write_yml(&context, "program_options:\n  file_manager: thunar\n");

    install_at(&context, &executable).expect("install");
    uninstall_at(&context).expect("uninstall");

    let root = mapping(&config_yml(&context));
    let options = root
        .get("program_options")
        .and_then(Value::as_mapping)
        .expect("program_options");
    assert!(
        options.get("event_hook").is_none(),
        "previously unset event_hook should be deleted"
    );
    assert!(
        options.get("password_prompt").is_none(),
        "previously unset password_prompt should be deleted"
    );
    assert_eq!(
        options.get("file_manager").and_then(Value::as_str),
        Some("thunar")
    );
}

#[test]
fn install_inserts_luks_rule_at_front() {
    let fixture = fixture();
    let context = context(fixture.path());
    let executable = executable(fixture.path());
    write_yml(
        &context,
        "device_config:\n  - is_ntfs: true\n    automount: true\n",
    );

    install_at(&context, &executable).expect("install");

    let devices = mapping(&config_yml(&context))
        .get("device_config")
        .and_then(Value::as_sequence)
        .cloned()
        .expect("device_config");
    assert!(
        super::is_luks_automount_off(&devices[0]),
        "LUKS rule should be inserted at the front"
    );
    assert_eq!(
        devices[1].as_mapping().and_then(|rule| rule.get("is_ntfs")),
        Some(&Value::Bool(true)),
        "existing rules should shift back"
    );
    assert!(
        load_state(&context).added_luks_automount_rule,
        "inserting the LUKS rule should set added_luks_automount_rule"
    );
}

#[test]
fn install_skips_identical_front_luks_rule() {
    let fixture = fixture();
    let context = context(fixture.path());
    let executable = executable(fixture.path());
    write_yml(
        &context,
        "device_config:\n  - is_luks: true\n    automount: false\n  - is_ntfs: true\n",
    );

    install_at(&context, &executable).expect("install");

    let devices = mapping(&config_yml(&context))
        .get("device_config")
        .and_then(Value::as_sequence)
        .cloned()
        .expect("device_config");
    assert_eq!(
        devices.len(),
        2,
        "identical front rule should not be duplicated"
    );
    assert!(super::is_luks_automount_off(&devices[0]));
    assert!(
        !load_state(&context).added_luks_automount_rule,
        "pre-existing front LUKS rule should not set added_luks_automount_rule"
    );
}

#[test]
fn uninstall_keeps_preexisting_front_luks_rule() {
    let fixture = fixture();
    let context = context(fixture.path());
    let executable = executable(fixture.path());
    write_yml(
        &context,
        "device_config:\n  - is_luks: true\n    automount: false\n  - is_ntfs: true\n",
    );

    install_at(&context, &executable).expect("install");
    uninstall_at(&context).expect("uninstall");

    let devices = mapping(&config_yml(&context))
        .get("device_config")
        .and_then(Value::as_sequence)
        .cloned()
        .expect("device_config");
    assert_eq!(devices.len(), 2);
    assert!(
        super::is_luks_automount_off(&devices[0]),
        "uninstall should not delete a user-written front LUKS rule"
    );
}

#[test]
fn install_refuses_malformed_config_without_writing() {
    for (original, needle) in [
        (
            "device_config:\n  is_luks: true\n  automount: false\n",
            "device_config must be a sequence",
        ),
        (
            "program_options: true\n",
            "program_options must be a mapping",
        ),
    ] {
        let fixture = fixture();
        let context = context(fixture.path());
        write_yml(&context, original);
        let error = install_at(&context, &executable(fixture.path())).expect_err("refuse");
        assert!(
            error.contains(needle),
            "error should mention {needle}, got {error}"
        );
        assert_eq!(
            fs::read_to_string(config_yml(&context)).expect("unchanged"),
            original,
            "refuse should not write"
        );
        assert!(
            read_state(&state_directory(&context))
                .expect("state")
                .is_none(),
            "refuse should not record install state"
        );
    }
}

#[test]
fn install_refuses_symlink_target() {
    let fixture = fixture();
    let context = context(fixture.path());
    let yml = config_yml(&context);
    fs::create_dir_all(yml.parent().expect("parent")).expect("parent");
    let target = fixture.path().join("elsewhere.yml");
    fs::write(&target, "program_options: {}\n").expect("symlink destination");
    std::os::unix::fs::symlink(&target, &yml).expect("symlink config");

    let error = install_at(&context, &executable(fixture.path())).expect_err("refuse symlink");
    assert!(
        error.contains("non-regular"),
        "error should mention non-regular file, got {error}"
    );
    assert!(
        yml.symlink_metadata()
            .expect("metadata")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(&target).expect("destination unchanged"),
        "program_options: {}\n"
    );
}

#[test]
fn json_only_migrates_and_uninstall_restores_json() {
    let fixture = fixture();
    let context = context(fixture.path());
    let executable = executable(fixture.path());
    let json = config_json(&context);
    fs::create_dir_all(json.parent().expect("parent")).expect("parent");
    let original = serde_json::json!({
        "program_options": {
            "file_manager": "thunar",
            "event_hook": "echo old"
        },
        "keep": 1
    });
    fs::write(&json, serde_json::to_vec_pretty(&original).expect("json")).expect("json config");

    install_at(&context, &executable).expect("install");

    let yml = config_yml(&context);
    assert!(yml.is_file(), "install should write config.yml");
    assert!(!json.exists(), "install should remove config.json");
    let root = mapping(&yml);
    assert_event_hook(&root, &executable);
    assert_eq!(
        root.get("program_options")
            .and_then(Value::as_mapping)
            .and_then(|options| options.get("file_manager"))
            .and_then(Value::as_str),
        Some("thunar")
    );
    assert_eq!(root.get("keep").and_then(Value::as_i64), Some(1));
    assert!(
        state_directory(&context).join("config.json.bak").is_file(),
        "JSON backup should remain until uninstall"
    );

    let mut updated = root;
    updated.insert("keep".into(), Value::Number(2.into()));
    fs::write(&yml, super::emit_managed(&updated).expect("yaml")).expect("user edit");
    uninstall_at(&context).expect("uninstall");

    assert!(json.is_file(), "uninstall should restore config.json");
    assert!(!yml.exists(), "uninstall should remove the YAML we created");
    let restored: serde_json::Value =
        serde_json::from_slice(&fs::read(&json).expect("restored json")).expect("parse json");
    let mut expected = original;
    expected["keep"] = 2.into();
    assert_eq!(restored, expected, "restore must preserve unrelated edits");
    assert!(
        !state_directory(&context).join("config.json.bak").exists(),
        "JSON backup should be removed after uninstall"
    );
    assert!(
        read_state(&state_directory(&context))
            .expect("state")
            .is_none()
    );
}

#[test]
fn reinstall_failure_preserves_current_configuration_and_restore_state() {
    for original in [None, Some("program_options: {}\n")] {
        let fixture = fixture();
        let context = context(fixture.path());
        let executable = executable(fixture.path());
        if let Some(original) = original {
            write_yml(&context, original);
        }
        install_at(&context, &executable).expect("install");
        let yml = config_yml(&context);
        let mut root = mapping(&yml);
        root.insert("keep".into(), Value::Bool(true));
        fs::write(&yml, super::emit_managed(&root).expect("yaml")).expect("user edit");
        let before = fs::read(&yml).expect("current configuration");
        let directory = state_directory(&context);
        let state = fs::read(directory.join(super::STATE_FILE)).expect("state");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o555))
            .expect("read-only state directory");
        let result = install_at(&context, &executable);
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755))
            .expect("restore permissions");
        assert!(result.is_err());
        assert_eq!(fs::read(&yml).expect("preserved configuration"), before);
        assert_eq!(
            fs::read(directory.join(super::STATE_FILE)).expect("preserved state"),
            state
        );
    }
}

#[test]
fn json_restore_refuses_to_overwrite_a_new_configuration() {
    let fixture = fixture();
    let context = context(fixture.path());
    let json = config_json(&context);
    fs::create_dir_all(json.parent().expect("parent")).expect("directory");
    fs::write(&json, b"{}").expect("original JSON");
    install_at(&context, &executable(fixture.path())).expect("install");
    fs::write(&json, b"{\"keep\":true}").expect("new JSON");
    assert!(uninstall_at(&context).is_err());
    assert_eq!(
        fs::read(&json).expect("new JSON survives"),
        b"{\"keep\":true}"
    );
    assert!(config_yml(&context).exists());
    assert!(
        read_state(&state_directory(&context))
            .expect("state")
            .is_some()
    );
}

#[test]
fn install_rejects_insecure_executable() {
    let fixture = fixture();
    let context = context(fixture.path());
    let executable = executable(fixture.path());
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o775))
        .expect("group-writable executable");

    assert!(
        install_at(&context, &executable).is_err(),
        "insecure executable should be rejected"
    );
    assert!(
        !config_yml(&context).exists(),
        "rejected install should not write config.yml"
    );
}

#[test]
fn uninstall_missing_source_deletes_empty_config() {
    let fixture = fixture();
    let context = context(fixture.path());
    let executable = executable(fixture.path());

    install_at(&context, &executable).expect("install");
    assert!(config_yml(&context).is_file());
    uninstall_at(&context).expect("uninstall");
    assert!(
        !config_yml(&context).exists(),
        "missing-file install should delete config.yml when only managed keys remain"
    );
}

#[test]
fn status_does_not_write_config() {
    let fixture = fixture();
    let context = context(fixture.path());
    let executable = executable(fixture.path());
    write_yml(&context, "program_options:\n  file_manager: thunar\n");
    let yml = config_yml(&context);
    let before = fs::read(&yml).expect("original bytes");

    status_at(&context, &executable, true, true).expect("status");

    assert_eq!(fs::read(&yml).expect("after status"), before);
    assert!(
        read_state(&state_directory(&context))
            .expect("state")
            .is_none()
    );
}

#[test]
fn discover_matches_cmdline_basename_udiskie() {
    let fixture = fixture();
    let proc_root = fixture.path().join("proc");
    let euid = rustix::process::geteuid().as_raw();
    let current = std::process::id();
    write_process(
        &proc_root,
        11,
        euid,
        "python3",
        &["/usr/bin/python3", "/usr/bin/udiskie", "--automount"],
    );
    write_process(&proc_root, 12, euid, "udiskie", &["/usr/bin/udiskie"]);
    write_process(
        &proc_root,
        13,
        euid,
        "python3",
        &["/usr/bin/python3", "-m", "something_else"],
    );
    write_process(
        &proc_root,
        14,
        euid,
        "udiskie-mount",
        &["/usr/bin/udiskie-mount", "/dev/sdb1"],
    );
    write_process(
        &proc_root,
        current,
        euid,
        "python3",
        &["/usr/bin/python3", "/usr/bin/udiskie"],
    );
    write_process(
        &proc_root,
        15,
        euid.wrapping_add(1),
        "udiskie",
        &["/usr/bin/udiskie"],
    );
    write_process(&proc_root, 16, euid, "cat", &["cat", "/usr/bin/udiskie"]);
    write_process(&proc_root, 17, euid, "man", &["man", "udiskie"]);
    write_process(
        &proc_root,
        18,
        euid,
        "python3",
        &["python3", "backup.py", "/usr/bin/udiskie"],
    );
    write_process(
        &proc_root,
        19,
        euid,
        "python3",
        &["python3", "-c", "print('hello')", "udiskie"],
    );

    let pids: Vec<u32> = discover_udiskie_at(&proc_root, euid, current)
        .into_iter()
        .map(|process| process.pid)
        .collect();
    assert_eq!(
        pids,
        vec![11, 12],
        "only argv0 udiskie or python-plus-udiskie processes for this euid should match"
    );
}

#[test]
fn restart_signals_only_snapshot_pids() {
    let fixture = fixture();
    let proc_root = fixture.path().join("proc");
    let euid = rustix::process::geteuid().as_raw();
    write_process(
        &proc_root,
        101,
        euid,
        "python3",
        &["/usr/bin/python3", "/usr/bin/udiskie", "--automount"],
    );
    write_process(
        &proc_root,
        202,
        euid,
        "udiskie",
        &["/usr/bin/udiskie", "--no-notify"],
    );

    let spawned = RefCell::new(None::<Vec<String>>);
    let signals = RefCell::new(Vec::new());
    let mut spawn = |argv: &[String]| {
        write_process(
            &proc_root,
            303,
            euid,
            "udiskie",
            &argv.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        *spawned.borrow_mut() = Some(argv.to_vec());
        Ok(())
    };
    let mut signal = |pid, kind| signals.borrow_mut().push((pid, kind));
    let mut wait = |_duration: Duration| {};
    let mut is_alive = |_pid: u32| true;
    let mut ops = RestartOps {
        lookup: &|_| None,
        spawn: &mut spawn,
        signal: &mut signal,
        wait: &mut wait,
        is_alive: &mut is_alive,
    };

    let warning = restart_udiskie_at(&proc_root, euid, 1, &mut ops, RestartMode::EnsureRunning)
        .expect("restart");
    assert_eq!(warning, None);
    assert_eq!(
        spawned.borrow().as_ref().map(Vec::as_slice),
        Some(
            [
                "/usr/bin/python3".to_owned(),
                "/usr/bin/udiskie".into(),
                "--automount".into()
            ]
            .as_slice()
        ),
        "successor should use the first snapshot argv"
    );
    assert_eq!(
        signals.borrow().as_slice(),
        [
            (101, ProcessSignal::Term),
            (202, ProcessSignal::Term),
            (101, ProcessSignal::Kill),
            (202, ProcessSignal::Kill)
        ],
        "only snapshot pids should be signaled"
    );
}

#[test]
fn restart_spawn_failure_does_not_signal() {
    let fixture = fixture();
    let proc_root = fixture.path().join("proc");
    let euid = rustix::process::geteuid().as_raw();
    write_process(&proc_root, 101, euid, "udiskie", &["/usr/bin/udiskie"]);

    let signals = RefCell::new(Vec::new());
    let mut spawn = |_argv: &[String]| Err("spawn failed".into());
    let mut signal = |pid, kind| signals.borrow_mut().push((pid, kind));
    let mut wait = |_duration: Duration| {};
    let mut is_alive = |_pid: u32| true;
    let mut ops = RestartOps {
        lookup: &|_| None,
        spawn: &mut spawn,
        signal: &mut signal,
        wait: &mut wait,
        is_alive: &mut is_alive,
    };

    let warning = restart_udiskie_at(&proc_root, euid, 1, &mut ops, RestartMode::EnsureRunning)
        .expect("non-fatal");
    assert_eq!(warning.as_deref(), Some(super::RESTART_WARNING));
    assert!(
        signals.borrow().is_empty(),
        "spawn failure must not SIGTERM the running daemon"
    );
}

#[test]
fn restart_empty_snapshot_install_spawns_default() {
    let fixture = fixture();
    let proc_root = fixture.path().join("proc");
    fs::create_dir_all(&proc_root).expect("empty proc");
    let spawned = RefCell::new(None::<Vec<String>>);
    let signals = RefCell::new(Vec::new());
    let mut spawn = |argv: &[String]| {
        *spawned.borrow_mut() = Some(argv.to_vec());
        Ok(())
    };
    let mut signal = |pid, kind| signals.borrow_mut().push((pid, kind));
    let mut wait = |_duration: Duration| {};
    let mut is_alive = |_pid: u32| true;
    let mut ops = RestartOps {
        lookup: &|name| match name {
            "udiskie" => Some(PathBuf::from("/usr/bin/udiskie")),
            _ => None,
        },
        spawn: &mut spawn,
        signal: &mut signal,
        wait: &mut wait,
        is_alive: &mut is_alive,
    };

    let warning = restart_udiskie_at(
        &proc_root,
        rustix::process::geteuid().as_raw(),
        1,
        &mut ops,
        RestartMode::EnsureRunning,
    )
    .expect("restart");
    assert_eq!(warning, None);
    assert_eq!(
        spawned.borrow().as_ref().map(Vec::as_slice),
        Some(
            [
                "/usr/bin/udiskie".to_owned(),
                "--automount".into(),
                "--no-notify".into(),
                "--no-tray".into()
            ]
            .as_slice()
        ),
        "install should spawn default udiskie when none is running"
    );
    assert!(
        signals.borrow().is_empty(),
        "empty snapshot should not signal"
    );
}

#[test]
fn restart_empty_snapshot_uninstall_does_not_spawn() {
    let fixture = fixture();
    let proc_root = fixture.path().join("proc");
    fs::create_dir_all(&proc_root).expect("empty proc");
    let spawned = RefCell::new(None::<Vec<String>>);
    let signals = RefCell::new(Vec::new());
    let mut spawn = |argv: &[String]| {
        *spawned.borrow_mut() = Some(argv.to_vec());
        Ok(())
    };
    let mut signal = |pid, kind| signals.borrow_mut().push((pid, kind));
    let mut wait = |_duration: Duration| {};
    let mut is_alive = |_pid: u32| true;
    let mut ops = RestartOps {
        lookup: &|name| match name {
            "udiskie" => Some(PathBuf::from("/usr/bin/udiskie")),
            _ => None,
        },
        spawn: &mut spawn,
        signal: &mut signal,
        wait: &mut wait,
        is_alive: &mut is_alive,
    };

    let warning = restart_udiskie_at(
        &proc_root,
        rustix::process::geteuid().as_raw(),
        1,
        &mut ops,
        RestartMode::ReloadIfRunning,
    )
    .expect("restart");
    assert_eq!(warning, None);
    assert!(
        spawned.borrow().is_none(),
        "uninstall should not start udiskie when none is running"
    );
    assert!(
        signals.borrow().is_empty(),
        "empty snapshot should not signal"
    );
}

fn context(root: &Path) -> SetupContext {
    let data_home = root.join("data");
    let config_home = root.join("config");
    SetupContext {
        search_roots: vec![config_home.clone()],
        data_home,
        config_home,
        config_names: vec!["portals.conf".to_owned()],
    }
}

fn executable(root: &Path) -> PathBuf {
    let path = root.join("bin/strata");
    fs::create_dir_all(path.parent().expect("executable parent")).expect("executable directory");
    fs::write(&path, b"binary").expect("executable file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("executable permissions");
    path
}

fn fixture() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("strata-udiskie-")
        .tempdir_in("target")
        .expect("fixture directory")
}

fn write_yml(context: &SetupContext, contents: &str) {
    let yml = config_yml(context);
    fs::create_dir_all(yml.parent().expect("parent")).expect("parent");
    fs::write(yml, contents).expect("udiskie yaml");
}

fn mapping(path: &Path) -> Mapping {
    super::into_mapping(
        super::parse_yaml(&fs::read_to_string(path).expect("config")).expect("yaml"),
    )
    .expect("mapping")
}

fn load_state(context: &SetupContext) -> UdiskieInstallState {
    read_state(&state_directory(context))
        .expect("state")
        .expect("installed state")
}

fn assert_event_hook(root: &Mapping, executable: &Path) {
    let expected = managed_event_hook(
        &fs::canonicalize(executable).unwrap_or_else(|_| executable.to_path_buf()),
    );
    let hook = root
        .get("program_options")
        .and_then(Value::as_mapping)
        .and_then(|options| options.get("event_hook"))
        .and_then(Value::as_sequence)
        .expect("event_hook list");
    let actual: Vec<&str> = hook.iter().filter_map(Value::as_str).collect();
    assert_eq!(
        actual,
        expected.iter().map(String::as_str).collect::<Vec<_>>(),
        "event_hook should be the managed argv list"
    );
}

fn write_process(root: &Path, pid: u32, euid: u32, comm: &str, cmdline: &[&str]) {
    let dir = root.join(pid.to_string());
    fs::create_dir_all(&dir).expect("pid directory");
    fs::write(dir.join("comm"), format!("{comm}\n")).expect("comm");
    let mut bytes = Vec::new();
    for token in cmdline {
        bytes.extend_from_slice(token.as_bytes());
        bytes.push(0);
    }
    fs::write(dir.join("cmdline"), bytes).expect("cmdline");
    fs::write(
        dir.join("status"),
        format!("Name:\t{comm}\nUid:\t{euid}\t{euid}\t{euid}\t{euid}\n"),
    )
    .expect("status");
}

// SPDX-License-Identifier: MIT

use std::{
    collections::HashSet,
    env,
    ffi::OsStr,
    fs, io,
    os::unix::ffi::OsStrExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_norway::{Mapping, Value};

use super::{
    SetupContext, file_mode, path_error, read_utf8, remove_if_exists, secure_executable,
    write_config,
};

#[cfg(test)]
mod tests;

const STATE_DIRECTORY: &str = "strata/udiskie-install";
const STATE_FILE: &str = "state.toml";
const CONFIG_YML: &str = "config.yml";
const CONFIG_JSON: &str = "config.json";
const JSON_BACKUP: &str = "config.json.bak";
const YAML_BACKUP: &str = "config.yml.bak";
const MANAGED_HEADER: &str =
    "# Managed by Strata. Unlock encrypted volumes. Do not edit this header.";
const RESTART_WARNING: &str =
    "Saved the udiskie configuration. Restart udiskie or log out for it to take effect.";
const TERMINATION_WAIT: Duration = Duration::from_secs(2);
const DEFAULT_UDISKIE_FLAGS: [&str; 3] = ["--automount", "--no-notify", "--no-tray"];
const SESSION_ENVIRONMENT: [&str; 4] = [
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "XDG_SESSION_TYPE",
    "HYPRLAND_INSTANCE_SIGNATURE",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UdiskieStatus {
    pub available: bool,
    pub configured: bool,
    pub has_installation: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum HookValue {
    Bool(bool),
    String(String),
    List(Vec<String>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum SourceFormat {
    Missing,
    Yaml,
    Json,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct UdiskieInstallState {
    previous_event_hook: Option<HookValue>,
    previous_password_prompt: Option<HookValue>,
    added_luks_automount_rule: bool,
    target_name: String,
    source_format: SourceFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProcessSignal {
    Term,
    Kill,
}

struct UdiskieProcess {
    pid: u32,
    argv: Vec<String>,
}

struct DetachedLaunch {
    program: PathBuf,
    args: Vec<String>,
}

struct RestartOps<'a> {
    lookup: &'a dyn Fn(&str) -> Option<PathBuf>,
    spawn: &'a mut dyn FnMut(&[String]) -> Result<(), String>,
    signal: &'a mut dyn FnMut(u32, ProcessSignal),
    wait: &'a mut dyn FnMut(Duration),
    is_alive: &'a mut dyn FnMut(u32) -> bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RestartMode {
    EnsureRunning,
    ReloadIfRunning,
}

pub(crate) fn install() -> Result<String, String> {
    let executable = env::current_exe()
        .map_err(|error| format!("Could not locate the Strata executable: {error}"))?;
    let context = SetupContext::from_environment()?;
    let config = install_at(&context, &executable)?;
    let restart_warning = restart_udiskie(RestartMode::EnsureRunning);
    Ok(format!(
        "Installed Strata as the encrypted-volume unlock handler.\nConfiguration: {}{}",
        config.display(),
        restart_warning
    ))
}

pub(crate) fn uninstall() -> Result<String, String> {
    let context = SetupContext::from_environment()?;
    uninstall_at(&context)?;
    let restart_warning = restart_udiskie(RestartMode::ReloadIfRunning);
    Ok(format!(
        "Removed the Strata encrypted-volume unlock integration.{restart_warning}"
    ))
}

pub(crate) fn status() -> Result<UdiskieStatus, String> {
    let executable = env::current_exe()
        .map_err(|error| format!("Could not locate the Strata executable: {error}"))?;
    status_at(
        &SetupContext::from_environment()?,
        &executable,
        super::omarchy_is_present(),
        glib::find_program_in_path("udiskie").is_some(),
    )
}

fn install_at(context: &SetupContext, executable: &Path) -> Result<PathBuf, String> {
    let executable = secure_executable(executable)?;
    let executable = utf8_path(&executable)?;
    let hook = managed_event_hook(Path::new(&executable));

    let yml = config_yml(context);
    let json = config_json(context);
    ensure_udiskie_target(&yml)?;

    let state_directory = state_directory(context);
    let existing_state = read_state(&state_directory)?;
    let original_yml = snapshot_config(&yml)?;
    let original_json = snapshot_config(&json)?;
    let state_path = state_directory.join(STATE_FILE);
    let original_state = snapshot_config(&state_path)?;
    if existing_state
        .as_ref()
        .is_some_and(|state| state.source_format == SourceFormat::Json)
        && original_json.is_some()
    {
        return Err("Refusing to overwrite a JSON configuration created after installation".into());
    }

    let (source_format, original, yml_existed, parse_as_json) = if existing_state.is_some() {
        let bytes = if yml.exists() {
            fs::read(&yml).map_err(|error| path_error("read", &yml, error))?
        } else {
            Vec::new()
        };
        (
            existing_state
                .as_ref()
                .map(|state| state.source_format)
                .unwrap_or(SourceFormat::Missing),
            bytes,
            yml.exists(),
            false,
        )
    } else if yml.exists() {
        let bytes = fs::read(&yml).map_err(|error| path_error("read", &yml, error))?;
        (SourceFormat::Yaml, bytes, true, false)
    } else if json.exists() {
        ensure_udiskie_target(&json)?;
        let bytes = fs::read(&json).map_err(|error| path_error("read", &json, error))?;
        (SourceFormat::Json, bytes, false, true)
    } else {
        (SourceFormat::Missing, Vec::new(), false, false)
    };

    let document =
        if parse_as_json {
            parse_json(&original)?
        } else {
            parse_yaml(std::str::from_utf8(&original).map_err(|_| {
                format!("udiskie configuration {} is not valid UTF-8", yml.display())
            })?)?
        };
    let mut root = into_mapping(document)?;
    let (previous_event_hook, previous_password_prompt) = match &existing_state {
        Some(state) => (
            state.previous_event_hook.clone(),
            state.previous_password_prompt.clone(),
        ),
        None => capture_previous(&root)?,
    };
    let added_luks = overlay_managed(&mut root, hook)?;
    let added_luks_automount_rule = existing_state
        .as_ref()
        .map(|state| state.added_luks_automount_rule)
        .unwrap_or(added_luks);
    let source_format = existing_state
        .as_ref()
        .map(|state| state.source_format)
        .unwrap_or(source_format);

    let emitted = emit_managed(&root)?;
    let mode = if yml_existed {
        file_mode(&yml)
    } else if source_format == SourceFormat::Json {
        file_mode(&json)
    } else {
        file_mode(&yml)
    };

    if let Some(parent) = yml.parent() {
        fs::create_dir_all(parent).map_err(|error| path_error("create", parent, error))?;
    }
    fs::create_dir_all(&state_directory)
        .map_err(|error| path_error("create", &state_directory, error))?;

    let yaml_backup = state_directory.join(YAML_BACKUP);
    let json_backup = state_directory.join(JSON_BACKUP);
    if existing_state.is_none() {
        match source_format {
            SourceFormat::Yaml => {
                write_config(&yaml_backup, &original, mode)?;
            }
            SourceFormat::Json => {
                write_config(&json_backup, &original, mode)?;
            }
            SourceFormat::Missing => {}
        }
    }

    let written = (|| {
        write_config(&yml, emitted.as_bytes(), mode)?;
        if source_format == SourceFormat::Json {
            remove_if_exists(&json)?;
        }
        write_state(
            &state_directory,
            &UdiskieInstallState {
                previous_event_hook,
                previous_password_prompt,
                added_luks_automount_rule,
                target_name: CONFIG_YML.to_owned(),
                source_format,
            },
        )?;
        validate_managed(&yml, Path::new(&executable))?;
        Ok(())
    })();

    if let Err(error) = written {
        for (path, snapshot) in [
            (&yml, &original_yml),
            (&json, &original_json),
            (&state_path, &original_state),
        ] {
            if let Err(rollback) = restore_snapshot(path, snapshot) {
                return Err(format!("{error}; rollback failed: {rollback}"));
            }
        }
        return Err(error);
    }

    if source_format == SourceFormat::Yaml {
        remove_if_exists(&yaml_backup)?;
    }
    Ok(yml)
}

fn uninstall_at(context: &SetupContext) -> Result<(), String> {
    let state_directory = state_directory(context);
    let yml = config_yml(context);
    let json = config_json(context);
    let Some(state) = read_state(&state_directory)? else {
        remove_if_exists(&state_directory.join(YAML_BACKUP))?;
        remove_if_exists(&state_directory.join(JSON_BACKUP))?;
        return Ok(());
    };

    match state.source_format {
        SourceFormat::Json => {
            ensure_udiskie_target(&json)?;
            ensure_udiskie_target(&yml)?;
            if json.exists() {
                return Err(
                    "Refusing to overwrite a JSON configuration created after installation".into(),
                );
            }
            let backup = state_directory.join(JSON_BACKUP);
            let (bytes, mode) = if yml.exists() {
                let mut root = into_mapping(parse_yaml(&read_utf8(&yml)?)?)?;
                restore_managed(&mut root, &state);
                let bytes = serde_json::to_vec_pretty(&root).map_err(|error| {
                    format!("Could not restore udiskie JSON configuration: {error}")
                })?;
                (bytes, file_mode(&yml))
            } else {
                ensure_udiskie_target(&backup)?;
                (
                    fs::read(&backup).map_err(|error| path_error("read", &backup, error))?,
                    file_mode(&backup),
                )
            };
            write_config(&json, &bytes, mode)?;
            remove_if_exists(&yml)?;
        }
        SourceFormat::Yaml | SourceFormat::Missing => {
            if yml.exists() {
                ensure_udiskie_target(&yml)?;
                let current = read_utf8(&yml)?;
                let mut root = into_mapping(parse_yaml(&current)?)?;
                restore_managed(&mut root, &state);
                if state.source_format == SourceFormat::Missing && document_has_no_user_keys(&root)
                {
                    remove_if_exists(&yml)?;
                } else if root.is_empty() {
                    write_config(&yml, b"", file_mode(&yml))?;
                } else {
                    write_config(&yml, emit_yaml(&root)?.as_bytes(), file_mode(&yml))?;
                }
            }
        }
    }

    remove_if_exists(&state_directory.join(STATE_FILE))?;
    remove_if_exists(&state_directory.join(YAML_BACKUP))?;
    remove_if_exists(&state_directory.join(JSON_BACKUP))?;
    match fs::remove_dir(&state_directory) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(path_error("remove", &state_directory, error)),
    }
}

fn status_at(
    context: &SetupContext,
    executable: &Path,
    omarchy_present: bool,
    udiskie_on_path: bool,
) -> Result<UdiskieStatus, String> {
    let executable = fs::canonicalize(executable).unwrap_or_else(|_| executable.to_path_buf());
    let yml = config_yml(context);
    let state_path = state_directory(context).join(STATE_FILE);
    let contents = if yml.exists() {
        ensure_udiskie_target(&yml)?;
        Some(read_utf8(&yml)?)
    } else {
        None
    };
    let parsed = contents
        .as_deref()
        .map(parse_yaml)
        .transpose()?
        .and_then(|value| value.as_mapping().cloned());
    let configured = parsed
        .as_ref()
        .is_some_and(|root| is_configured(root, &executable));
    let has_installation = state_path.exists()
        || contents
            .as_deref()
            .is_some_and(|text| text.starts_with(MANAGED_HEADER))
        || parsed
            .as_ref()
            .is_some_and(|root| event_hook_is_managed(root, &executable));
    Ok(UdiskieStatus {
        available: omarchy_present && udiskie_on_path,
        configured,
        has_installation,
    })
}

fn managed_event_hook(executable: &Path) -> Vec<String> {
    vec![
        executable.display().to_string(),
        "--udiskie-hook".into(),
        "{event}".into(),
        "{id_usage}".into(),
        "{device_file}".into(),
        "{id_uuid}".into(),
    ]
}

fn overlay_managed(root: &mut Mapping, hook: Vec<String>) -> Result<bool, String> {
    if let Some(existing) = root.get("program_options")
        && !existing.is_mapping()
    {
        return Err(
            "Refusing to change udiskie configuration: program_options must be a mapping".into(),
        );
    }
    if !root.contains_key("program_options") {
        root.insert("program_options".into(), Value::Mapping(Mapping::new()));
    }
    let options = root
        .get_mut("program_options")
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| {
            "Refusing to change udiskie configuration: program_options must be a mapping".to_owned()
        })?;
    options.insert(
        "event_hook".into(),
        Value::Sequence(hook.into_iter().map(Value::String).collect()),
    );
    options.insert("password_prompt".into(), Value::Bool(false));

    if let Some(existing) = root.get("device_config")
        && !matches!(existing, Value::Sequence(_))
    {
        return Err(
            "Refusing to change udiskie configuration: device_config must be a sequence".into(),
        );
    }
    if !root.contains_key("device_config") {
        root.insert("device_config".into(), Value::Sequence(Vec::new()));
    }
    let devices = root
        .get_mut("device_config")
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| {
            "Refusing to change udiskie configuration: device_config must be a sequence".to_owned()
        })?;
    Ok(ensure_luks_rule(devices))
}

fn ensure_luks_rule(devices: &mut Vec<Value>) -> bool {
    if devices.first().is_some_and(is_luks_automount_off) {
        return false;
    }
    devices.insert(0, luks_automount_off());
    true
}

fn restore_managed(root: &mut Mapping, state: &UdiskieInstallState) {
    let restore_options = root.get("program_options").is_some_and(Value::is_mapping)
        || (root.get("program_options").is_none()
            && (state.previous_event_hook.is_some() || state.previous_password_prompt.is_some()));
    if restore_options {
        if !root.contains_key("program_options") {
            root.insert("program_options".into(), Value::Mapping(Mapping::new()));
        }
        if let Some(options) = root
            .get_mut("program_options")
            .and_then(Value::as_mapping_mut)
        {
            restore_hook(options, "event_hook", state.previous_event_hook.as_ref());
            restore_hook(
                options,
                "password_prompt",
                state.previous_password_prompt.as_ref(),
            );
            if options.is_empty() {
                root.remove("program_options");
            }
        }
    }

    if state.added_luks_automount_rule {
        let empty = root
            .get_mut("device_config")
            .and_then(Value::as_sequence_mut)
            .map(|devices| {
                if let Some(index) = devices.iter().position(is_luks_automount_off) {
                    devices.remove(index);
                }
                devices.is_empty()
            })
            .unwrap_or(false);
        if empty {
            root.remove("device_config");
        }
    }
}

fn restore_hook(options: &mut Mapping, name: &str, previous: Option<&HookValue>) {
    match previous {
        Some(value) => {
            options.insert(name.into(), hook_to_value(value));
        }
        None => {
            options.remove(name);
        }
    }
}

fn capture_previous(root: &Mapping) -> Result<(Option<HookValue>, Option<HookValue>), String> {
    let options = match root.get("program_options") {
        None => return Ok((None, None)),
        Some(value) if value.is_mapping() => value.as_mapping(),
        Some(_) => {
            return Err(
                "Refusing to change udiskie configuration: program_options must be a mapping"
                    .into(),
            );
        }
    };
    let Some(options) = options else {
        return Ok((None, None));
    };
    Ok((
        capture_hook(options, "event_hook")?,
        capture_hook(options, "password_prompt")?,
    ))
}

fn capture_hook(options: &Mapping, name: &str) -> Result<Option<HookValue>, String> {
    match options.get(name) {
        None => Ok(None),
        Some(value) => Ok(Some(hook_from_value(value, name)?)),
    }
}

fn hook_from_value(value: &Value, name: &str) -> Result<HookValue, String> {
    match value {
        Value::Bool(flag) => Ok(HookValue::Bool(*flag)),
        Value::String(text) => Ok(HookValue::String(text.clone())),
        Value::Sequence(items) => {
            let mut list = Vec::with_capacity(items.len());
            for item in items {
                let Some(text) = item.as_str() else {
                    return Err(format!(
                        "Refusing to change udiskie configuration: {name} list entries must be strings"
                    ));
                };
                list.push(text.to_owned());
            }
            Ok(HookValue::List(list))
        }
        _ => Err(format!(
            "Refusing to change udiskie configuration: {name} must be a boolean, string, or list"
        )),
    }
}

fn hook_to_value(value: &HookValue) -> Value {
    match value {
        HookValue::Bool(flag) => Value::Bool(*flag),
        HookValue::String(text) => Value::String(text.clone()),
        HookValue::List(items) => {
            Value::Sequence(items.iter().cloned().map(Value::String).collect())
        }
    }
}

fn luks_automount_off() -> Value {
    let mut rule = Mapping::new();
    rule.insert("is_luks".into(), Value::Bool(true));
    rule.insert("automount".into(), Value::Bool(false));
    Value::Mapping(rule)
}

fn is_luks_automount_off(value: &Value) -> bool {
    let Some(mapping) = value.as_mapping() else {
        return false;
    };
    mapping.len() == 2
        && mapping.get("is_luks") == Some(&Value::Bool(true))
        && mapping.get("automount") == Some(&Value::Bool(false))
}

fn is_configured(root: &Mapping, executable: &Path) -> bool {
    event_hook_is_managed(root, executable)
        && password_prompt_is_false(root)
        && root
            .get("device_config")
            .and_then(Value::as_sequence)
            .and_then(|devices| devices.first())
            .is_some_and(is_luks_automount_off)
}

fn event_hook_is_managed(root: &Mapping, executable: &Path) -> bool {
    let Some(options) = root.get("program_options").and_then(Value::as_mapping) else {
        return false;
    };
    let Some(Value::Sequence(items)) = options.get("event_hook") else {
        return false;
    };
    let expected = managed_event_hook(executable);
    if items.len() != expected.len() {
        return false;
    }
    items
        .iter()
        .zip(expected)
        .all(|(item, want)| item.as_str() == Some(want.as_str()))
}

fn password_prompt_is_false(root: &Mapping) -> bool {
    root.get("program_options")
        .and_then(Value::as_mapping)
        .and_then(|options| options.get("password_prompt"))
        .is_some_and(|value| value.as_bool() == Some(false))
}

fn document_has_no_user_keys(root: &Mapping) -> bool {
    root.iter().all(|(key, value)| match key.as_str() {
        Some("program_options") => value.as_mapping().is_some_and(Mapping::is_empty),
        Some("device_config") => value.as_sequence().is_some_and(Vec::is_empty),
        _ => false,
    })
}

fn into_mapping(value: Value) -> Result<Mapping, String> {
    match value {
        Value::Mapping(mapping) => Ok(mapping),
        Value::Null => Ok(Mapping::new()),
        _ => Err("Refusing to change udiskie configuration: the document must be a mapping".into()),
    }
}

fn parse_yaml(text: &str) -> Result<Value, String> {
    if text.trim().is_empty() {
        return Ok(Value::Mapping(Mapping::new()));
    }
    serde_norway::from_str(text)
        .map_err(|error| format!("Could not parse udiskie configuration: {error}"))
}

fn parse_json(bytes: &[u8]) -> Result<Value, String> {
    let json: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("Could not parse udiskie JSON configuration: {error}"))?;
    serde_norway::to_value(json)
        .map_err(|error| format!("Could not convert udiskie JSON configuration: {error}"))
}

fn emit_yaml(root: &Mapping) -> Result<String, String> {
    serde_norway::to_string(root)
        .map_err(|error| format!("Could not write udiskie configuration: {error}"))
}

fn emit_managed(root: &Mapping) -> Result<String, String> {
    Ok(format!("{MANAGED_HEADER}\n{}", emit_yaml(root)?))
}

fn validate_managed(path: &Path, executable: &Path) -> Result<(), String> {
    let contents = read_utf8(path)?;
    if !contents.starts_with(MANAGED_HEADER) {
        return Err("The written udiskie configuration is missing the managed header".into());
    }
    let root = into_mapping(parse_yaml(&contents)?)?;
    if !is_configured(&root, executable) {
        return Err(
            "The written udiskie configuration is missing the managed unlock settings".into(),
        );
    }
    Ok(())
}

fn snapshot_config(path: &Path) -> Result<Option<(Vec<u8>, u32)>, String> {
    ensure_udiskie_target(path)?;
    match fs::read(path) {
        Ok(bytes) => Ok(Some((bytes, file_mode(path)))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(path_error("read", path, error)),
    }
}

fn restore_snapshot(path: &Path, snapshot: &Option<(Vec<u8>, u32)>) -> Result<(), String> {
    match snapshot {
        Some((bytes, mode)) => write_config(path, bytes, *mode),
        None => remove_if_exists(path),
    }
}

fn ensure_udiskie_target(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(format!(
            "Refusing to replace non-regular udiskie configuration {}",
            path.display()
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(path_error("inspect", path, error)),
    }
}

fn config_yml(context: &SetupContext) -> PathBuf {
    context.config_home.join("udiskie").join(CONFIG_YML)
}

fn config_json(context: &SetupContext) -> PathBuf {
    context.config_home.join("udiskie").join(CONFIG_JSON)
}

fn state_directory(context: &SetupContext) -> PathBuf {
    context.data_home.join(STATE_DIRECTORY)
}

fn write_state(directory: &Path, state: &UdiskieInstallState) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|error| path_error("create", directory, error))?;
    let path = directory.join(STATE_FILE);
    let contents = toml::to_string(state)
        .map_err(|error| format!("Could not serialize udiskie installation state: {error}"))?;
    crate::storage::atomic_write(&path, contents.as_bytes())
        .map_err(|error| path_error("write", &path, error))
}

fn read_state(directory: &Path) -> Result<Option<UdiskieInstallState>, String> {
    let path = directory.join(STATE_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let contents = read_utf8(&path)?;
    toml::from_str(&contents)
        .map_err(|error| format!("Could not read udiskie installation state: {error}"))
        .map(Some)
}

fn utf8_path(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| "Strata must be installed at a UTF-8 path".to_owned())
}

fn program_on_path(name: &str) -> Option<PathBuf> {
    glib::find_program_in_path(name)
}

fn restart_udiskie(mode: RestartMode) -> String {
    let mut spawn = |argv: &[String]| spawn_detached_with(argv, program_on_path);
    let mut signal = send_os_signal;
    let mut wait = |duration| std::thread::sleep(duration);
    let mut is_alive = |pid: u32| Path::new("/proc").join(pid.to_string()).exists();
    let mut ops = RestartOps {
        lookup: &program_on_path,
        spawn: &mut spawn,
        signal: &mut signal,
        wait: &mut wait,
        is_alive: &mut is_alive,
    };
    match restart_udiskie_at(
        Path::new("/proc"),
        rustix::process::geteuid().as_raw(),
        std::process::id(),
        &mut ops,
        mode,
    ) {
        Ok(None) => String::new(),
        Ok(Some(warning)) => format!("\n{warning}"),
        Err(error) => {
            tracing::warn!("Could not restart udiskie: {error}");
            format!("\n{RESTART_WARNING}")
        }
    }
}

fn restart_udiskie_at(
    proc_root: &Path,
    euid: u32,
    current_pid: u32,
    ops: &mut RestartOps<'_>,
    mode: RestartMode,
) -> Result<Option<String>, String> {
    let snapshot = discover_udiskie_at(proc_root, euid, current_pid);
    if snapshot.is_empty() && mode == RestartMode::ReloadIfRunning {
        return Ok(None);
    }
    let argv = snapshot
        .iter()
        .map(|process| process.argv.clone())
        .find(|argv| argv_is_usable(argv))
        .or_else(|| default_udiskie_argv(ops.lookup));
    let Some(argv) = argv else {
        return Ok(Some(RESTART_WARNING.to_owned()));
    };
    if let Err(error) = (ops.spawn)(&argv) {
        tracing::warn!("Could not relaunch udiskie: {error}");
        return Ok(Some(RESTART_WARNING.to_owned()));
    }
    let pids: Vec<u32> = snapshot.iter().map(|process| process.pid).collect();
    if pids.is_empty() {
        return Ok(None);
    }
    for pid in &pids {
        (ops.signal)(*pid, ProcessSignal::Term);
    }
    (ops.wait)(TERMINATION_WAIT);
    for pid in &pids {
        if (ops.is_alive)(*pid) {
            (ops.signal)(*pid, ProcessSignal::Kill);
        }
    }
    Ok(None)
}

fn argv_is_usable(argv: &[String]) -> bool {
    !argv.is_empty() && argv.iter().all(|token| !token.is_empty())
}

fn default_udiskie_argv(lookup: &dyn Fn(&str) -> Option<PathBuf>) -> Option<Vec<String>> {
    let binary = lookup("udiskie")?;
    let mut argv = vec![utf8_path(&binary).ok()?];
    argv.extend(DEFAULT_UDISKIE_FLAGS.iter().map(|flag| (*flag).to_owned()));
    Some(argv)
}

fn discover_udiskie_at(proc_root: &Path, euid: u32, current_pid: u32) -> Vec<UdiskieProcess> {
    let Ok(entries) = fs::read_dir(proc_root) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut processes = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid): Option<u32> = name.to_str().and_then(|name| name.parse().ok()) else {
            continue;
        };
        if pid == current_pid || !seen.insert(pid) {
            continue;
        }
        let path = entry.path();
        let Ok(status) = fs::read_to_string(path.join("status")) else {
            continue;
        };
        if effective_uid(&status) != Some(euid) {
            continue;
        }
        let Ok(cmdline) = fs::read(path.join("cmdline")) else {
            continue;
        };
        let tokens: Vec<&[u8]> = cmdline
            .split(|byte| *byte == 0)
            .filter(|token| !token.is_empty())
            .collect();
        if !cmdline_is_udiskie(&tokens) {
            continue;
        }
        let argv = tokens
            .iter()
            .map(|token| str::from_utf8(token).map(str::to_owned))
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_default();
        processes.push(UdiskieProcess { pid, argv });
    }
    processes.sort_by_key(|process| process.pid);
    processes
}

fn cmdline_is_udiskie(tokens: &[&[u8]]) -> bool {
    let Some((argv0, rest)) = tokens.split_first() else {
        return false;
    };
    if token_basename_is(argv0, "udiskie") {
        return true;
    }
    token_is_python(argv0)
        && rest
            .first()
            .is_some_and(|token| token_basename_is(token, "udiskie"))
}

fn token_basename_is(token: &[u8], name: &str) -> bool {
    Path::new(OsStr::from_bytes(token)).file_name() == Some(OsStr::new(name))
}

fn token_is_python(token: &[u8]) -> bool {
    let Some(file_name) = Path::new(OsStr::from_bytes(token)).file_name() else {
        return false;
    };
    let Some(suffix) = file_name.as_bytes().strip_prefix(b"python") else {
        return false;
    };
    suffix
        .iter()
        .all(|byte| byte.is_ascii_digit() || *byte == b'.')
}

fn effective_uid(status: &str) -> Option<u32> {
    for line in status.lines() {
        let mut parts = line.split_whitespace();
        if parts.next() == Some("Uid:") {
            let _real = parts.next()?;
            return parts.next()?.parse().ok();
        }
    }
    None
}

fn detached_command(
    argv: &[String],
    lookup: impl Fn(&str) -> Option<PathBuf>,
    env_is_set: impl Fn(&str) -> bool,
) -> Result<DetachedLaunch, String> {
    if !argv_is_usable(argv) {
        return Err("Cannot relaunch udiskie without a command".into());
    }
    if let Some(systemd_run) = lookup("systemd-run") {
        let mut args = vec!["--user".into(), "--collect".into(), "--quiet".into()];
        for name in SESSION_ENVIRONMENT {
            if env_is_set(name) {
                args.push("-E".into());
                args.push(name.to_owned());
            }
        }
        args.push("--".into());
        args.extend(argv.iter().cloned());
        return Ok(DetachedLaunch {
            program: systemd_run,
            args,
        });
    }
    let setsid = lookup("setsid")
        .ok_or_else(|| "Could not relaunch udiskie: setsid is not available".to_owned())?;
    let mut args = vec!["-f".into()];
    if let Some(uwsm) = lookup("uwsm-app").filter(|_| !command_starts_with(argv, "uwsm-app")) {
        args.push(utf8_path(&uwsm)?);
        args.push("--".into());
    }
    args.extend(argv.iter().cloned());
    Ok(DetachedLaunch {
        program: setsid,
        args,
    })
}

fn command_starts_with(argv: &[String], name: &str) -> bool {
    argv.first()
        .and_then(|token| Path::new(token).file_name()?.to_str())
        == Some(name)
}

fn spawn_detached_with(
    argv: &[String],
    lookup: impl Fn(&str) -> Option<PathBuf>,
) -> Result<(), String> {
    let launch = detached_command(argv, lookup, |name| env::var_os(name).is_some())?;
    let status = Command::new(&launch.program)
        .args(&launch.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("Could not relaunch udiskie: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Could not relaunch udiskie: helper exited {status}"
        ))
    }
}

fn send_os_signal(pid: u32, signal: ProcessSignal) {
    let Ok(raw) = i32::try_from(pid) else {
        return;
    };
    let Some(pid) = rustix::process::Pid::from_raw(raw) else {
        return;
    };
    let signal = match signal {
        ProcessSignal::Term => rustix::process::Signal::TERM,
        ProcessSignal::Kill => rustix::process::Signal::KILL,
    };
    let _ = rustix::process::kill_process(pid, signal);
}

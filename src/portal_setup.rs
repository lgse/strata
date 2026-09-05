// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

use std::{
    env, fs, io,
    io::Write as _,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};

use glib::{KeyFile, KeyFileFlags};

const FILE_CHOOSER_KEY: &str = "org.freedesktop.impl.portal.FileChooser";
const PORTAL_FILE: &str = "strata.portal";
const SERVICE_FILE: &str = "org.freedesktop.impl.portal.desktop.strata.service";
const STATE_DIRECTORY: &str = "strata/portal-install";
const STATE_FILE: &str = "state.toml";

pub(crate) fn install() -> Result<String, String> {
    let executable = env::current_exe()
        .map_err(|error| format!("Could not locate the Strata executable: {error}"))?;
    let context = SetupContext::from_environment()?;
    let config = install_at(&context, &executable)?;
    dismiss_prompt_at(&context)?;
    let restart_warning = refresh_portals();
    Ok(format!(
        "Installed Strata as the per-user file chooser.\nConfiguration: {}{}",
        config.display(),
        restart_warning
    ))
}

pub(crate) fn uninstall() -> Result<String, String> {
    let context = SetupContext::from_environment()?;
    let preserved_edits = uninstall_at(&context)?;
    dismiss_prompt_at(&context)?;
    let restart_warning = refresh_portals();
    let edit_note = if preserved_edits {
        "\nKept the remaining portal configuration and removed only Strata."
    } else {
        ""
    };
    Ok(format!(
        "Removed the per-user Strata file chooser integration.{edit_note}{restart_warning}"
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PortalStatus {
    pub configured: bool,
    pub has_installation: bool,
}

pub(crate) fn status() -> Result<PortalStatus, String> {
    status_at(&SetupContext::from_environment()?)
}

fn status_at(context: &SetupContext) -> Result<PortalStatus, String> {
    let metadata = context
        .data_home
        .join("xdg-desktop-portal/portals")
        .join(PORTAL_FILE);
    let service = context.data_home.join("dbus-1/services").join(SERVICE_FILE);
    let has_installation = metadata.exists()
        || service.exists()
        || context.state_directory().join(STATE_FILE).exists();
    let preferred = if let Some(path) = active_config(context) {
        let config = KeyFile::new();
        config
            .load_from_data(&read_utf8(&path)?, KeyFileFlags::NONE)
            .map_err(|error| format!("Could not parse portal configuration: {error}"))?;
        config
            .value("preferred", FILE_CHOOSER_KEY)
            .or_else(|_| config.value("preferred", "default"))
            .is_ok_and(|value| {
                backend_values(&value)
                    .first()
                    .is_some_and(|name| name == "strata")
            })
    } else {
        false
    };
    Ok(PortalStatus {
        configured: preferred && metadata.is_file() && service.is_file(),
        has_installation,
    })
}

pub(crate) fn dismiss_prompt() -> Result<String, String> {
    dismiss_prompt_at(&SetupContext::from_environment()?)?;
    Ok("File chooser offer dismissed. You can enable it later in Settings → General → System file chooser.".into())
}

pub(crate) fn take_prompt_offer() -> Result<bool, String> {
    take_prompt_offer_at(&SetupContext::from_environment()?)
}

fn take_prompt_offer_at(context: &SetupContext) -> Result<bool, String> {
    if prompt_path(context).exists() {
        return Ok(false);
    }
    if status_at(context)?.configured {
        dismiss_prompt_at(context)?;
        return Ok(false);
    }
    dismiss_prompt_at(context)
}

fn prompt_path(context: &SetupContext) -> PathBuf {
    context.config_home.join("strata/portal-opt-in-v1")
}

fn dismiss_prompt_at(context: &SetupContext) -> Result<bool, String> {
    let path = prompt_path(context);
    let directory = path.parent().expect("portal prompt has a parent");
    fs::create_dir_all(directory).map_err(|error| path_error("create", directory, error))?;
    // The installer, CLI, and every app window share one first-offer marker.
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
    {
        Ok(mut file) => {
            file.write_all(b"1\n")
                .map_err(|error| path_error("write", &path, error))?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(path_error("create", &path, error)),
    }
}

#[derive(Debug)]
struct SetupContext {
    data_home: PathBuf,
    config_home: PathBuf,
    search_roots: Vec<PathBuf>,
    config_names: Vec<String>,
}

impl SetupContext {
    fn from_environment() -> Result<Self, String> {
        let home = env::var_os("HOME").filter(|value| !value.is_empty());
        let data_home = xdg_home("XDG_DATA_HOME", home.as_deref(), ".local/share")?;
        let config_home = xdg_home("XDG_CONFIG_HOME", home.as_deref(), ".config")?;

        let mut search_roots = vec![config_home.clone()];
        search_roots.extend(xdg_directories("XDG_CONFIG_DIRS", "/etc/xdg"));
        search_roots.push(PathBuf::from("/etc"));
        search_roots.push(data_home.clone());
        search_roots.extend(xdg_directories(
            "XDG_DATA_DIRS",
            "/usr/local/share:/usr/share",
        ));

        let mut config_names = env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .filter_map(portal_config_name)
            .collect::<Vec<_>>();
        config_names.push("portals.conf".to_owned());
        config_names.dedup();

        Ok(Self {
            data_home,
            config_home,
            search_roots,
            config_names,
        })
    }

    fn portal_directory(&self) -> PathBuf {
        self.config_home.join("xdg-desktop-portal")
    }

    fn state_directory(&self) -> PathBuf {
        self.data_home.join(STATE_DIRECTORY)
    }
}

fn xdg_home(
    variable: &str,
    home: Option<&std::ffi::OsStr>,
    fallback: &str,
) -> Result<PathBuf, String> {
    let path = env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(Path::new).map(|path| path.join(fallback)))
        .ok_or_else(|| format!("Neither {variable} nor HOME is set"))?;
    path.is_absolute()
        .then_some(path)
        .ok_or_else(|| format!("{variable} must resolve to an absolute path"))
}

fn xdg_directories(variable: &str, default: &str) -> Vec<PathBuf> {
    let value = env::var_os(variable)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.into());
    env::split_paths(&value)
        .filter(|path| path.is_absolute())
        .collect()
}

fn portal_config_name(desktop: &str) -> Option<String> {
    let desktop = desktop.trim().to_ascii_lowercase();
    (!desktop.is_empty()
        && desktop
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
    .then(|| format!("{desktop}-portals.conf"))
}

fn install_at(context: &SetupContext, executable: &Path) -> Result<PathBuf, String> {
    let executable = secure_executable(executable)?;
    let executable = executable
        .to_str()
        .ok_or_else(|| "Strata must be installed at a UTF-8 path".to_owned())?;
    if executable
        .chars()
        .any(|character| character.is_whitespace() || matches!(character, '\\' | '\'' | '"'))
    {
        return Err(
            "The Strata executable path contains characters unsupported by D-Bus activation"
                .to_owned(),
        );
    }

    let config_directory = context.portal_directory();
    let state_directory = context.state_directory();
    let (target, state, installed) = if let Some(state) = read_state(&state_directory)? {
        let target = config_directory.join(&state.target_name);
        ensure_regular_or_missing(&target)?;
        let current = if target.exists() {
            read_utf8(&target)?
        } else {
            state.installed.clone()
        };
        (target, None, enable_config(&current)?)
    } else {
        let source = active_config(context);
        let target_name = source
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("portals.conf")
            .to_owned();
        let target = config_directory.join(&target_name);
        ensure_regular_or_missing(&target)?;
        let target_existed = target.exists();
        let original = target_existed.then(|| read_utf8(&target)).transpose()?;
        let base = match &original {
            Some(contents) => contents.clone(),
            None => source
                .as_ref()
                .map_or_else(|| Ok(String::new()), |path| read_utf8(path))?,
        };
        let original = original
            .as_deref()
            .map(disable_config)
            .transpose()?
            .unwrap_or_default();
        let installed = enable_config(&base)?;
        (
            target,
            Some(InstallState {
                target_name,
                target_existed,
                original,
                installed: installed.clone(),
            }),
            installed,
        )
    };

    let portal_directory = context.data_home.join("xdg-desktop-portal/portals");
    let service_directory = context.data_home.join("dbus-1/services");
    for directory in [&portal_directory, &service_directory, &config_directory] {
        fs::create_dir_all(directory).map_err(|error| path_error("create", directory, error))?;
    }
    write_public(
        &portal_directory.join(PORTAL_FILE),
        include_bytes!("../data/portal/strata.portal"),
    )?;
    let service =
        include_str!("../data/portal/org.freedesktop.impl.portal.desktop.strata.service.in")
            .replace("@STRATA_EXECUTABLE@", executable);
    write_public(&service_directory.join(SERVICE_FILE), service.as_bytes())?;
    if let Some(state) = state {
        write_state(&state_directory, &state)?;
    }
    write_config(&target, installed.as_bytes(), file_mode(&target))?;
    Ok(target)
}

fn secure_executable(path: &Path) -> Result<PathBuf, String> {
    secure_executable_for_user(path, rustix::process::geteuid().as_raw())
}

fn secure_executable_for_user(path: &Path, effective_user: u32) -> Result<PathBuf, String> {
    let path = fs::canonicalize(path)
        .map_err(|error| path_error("resolve the Strata executable", path, error))?;
    for (index, component) in path.ancestors().enumerate() {
        let metadata = fs::metadata(component)
            .map_err(|error| path_error("inspect the Strata executable path", component, error))?;
        if index == 0 {
            if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
                return Err("The Strata executable must be a regular executable file".to_owned());
            }
        } else if !metadata.is_dir() {
            return Err("The Strata executable path must contain only directories".to_owned());
        }
        if !trusted_owner(metadata.uid(), effective_user) {
            return Err(
                "The Strata executable path must be owned by the current user or root".to_owned(),
            );
        }
        if metadata.permissions().mode() & 0o022 != 0 {
            return Err(
                "The Strata executable path must not be writable by other users".to_owned(),
            );
        }
    }
    Ok(path)
}

fn trusted_owner(owner: u32, effective_user: u32) -> bool {
    owner == 0 || owner == effective_user
}

fn uninstall_at(context: &SetupContext) -> Result<bool, String> {
    let state_directory = context.state_directory();
    let mut preserved_edits = false;
    if let Some(state) = read_state(&state_directory)? {
        let target = context.portal_directory().join(&state.target_name);
        if target.exists() {
            let current = read_utf8(&target)?;
            if current == state.installed {
                if state.target_existed {
                    write_config(&target, state.original.as_bytes(), file_mode(&target))?;
                } else {
                    remove_if_exists(&target)?;
                }
            } else {
                let disabled = disable_config(&current)?;
                write_config(&target, disabled.as_bytes(), file_mode(&target))?;
                preserved_edits = true;
            }
        }
        remove_state(&state_directory)?;
    } else {
        preserved_edits = disable_user_configs(context)?;
    }
    remove_if_exists(
        &context
            .data_home
            .join("xdg-desktop-portal/portals")
            .join(PORTAL_FILE),
    )?;
    remove_if_exists(&context.data_home.join("dbus-1/services").join(SERVICE_FILE))?;
    Ok(preserved_edits)
}

#[derive(Debug, Deserialize, Serialize)]
struct InstallState {
    target_name: String,
    target_existed: bool,
    original: String,
    installed: String,
}

fn write_state(directory: &Path, state: &InstallState) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|error| path_error("create", directory, error))?;
    let path = directory.join(STATE_FILE);
    let contents = toml::to_string(state)
        .map_err(|error| format!("Could not serialize portal installation state: {error}"))?;
    crate::storage::atomic_write(&path, contents.as_bytes())
        .map_err(|error| path_error("write", &path, error))
}

fn read_state(directory: &Path) -> Result<Option<InstallState>, String> {
    let path = directory.join(STATE_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let contents = read_utf8(&path)?;
    let state: InstallState = toml::from_str(&contents)
        .map_err(|error| format!("Could not read portal installation state: {error}"))?;
    if portal_config_name_is_unsafe(&state.target_name) {
        return Err("The saved portal configuration filename is invalid".to_owned());
    }
    Ok(Some(state))
}

fn portal_config_name_is_unsafe(name: &str) -> bool {
    name.is_empty()
        || Path::new(name).file_name().and_then(|value| value.to_str()) != Some(name)
        || !(name == "portals.conf" || name.ends_with("-portals.conf"))
}

fn remove_state(directory: &Path) -> Result<(), String> {
    remove_if_exists(&directory.join(STATE_FILE))?;
    match fs::remove_dir(directory) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(path_error("remove", directory, error)),
    }
}

fn active_config(context: &SetupContext) -> Option<PathBuf> {
    context.search_roots.iter().find_map(|root| {
        context.config_names.iter().find_map(|name| {
            let path = root.join("xdg-desktop-portal").join(name);
            path.is_file().then_some(path)
        })
    })
}

fn enable_config(contents: &str) -> Result<String, String> {
    update_config(contents, true)
}

fn disable_config(contents: &str) -> Result<String, String> {
    update_config(contents, false)
}

fn update_config(contents: &str, enable: bool) -> Result<String, String> {
    let config = KeyFile::new();
    if !contents.is_empty() {
        config
            .load_from_data(contents, KeyFileFlags::KEEP_COMMENTS)
            .map_err(|error| format!("Could not parse portal configuration: {error}"))?;
    }
    let chooser = config.value("preferred", FILE_CHOOSER_KEY).ok();
    if chooser.is_none() && !enable {
        return Ok(contents.to_owned());
    }

    let mut fallback = config
        .value("preferred", "default")
        .map_or_else(|_| vec!["*".to_owned()], |value| backend_values(&value));
    fallback.retain(|value| value != "strata");
    if fallback.is_empty() {
        fallback.push("*".to_owned());
    }
    let mut values = chooser
        .as_deref()
        .map_or_else(|| fallback.clone(), backend_values);
    values.retain(|value| value != "strata");

    if enable {
        if values.is_empty() {
            values = fallback;
        }
        values.insert(0, "strata".to_owned());
        config.set_value(
            "preferred",
            FILE_CHOOSER_KEY,
            &format!("{};", values.join(";")),
        );
    } else if values.is_empty() || values == fallback {
        config
            .remove_key("preferred", FILE_CHOOSER_KEY)
            .map_err(|error| format!("Could not remove portal preference: {error}"))?;
    } else {
        config.set_value(
            "preferred",
            FILE_CHOOSER_KEY,
            &format!("{};", values.join(";")),
        );
    }
    Ok(config.to_data().to_string())
}

fn backend_values(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn disable_user_configs(context: &SetupContext) -> Result<bool, String> {
    let directory = context.portal_directory();
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(path_error("read", &directory, error)),
    };
    let mut changed = false;
    for entry in entries {
        let path = entry
            .map_err(|error| path_error("read", &directory, error))?
            .path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if portal_config_name_is_unsafe(name) || !path.is_file() {
            continue;
        }
        let current = read_utf8(&path)?;
        let disabled = disable_config(&current)?;
        if current != disabled {
            write_config(&path, disabled.as_bytes(), file_mode(&path))?;
            changed = true;
        }
    }
    Ok(changed)
}

fn read_utf8(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| path_error("read", path, error))
}

fn write_public(path: &Path, contents: &[u8]) -> Result<(), String> {
    crate::storage::atomic_write(path, contents)
        .map_err(|error| path_error("write", path, error))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o644))
        .map_err(|error| path_error("set permissions on", path, error))
}

fn write_config(path: &Path, contents: &[u8], mode: u32) -> Result<(), String> {
    crate::storage::atomic_write(path, contents)
        .map_err(|error| path_error("write", path, error))?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| path_error("set permissions on", path, error))
}

fn file_mode(path: &Path) -> u32 {
    fs::metadata(path)
        .map(|metadata| metadata.permissions().mode())
        .unwrap_or(0o600)
}

fn ensure_regular_or_missing(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(format!(
            "Refusing to replace non-regular portal configuration {}",
            path.display()
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(path_error("inspect", path, error)),
    }
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(path_error("remove", path, error)),
    }
}

fn path_error(action: &str, path: &Path, error: io::Error) -> String {
    format!("Could not {action} {}: {error}", path.display())
}

fn refresh_portals() -> &'static str {
    let dbus_reloaded = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.DBus",
            "--object-path",
            "/org/freedesktop/DBus",
            "--method",
            "org.freedesktop.DBus.ReloadConfig",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    let portal_restarted = Command::new("systemctl")
        .args(["--user", "restart", "xdg-desktop-portal.service"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if dbus_reloaded && portal_restarted {
        ""
    } else {
        "\nCould not reload every portal service. Ensure xdg-desktop-portal is installed, then log out and back in before testing."
    }
}

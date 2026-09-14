// SPDX-License-Identifier: MIT

use super::*;

#[cfg(test)]
mod tests;

fn bindings_path(context: &SetupContext) -> Option<(u8, PathBuf)> {
    let major = detected_major()?;
    let path = context.config_home.join("hypr").join(if major == 4 {
        "bindings.lua"
    } else {
        "bindings.conf"
    });
    Some((major, path))
}

pub(super) fn has_installation(context: &SetupContext) -> Result<bool, String> {
    let Some((_, path)) = bindings_path(context) else {
        return Ok(false);
    };
    if !path.exists() {
        return Ok(false);
    }
    Ok(read_utf8(&path)?.contains("strata-installer: file-manager start"))
}

pub(super) fn shortcuts_configured(context: &SetupContext) -> Result<Option<bool>, String> {
    let Some((major, path)) = bindings_path(context) else {
        return Ok(None);
    };
    let contents = if path.exists() {
        read_utf8(&path)?
    } else {
        String::new()
    };
    Ok(Some(shortcuts_configured_in(&contents, major)))
}

fn shortcuts_configured_in(contents: &str, major: u8) -> bool {
    restored_bindings(contents, major).is_ok_and(|replacement| replacement.is_some())
}

pub(super) fn install(context: &SetupContext, executable: &Path) -> Result<(), String> {
    let Some((major, path)) = bindings_path(context) else {
        return Ok(());
    };
    let original = if path.exists() {
        read_utf8(&path)?
    } else {
        String::new()
    };
    let updated = installed_bindings(&original, major, &secure_executable(executable)?)?;
    if original == updated {
        return reload();
    }
    let parent = path.parent().ok_or("Invalid keyboard configuration path")?;
    fs::create_dir_all(parent).map_err(|error| path_error("create", parent, error))?;
    replace_bindings(&path, &original, &updated, reload)
}

fn installed_bindings(original: &str, major: u8, executable: &Path) -> Result<String, String> {
    let executable = executable
        .to_str()
        .ok_or("The shortcut executable path must be UTF-8")?;
    if !executable.starts_with('/')
        || !executable
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/._-".contains(&byte))
    {
        return Err(
            "The Strata executable path contains characters unsupported by keyboard shortcuts."
                .into(),
        );
    }
    let prefix = if major == 4 { "--" } else { "#" };
    let start = format!("{prefix} strata-installer: file-manager start");
    let end = format!("{prefix} strata-installer: file-manager end");
    let commands = if major == 4 {
        format!(
            "hl.unbind(\"SUPER + SHIFT + F\")\nhl.unbind(\"SUPER + ALT + SHIFT + F\")\no.bind(\"SUPER + SHIFT + F\", \"File manager\", {{ launch = \"{executable}\" }})\no.bind(\"SUPER + ALT + SHIFT + F\", \"File manager (cwd)\",\n  \"uwsm-app -- {executable} \\\"$(omarchy-cmd-terminal-cwd)\\\"\")"
        )
    } else {
        format!(
            "unbind = SUPER SHIFT, F\nunbind = SUPER ALT SHIFT, F\nbindd = SUPER SHIFT, F, File manager, exec, uwsm-app -- {executable}\nbindd = SUPER ALT SHIFT, F, File manager (cwd), exec, uwsm-app -- {executable} \"$(omarchy-cmd-terminal-cwd)\""
        )
    };
    let block = format!("{start}\n{commands}\n{end}");
    if restored_bindings(original, major)?.is_some() {
        let from = original
            .find(&start)
            .ok_or("Missing shortcut block start")?;
        let to = original.find(&end).ok_or("Missing shortcut block end")? + end.len();
        Ok(format!("{}{block}{}", &original[..from], &original[to..]))
    } else {
        let separator = if original.is_empty() || original.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        Ok(format!("{original}{separator}{block}\n"))
    }
}

pub(super) fn restore(context: &SetupContext) -> Result<(), String> {
    let Some((major, path)) = bindings_path(context) else {
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }
    let original = read_utf8(&path)?;
    let Some(updated) = restored_bindings(&original, major)? else {
        return Ok(());
    };
    if detected_nautilus().is_none() {
        return Err(
            "Nautilus was not detected; the Strata keyboard shortcuts were left unchanged.".into(),
        );
    }
    replace_bindings(&path, &original, &updated, reload)
}

fn detected_major() -> Option<u8> {
    if let Ok(output) = Command::new("omarchy").arg("version").output()
        && output.status.success()
        && let Some(major) = major_version(&String::from_utf8_lossy(&output.stdout))
    {
        return Some(major);
    }
    let mut paths = vec![PathBuf::from("/usr/share/omarchy/version")];
    if let Some(home) = env::var_os("HOME") {
        paths.push(PathBuf::from(home).join(".local/share/omarchy/version"));
    }
    paths.into_iter().find_map(|path| {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| major_version(&text))
    })
}

fn major_version(text: &str) -> Option<u8> {
    let version = text.trim().strip_prefix("Omarchy ").unwrap_or(text.trim());
    match version.trim_start_matches('v').split('.').next()? {
        "3" => Some(3),
        "4" => Some(4),
        _ => None,
    }
}

fn restored_bindings(original: &str, major: u8) -> Result<Option<String>, String> {
    let prefix = if major == 4 { "--" } else { "#" };
    let start = format!("{prefix} strata-installer: file-manager start");
    let end = format!("{prefix} strata-installer: file-manager end");
    if !original.contains(&start) && !original.contains(&end) {
        return Ok(None);
    }
    if original.matches(&start).count() != 1 || original.matches(&end).count() != 1 {
        return Err("The Strata keyboard shortcut block is ambiguous; restore it manually.".into());
    }
    let start_index = original
        .find(&start)
        .ok_or("Missing shortcut block start")?;
    let end_index = original.find(&end).ok_or("Missing shortcut block end")?;
    if end_index < start_index {
        return Err("The Strata keyboard shortcut block is malformed; restore it manually.".into());
    }
    let block = &original[start_index + start.len()..end_index];
    let lines: Vec<_> = block
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let recognized = if major == 4 {
        lines.len() == 5
            && lines[0] == "hl.unbind(\"SUPER + SHIFT + F\")"
            && lines[1] == "hl.unbind(\"SUPER + ALT + SHIFT + F\")"
            && lines[2].starts_with("o.bind(\"SUPER + SHIFT + F\", \"File manager\", { launch = \"")
            && lines[2].ends_with("strata\" })")
            && lines[3] == "o.bind(\"SUPER + ALT + SHIFT + F\", \"File manager (cwd)\","
            && lines[4].starts_with("\"uwsm-app -- ")
            && lines[4].ends_with("strata \\\"$(omarchy-cmd-terminal-cwd)\\\"\")")
    } else {
        lines.len() == 4
            && lines[0] == "unbind = SUPER SHIFT, F"
            && lines[1] == "unbind = SUPER ALT SHIFT, F"
            && lines[2].starts_with("bindd = SUPER SHIFT, F, File manager, exec, uwsm-app -- ")
            && lines[2].ends_with("strata")
            && lines[3]
                .starts_with("bindd = SUPER ALT SHIFT, F, File manager (cwd), exec, uwsm-app -- ")
            && lines[3].ends_with("strata \"$(omarchy-cmd-terminal-cwd)\"")
    };
    if !recognized {
        return Err("The Strata keyboard shortcuts have been customized; restore them manually to avoid losing your edits.".into());
    }
    let replacement = if major == 4 {
        "hl.unbind(\"SUPER + SHIFT + F\")\nhl.unbind(\"SUPER + ALT + SHIFT + F\")\no.bind(\"SUPER + SHIFT + F\", \"File manager\", { launch = \"nautilus --new-window\" })\no.bind(\"SUPER + ALT + SHIFT + F\", \"File manager (cwd)\",\n  \"uwsm-app -- nautilus --new-window \\\"$(omarchy-cmd-terminal-cwd)\\\"\")"
    } else {
        "unbind = SUPER SHIFT, F\nunbind = SUPER ALT SHIFT, F\nbindd = SUPER SHIFT, F, File manager, exec, uwsm-app -- nautilus --new-window\nbindd = SUPER ALT SHIFT, F, File manager (cwd), exec, uwsm-app -- nautilus --new-window \"$(omarchy-cmd-terminal-cwd)\""
    };
    Ok(Some(format!(
        "{}{replacement}{}",
        &original[..start_index],
        &original[end_index + end.len()..]
    )))
}

fn replace_bindings(
    path: &Path,
    original: &str,
    updated: &str,
    validate: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    let mut backup = tempfile::Builder::new()
        .prefix("strata-keybind-backup-")
        .tempfile_in(path.parent().ok_or("Invalid keyboard configuration path")?)
        .map_err(|error| path_error("back up", path, error))?;
    backup
        .write_all(original.as_bytes())
        .map_err(|error| path_error("back up", path, error))?;
    let (_, backup_path) = backup
        .keep()
        .map_err(|error| format!("Could not keep keyboard configuration backup: {error}"))?;
    crate::storage::atomic_write(path, updated.as_bytes())
        .map_err(|error| path_error("write", path, error))?;
    if let Err(error) = validate() {
        crate::storage::atomic_write(path, original.as_bytes()).map_err(|rollback| {
            format!(
                "{error}; rollback failed: {rollback}. Backup: {}",
                backup_path.display()
            )
        })?;
        let rollback = validate()
            .err()
            .map(|error| format!("; rollback reload failed: {error}"))
            .unwrap_or_default();
        return Err(format!(
            "{error}{rollback}. Restored keyboard configuration; backup: {}",
            backup_path.display()
        ));
    }
    Ok(())
}

fn reload() -> Result<(), String> {
    if env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        return Ok(());
    }
    for argument in ["reload", "configerrors"] {
        let output = Command::new("hyprctl")
            .arg(argument)
            .output()
            .map_err(|error| format!("Could not {argument} Hyprland: {error}"))?;
        if !output.status.success()
            || (argument == "configerrors" && !output.stdout.trim_ascii().is_empty())
        {
            return Err(format!(
                "Hyprland {argument} failed: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    Ok(())
}

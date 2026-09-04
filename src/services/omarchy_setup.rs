// SPDX-License-Identifier: GPL-3.0-or-later

//! Makes Strata Omarchy's default file manager.
//!
//! Two separate pieces of system state have to agree before folders always
//! open in Strata: the XDG `inode/directory` association (used when an
//! application asks the desktop to open a folder) and Omarchy's own
//! file-manager keyboard shortcuts, which launch Nautilus directly and never
//! consult the association.
//!
//! Omarchy ships two generations of user Hyprland configuration and only the
//! user's own file is ever touched; `/usr/share/omarchy/` is package-owned and
//! is replaced on update.

use std::{
    fs,
    io::{self},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use gtk::{gio, prelude::*};

pub const DESKTOP_ENTRY: &str = "io.github.lgse.Strata.desktop";
const DIRECTORY_MIME: &str = "inode/directory";
const BLOCK_TAG: &str = "strata:file-manager-bindings";
const MANAGED_NOTE: &str =
    "Managed by Strata. Delete this block to restore Omarchy's file-manager shortcuts.";

/// Which generation of Omarchy user configuration is installed. The two
/// generations use different files and different binding syntax.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OmarchyGeneration {
    /// Omarchy 3.x: `~/.config/hypr/bindings.conf`, Hyprland's own syntax.
    Legacy,
    /// Omarchy Quattro: `~/.config/hypr/bindings.lua`, Lua helpers.
    Quattro,
}

impl OmarchyGeneration {
    fn bindings_file_name(self) -> &'static str {
        match self {
            OmarchyGeneration::Legacy => "bindings.conf",
            OmarchyGeneration::Quattro => "bindings.lua",
        }
    }

    fn comment_prefix(self) -> &'static str {
        match self {
            OmarchyGeneration::Legacy => "#",
            OmarchyGeneration::Quattro => "--",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            OmarchyGeneration::Legacy => "Omarchy 3.x",
            OmarchyGeneration::Quattro => "Omarchy Quattro",
        }
    }
}

/// A failure that left the system exactly as it was found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupError {
    pub summary: String,
    pub detail: String,
}

impl SetupError {
    fn new(summary: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            detail: detail.into(),
        }
    }
}

/// A detected, supported Omarchy installation and the current state of the
/// two integrations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OmarchyIntegration {
    pub generation: OmarchyGeneration,
    pub bindings_path: PathBuf,
    pub folder_association: bool,
    pub shortcuts: bool,
}

impl OmarchyIntegration {
    pub fn is_complete(&self) -> bool {
        self.folder_association && self.shortcuts
    }

    /// A one-line description of what still needs doing.
    pub fn status_summary(&self) -> String {
        match (self.folder_association, self.shortcuts) {
            (true, true) => format!("Strata is the {} file manager.", self.generation.label()),
            (true, false) => {
                "Folders open in Strata, but Omarchy's shortcuts still launch Nautilus.".to_owned()
            }
            (false, true) => {
                "Omarchy's shortcuts launch Strata, but folders still open in another application."
                    .to_owned()
            }
            (false, false) => format!(
                "Folder associations and {}'s file-manager shortcuts still point elsewhere.",
                self.generation.label()
            ),
        }
    }

    /// Applies both integrations, restoring the previous bindings if Hyprland
    /// rejects the result.
    pub fn apply(&self) -> Result<(), SetupError> {
        if desktop_entry().is_none() {
            return Err(SetupError::new(
                "Strata's desktop entry is missing",
                format!(
                    "{DESKTOP_ENTRY} is not installed, so folders cannot be associated with Strata. Install Strata's desktop entry, then try again."
                ),
            ));
        }

        let original = fs::read_to_string(&self.bindings_path).map_err(|error| {
            SetupError::new(
                "Unable to read your Hyprland bindings",
                format!("{}: {error}", self.bindings_path.display()),
            )
        })?;
        let updated = with_managed_block(&original, self.generation);
        let backup = backup_path(&self.bindings_path);
        fs::copy(&self.bindings_path, &backup).map_err(|error| {
            SetupError::new(
                "Unable to back up your Hyprland bindings",
                format!("{}: {error}", backup.display()),
            )
        })?;

        if let Err(error) = crate::storage::atomic_write(&self.bindings_path, updated.as_bytes()) {
            return Err(SetupError::new(
                "Unable to update your Hyprland bindings",
                format!("{}: {error}", self.bindings_path.display()),
            ));
        }

        if let Err(error) = self.reload_hyprland() {
            self.restore(&original, &backup);
            return Err(error);
        }

        if let Err(error) = set_folder_association() {
            self.restore(&original, &backup);
            return Err(error);
        }

        if let Err(error) = self.verify() {
            self.restore(&original, &backup);
            return Err(error);
        }

        let _ = fs::remove_file(&backup);
        Ok(())
    }

    fn reload_hyprland(&self) -> Result<(), SetupError> {
        // A missing hyprctl means Strata is not running under Hyprland; the
        // bindings are still correct for the next login.
        match run_hyprctl(&["reload"]) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(SetupError::new(
                    "Unable to reload Hyprland",
                    format!("hyprctl reload failed: {error}"),
                ));
            }
        }
        let errors = run_hyprctl(&["configerrors"]).unwrap_or_default();
        if config_errors_reported(&errors) {
            return Err(SetupError::new(
                "Hyprland rejected the new bindings",
                format!("Your previous bindings were restored.\n\n{}", errors.trim()),
            ));
        }
        Ok(())
    }

    fn verify(&self) -> Result<(), SetupError> {
        let written = fs::read_to_string(&self.bindings_path).unwrap_or_default();
        if !has_managed_block(&written, self.generation) {
            return Err(SetupError::new(
                "The new bindings could not be verified",
                format!(
                    "{} no longer contains Strata's managed block.",
                    self.bindings_path.display()
                ),
            ));
        }
        if !folder_association_is_strata() {
            return Err(SetupError::new(
                "The folder association could not be verified",
                format!("{DIRECTORY_MIME} still opens in another application."),
            ));
        }
        Ok(())
    }

    fn restore(&self, original: &str, backup: &Path) {
        if crate::storage::atomic_write(&self.bindings_path, original.as_bytes()).is_err() {
            tracing::warn!(
                backup = %backup.display(),
                "unable to restore Hyprland bindings; the backup was kept"
            );
            return;
        }
        let _ = fs::remove_file(backup);
        let _ = run_hyprctl(&["reload"]);
    }
}

/// Reports the integration state, or `None` when this is not a supported
/// Omarchy installation.
pub fn integration() -> Option<OmarchyIntegration> {
    integration_at(&omarchy_share_dir(), &hypr_config_dir())
}

fn integration_at(share: &Path, hypr_config: &Path) -> Option<OmarchyIntegration> {
    let generation = detect_generation(share, hypr_config)?;
    let bindings_path = hypr_config.join(generation.bindings_file_name());
    let contents = fs::read_to_string(&bindings_path).ok()?;
    Some(OmarchyIntegration {
        generation,
        folder_association: folder_association_is_strata(),
        shortcuts: has_managed_block(&contents, generation),
        bindings_path,
    })
}

/// Quattro moved user bindings to Lua, but upgraded systems keep the old
/// `bindings.conf` on disk, so the installed Omarchy version decides which
/// file is live and the user file only confirms it.
fn detect_generation(share: &Path, hypr_config: &Path) -> Option<OmarchyGeneration> {
    let candidate = match major_version(share) {
        Some(major) if major >= 4 => OmarchyGeneration::Quattro,
        Some(_) => OmarchyGeneration::Legacy,
        None if hypr_config.join("hyprland.lua").is_file() => OmarchyGeneration::Quattro,
        None if hypr_config.join("hyprland.conf").is_file() => OmarchyGeneration::Legacy,
        None => return None,
    };
    hypr_config
        .join(candidate.bindings_file_name())
        .is_file()
        .then_some(candidate)
}

fn major_version(share: &Path) -> Option<u32> {
    let version = fs::read_to_string(share.join("version")).ok()?;
    version.trim().split('.').next()?.parse().ok()
}

fn omarchy_share_dir() -> PathBuf {
    std::env::var_os("OMARCHY_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/share/omarchy"))
}

fn hypr_config_dir() -> PathBuf {
    gtk::glib::home_dir().join(".config/hypr")
}

fn desktop_entry() -> Option<gio::AppInfo> {
    gio::AppInfo::all()
        .into_iter()
        .find(|info| info.id().is_some_and(|id| id == DESKTOP_ENTRY))
}

fn folder_association_is_strata() -> bool {
    gio::AppInfo::default_for_type(DIRECTORY_MIME, false)
        .and_then(|info| info.id())
        .is_some_and(|id| id == DESKTOP_ENTRY)
}

fn set_folder_association() -> Result<(), SetupError> {
    let entry = desktop_entry().ok_or_else(|| {
        SetupError::new(
            "Strata's desktop entry is missing",
            format!("{DESKTOP_ENTRY} is not installed."),
        )
    })?;
    entry
        .set_as_default_for_type(DIRECTORY_MIME)
        .map_err(|error| {
            SetupError::new(
                "Unable to make Strata the default for folders",
                error.to_string(),
            )
        })
}

fn run_hyprctl(args: &[&str]) -> io::Result<String> {
    let output = Command::new("hyprctl")
        .args(args)
        .stdin(Stdio::null())
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `hyprctl configerrors` prints nothing, or a "no errors" line, when the
/// configuration is clean.
fn config_errors_reported(output: &str) -> bool {
    let trimmed = output.trim();
    !trimmed.is_empty() && !trimmed.to_ascii_lowercase().contains("no errors")
}

fn backup_path(bindings: &Path) -> PathBuf {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let mut name = bindings.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".strata.bak.{seconds}"));
    bindings.with_file_name(name)
}

fn begin_marker(generation: OmarchyGeneration) -> String {
    format!("{} {BLOCK_TAG}:begin", generation.comment_prefix())
}

fn end_marker(generation: OmarchyGeneration) -> String {
    format!("{} {BLOCK_TAG}:end", generation.comment_prefix())
}

fn managed_block(generation: OmarchyGeneration) -> String {
    let comment = generation.comment_prefix();
    let bindings = match generation {
        OmarchyGeneration::Legacy => concat!(
            "unbind = SUPER SHIFT, F\n",
            "unbind = SUPER ALT SHIFT, F\n",
            "bindd = SUPER SHIFT, F, File manager, exec, uwsm-app -- strata\n",
            "bindd = SUPER ALT SHIFT, F, File manager (cwd), exec, uwsm-app -- strata \"$(omarchy-cmd-terminal-cwd)\"\n",
        ),
        OmarchyGeneration::Quattro => concat!(
            "hl.unbind(\"SUPER + SHIFT + F\")\n",
            "hl.unbind(\"SUPER + ALT + SHIFT + F\")\n",
            "o.bind(\"SUPER + SHIFT + F\", \"File manager\", { launch = \"strata\" })\n",
            "o.bind(\"SUPER + ALT + SHIFT + F\", \"File manager (cwd)\",\n",
            "  \"uwsm-app -- strata \\\"$(omarchy-cmd-terminal-cwd)\\\"\")\n",
        ),
    };
    format!(
        "{}\n{comment} {MANAGED_NOTE}\n{bindings}{}\n",
        begin_marker(generation),
        end_marker(generation)
    )
}

/// Replaces an existing managed block, or appends one, leaving every other
/// line untouched so the edit is idempotent and repeatable.
fn with_managed_block(contents: &str, generation: OmarchyGeneration) -> String {
    let stripped = without_managed_block(contents, generation);
    let body = stripped.trim_end();
    if body.is_empty() {
        return managed_block(generation);
    }
    format!("{body}\n\n{}", managed_block(generation))
}

fn without_managed_block(contents: &str, generation: OmarchyGeneration) -> String {
    let begin = begin_marker(generation);
    let end = end_marker(generation);
    let mut kept = String::with_capacity(contents.len());
    let mut inside = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if !inside && trimmed == begin {
            inside = true;
            continue;
        }
        if inside {
            inside = trimmed != end;
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    kept
}

fn has_managed_block(contents: &str, generation: OmarchyGeneration) -> bool {
    let begin = begin_marker(generation);
    let end = end_marker(generation);
    let mut lines = contents.lines().map(str::trim);
    lines.any(|line| line == begin) && lines.any(|line| line == end)
}

#[cfg(test)]
mod tests;

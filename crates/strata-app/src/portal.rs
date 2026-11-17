// SPDX-License-Identifier: MIT

//! XDG Desktop Portal `FileChooser` backend (`src/portal.rs`): request validation, safe path and
//! URI helpers, destination overwrite checks, D-Bus service registration ([`dbus`]), Hyprland
//! placement hints ([`window_geometry`]), and standalone `--portal` windowed execution.

pub(crate) mod dbus;
pub(crate) mod inbox;
pub(crate) mod window_geometry;

#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    os::unix::ffi::OsStrExt as _,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use gio::prelude::*;
use strata_core::{
    i18n::tr,
    model::{FileEntry, Location},
};
use strata_fs::{adapters::gio_close, services::validate_basename};

pub(crate) const BACKEND_NAME: &str = "org.freedesktop.impl.portal.desktop.strata";
pub(crate) const CHOOSER_APPLICATION_ID: &str = "io.github.lgse.Strata.FileChooser";
pub(crate) const FILE_CHOOSER_VERSION: u32 = 4;

const MAX_ACTIVE_REQUESTS: usize = 16;
const MAX_CHOICES: usize = 16;
const MAX_CHOICE_OPTIONS: usize = 32;
const MAX_TOTAL_CHOICE_OPTIONS: usize = 128;
const FILTER_COUNT_WARNING_THRESHOLD: usize = 128;
const FILTER_RULE_WARNING_THRESHOLD: usize = 1024;
const MAX_GLOB_BYTES: usize = 256;
const MAX_GLOB_STAR_RUNS: usize = 2;
const MAX_SAVE_FILES: usize = 256;
pub(crate) const MAX_STRING_BYTES: usize = 4_096;
const MAX_FILENAME_BYTES: usize = 255;
const PATH_IO_TIMEOUT: Duration = Duration::from_secs(3);
pub(crate) const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PortalError {
    InvalidArgument(String),
    Failed(String),
    Cancelled(String),
}

impl PortalError {
    pub(crate) fn dbus_name(&self) -> &'static str {
        match self {
            Self::InvalidArgument(_) => "org.freedesktop.portal.Error.InvalidArgument",
            Self::Failed(_) => "org.freedesktop.portal.Error.Failed",
            Self::Cancelled(_) => "org.freedesktop.portal.Error.Cancelled",
        }
    }

    pub(crate) fn message(&self) -> &str {
        match self {
            Self::InvalidArgument(msg) | Self::Failed(msg) | Self::Cancelled(msg) => msg,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FilterRule {
    Pattern(String),
    MimeType(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PortalFileFilter {
    pub(crate) label: String,
    pub(crate) rules: Vec<FilterRule>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PortalChoice {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) pairs: Vec<(String, String)>,
    pub(crate) initial_selection: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChooserKind {
    Open { directory: bool, multiple: bool },
    SaveFile { current_name: Option<OsString> },
    SaveFiles { names: Vec<OsString> },
}

#[derive(Clone, Debug)]
pub(crate) struct ChooserRequest {
    pub(crate) token: String,
    pub(crate) title: String,
    pub(crate) accept_label: String,
    pub(crate) modal: bool,
    pub(crate) parent: Option<String>,
    pub(crate) parent_size_hint: Option<(i32, i32)>,
    pub(crate) initial_directory: PathBuf,
    pub(crate) kind: ChooserKind,
    pub(crate) filters: Vec<PortalFileFilter>,
    pub(crate) current_filter: Option<PortalFileFilter>,
    pub(crate) choices: Vec<PortalChoice>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SelectedResult {
    pub(crate) uris: Vec<String>,
    pub(crate) choices: Vec<(String, String)>,
    pub(crate) current_filter: Option<PortalFileFilter>,
    pub(crate) writable: Option<bool>,
}

pub(crate) type Completion = Box<dyn FnOnce(Result<SelectedResult, PortalError>)>;

pub(crate) struct PortalCallbacks {
    pub(crate) present: Rc<dyn Fn(ChooserRequest, Arc<AtomicBool>, Completion)>,
    pub(crate) cancel: Rc<dyn Fn(&str)>,
    pub(crate) quit: Rc<dyn Fn()>,
}

struct RequestState {
    active: HashMap<String, Arc<AtomicBool>>,
    last_activity: Instant,
    shutting_down: bool,
}

impl Default for RequestState {
    fn default() -> Self {
        Self {
            active: HashMap::new(),
            last_activity: Instant::now(),
            shutting_down: false,
        }
    }
}

#[derive(Default)]
pub(crate) struct RequestTracker {
    state: Mutex<RequestState>,
}

impl RequestTracker {
    pub(crate) fn begin_shutdown_if_idle(&self, now: Instant) -> bool {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.active.is_empty()
            && now.saturating_duration_since(state.last_activity) >= IDLE_TIMEOUT
        {
            state.shutting_down = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn begin(self: &Arc<Self>, token: String) -> Result<TrackedRequest, PortalError> {
        if token.len() > MAX_STRING_BYTES {
            return invalid_argument("file chooser request token is too long");
        }
        let mut active = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if active.shutting_down {
            return Err(PortalError::Failed(
                "file chooser backend is shutting down".into(),
            ));
        }
        if active.active.contains_key(&token) {
            return invalid_argument("file chooser request token is already active");
        }
        if active.active.len() >= MAX_ACTIVE_REQUESTS {
            return Err(PortalError::Failed(
                "too many active file chooser requests".into(),
            ));
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        active.active.insert(token.clone(), cancelled.clone());
        active.last_activity = Instant::now();
        Ok(TrackedRequest {
            tracker: self.clone(),
            token,
            cancelled,
        })
    }

    pub(crate) fn cancel(&self, token: &str) -> bool {
        let cancelled = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .active
            .get(token)
            .cloned();
        cancelled.is_some_and(|flag| !flag.swap(true, Ordering::SeqCst))
    }

    fn finish(&self, token: &str, cancelled: &Arc<AtomicBool>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state
            .active
            .get(token)
            .is_some_and(|current| Arc::ptr_eq(current, cancelled))
        {
            state.active.remove(token);
            state.last_activity = Instant::now();
        }
    }
}

pub(crate) struct TrackedRequest {
    tracker: Arc<RequestTracker>,
    pub(crate) token: String,
    pub(crate) cancelled: Arc<AtomicBool>,
}

impl Drop for TrackedRequest {
    fn drop(&mut self) {
        self.tracker.finish(&self.token, &self.cancelled);
    }
}

pub(crate) fn default_save_folder() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Downloads)
        .unwrap_or_else(crate::launch::home_directory)
}

pub(crate) fn request_title(title: &str, fallback: &str) -> String {
    if title.trim().is_empty() {
        tr(fallback)
    } else {
        title.to_owned()
    }
}

pub(crate) fn default_accept_label(label: Option<&str>, fallback: &str) -> String {
    label.map_or_else(|| tr(fallback), ToOwned::to_owned)
}

pub(crate) fn validate_common_request(
    title: &str,
    accept_label: Option<&str>,
    parent: Option<&str>,
    choices: &[PortalChoice],
) -> Result<(), PortalError> {
    validate_string(title, MAX_STRING_BYTES, "title")?;
    if let Some(label) = accept_label {
        validate_string(label, MAX_STRING_BYTES, "accept label")?;
    }
    if let Some(parent) = parent {
        validate_string(parent, MAX_STRING_BYTES, "parent window handle")?;
    }
    validate_choices(choices)
}

pub(crate) fn validate_filters(
    filters: &[PortalFileFilter],
    current: Option<&PortalFileFilter>,
) -> Result<(), PortalError> {
    let mut total_rules = 0usize;
    for filter in filters
        .iter()
        .chain(current.filter(|filter| !filters.contains(filter)))
    {
        validate_string(&filter.label, MAX_STRING_BYTES, "file filter label")?;
        total_rules = total_rules.saturating_add(filter.rules.len());
        for rule in &filter.rules {
            match rule {
                FilterRule::Pattern(pattern) => validate_glob(pattern)?,
                FilterRule::MimeType(mime) => {
                    validate_string(mime, MAX_STRING_BYTES, "MIME filter rule")?;
                }
            }
        }
    }
    if filters.len() > FILTER_COUNT_WARNING_THRESHOLD {
        tracing::warn!(
            filters = filters.len(),
            "file filter list exceeded the count budget and was accepted untrimmed"
        );
    }
    if total_rules > FILTER_RULE_WARNING_THRESHOLD {
        tracing::warn!(
            rules = total_rules,
            "file filters exceeded the rule budget and was accepted untrimmed"
        );
    }
    Ok(())
}

fn validate_glob(pattern: &str) -> Result<(), PortalError> {
    validate_string(pattern, MAX_GLOB_BYTES, "glob filter rule")?;
    let mut star_runs = 0usize;
    let mut previous_star = false;
    for byte in pattern.bytes() {
        if byte == b'*' && !previous_star {
            star_runs += 1;
            if star_runs > MAX_GLOB_STAR_RUNS {
                return invalid_argument("glob filter rule has too many wildcard groups");
            }
        }
        previous_star = byte == b'*';
    }
    Ok(())
}

fn validate_choices(choices: &[PortalChoice]) -> Result<(), PortalError> {
    if choices.len() > MAX_CHOICES {
        return invalid_argument("too many file chooser choices");
    }
    let mut total_options = 0usize;
    for choice in choices {
        validate_string(&choice.id, MAX_STRING_BYTES, "choice identifier")?;
        validate_string(&choice.label, MAX_STRING_BYTES, "choice label")?;
        validate_string(
            &choice.initial_selection,
            MAX_STRING_BYTES,
            "initial choice value",
        )?;
        if choice.pairs.len() > MAX_CHOICE_OPTIONS {
            return invalid_argument("a file chooser choice has too many options");
        }
        total_options = total_options.saturating_add(choice.pairs.len());
        if total_options > MAX_TOTAL_CHOICE_OPTIONS {
            return invalid_argument("file chooser choices have too many options");
        }
        for (id, label) in &choice.pairs {
            validate_string(id, MAX_STRING_BYTES, "choice option identifier")?;
            validate_string(label, MAX_STRING_BYTES, "choice option label")?;
        }
    }
    Ok(())
}

pub(crate) fn validate_string(value: &str, limit: usize, field: &str) -> Result<(), PortalError> {
    if value.len() > limit || value.contains('\0') {
        return invalid_argument(format!("{field} is too long or contains a NUL byte"));
    }
    Ok(())
}

pub(crate) fn validate_filename(value: &str) -> Result<(), PortalError> {
    validate_string(value, MAX_FILENAME_BYTES, "current filename")
}

pub(crate) fn validate_path(path: Option<&Path>, field: &str) -> Result<(), PortalError> {
    let Some(path) = path else {
        return Ok(());
    };
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() > MAX_STRING_BYTES || bytes.contains(&0) {
        return invalid_argument(format!("{field} is too long or contains a NUL byte"));
    }
    Ok(())
}

pub(crate) fn invalid_argument<T>(message: impl Into<String>) -> Result<T, PortalError> {
    Err(PortalError::InvalidArgument(message.into()))
}

pub(crate) async fn accessible_folder(suggestion: Option<PathBuf>) -> PathBuf {
    let default = default_save_folder();
    let Some(path) = suggestion.filter(|path| path.is_absolute()) else {
        return default;
    };
    match glib::future_with_timeout(PATH_IO_TIMEOUT, directory_is_accessible(&path)).await {
        Ok(true) => path,
        Ok(false) | Err(_) => default,
    }
}

pub(crate) async fn directory_is_accessible(path: &Path) -> bool {
    gio_close::enumerate_children(
        &gio::File::for_path(path),
        gio::FILE_ATTRIBUTE_STANDARD_TYPE,
        gio::FileQueryInfoFlags::NONE,
        None,
    )
    .await
    .is_ok()
}

pub(crate) async fn save_file_suggestion(
    current_file: Option<PathBuf>,
    current_folder: Option<PathBuf>,
    current_name: Option<String>,
) -> (PathBuf, Option<OsString>) {
    let fallback = (default_save_folder(), None);
    glib::future_with_timeout(
        PATH_IO_TIMEOUT,
        resolve_save_file_suggestion(current_file, current_folder, current_name),
    )
    .await
    .unwrap_or(fallback)
}

async fn resolve_save_file_suggestion(
    current_file: Option<PathBuf>,
    current_folder: Option<PathBuf>,
    current_name: Option<String>,
) -> (PathBuf, Option<OsString>) {
    if let Some(file) = current_file {
        let valid_save_target = if file.is_absolute() {
            match gio::File::for_path(&file)
                .query_info_future(
                    gio::FILE_ATTRIBUTE_STANDARD_TYPE,
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                )
                .await
            {
                Ok(info) => info.file_type() == gio::FileType::Regular,
                Err(error) => error.matches(gio::IOErrorEnum::NotFound),
            }
        } else {
            false
        };
        if valid_save_target
            && let (Some(parent), Some(name)) = (file.parent(), file.file_name())
            && safe_filename(name)
            && directory_is_accessible(parent).await
        {
            return (parent.to_path_buf(), Some(name.to_owned()));
        }
        return (default_save_folder(), None);
    }

    let name = current_name
        .as_deref()
        .filter(|name| validate_basename(name).is_ok())
        .map(OsString::from);
    let folder = if let Some(folder) = current_folder.filter(|folder| folder.is_absolute())
        && directory_is_accessible(&folder).await
    {
        folder
    } else {
        default_save_folder()
    };
    (folder, name)
}

pub(crate) fn safe_filename(name: &OsStr) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && !bytes.contains(&b'/')
        && !bytes.contains(&0)
        && !matches!(bytes, b"." | b"..")
}

pub(crate) fn writable_from_read_only(read_only: bool) -> bool {
    !read_only
}

pub(crate) fn validate_save_filenames(names: &[OsString]) -> Result<(), PortalError> {
    if names.is_empty() || names.len() > MAX_SAVE_FILES {
        return invalid_argument("SaveFiles requires a bounded, non-empty filename list");
    }
    if names
        .iter()
        .any(|name| !safe_filename(name) || name.as_bytes().len() > MAX_FILENAME_BYTES)
    {
        return invalid_argument("SaveFiles filenames must be bounded safe basenames");
    }
    Ok(())
}

pub(crate) fn local_uri(path: &Path) -> Result<String, PortalError> {
    if !path.is_absolute() {
        return invalid_argument("file chooser results must be absolute local paths");
    }
    let uri = gio::File::for_path(path).uri().to_string();
    if !uri.starts_with("file://") {
        return Err(PortalError::Failed(
            "GIO did not encode a local file URI".into(),
        ));
    }
    Ok(uri)
}

pub(crate) fn open_selection(
    entries: &[FileEntry],
    current: &Location,
    directory: bool,
    multiple: bool,
) -> Result<Vec<PathBuf>, &'static str> {
    if entries.is_empty() && directory {
        return current
            .native_path()
            .map(|path| vec![path.to_path_buf()])
            .ok_or("Choose a local folder");
    }
    if entries.is_empty() {
        return Err("Choose a file");
    }
    if !multiple && entries.len() != 1 {
        return Err("Choose one item");
    }
    if entries
        .iter()
        .any(|entry| entry.is_directory() != directory)
    {
        return Err(if directory {
            "Choose folders only"
        } else {
            "Choose files only"
        });
    }
    entries
        .iter()
        .map(|entry| {
            entry
                .location
                .native_path()
                .map(Path::to_path_buf)
                .ok_or("Choose local items only")
        })
        .collect()
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct DestinationCheck {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) existing: Vec<PathBuf>,
}

pub(crate) async fn check_destinations(
    folder: &Path,
    names: &[OsString],
) -> Result<DestinationCheck, String> {
    if !folder.is_absolute() || folder.as_os_str().as_bytes().len() > MAX_STRING_BYTES {
        return Err(tr("Choose an accessible local folder"));
    }
    validate_save_filenames(names).map_err(|_| tr("Enter bounded, safe filenames"))?;
    glib::future_with_timeout(PATH_IO_TIMEOUT, inspect_destinations(folder, names))
        .await
        .map_err(|_| tr("Timed out while inspecting the destination"))?
}

async fn inspect_destinations(
    folder: &Path,
    names: &[OsString],
) -> Result<DestinationCheck, String> {
    if !directory_is_accessible(folder).await {
        return Err(tr("Choose an accessible local folder"));
    }
    let mut paths = Vec::with_capacity(names.len());
    let mut existing = Vec::new();
    for name in names {
        let path = folder.join(name);
        if classify_destination(&path, name).await? {
            existing.push(path.clone());
        }
        paths.push(path);
    }
    Ok(DestinationCheck { paths, existing })
}

async fn classify_destination(path: &Path, name: &OsStr) -> Result<bool, String> {
    let file = gio::File::for_path(path);
    match file
        .query_info_future(
            gio::FILE_ATTRIBUTE_STANDARD_TYPE,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await
    {
        Ok(info) if info.file_type() == gio::FileType::Directory => {
            Err(folder_exists_message(name))
        }
        Ok(info) if info.file_type() == gio::FileType::SymbolicLink => {
            if file
                .query_info_future(
                    gio::FILE_ATTRIBUTE_STANDARD_TYPE,
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                )
                .await
                .is_ok_and(|target| target.file_type() == gio::FileType::Directory)
            {
                return Err(folder_exists_message(name));
            }
            Ok(true)
        }
        Ok(_) => Ok(true),
        Err(error) if error.matches(gio::IOErrorEnum::NotFound) => Ok(false),
        Err(error) => Err(rust_i18n::t!(
            "Unable to inspect the destination: %{error}",
            error = error
        )
        .into_owned()),
    }
}

fn folder_exists_message(name: &OsStr) -> String {
    rust_i18n::t!(
        "A folder named “%{name}” already exists",
        name = name.to_string_lossy()
    )
    .into_owned()
}

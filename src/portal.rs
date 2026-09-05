// SPDX-License-Identifier: GPL-3.0-or-later

mod dbus;

#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    future::Future,
    os::unix::ffi::OsStrExt as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use ashpd::{
    FilePath, PortalError, Uri, WindowIdentifierType,
    desktop::file_chooser::{
        Choice, FileFilter, OpenFileOptions, SaveFileOptions, SaveFilesOptions, SelectedFiles,
    },
};
use futures_channel::oneshot;
use gio::prelude::*;

use crate::model::{FileEntry, Location};

const BACKEND_NAME: &str = "org.freedesktop.impl.portal.desktop.strata";
pub(crate) const FILE_CHOOSER_VERSION: u32 = 4;
const MAX_ACTIVE_REQUESTS: usize = 16;
const MAX_CHOICES: usize = 16;
const MAX_CHOICE_OPTIONS: usize = 32;
const MAX_TOTAL_CHOICE_OPTIONS: usize = 128;
const MAX_FILTERS: usize = 32;
const MAX_FILTER_RULES: usize = 64;
const MAX_TOTAL_FILTER_RULES: usize = 256;
const MAX_GLOB_BYTES: usize = 256;
const MAX_GLOB_STAR_RUNS: usize = 2;
const MAX_SAVE_FILES: usize = 256;
const MAX_STRING_BYTES: usize = 4_096;
const MAX_FILENAME_BYTES: usize = 255;
const PATH_IO_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub(crate) enum ChooserKind {
    Open { directory: bool, multiple: bool },
    SaveFile { current_name: Option<OsString> },
    SaveFiles { names: Vec<OsString> },
}

#[derive(Debug)]
pub(crate) struct ChooserRequest {
    pub token: String,
    pub title: String,
    pub accept_label: String,
    pub modal: bool,
    pub parent: Option<WindowIdentifierType>,
    pub initial_directory: PathBuf,
    pub kind: ChooserKind,
    pub filters: Vec<FileFilter>,
    pub current_filter: Option<FileFilter>,
    pub choices: Vec<Choice>,
}

#[derive(Default)]
struct RequestTracker {
    active: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl RequestTracker {
    fn begin(self: &Arc<Self>, token: String) -> ashpd::backend::Result<TrackedRequest> {
        if token.len() > MAX_STRING_BYTES {
            return Err(PortalError::InvalidArgument(
                "file chooser request token is too long".into(),
            ));
        }
        let mut active = self.active.lock().expect("request tracker poisoned");
        if active.contains_key(&token) {
            return Err(PortalError::InvalidArgument(
                "file chooser request token is already active".into(),
            ));
        }
        if active.len() >= MAX_ACTIVE_REQUESTS {
            return Err(PortalError::Failed(
                "too many active file chooser requests".into(),
            ));
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        active.insert(token.clone(), cancelled.clone());
        Ok(TrackedRequest {
            tracker: self.clone(),
            token,
            cancelled,
        })
    }

    fn cancel(&self, token: &str) -> bool {
        let Some(cancelled) = self
            .active
            .lock()
            .expect("request tracker poisoned")
            .get(token)
            .cloned()
        else {
            return false;
        };
        !cancelled.swap(true, Ordering::SeqCst)
    }

    fn finish(&self, token: &str, cancelled: &Arc<AtomicBool>) {
        let mut active = self.active.lock().expect("request tracker poisoned");
        if active
            .get(token)
            .is_some_and(|current| Arc::ptr_eq(current, cancelled))
        {
            active.remove(token);
        }
    }
}

struct TrackedRequest {
    tracker: Arc<RequestTracker>,
    token: String,
    cancelled: Arc<AtomicBool>,
}

impl Drop for TrackedRequest {
    fn drop(&mut self) {
        self.tracker.finish(&self.token, &self.cancelled);
    }
}

#[derive(Clone, Default)]
struct FileChooserBackend {
    requests: Arc<RequestTracker>,
}

impl FileChooserBackend {
    async fn choose(
        &self,
        tracked: &TrackedRequest,
        request: ChooserRequest,
    ) -> ashpd::backend::Result<SelectedFiles> {
        let token = request.token.clone();
        let cancelled = tracked.cancelled.clone();
        let (send, receive) = oneshot::channel();
        glib::MainContext::default().invoke(move || {
            crate::ui::present_chooser(request, cancelled, move |result| {
                let _result = send.send(result);
            });
        });
        receive.await.unwrap_or_else(|_| {
            Err(PortalError::Cancelled(format!(
                "file chooser request {token} ended without a response"
            )))
        })
    }
}

pub(crate) fn run() -> glib::ExitCode {
    // Keep worker-thread invocations queued until GTK is ready on this thread.
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("portal main context is available");
    let main_loop = glib::MainLoop::new(None, false);
    let service_loop = main_loop.clone();
    let service_failed = Arc::new(AtomicBool::new(false));
    let failed = service_failed.clone();
    std::thread::spawn(move || {
        let result: Result<(), zbus::Error> = async_io::block_on(async {
            use futures_lite::StreamExt as _;

            let connection = zbus::connection::Builder::session()?
                .serve_at(
                    "/org/freedesktop/portal/desktop",
                    dbus::FileChooserInterface::new(),
                )?
                .name(BACKEND_NAME)?
                .build()
                .await?;
            let proxy = zbus::fdo::DBusProxy::new(&connection).await?;
            let mut lost = proxy.receive_name_lost().await?;
            let _signal = lost.next().await;
            Ok(())
        });
        if let Err(error) = result {
            failed.store(true, Ordering::SeqCst);
            eprintln!("Strata portal backend failed: {error}");
        }
        glib::MainContext::default().invoke(move || service_loop.quit());
    });

    if let Err(error) = gtk::init() {
        eprintln!("Unable to initialize the Strata portal UI: {error}");
        return glib::ExitCode::FAILURE;
    }
    crate::metrics::initialize();
    if let Err(error) = tracing_subscriber::fmt::try_init() {
        eprintln!("Unable to initialize logging: {error}");
    }
    tracing::info!(
        version = FILE_CHOOSER_VERSION,
        "starting Strata FileChooser portal backend"
    );
    if let Err(error) = crate::assets::prepare() {
        eprintln!("Unable to prepare bundled assets: {error}");
    }
    crate::assets::register_icon_theme();
    crate::ui::prepare_portal_ui();

    if service_failed.load(Ordering::SeqCst) {
        return glib::ExitCode::FAILURE;
    }
    main_loop.run();
    if service_failed.load(Ordering::SeqCst) {
        glib::ExitCode::FAILURE
    } else {
        glib::ExitCode::SUCCESS
    }
}

async fn open_request(
    token: String,
    parent: Option<WindowIdentifierType>,
    title: &str,
    options: OpenFileOptions,
) -> ashpd::backend::Result<ChooserRequest> {
    validate_common_request(
        title,
        options.accept_label(),
        parent.as_ref(),
        options.choices(),
    )?;
    validate_filters(options.filters(), options.current_filter())?;
    validate_path(
        options.current_folder().map(AsRef::as_ref),
        "current folder",
    )?;
    let initial_directory = accessible_folder(
        options
            .current_folder()
            .map(|path| path.as_ref().to_path_buf()),
    )
    .await?;
    Ok(ChooserRequest {
        token,
        title: request_title(title, "Open Files"),
        accept_label: options.accept_label().unwrap_or("Open").to_owned(),
        modal: options.modal().unwrap_or(true),
        parent,
        initial_directory,
        kind: ChooserKind::Open {
            directory: options.directory().unwrap_or(false),
            multiple: options.multiple().unwrap_or(false),
        },
        filters: options.filters().to_vec(),
        current_filter: options.current_filter().cloned(),
        choices: options.choices().to_vec(),
    })
}

async fn save_file_request(
    token: String,
    parent: Option<WindowIdentifierType>,
    title: &str,
    options: SaveFileOptions,
) -> ashpd::backend::Result<ChooserRequest> {
    validate_common_request(
        title,
        options.accept_label(),
        parent.as_ref(),
        options.choices(),
    )?;
    validate_filters(options.filters(), options.current_filter())?;
    validate_path(options.current_file().map(AsRef::as_ref), "current file")?;
    validate_path(
        options.current_folder().map(AsRef::as_ref),
        "current folder",
    )?;
    if let Some(name) = options.current_name() {
        validate_string(name, MAX_FILENAME_BYTES, "current filename")?;
    }
    let (initial_directory, current_name) = save_file_suggestion(
        options
            .current_file()
            .map(|path| path.as_ref().to_path_buf()),
        options
            .current_folder()
            .map(|path| path.as_ref().to_path_buf()),
        options.current_name().map(str::to_owned),
    )
    .await?;
    Ok(ChooserRequest {
        token,
        title: request_title(title, "Save File"),
        accept_label: options.accept_label().unwrap_or("Save").to_owned(),
        modal: options.modal().unwrap_or(true),
        parent,
        initial_directory,
        kind: ChooserKind::SaveFile { current_name },
        filters: options.filters().to_vec(),
        current_filter: options.current_filter().cloned(),
        choices: options.choices().to_vec(),
    })
}

async fn save_files_request(
    token: String,
    parent: Option<WindowIdentifierType>,
    title: &str,
    options: SaveFilesOptions,
) -> ashpd::backend::Result<ChooserRequest> {
    validate_common_request(
        title,
        options.accept_label(),
        parent.as_ref(),
        options.choices(),
    )?;
    validate_path(
        options.current_folder().map(AsRef::as_ref),
        "current folder",
    )?;
    validate_save_file_paths(options.files())?;
    let names = options
        .files()
        .iter()
        .map(|path| path.as_ref().as_os_str().to_owned())
        .collect::<Vec<_>>();
    let initial_directory = accessible_folder(
        options
            .current_folder()
            .map(|path| path.as_ref().to_path_buf()),
    )
    .await?;
    Ok(ChooserRequest {
        token,
        title: request_title(title, "Save Files"),
        accept_label: options.accept_label().unwrap_or("Save").to_owned(),
        modal: options.modal().unwrap_or(true),
        parent,
        initial_directory,
        kind: ChooserKind::SaveFiles { names },
        filters: Vec::new(),
        current_filter: None,
        choices: options.choices().to_vec(),
    })
}

fn request_title(title: &str, fallback: &str) -> String {
    if title.trim().is_empty() {
        fallback.to_owned()
    } else {
        title.to_owned()
    }
}

fn validate_common_request(
    title: &str,
    accept_label: Option<&str>,
    parent: Option<&WindowIdentifierType>,
    choices: &[Choice],
) -> ashpd::backend::Result<()> {
    validate_string(title, MAX_STRING_BYTES, "title")?;
    if let Some(label) = accept_label {
        validate_string(label, MAX_STRING_BYTES, "accept label")?;
    }
    if let Some(WindowIdentifierType::Wayland(handle)) = parent {
        validate_string(handle, MAX_STRING_BYTES, "parent window handle")?;
    }
    validate_choices(choices)
}

fn validate_filters(
    filters: &[FileFilter],
    current: Option<&FileFilter>,
) -> ashpd::backend::Result<()> {
    if filters.len() > MAX_FILTERS
        || current.is_some_and(|current| !filters.contains(current) && filters.len() == MAX_FILTERS)
    {
        return invalid_argument("too many file filters");
    }
    let mut total_rules = 0usize;
    for filter in filters
        .iter()
        .chain(current.filter(|filter| !filters.contains(filter)))
    {
        validate_string(filter.label(), MAX_STRING_BYTES, "file filter label")?;
        let patterns = filter.pattern_filters();
        let mimetypes = filter.mimetype_filters();
        let rules = patterns.len().saturating_add(mimetypes.len());
        if rules > MAX_FILTER_RULES {
            return invalid_argument("a file filter has too many rules");
        }
        total_rules = total_rules.saturating_add(rules);
        if total_rules > MAX_TOTAL_FILTER_RULES {
            return invalid_argument("file filters have too many rules");
        }
        for pattern in patterns {
            validate_glob(pattern)?;
        }
        for mimetype in mimetypes {
            validate_string(mimetype, MAX_STRING_BYTES, "MIME filter rule")?;
        }
    }
    Ok(())
}

fn validate_glob(pattern: &str) -> ashpd::backend::Result<()> {
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

fn validate_choices(choices: &[Choice]) -> ashpd::backend::Result<()> {
    if choices.len() > MAX_CHOICES {
        return invalid_argument("too many file chooser choices");
    }
    let mut total_options = 0usize;
    for choice in choices {
        validate_string(choice.id(), MAX_STRING_BYTES, "choice identifier")?;
        validate_string(choice.label(), MAX_STRING_BYTES, "choice label")?;
        validate_string(
            choice.initial_selection(),
            MAX_STRING_BYTES,
            "initial choice value",
        )?;
        let options = choice.pairs();
        if options.len() > MAX_CHOICE_OPTIONS {
            return invalid_argument("a file chooser choice has too many options");
        }
        total_options = total_options.saturating_add(options.len());
        if total_options > MAX_TOTAL_CHOICE_OPTIONS {
            return invalid_argument("file chooser choices have too many options");
        }
        for (id, label) in options {
            validate_string(id, MAX_STRING_BYTES, "choice option identifier")?;
            validate_string(label, MAX_STRING_BYTES, "choice option label")?;
        }
    }
    Ok(())
}

fn validate_string(value: &str, limit: usize, field: &str) -> ashpd::backend::Result<()> {
    if value.len() > limit || value.contains('\0') {
        return invalid_argument(format!("{field} is too long or contains a NUL byte"));
    }
    Ok(())
}

fn validate_path(path: Option<&Path>, field: &str) -> ashpd::backend::Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() > MAX_STRING_BYTES || bytes.contains(&0) {
        return invalid_argument(format!("{field} is too long or contains a NUL byte"));
    }
    Ok(())
}

fn invalid_argument<T>(message: impl Into<String>) -> ashpd::backend::Result<T> {
    Err(PortalError::InvalidArgument(message.into()))
}

async fn run_on_main<T, F, Fut>(task: F) -> ashpd::backend::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = T> + 'static,
{
    let (send, receive) = oneshot::channel();
    let context = glib::MainContext::ref_thread_default();
    let task_context = context.clone();
    context.invoke(move || {
        let _task = task_context.spawn_local(async move {
            let _result = send.send(task().await);
        });
    });
    receive
        .await
        .map_err(|_| PortalError::Failed("portal UI context stopped responding".into()))
}

async fn accessible_folder(suggestion: Option<PathBuf>) -> ashpd::backend::Result<PathBuf> {
    run_on_main(move || async move {
        let home = crate::ui::home_directory();
        let Some(path) = suggestion.filter(|path| path.is_absolute()) else {
            return home;
        };
        match glib::future_with_timeout(PATH_IO_TIMEOUT, directory_is_accessible(&path)).await {
            Ok(true) => path,
            Ok(false) | Err(_) => home,
        }
    })
    .await
}

async fn directory_is_accessible(path: &Path) -> bool {
    gio::File::for_path(path)
        .enumerate_children_future(
            gio::FILE_ATTRIBUTE_STANDARD_TYPE,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await
        .is_ok()
}

async fn save_file_suggestion(
    current_file: Option<PathBuf>,
    current_folder: Option<PathBuf>,
    current_name: Option<String>,
) -> ashpd::backend::Result<(PathBuf, Option<OsString>)> {
    run_on_main(move || async move {
        let home = crate::ui::home_directory();
        let fallback = (home.clone(), None);
        glib::future_with_timeout(
            PATH_IO_TIMEOUT,
            resolve_save_file_suggestion(current_file, current_folder, current_name),
        )
        .await
        .unwrap_or(fallback)
    })
    .await
}

async fn resolve_save_file_suggestion(
    current_file: Option<PathBuf>,
    current_folder: Option<PathBuf>,
    current_name: Option<String>,
) -> (PathBuf, Option<OsString>) {
    if let Some(file) = current_file {
        let file_type = if file.is_absolute() {
            gio::File::for_path(&file)
                .query_info_future(
                    gio::FILE_ATTRIBUTE_STANDARD_TYPE,
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                )
                .await
                .ok()
                .map(|info| info.file_type())
        } else {
            None
        };
        if file_type == Some(gio::FileType::Regular)
            && let (Some(parent), Some(name)) = (file.parent(), file.file_name())
            && safe_filename(name)
            && directory_is_accessible(parent).await
        {
            return (parent.to_path_buf(), Some(name.to_owned()));
        }
        return (crate::ui::home_directory(), None);
    }

    let name = current_name
        .as_deref()
        .filter(|name| crate::services::validate_basename(name).is_ok())
        .map(OsString::from);
    let folder = if let Some(folder) = current_folder.filter(|folder| folder.is_absolute())
        && directory_is_accessible(&folder).await
    {
        folder
    } else {
        crate::ui::home_directory()
    };
    (folder, name)
}

pub(crate) fn safe_filename(name: &OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt as _;

    let bytes = name.as_bytes();
    !bytes.is_empty()
        && !bytes.contains(&b'/')
        && !bytes.contains(&0)
        && !matches!(bytes, b"." | b"..")
}

pub(crate) fn writable_from_read_only(read_only: bool) -> bool {
    !read_only
}

fn validate_save_filenames(names: &[OsString]) -> ashpd::backend::Result<()> {
    if names.is_empty() || names.len() > MAX_SAVE_FILES {
        return Err(PortalError::InvalidArgument(
            "SaveFiles requires a bounded, non-empty filename list".into(),
        ));
    }
    if names
        .iter()
        .any(|name| !safe_filename(name) || name.as_bytes().len() > MAX_FILENAME_BYTES)
    {
        return Err(PortalError::InvalidArgument(
            "SaveFiles filenames must be bounded safe basenames".into(),
        ));
    }
    Ok(())
}

fn validate_save_file_paths(files: &[FilePath]) -> ashpd::backend::Result<()> {
    if files.is_empty() || files.len() > MAX_SAVE_FILES {
        return invalid_argument("SaveFiles requires a bounded, non-empty filename list");
    }
    for file in files {
        let name = file.as_ref().as_os_str();
        if !safe_filename(name) || name.as_bytes().len() > MAX_FILENAME_BYTES {
            return invalid_argument("SaveFiles filenames must be bounded safe basenames");
        }
    }
    Ok(())
}

pub(crate) fn local_uri(path: &Path) -> ashpd::backend::Result<Uri> {
    if !path.is_absolute() {
        return Err(PortalError::InvalidArgument(
            "file chooser results must be absolute local paths".into(),
        ));
    }
    let uri = gio::File::for_path(path).uri();
    if !uri.starts_with("file://") {
        return Err(PortalError::Failed(
            "GIO did not encode a local file URI".into(),
        ));
    }
    Uri::parse(&uri).map_err(|error| PortalError::Failed(error.to_string()))
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
    pub paths: Vec<PathBuf>,
    pub existing_files: bool,
}

pub(crate) async fn check_destinations(
    folder: &Path,
    names: &[OsString],
) -> Result<DestinationCheck, String> {
    if !folder.is_absolute() || folder.as_os_str().as_bytes().len() > MAX_STRING_BYTES {
        return Err("Choose an accessible local folder".into());
    }
    validate_save_filenames(names).map_err(|_| "Enter bounded, safe filenames".to_owned())?;
    glib::future_with_timeout(PATH_IO_TIMEOUT, inspect_destinations(folder, names))
        .await
        .map_err(|_| "Timed out while inspecting the destination".to_owned())?
}

async fn inspect_destinations(
    folder: &Path,
    names: &[OsString],
) -> Result<DestinationCheck, String> {
    if !directory_is_accessible(folder).await {
        return Err("Choose an accessible local folder".into());
    }
    let mut paths = Vec::with_capacity(names.len());
    let mut existing_files = false;
    for name in names {
        let path = folder.join(name);
        let file = gio::File::for_path(&path);
        match file
            .query_info_future(
                gio::FILE_ATTRIBUTE_STANDARD_TYPE,
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
            )
            .await
        {
            Ok(info) if info.file_type() == gio::FileType::Directory => {
                return Err(format!(
                    "A folder named “{}” already exists",
                    name.to_string_lossy()
                ));
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
                    return Err(format!(
                        "A folder named “{}” already exists",
                        name.to_string_lossy()
                    ));
                }
                existing_files = true;
            }
            Ok(_) => existing_files = true,
            Err(error) if error.matches(gio::IOErrorEnum::NotFound) => {}
            Err(error) => return Err(format!("Unable to inspect the destination: {error}")),
        }
        paths.push(path);
    }
    Ok(DestinationCheck {
        paths,
        existing_files,
    })
}

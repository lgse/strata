// SPDX-License-Identifier: MIT

use std::{collections::HashSet, path::Path, time::Duration};

use gio::prelude::*;
use strata_core::{
    i18n::tr,
    model::{Location, diagnostic_uri},
    storage::writer,
};
use strata_fs::{
    adapters::{CONTENT_TYPE_ATTRIBUTES, content_type_of, gio_file_for_location},
    child_environment::restore_launch_context,
    services::{WebLink, has_link_extension},
};

use super::{
    ChooserSpec, OpenWithContext,
    apps::{choices_for_type, requires_uri_handlers, uses_portal},
    recent,
};
use crate::{
    App, clipboard,
    error_dialog::{self, Error},
    run, shell,
    slots::Later,
};

pub(crate) const ADMINISTRATOR_OPEN_REFUSAL: &str = "Other applications can't open files in a folder opened as administrator. Preview the file here, or copy it to another folder first.";

const RECENTLY_USED_XBEL: &str = "recently-used.xbel";

#[derive(Clone, Copy)]
pub(super) struct LaunchBatch<'a> {
    pub(super) files: &'a [gio::File],
    pub(super) content_types: &'a [String],
}

struct OpenRequest {
    file: gio::File,
    location: Location,
    position: Option<Duration>,
    context: gio::AppLaunchContext,
}

pub(super) fn launch_context() -> gio::AppLaunchContext {
    let context = gio::AppLaunchContext::new();
    restore_launch_context(&context);
    context
}

/// Opens `location` at optional media playback `position`, matching `desktop::open_location_at`.
pub(crate) fn open_location_at(app: &mut App, location: &Location, position: Option<Duration>) {
    if clipboard::paths::is_trash(location) {
        error_dialog::show(
            app,
            Error::new(
                tr("Unable to open item"),
                tr("Items in Trash cannot be opened"),
            ),
        );
        return;
    }
    if location.is_administrator() {
        error_dialog::show(
            app,
            Error::new(tr("Unable to open item"), tr(ADMINISTRATOR_OPEN_REFUSAL)),
        );
        return;
    }
    let request = OpenRequest {
        file: gio_file_for_location(location),
        location: location.clone(),
        position,
        context: launch_context(),
    };
    let later = app.open_with.later.clone();
    glib::MainContext::default().spawn_local(async move {
        open_resolved_file(request, later).await;
    });
}

fn can_confirm_executable(file: &gio::File, location: &Location, executable: bool) -> bool {
    executable && file.is_native() && location.native_path().is_some()
}

async fn open_resolved_file(request: OpenRequest, later: Later<App>) {
    let OpenRequest {
        file,
        location,
        position,
        context,
    } = request;
    if let Some(link) = read_web_link(&file).await {
        let result =
            gio::AppInfo::launch_default_for_uri_future(link.address(), Some(&context)).await;
        report_later(&later, location, result);
        return;
    }
    match resolve_default_application(&file).await {
        Ok(DefaultApplication {
            content_type,
            app: Some(default),
            ..
        }) => {
            let result = launch_at(&default, (&file, &content_type), position, Some(&context));
            report_later(&later, location, result);
        }
        Ok(DefaultApplication {
            content_type,
            app: None,
            executable,
        }) => {
            if can_confirm_executable(&file, &location, executable) {
                confirm_executable_later(&later, location);
            } else {
                show_fallback(file, content_type, later).await;
            }
        }
        Err(error) => report_later(&later, location, Err(error)),
    }
}

fn confirm_executable_later(later: &Later<App>, location: Location) {
    later.then(move |app, tx| {
        if let Some(path) = location.native_path() {
            run::confirm(app, path.to_path_buf(), location.display_name());
            shell::settle(app, tx);
        }
        true
    });
}

async fn show_fallback(file: gio::File, content_type: String, later: Later<App>) {
    let history = recent::read().await;
    let requires_uris = requires_uri_handlers(std::slice::from_ref(&file));
    let choices = choices_for_type(&history, &content_type, requires_uris);
    later.then(move |app, tx| {
        if super::foreign_modal_open(app) {
            return false;
        }
        super::show_chooser(
            app,
            ChooserSpec {
                files: vec![file],
                content_types: vec![content_type],
                choices,
                context: OpenWithContext::ActivationFallback,
            },
        );
        shell::settle(app, tx);
        true
    });
}

fn report_later(later: &Later<App>, location: Location, result: Result<(), glib::Error>) {
    let Err(error) = result else {
        return;
    };
    later.then(move |app, tx| {
        report_open_error(app, &location, &error);
        shell::settle(app, tx);
        true
    });
}

pub(super) fn report_open_error(app: &mut App, location: &Location, error: &glib::Error) {
    tracing::warn!(
        backend = %location.backend_name(),
        error_domain = ?error.domain(),
        error_code = error.code(),
        "unable to open file"
    );
    tracing::debug!(
        location = %location.diagnostic_path(),
        "file open location"
    );
    let detail = if error.matches(gio::IOErrorEnum::NotSupported)
        && gio_file_for_location(location).is_native()
    {
        tr("No application is registered for this file")
    } else {
        error.message().to_owned()
    };
    error_dialog::show(app, Error::new(tr("Unable to open file"), detail));
}

async fn read_web_link(file: &gio::File) -> Option<WebLink> {
    const MAX_LINK_BYTES: i64 = 64 * 1024;
    if !file
        .basename()
        .is_some_and(|name| has_link_extension(&name))
    {
        return None;
    }
    let info = file
        .query_info_future(
            "standard::type,standard::size",
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await
        .ok()?;
    if info.file_type() != gio::FileType::Regular || info.size() > MAX_LINK_BYTES {
        return None;
    }
    let (contents, _) = file.load_contents_future().await.ok()?;
    WebLink::from_desktop_entry(std::str::from_utf8(&contents).ok()?)
}

struct DefaultApplication {
    content_type: String,
    app: Option<gio::AppInfo>,
    executable: bool,
}

async fn resolve_default_application(file: &gio::File) -> Result<DefaultApplication, glib::Error> {
    let info = file
        .query_info_future(
            &format!("standard::type,{CONTENT_TYPE_ATTRIBUTES},unix::mode"),
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await?;
    if info.file_type() == gio::FileType::SymbolicLink {
        return Err(glib::Error::new(
            gio::IOErrorEnum::Failed,
            &tr("Broken symbolic links cannot be opened with an application"),
        ));
    }
    let content_type = content_type_of(&info)
        .map(|value| value.to_string())
        .ok_or_else(|| {
            glib::Error::new(
                gio::IOErrorEnum::Failed,
                &tr("Unable to determine the selected file type"),
            )
        })?;
    let executable = info.file_type() == gio::FileType::Regular
        && info.attribute_uint32(gio::FILE_ATTRIBUTE_UNIX_MODE) & 0o111 != 0;
    let app = if uses_portal() {
        None
    } else {
        let requires_uris = requires_uri_handlers(std::slice::from_ref(file));
        gio::AppInfo::default_for_type(&content_type, requires_uris)
    };
    Ok(DefaultApplication {
        content_type,
        app,
        executable,
    })
}

pub(super) fn launch_with_portal(files: Vec<gio::File>, later: Later<App>) {
    let context = launch_context();
    glib::MainContext::default().spawn_local(async move {
        for file in files {
            let uri = file.uri();
            let Err(error) =
                gio::AppInfo::launch_default_for_uri_future(uri.as_str(), Some(&context)).await
            else {
                continue;
            };
            if !error.matches(gio::IOErrorEnum::Cancelled) {
                let message = error.message().to_owned();
                later.then(move |app, tx| {
                    error_dialog::show(app, Error::new(tr("Unable to open file"), message));
                    shell::settle(app, tx);
                    true
                });
            }
            return;
        }
    });
}

pub(super) fn launch(
    app: &gio::AppInfo,
    batch: LaunchBatch<'_>,
    context: Option<&impl IsA<gio::AppLaunchContext>>,
) -> Result<(), glib::Error> {
    launch_with_recent_registration(app, app, batch, context)
}

pub(super) fn launch_at(
    app: &gio::AppInfo,
    (file, content_type): (&gio::File, &str),
    position: Option<Duration>,
    context: Option<&impl IsA<gio::AppLaunchContext>>,
) -> Result<(), glib::Error> {
    let files = std::slice::from_ref(file);
    let content_types = [content_type.to_owned()];
    let batch = LaunchBatch {
        files,
        content_types: &content_types,
    };
    if let Some(timed) = timed_launcher(app, position)
        && launch_with_recent_registration(app, &timed, batch, context).is_ok()
    {
        return Ok(());
    }
    launch(app, batch, context)
}

fn timed_launcher(app: &gio::AppInfo, position: Option<Duration>) -> Option<gio::AppInfo> {
    let position = position?;
    let exec = app.commandline()?;
    let command_line = command_line_at(&exec.to_string_lossy(), position.as_secs_f64())?;
    let mut flags = if app.supports_uris() {
        gio::AppInfoCreateFlags::SUPPORTS_URIS
    } else {
        gio::AppInfoCreateFlags::NONE
    };
    if app
        .downcast_ref::<gio_unix::DesktopAppInfo>()
        .is_some_and(|desktop| desktop.boolean("StartupNotify"))
    {
        flags |= gio::AppInfoCreateFlags::SUPPORTS_STARTUP_NOTIFICATION;
    }
    gio::AppInfo::create_from_commandline(command_line, Some(app.name().as_str()), flags).ok()
}

fn is_placeholder(token: &str) -> bool {
    matches!(
        token,
        "%f" | "%F" | "%u" | "%U" | "%i" | "%c" | "%k" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m"
    ) || token.starts_with("@@")
}

fn basename(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

fn player_name(tokens: &[String]) -> Option<String> {
    let mut command = tokens.iter().peekable();
    let mut program = command.next()?;
    while basename(program) == "env" {
        program = command.find(|token| !token.contains('=') && !token.starts_with('-'))?;
    }
    if basename(program) == "flatpak" {
        return flatpak_player_name(tokens);
    }
    Some(basename(program).to_owned())
}

fn flatpak_player_name(tokens: &[String]) -> Option<String> {
    if let Some(name) = tokens
        .iter()
        .find_map(|token| token.strip_prefix("--command="))
    {
        return Some(basename(name).to_owned());
    }
    tokens
        .iter()
        .skip(1)
        .find(|token| {
            token.contains('.')
                && !token.starts_with('-')
                && !token.contains('/')
                && !is_placeholder(token)
        })
        .cloned()
}

fn start_arguments(player: &str, seconds: f64) -> Option<Vec<String>> {
    let seconds = format!("{seconds:.3}");
    Some(match player {
        "mpv" | "io.mpv.Mpv" => vec![format!("--start={seconds}")],
        "vlc" | "cvlc" | "qvlc" | "nvlc" | "org.videolan.VLC" => {
            vec![format!("--start-time={seconds}")]
        }
        "celluloid" | "io.github.celluloid_player.Celluloid" => {
            vec![format!("--mpv-start={seconds}")]
        }
        "mplayer" => vec!["-ss".to_owned(), seconds],
        _ => return None,
    })
}

pub(super) fn command_line_at(exec: &str, seconds: f64) -> Option<String> {
    let mut tokens: Vec<String> = glib::shell_parse_argv(exec)
        .ok()?
        .into_iter()
        .map(|token| token.to_string_lossy().into_owned())
        .collect();
    let arguments = start_arguments(&player_name(&tokens)?, seconds)?;
    let insert_at = tokens
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, token)| is_placeholder(token) || *token == "--")
        .map_or(tokens.len(), |(index, _)| index);
    tokens.splice(insert_at..insert_at, arguments);
    Some(
        tokens
            .iter()
            .map(|token| shell_token(token))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn shell_token(token: &str) -> String {
    let plain = !token.is_empty()
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-./=:@%+,".contains(&byte));
    if plain || is_placeholder(token) {
        token.to_owned()
    } else {
        glib::shell_quote(token).to_string_lossy().into_owned()
    }
}

pub(super) fn launch_in_folder(
    launcher: &gio::AppInfo,
    files: &[gio::File],
    context: Option<&impl IsA<gio::AppLaunchContext>>,
) -> Result<(), glib::Error> {
    let folder = files
        .first()
        .and_then(|file| file.path())
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let previous = match (folder, std::env::current_dir()) {
        (Some(folder), Ok(previous)) if std::env::set_current_dir(&folder).is_ok() => {
            Some(previous)
        }
        _ => None,
    };
    let result = launcher.launch(files, context);
    if let Some(previous) = previous
        && let Err(error) = std::env::set_current_dir(&previous)
    {
        tracing::warn!(%error, "unable to return from the launched program's folder");
    }
    result
}

fn launch_with_recent_registration(
    app: &gio::AppInfo,
    launcher: &gio::AppInfo,
    batch: LaunchBatch<'_>,
    context: Option<&impl IsA<gio::AppLaunchContext>>,
) -> Result<(), glib::Error> {
    if !launcher.supports_uris() && requires_uri_handlers(batch.files) {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            &tr("This application cannot open files at this location"),
        ));
    }
    launch_in_folder(launcher, batch.files, context)?;
    if let Some(app_id) = app.id() {
        let known_ids = gio::AppInfo::all()
            .into_iter()
            .filter_map(|candidate| candidate.id().map(|id| id.to_string()))
            .collect::<HashSet<_>>();
        recent::record(batch.content_types, app_id.as_str(), known_ids);
    }
    schedule_recent_files(batch.files, batch.content_types.first().cloned());
    Ok(())
}

fn schedule_recent_files(files: &[gio::File], fallback_mime: Option<String>) {
    let candidates: Vec<gio::File> = files
        .iter()
        .filter(|file| !file.has_uri_scheme("recent"))
        .cloned()
        .collect();
    if candidates.is_empty() {
        return;
    }
    glib::MainContext::default().spawn_local(async move {
        for file in candidates {
            let info = file
                .query_info_future(
                    &format!("standard::type,{CONTENT_TYPE_ATTRIBUTES}"),
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                )
                .await
                .ok();
            let file_type = info.as_ref().map(gio::FileInfo::file_type);
            if matches!(
                file_type,
                Some(gio::FileType::Directory | gio::FileType::Mountable)
            ) {
                continue;
            }
            let mime = info
                .as_ref()
                .and_then(content_type_of)
                .map(|m| m.to_string())
                .or_else(|| fallback_mime.clone())
                .unwrap_or_else(|| "application/octet-stream".to_owned());
            register_recent_file(file.uri().to_string(), mime);
        }
    });
}

fn register_recent_file(uri: String, mime: String) {
    drop(writer::run(move || {
        let path = glib::user_data_dir().join(RECENTLY_USED_XBEL);
        let contents = update_xbel(std::fs::read_to_string(&path).ok().as_deref(), &uri, &mime);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(error) = strata_core::storage::atomic_write(&path, contents.as_bytes()) {
            tracing::debug!(uri = %diagnostic_uri(&uri), %error, "unable to record file in Recent history");
        }
    }));
}

fn update_xbel(existing: Option<&str>, uri: &str, mime: &str) -> String {
    let stamp = glib::DateTime::now_utc()
        .and_then(|now| now.format_iso8601())
        .map_or_else(|_| "1970-01-01T00:00:00Z".to_owned(), |s| s.to_string());
    let escaped_uri = escape_xml_attr(uri);
    let escaped_mime = escape_xml_attr(mime);
    let entry = format!(
        "  <bookmark href=\"{escaped_uri}\" added=\"{stamp}\" modified=\"{stamp}\" visited=\"{stamp}\">\n    <info>\n      <metadata owner=\"http://freedesktop.org\">\n        <mime:mime-type type=\"{escaped_mime}\"/>\n        <bookmark:applications>\n          <bookmark:application name=\"strata\" exec=\"&apos;strata %u&apos;\" modified=\"{stamp}\" count=\"1\"/>\n        </bookmark:applications>\n      </metadata>\n    </info>\n  </bookmark>\n"
    );
    let needle = format!("<bookmark href=\"{escaped_uri}\"");
    if let Some(body) = existing
        && let Some(close) = body.rfind("</xbel>")
    {
        let prefix = strip_existing_bookmark(&body[..close], &needle);
        return format!("{prefix}{entry}</xbel>\n");
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel version=\"1.0\" xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\" xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\">\n{entry}</xbel>\n"
    )
}

fn strip_existing_bookmark(xml: &str, needle: &str) -> String {
    let Some(start) = xml.find(needle) else {
        return xml.to_owned();
    };
    let line_start = xml[..start].rfind('\n').map_or(0, |pos| pos + 1);
    let Some(end_rel) = xml[start..].find("</bookmark>") else {
        return xml.to_owned();
    };
    let after = start + end_rel + "</bookmark>".len();
    let after_line = xml[after..]
        .strip_prefix('\n')
        .map_or(&xml[after..], |rest| rest);
    format!("{}{after_line}", &xml[..line_start])
}

fn escape_xml_attr(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

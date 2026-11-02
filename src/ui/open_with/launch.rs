// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use gtk::{gio, glib, prelude::*};

use super::requires_uri_handlers;
use crate::ui::{browser::show_error_dialog, recent_apps};

/// Opens each file with the host's default handler, or lets the desktop's
/// own chooser pick one when `always_ask` is set. Files open one after
/// another so choosers do not stack; dismissing one stops the rest.
pub(in crate::ui) fn launch_with_portal(
    parent: &impl IsA<gtk::Widget>,
    files: Vec<gio::File>,
    always_ask: bool,
) {
    let window = parent
        .as_ref()
        .root()
        .and_downcast::<gtk::Window>()
        .map(|window| window.downgrade());
    let parent = parent.as_ref().downgrade();
    glib::MainContext::default().spawn_local(async move {
        for file in files {
            let window = window.as_ref().and_then(glib::WeakRef::upgrade);
            let launcher = gtk::FileLauncher::new(Some(&file));
            launcher.set_always_ask(always_ask);
            let Err(error) = launcher.launch_future(window.as_ref()).await else {
                continue;
            };
            if is_launch_failure(&error)
                && let Some(parent) = parent.upgrade()
            {
                show_error_dialog(
                    &parent,
                    &crate::i18n::tr("Unable to open file"),
                    error.message(),
                );
            }
            return;
        }
    });
}

/// Dismissing the desktop's chooser is not an error worth reporting.
fn is_launch_failure(error: &glib::Error) -> bool {
    !error.matches(gtk::DialogError::Dismissed) && !error.matches(gio::IOErrorEnum::Cancelled)
}

pub(in crate::ui) fn launch(
    app: &gio::AppInfo,
    files: &[gio::File],
    content_types: &[String],
    context: Option<&impl IsA<gio::AppLaunchContext>>,
) -> Result<(), glib::Error> {
    launch_with_recent_registration(
        app,
        app,
        files,
        content_types,
        context,
        register_recent_file,
    )
}

pub(in crate::ui) fn launch_at(
    app: &gio::AppInfo,
    file: &gio::File,
    content_type: &str,
    position: Option<std::time::Duration>,
    context: Option<&impl IsA<gio::AppLaunchContext>>,
) -> Result<(), glib::Error> {
    let files = std::slice::from_ref(file);
    let content_types = [content_type.to_owned()];
    let timed = position
        .and_then(|position| {
            let exec = app.commandline()?;
            command_line_at(&exec.to_string_lossy(), position.as_secs_f64())
        })
        .and_then(|command_line| {
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
            gio::AppInfo::create_from_commandline(command_line, Some(app.name().as_str()), flags)
                .ok()
        });
    if let Some(timed) = timed
        && launch_with_recent_registration(
            app,
            &timed,
            files,
            &content_types,
            context,
            register_recent_file,
        )
        .is_ok()
    {
        return Ok(());
    }
    launch(app, files, &content_types, context)
}

/// Field codes and Flatpak forwarding markers must stay unquoted and mark
/// where the file arguments go.
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
        if let Some(name) = tokens
            .iter()
            .find_map(|token| token.strip_prefix("--command="))
        {
            return Some(basename(name).to_owned());
        }
        return tokens
            .iter()
            .skip(1)
            .find(|token| {
                token.contains('.')
                    && !token.starts_with('-')
                    && !token.contains('/')
                    && !is_placeholder(token)
            })
            .cloned();
    }
    Some(basename(program).to_owned())
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
    let tokens: Vec<String> = glib::shell_parse_argv(exec)
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
    let mut tokens = tokens;
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

/// GIO spawns a program in Strata's working folder, which is `/` so Strata
/// never keeps a mount busy. Enter the first file's folder only for the
/// synchronous spawn; a desktop entry's `Path` still wins.
pub(super) fn launch_in_folder(
    launcher: &gio::AppInfo,
    files: &[gio::File],
    context: Option<&impl IsA<gio::AppLaunchContext>>,
) -> Result<(), glib::Error> {
    let folder = files
        .first()
        .and_then(|file| file.path())
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf));
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
    files: &[gio::File],
    content_types: &[String],
    context: Option<&impl IsA<gio::AppLaunchContext>>,
    register_recent: impl Fn(&gio::File) -> bool + 'static,
) -> Result<(), glib::Error> {
    // GIO drops files without a local path when expanding %f/%F.
    if !launcher.supports_uris() && requires_uri_handlers(files) {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            &crate::i18n::tr("This application cannot open files at this location"),
        ));
    }
    launch_in_folder(launcher, files, context)?;
    if let Some(app_id) = app.id() {
        let known_ids = gio::AppInfo::all()
            .into_iter()
            .filter_map(|candidate| candidate.id().map(|id| id.to_string()))
            .collect::<HashSet<_>>();
        recent_apps::record(content_types, app_id.as_str(), known_ids);
    }
    let candidates: Vec<gio::File> = files
        .iter()
        .filter(|file| !file.has_uri_scheme("recent"))
        .cloned()
        .collect();
    if candidates.is_empty() {
        return Ok(());
    }
    // Advisory history updates must not block launching on slow mounts.
    glib::MainContext::default().spawn_local(async move {
        for file in candidates {
            let file_type = file
                .query_info_future(
                    "standard::type",
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                )
                .await
                .map(|info| info.file_type());
            // A successful launch remains worth recording when metadata is unavailable.
            if !matches!(
                file_type,
                Ok(gio::FileType::Directory | gio::FileType::Mountable)
            ) {
                register_recent(&file);
            }
        }
    });
    Ok(())
}

fn register_recent_file(file: &gio::File) -> bool {
    let manager = gtk::RecentManager::default();
    let uri = file.uri();
    let added = manager.add_item(uri.as_str());
    if !added {
        tracing::debug!(uri = %crate::model::diagnostic_uri(uri.as_str()), "unable to record file in Recent history");
    }
    added
}

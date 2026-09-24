// SPDX-License-Identifier: MIT

mod adapters;
mod app;
mod assets;
mod build_info;
mod media;
mod metrics;
mod model;
mod portal;
mod portal_setup;
mod sandbox;
mod sandbox_helper;
mod services;
mod storage;
#[cfg(test)]
mod test_support;
mod trusted_command;
mod ui;
mod util;

use std::{
    ffi::OsString, ops::ControlFlow, os::unix::process::CommandExt, process::Stdio, time::Duration,
};

use gtk::{gio, glib, prelude::*};

use ui::UnlockTarget;

const APPLICATION_ID: &str = "io.github.lgse.Strata";
const CAIRO_SELECTED_BY_STRATA: &str = "STRATA_CAIRO_SELECTED_BY_STRATA";
const GVFS_PROBE_ARGUMENT: &str = "--gvfs-probe";
const GVFS_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const GIO_FALLBACK_BACKENDS: [(&str, &str); 2] =
    [("GIO_USE_VFS", "local"), ("GIO_USE_VOLUME_MONITOR", "unix")];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaunchMode {
    PreviewHelper,
    BrowserWorker,
    GvfsProbe,
    Portal,
    InstallPortal,
    DismissPortalPrompt,
    UninstallPortal,
    Version,
    UdiskieHook,
    InstallUdiskie,
    UninstallUdiskie,
    Application,
}

/// Byte-safe: a non-UTF-8 path argument is an ordinary application launch,
/// not a reason to abort before GIO ever sees it.
fn launch_mode(arguments: &[OsString]) -> LaunchMode {
    match arguments.get(1).and_then(|argument| argument.to_str()) {
        Some("--preview-helper") => LaunchMode::PreviewHelper,
        Some("--browser-worker") => LaunchMode::BrowserWorker,
        Some(GVFS_PROBE_ARGUMENT) => LaunchMode::GvfsProbe,
        Some("--portal") => LaunchMode::Portal,
        Some("--install-portal") => LaunchMode::InstallPortal,
        Some("--dismiss-portal-prompt") => LaunchMode::DismissPortalPrompt,
        Some("--uninstall-portal") => LaunchMode::UninstallPortal,
        Some("--version") => LaunchMode::Version,
        Some("--udiskie-hook") => LaunchMode::UdiskieHook,
        Some("--install-udiskie-unlock") => LaunchMode::InstallUdiskie,
        Some("--uninstall-udiskie-unlock") => LaunchMode::UninstallUdiskie,
        _ => LaunchMode::Application,
    }
}

fn should_select_cairo(
    supplied_renderer: Option<&std::ffi::OsStr>,
    choice: ui::preferences::InterfaceRenderer,
) -> bool {
    supplied_renderer.is_none() && choice == ui::preferences::InterfaceRenderer::Cairo
}

fn version_line() -> String {
    format!(
        "{} {}",
        env!("CARGO_PKG_NAME"),
        build_info::installed_version()
    )
}

fn classify_udiskie_hook(arguments: &[OsString]) -> Option<&str> {
    let [event, id_usage, device_file, id_uuid, ..] = arguments else {
        return None;
    };
    let event = event.to_str()?;
    let id_usage = id_usage.to_str()?;
    let device_file = device_file.to_str()?;
    let id_uuid = id_uuid.to_str()?;
    if event != "device_added" || id_usage != "crypto" {
        tracing::debug!(event, id_usage, "udiskie hook rejected");
        return None;
    }
    if !device_file.is_empty() {
        return Some(device_file);
    }
    if !id_uuid.is_empty() {
        return Some(id_uuid);
    }
    tracing::debug!(event, id_usage, "udiskie hook rejected");
    None
}

fn run_udiskie_hook(arguments: &[OsString]) -> gtk::glib::ExitCode {
    let Some(operand) = classify_udiskie_hook(arguments) else {
        return gtk::glib::ExitCode::SUCCESS;
    };
    let Ok(executable) = std::env::current_exe() else {
        eprintln!("Unable to locate the Strata executable");
        return gtk::glib::ExitCode::FAILURE;
    };
    let error = std::process::Command::new(executable)
        .arg("--unlock-volume")
        .arg(operand)
        .exec();
    eprintln!("Unable to start --unlock-volume: {error}");
    gtk::glib::ExitCode::FAILURE
}

fn main() -> gtk::glib::ExitCode {
    let arguments: Vec<OsString> = std::env::args_os().collect();
    match launch_mode(&arguments) {
        LaunchMode::BrowserWorker => {
            return match sandbox::browser::run() {
                Ok(()) => gtk::glib::ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("Browser helper failed: {error}");
                    gtk::glib::ExitCode::FAILURE
                }
            };
        }
        LaunchMode::PreviewHelper => {
            if let Err(error) = run_preview_helper(&arguments[2..]) {
                eprintln!("Preview helper failed: {error}");
                return gtk::glib::ExitCode::FAILURE;
            }
            return gtk::glib::ExitCode::SUCCESS;
        }
        LaunchMode::GvfsProbe => {
            let _vfs = gio::Vfs::default();
            let _volumes = gio::VolumeMonitor::get();
            return gtk::glib::ExitCode::SUCCESS;
        }
        LaunchMode::Portal => {
            restart_with_local_vfs_if_gvfs_is_unresponsive();
            return portal::run();
        }
        LaunchMode::InstallPortal => return finish_portal_setup(portal_setup::install()),
        LaunchMode::DismissPortalPrompt => {
            return finish_portal_setup(portal_setup::dismiss_prompt());
        }
        LaunchMode::UninstallPortal => return finish_portal_setup(portal_setup::uninstall()),
        LaunchMode::Version => {
            println!("{}", version_line());
            return gtk::glib::ExitCode::SUCCESS;
        }
        LaunchMode::UdiskieHook => return run_udiskie_hook(&arguments[2..]),
        LaunchMode::InstallUdiskie => {
            return finish_portal_setup(portal_setup::udiskie::install());
        }
        LaunchMode::UninstallUdiskie => {
            return finish_portal_setup(portal_setup::udiskie::uninstall());
        }
        LaunchMode::Application => {}
    }

    install_application_identity();
    metrics::initialize();
    if let Err(error) = tracing_subscriber::fmt::try_init() {
        eprintln!("Unable to initialize logging: {error}");
    }
    if should_select_cairo(
        std::env::var_os("GSK_RENDERER").as_deref(),
        ui::preferences::PreferenceManager::shared().interface_renderer(),
    ) {
        // GTK selects its renderer during startup. Re-exec before initializing GTK
        // rather than mutating the environment after libraries may start threads.
        let Ok(executable) = std::env::current_exe() else {
            eprintln!("Unable to locate Strata to select the Cairo renderer");
            return gtk::glib::ExitCode::FAILURE;
        };
        let error = std::process::Command::new(executable)
            .args(&arguments[1..])
            .env("GSK_RENDERER", "cairo")
            .env(CAIRO_SELECTED_BY_STRATA, "1")
            .exec();
        eprintln!("Unable to restart Strata with Cairo renderer: {error}");
        return gtk::glib::ExitCode::FAILURE;
    }

    if let Err(error) = portal_setup::refresh_stale_portal() {
        tracing::warn!(%error, "could not refresh the stale Strata portal");
    }

    // Timed from here so `window presented` covers the whole launch, the way
    // the field harness measures mapped from process start.
    let launched = std::time::Instant::now();
    restart_with_local_vfs_if_gvfs_is_unresponsive();
    tracing::debug!(
        elapsed_ms = launched.elapsed().as_millis() as u64,
        "startup gvfs probe finished"
    );

    let assets_started = std::time::Instant::now();
    if let Err(error) = assets::prepare() {
        eprintln!("Unable to prepare bundled assets: {error}");
    }
    tracing::debug!(
        elapsed_ms = assets_started.elapsed().as_millis() as u64,
        total_elapsed_ms = launched.elapsed().as_millis() as u64,
        "startup bundled assets prepared"
    );

    let application = gtk::Application::builder()
        .application_id(APPLICATION_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN | gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    application.add_main_option(
        "unlock-volume",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::String,
        "Unlock an encrypted volume and show the password prompt",
        Some("DEVICE"),
    );
    application.connect_handle_local_options(|_, _| ControlFlow::Continue(()));
    application.connect_startup(|_| install_x11_program_class());
    application.connect_startup(export_file_manager_interface);
    application.connect_activate(ui::present);
    application.connect_open(|application, files, _| {
        if files.is_empty() {
            ui::present(application);
        }
        for file in files {
            ui::present_open(application, file.clone());
        }
    });
    application.connect_command_line(handle_command_line);
    application.run()
}

const UNLOCK_VOLUME_WITH_FILES: &str = "cannot combine --unlock-volume with file arguments";

#[derive(Debug)]
enum CommandLineAction {
    Unlock(UnlockTarget),
    Open(Vec<gio::File>),
    Activate,
    ServiceNoop,
    Usage(&'static str),
}

fn classify_command_line(
    unlock_volume: Option<&str>,
    remaining_files: &[gio::File],
    is_service: bool,
    is_remote: bool,
) -> CommandLineAction {
    match unlock_volume {
        Some(_) if !remaining_files.is_empty() => {
            CommandLineAction::Usage(UNLOCK_VOLUME_WITH_FILES)
        }
        Some(operand) => match UnlockTarget::parse(operand) {
            Ok(target) => CommandLineAction::Unlock(target),
            Err(message) => CommandLineAction::Usage(message),
        },
        None if !remaining_files.is_empty() => CommandLineAction::Open(remaining_files.to_vec()),
        None if !is_remote && is_service => CommandLineAction::ServiceNoop,
        None => CommandLineAction::Activate,
    }
}

fn handle_command_line(
    application: &gtk::Application,
    cmdline: &gio::ApplicationCommandLine,
) -> glib::ExitCode {
    let unlock_volume = cmdline
        .options_dict()
        .lookup::<String>("unlock-volume")
        .ok()
        .flatten();
    let files: Vec<gio::File> = cmdline
        .arguments()
        .iter()
        .skip(1)
        .map(|argument| cmdline.create_file_for_arg(argument))
        .collect();
    let is_service = application
        .flags()
        .contains(gio::ApplicationFlags::IS_SERVICE);
    match classify_command_line(
        unlock_volume.as_deref(),
        &files,
        is_service,
        cmdline.is_remote(),
    ) {
        CommandLineAction::Unlock(target) => {
            ui::present_unlock(application, target);
            glib::ExitCode::SUCCESS
        }
        CommandLineAction::Open(files) => {
            application.open(&files, "");
            glib::ExitCode::SUCCESS
        }
        CommandLineAction::Activate => {
            application.activate();
            glib::ExitCode::SUCCESS
        }
        CommandLineAction::ServiceNoop => glib::ExitCode::SUCCESS,
        CommandLineAction::Usage(message) => {
            eprintln!("{message}");
            glib::ExitCode::FAILURE
        }
    }
}

fn install_application_identity() {
    glib::set_prgname(Some(APPLICATION_ID));
    glib::set_application_name("Strata");
}

fn install_x11_program_class() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let Ok(x11) = display.downcast::<gdk4_x11::X11Display>() else {
        return;
    };
    x11.set_program_class(APPLICATION_ID);
}

fn run_preview_helper(arguments: &[OsString]) -> Result<(), String> {
    let arguments = arguments
        .iter()
        .map(|argument| {
            argument
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| "Invalid UTF-8 in preview helper arguments".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    sandbox_helper::run(&arguments)
}

fn finish_portal_setup(result: Result<String, String>) -> gtk::glib::ExitCode {
    match result {
        Ok(message) => {
            println!("{message}");
            gtk::glib::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            gtk::glib::ExitCode::FAILURE
        }
    }
}

/// Answers "Open file location" from browsers and other desktop apps, which
/// call `org.freedesktop.FileManager1` instead of the `inode/directory`
/// handler.
fn export_file_manager_interface(application: &gtk::Application) {
    let Some(connection) = application.dbus_connection() else {
        return;
    };
    let target = application.clone();
    if let Err(error) = adapters::export_file_manager(&connection, move |request| {
        ui::present_reveal(&target, request);
    }) {
        tracing::warn!(%error, "unable to export the file manager interface");
    }
}

fn restart_with_local_vfs_if_gvfs_is_unresponsive() {
    if std::env::var_os("GIO_USE_VFS").is_some() {
        return;
    }
    // Skip the subprocess probe when the marker matches the current `gvfsd`
    // generation; a daemon restart (new pid) re-arms it. A daemon wedging
    // under the same pid is not detected.
    if gvfs_probe_marker_is_fresh() {
        return;
    }
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let responsive = sandbox_helper::run_command_with_timeout(
        std::process::Command::new(&executable)
            .arg(GVFS_PROBE_ARGUMENT)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
        GVFS_PROBE_TIMEOUT,
    )
    .unwrap_or(true);
    if responsive {
        write_gvfs_probe_marker();
        return;
    }

    eprintln!("GVFS is unresponsive; using local filesystem and volume support for this session.");
    let error = std::process::Command::new(executable)
        .args(std::env::args_os().skip(1))
        .envs(GIO_FALLBACK_BACKENDS)
        .exec();
    eprintln!("Unable to restart Strata with local filesystem and volume support: {error}");
}

fn gvfs_probe_marker_path() -> Option<std::path::PathBuf> {
    gvfs_probe_marker_path_in(std::env::var_os("XDG_RUNTIME_DIR"))
}

fn gvfs_probe_marker_path_in(runtime: Option<std::ffi::OsString>) -> Option<std::path::PathBuf> {
    let runtime = runtime?;
    if runtime.is_empty() {
        return None;
    }
    Some(std::path::Path::new(&runtime).join("strata-gvfs-probe-ok"))
}

/// Kernel pids of `gvfsd*` processes, read from `/proc` to avoid a subprocess
/// spawn; any restart re-arms the probe.
fn gvfs_daemon_pids(proc_root: &std::path::Path) -> Vec<u32> {
    let mut pids = Vec::new();
    let Ok(entries) = std::fs::read_dir(proc_root) else {
        return pids;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid): Option<u32> = name.to_str().and_then(|name| name.parse().ok()) else {
            continue;
        };
        let comm = entry.path().join("comm");
        if std::fs::read_to_string(comm)
            .is_ok_and(|contents| contents.trim_end().starts_with("gvfsd"))
        {
            pids.push(pid);
        }
    }
    pids.sort_unstable();
    pids
}

fn encode_daemon_pids(pids: &[u32]) -> String {
    pids.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn gvfs_probe_marker_is_fresh() -> bool {
    let Some(path) = gvfs_probe_marker_path() else {
        return false;
    };
    gvfs_probe_marker_is_fresh_at(&path, std::path::Path::new("/proc"))
}

fn gvfs_probe_marker_is_fresh_at(path: &std::path::Path, proc_root: &std::path::Path) -> bool {
    let Ok(stored) = std::fs::read_to_string(path) else {
        return false;
    };
    stored == encode_daemon_pids(&gvfs_daemon_pids(proc_root))
}

fn write_gvfs_probe_marker() {
    let Some(path) = gvfs_probe_marker_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ignored = std::fs::create_dir_all(parent);
    }
    let contents = encode_daemon_pids(&gvfs_daemon_pids(std::path::Path::new("/proc")));
    let _ignored = std::fs::write(&path, contents);
}

#[cfg(test)]
mod tests;

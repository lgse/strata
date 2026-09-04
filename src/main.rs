// SPDX-License-Identifier: GPL-3.0-or-later

mod adapters;
mod app;
mod assets;
mod build_info;
mod metrics;
mod model;
mod sandbox;
mod sandbox_helper;
mod services;
mod storage;
#[cfg(test)]
mod test_support;
mod ui;
mod util;

use std::{process::Stdio, time::Duration};

use gtk::{gio, prelude::*};

const APPLICATION_ID: &str = "io.github.lgse.Strata";
const GVFS_PROBE_ARGUMENT: &str = "--gvfs-probe";
const GVFS_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

fn main() -> gtk::glib::ExitCode {
    let arguments: Vec<_> = std::env::args().collect();
    if arguments
        .get(1)
        .is_some_and(|value| value == "--preview-helper")
    {
        if let Err(error) = sandbox_helper::run(&arguments[2..]) {
            eprintln!("Preview helper failed: {error}");
            return gtk::glib::ExitCode::FAILURE;
        }
        return gtk::glib::ExitCode::SUCCESS;
    }
    if arguments
        .get(1)
        .is_some_and(|value| value == GVFS_PROBE_ARGUMENT)
    {
        let _vfs = gio::Vfs::default();
        let _volumes = gio::VolumeMonitor::get();
        return gtk::glib::ExitCode::SUCCESS;
    }

    if let Some(exit_code) = restart_with_local_vfs_if_gvfs_is_unresponsive() {
        return exit_code;
    }

    metrics::initialize();
    if let Err(error) = tracing_subscriber::fmt::try_init() {
        eprintln!("Unable to initialize logging: {error}");
    }

    if let Err(error) = assets::prepare() {
        eprintln!("Unable to prepare bundled assets: {error}");
    }

    let application = gtk::Application::builder()
        .application_id(APPLICATION_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    application.connect_activate(ui::present);
    application.connect_open(|application, files, _| {
        let location = files.first().and_then(gio::File::path);
        ui::present_location(application, location);
    });
    application.run()
}

fn restart_with_local_vfs_if_gvfs_is_unresponsive() -> Option<gtk::glib::ExitCode> {
    if std::env::var_os("GIO_USE_VFS").is_some() {
        return None;
    }
    let Ok(executable) = std::env::current_exe() else {
        return None;
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
        return None;
    }

    eprintln!("GVFS is unresponsive; using local filesystem support for this session.");
    match std::process::Command::new(executable)
        .args(std::env::args_os().skip(1))
        .env("GIO_USE_VFS", "local")
        .status()
    {
        Ok(status) if status.success() => Some(gtk::glib::ExitCode::SUCCESS),
        Ok(_) => Some(gtk::glib::ExitCode::FAILURE),
        Err(error) => {
            eprintln!("Unable to restart Strata with local filesystem support: {error}");
            None
        }
    }
}

// SPDX-License-Identifier: MIT

use std::{cell::Cell, collections::HashSet, path::Path};

use super::*;
use crate::services::package_manager::PackageManager::{Apt, Dnf, Pacman, Zypper};

fn names(list: &[&str]) -> HashSet<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn install_hints_use_the_detected_distribution_or_name_a_reference_package() {
    assert_eq!(
        install_hint(packages::GVFS_SMB, Some(Apt)),
        Some(InstallHint::Command(
            "sudo apt install gvfs-backends".to_owned()
        ))
    );
    assert_eq!(
        install_hint(packages::FFMPEG, Some(Dnf)),
        Some(InstallHint::Command(
            "sudo dnf install ffmpeg-free".to_owned()
        ))
    );
    assert_eq!(
        install_hint(packages::GTKSOURCEVIEW, Some(Zypper)),
        Some(InstallHint::Package("gtksourceview5"))
    );
    assert_eq!(
        install_hint(packages::BUBBLEWRAP, None),
        Some(InstallHint::Package("bubblewrap"))
    );
    assert_eq!(install_hint(&[], Some(Pacman)), None);
}

#[test]
fn preview_sandbox_reports_the_first_missing_requirement_before_rendering() {
    let existing = std::env::current_exe().expect("test executable");
    let missing = Path::new("/nonexistent/prlimit");
    let rendered = Cell::new(0);
    let render = |result: Result<(), String>| {
        let rendered = &rendered;
        move || {
            rendered.set(rendered.get() + 1);
            result
        }
    };

    let check = preview_sandbox(false, &existing, render(Ok(())));
    assert_eq!(
        (check.status, check.finding, check.packages),
        (
            Status::Problem,
            Finding::BubblewrapMissing,
            packages::BUBBLEWRAP
        )
    );
    let check = preview_sandbox(true, missing, render(Ok(())));
    assert_eq!(check.status, Status::Problem);
    assert_eq!(
        check.finding,
        Finding::PrlimitMissing {
            path: "/nonexistent/prlimit".to_owned()
        }
    );
    assert_eq!(check.packages, packages::UTIL_LINUX);
    assert_eq!(rendered.get(), 0);

    let check = preview_sandbox(true, &existing, render(Err("setting up uid map".into())));
    assert_eq!(
        (check.status, check.finding, check.detail.as_deref()),
        (
            Status::Problem,
            Finding::SandboxFailed,
            Some("setting up uid map")
        )
    );
    let check = preview_sandbox(true, &existing, render(Ok(())));
    assert_eq!(
        (check.status, check.finding),
        (Status::Passed, Finding::SandboxWorks)
    );
    assert_eq!(rendered.get(), 2);
}

#[test]
fn workers_without_landlock_warn_without_rendering() {
    let check = workers(false, || panic!("workers are unavailable"));
    assert_eq!(
        (check.status, check.finding),
        (Status::Warning, Finding::LandlockUnavailable)
    );
    let check = workers(true, || Err("Thumbnail unavailable".into()));
    assert_eq!(
        (check.status, check.finding),
        (Status::Problem, Finding::WorkersFailed)
    );
}

#[test]
fn workers_warn_when_workers_disabled_during_render() {
    let check = workers(true, || {
        crate::sandbox::browser::disable_workers();
        Ok(())
    });
    assert_eq!(
        (check.status, check.finding),
        (Status::Warning, Finding::LandlockUnavailable)
    );
    crate::sandbox::browser::enable_workers();
}

#[test]
fn video_thumbnails_distinguish_missing_slow_and_broken_decoders() {
    for (ffmpeg, thumbnailer, render, status, finding, fixed) in [
        (
            false,
            false,
            Ok(()),
            Status::Problem,
            Finding::Missing(vec!["ffmpegthumbnailer".into()]),
            true,
        ),
        (
            true,
            false,
            Ok(()),
            Status::Warning,
            Finding::SlowVideoThumbnails,
            true,
        ),
        (
            true,
            false,
            Err("Thumbnail unavailable".to_owned()),
            Status::Problem,
            Finding::SampleFailed,
            true,
        ),
        (
            false,
            true,
            Ok(()),
            Status::Passed,
            Finding::SampleProcessed,
            false,
        ),
        (
            true,
            true,
            Err("Thumbnail unavailable".to_owned()),
            Status::Problem,
            Finding::SampleFailed,
            false,
        ),
    ] {
        let check = video_thumbnails(ffmpeg, thumbnailer, || render.clone());
        let case = format!("ffmpeg={ffmpeg} thumbnailer={thumbnailer} render={render:?}");
        assert_eq!(check.status, status, "{case}");
        assert_eq!(check.finding, finding, "{case}");
        assert_eq!(!check.packages.is_empty(), fixed, "{case}");
    }
}

#[test]
fn media_previews_name_each_missing_tool_before_reading_a_sample() {
    let check = media_previews(true, false, || panic!("ffprobe is missing"));
    assert_eq!(check.finding, Finding::Missing(vec!["ffprobe".into()]));
    assert_eq!(check.packages, packages::FFMPEG);
    let check = media_previews(false, false, || panic!("tools are missing"));
    assert_eq!(
        check.finding,
        Finding::Missing(vec!["ffmpeg".into(), "ffprobe".into()])
    );
    let check = media_previews(true, true, || Ok(()));
    assert_eq!(check.status, Status::Passed);
}

#[test]
fn audio_playback_points_to_the_package_of_the_missing_element() {
    let probe = |missing: Vec<&'static str>, sink: Option<&str>| {
        Ok(GstreamerProbe {
            missing,
            sink: sink.map(str::to_owned),
        })
    };
    let check = audio_playback(probe(vec!["autoaudiosink"], Some("PulseAudio Audio Sink")));
    assert_eq!(check.status, Status::Problem);
    assert_eq!(check.packages, packages::GST_GOOD);
    let check = audio_playback(probe(vec!["audioconvert", "autoaudiosink"], None));
    assert_eq!(
        check.finding,
        Finding::Missing(vec!["audioconvert".into(), "autoaudiosink".into()])
    );
    assert_eq!(check.packages, packages::GST_BASE);
    let check = audio_playback(probe(Vec::new(), None));
    assert_eq!(
        (check.finding, check.packages),
        (Finding::NoAudioSink, packages::GST_GOOD)
    );
    let check = audio_playback(probe(Vec::new(), Some("PulseAudio Audio Sink")));
    assert_eq!(
        check.finding,
        Finding::AudioOutput {
            sink: "PulseAudio Audio Sink".into()
        }
    );
    let check = audio_playback(Err("no plugin registry".into()));
    assert_eq!(
        (check.finding, check.detail.as_deref()),
        (Finding::GstreamerFailed, Some("no plugin registry"))
    );
}

#[test]
fn remote_backends_follow_gvfs_state() {
    let running = VfsState::Running(names(&["file", "sftp", "trash"]));
    let check = remote_backend(CheckId::Sftp, &running);
    assert_eq!(
        (check.status, check.finding),
        (Status::Passed, Finding::BackendInstalled)
    );
    let check = remote_backend(CheckId::Smb, &running);
    assert_eq!(
        (check.status, check.finding, check.packages),
        (Status::Warning, Finding::BackendMissing, packages::GVFS_SMB)
    );
    for state in [VfsState::Bypassed, VfsState::Missing] {
        let check = remote_backend(CheckId::Mtp, &state);
        assert_eq!(check.finding, Finding::RequiresGvfs, "{state:?}");
        assert!(check.packages.is_empty(), "{state:?}");
        assert_eq!(gvfs(&state).status, Status::Problem, "{state:?}");
    }
    assert_eq!(gvfs(&running).status, Status::Passed);
}

#[test]
fn drives_and_session_services_read_bus_names() {
    let system = names(&[UDISKS_NAME]);
    let session = names(&[VOLUME_MONITOR_NAME, PORTAL_NAME]);
    assert_eq!(
        drives(Some(&session), Some(&system)).finding,
        Finding::DrivesReady
    );
    assert_eq!(drives(Some(&session), None).finding, Finding::UdisksMissing);
    let check = drives(Some(&names(&[])), Some(&system));
    assert_eq!(
        (check.finding, check.packages),
        (Finding::VolumeMonitorMissing, packages::GVFS_DAEMONS)
    );
    assert_eq!(
        session_service(CheckId::Portal, Some(&session)).finding,
        Finding::ServiceAvailable
    );
    let check = session_service(CheckId::SecretService, Some(&session));
    assert_eq!(
        (check.finding, check.packages),
        (Finding::SecretServiceMissing, packages::KEYRING)
    );
    assert_eq!(
        session_service(CheckId::Portal, None).finding,
        Finding::SessionBusUnavailable
    );
}

#[test]
fn folder_monitoring_warns_about_a_low_watch_limit() {
    for (monitored, limit, status) in [
        (Ok(()), Some(524_288), Status::Passed),
        (Ok(()), None, Status::Passed),
        (Ok(()), Some(1024), Status::Warning),
        (
            Err("Too many open files".to_owned()),
            Some(524_288),
            Status::Problem,
        ),
    ] {
        let failed = monitored.is_err();
        let check = folder_monitoring_for(monitored, limit);
        assert_eq!(check.status, status, "{limit:?} failed={failed}");
    }
    assert_eq!(
        folder_monitoring_for(Ok(()), Some(1024)).finding,
        Finding::WatchLimitLow { limit: 1024 }
    );
}

#[test]
fn custom_actions_list_the_ones_that_cannot_run() {
    assert_eq!(
        custom_actions(0, Vec::new(), Vec::new()).finding,
        Finding::NoActions
    );
    assert_eq!(
        custom_actions(2, Vec::new(), Vec::new()).finding,
        Finding::ActionsReady
    );
    let check = custom_actions(2, vec!["Resize".into()], vec!["broken".into()]);
    assert_eq!(check.status, Status::Warning);
    assert_eq!(
        check.finding,
        Finding::ActionsBroken {
            unavailable: vec!["Resize".into()],
            failed: vec!["broken".into()],
        }
    );
}

#[test]
fn sample_documents_render_through_the_preview_helper() {
    let samples = Samples::create().expect("sample files");
    let output = tempfile::tempdir().expect("output directory");
    for (operation, input) in [
        ("thumbnail-image", &samples.image),
        ("thumbnail-pdf", &samples.pdf),
    ] {
        let result = output.path().join(format!("{operation}.png"));
        crate::sandbox_helper::run(&[
            operation.to_owned(),
            input.display().to_string(),
            result.display().to_string(),
            "256".to_owned(),
            "software".to_owned(),
        ])
        .unwrap_or_else(|error| panic!("{operation}: {error}"));
        let png = std::fs::read(&result).expect("rendered thumbnail");
        assert!(
            crate::sandbox::png_dimensions(&png).is_some(),
            "{operation}"
        );
    }
}

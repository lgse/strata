// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    collections::HashSet,
    path::Path,
    rc::Rc,
    sync::{Arc, Mutex, mpsc::Sender},
    time::Duration,
};

use rune::{Key, Mods};
use serde_json::Value;
use strata_core::i18n::tr;
use strata_desktop::{
    build_info::displayed_version,
    services::package_manager::PackageManager::{Apt, Dnf, Pacman, Zypper},
};
use strata_preview::sandbox::{self, browser::WorkerSupport};

use super::{
    checks::{
        self, Check, CheckId, Finding, GROUPS, InstallHint, Status, VfsState, install_hint,
        packages,
    },
    format,
    runner::Samples,
};
use crate::{
    clipboard,
    host::offscreen::Offscreen,
    settings_dialog::tests::{click, has_state, press},
    test_support::{
        PATIENCE, hosted,
        view::{self, idle},
    },
};

const MODULE: &str = "settings_dialog::diagnostics::tests";

fn open_diagnostics(host: &mut Offscreen) {
    click(host, "header-settings");
    click(host, "settings-nav-diagnostics");
}

fn find_by_key<'a>(tree: &'a Value, key: &str) -> Option<&'a Value> {
    view::all(tree, &|node| node["key"] == key)
        .into_iter()
        .next()
}

fn names(list: &[&str]) -> HashSet<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn run_populates_rows_copies_report_and_replaces_results_on_rerun() {
    hosted(
        MODULE,
        "run_populates_rows_copies_report_and_replaces_results_on_rerun",
        |mut host| {
            open_diagnostics(&mut host);

            let tree = host.tree();
            let copy = find_by_key(&tree, "settings-diagnostics-copy").expect("copy button");
            let run = find_by_key(&tree, "settings-diagnostics-run").expect("run button");
            assert!(
                has_state(copy, "disabled"),
                "Copy report is disabled before a run"
            );
            assert!(!has_state(run, "disabled"), "Run is enabled initially");
            assert!(
                find_by_key(&tree, "settings-diagnostics-row-sandbox").is_none(),
                "no rows before a run"
            );

            let first_run = Cell::new(true);
            let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
            let release_rx = Arc::new(Mutex::new(Some(release_rx)));
            host.app_mut().settings_dialog.diagnostics.spawner =
                Some(Rc::new(move |sender: Sender<Check>| {
                    if !first_run.replace(false) {
                        return;
                    }
                    let rx = release_rx.lock().expect("lock gate").take();
                    let sandbox = Check::problem(CheckId::Sandbox, Finding::BubblewrapMissing)
                        .fixed_by(packages::BUBBLEWRAP)
                        .with_detail("bwrap: not found");
                    let gvfs = Check::passed(CheckId::Gvfs, Finding::GvfsRunning)
                        .fixed_by(packages::BUBBLEWRAP);
                    std::thread::spawn(move || {
                        if let Some(rx) = rx {
                            let _ = rx.recv();
                        }
                        for check in [sandbox, gvfs] {
                            let _ = sender.send(check);
                        }
                    });
                }));

            click(&mut host, "settings-diagnostics-run");
            assert!(
                host.app().settings_dialog.diagnostics.running,
                "a run starts asynchronously"
            );
            let tree = host.tree();
            let run = find_by_key(&tree, "settings-diagnostics-run").expect("run button");
            assert!(has_state(run, "disabled"), "Run is disabled while running");
            let summary = find_by_key(&tree, "settings-diagnostics-summary").expect("summary");
            assert_eq!(view::text(summary, "name"), tr("Running checks…"));

            release_tx.send(()).expect("release worker");
            assert!(
                host.run_until(PATIENCE, |h| !h.app().settings_dialog.diagnostics.running),
                "diagnostics finished"
            );
            idle(&mut host);

            let total: usize = GROUPS.iter().map(|(_, ids)| ids.len()).sum();
            let diag = &host.app().settings_dialog.diagnostics;
            assert_eq!(diag.results.len(), total);
            assert!(diag.can_copy());
            assert!(
                diag.environment
                    .iter()
                    .any(|(_, value)| value == &displayed_version()),
                "system info includes the application version"
            );

            let sandbox = diag.results.get(&CheckId::Sandbox).expect("sandbox result");
            assert_eq!(sandbox.status, Status::Problem);
            assert_eq!(sandbox.finding, Finding::BubblewrapMissing);
            assert_eq!(sandbox.detail.as_deref(), Some("bwrap: not found"));
            assert!(!sandbox.packages.is_empty());

            let gvfs = diag.results.get(&CheckId::Gvfs).expect("gvfs result");
            assert_eq!(gvfs.status, Status::Passed);
            assert_eq!(format::message(gvfs), tr("GVfs is running."));

            let workers = diag.results.get(&CheckId::Workers).expect("workers result");
            assert_eq!(workers.finding, Finding::NotRun);
            assert_eq!(format::message(workers), tr("This check couldn't run."));

            let tree = host.tree();
            let copy = find_by_key(&tree, "settings-diagnostics-copy").expect("copy button");
            assert!(!has_state(copy, "disabled"), "Copy report is enabled");
            assert_eq!(view::text(copy, "name"), tr("Copy report"));

            click(&mut host, "settings-diagnostics-copy");
            let report = clipboard::fallback_text(host.app())
                .expect("report copied to clipboard")
                .to_owned();
            for expected in [
                "Strata diagnostics",
                &format!("Version: {}", displayed_version()),
                "PREVIEW SANDBOX",
                "[Problem] Preview sandbox: Bubblewrap (bwrap) isn't installed",
                "    bwrap: not found",
                "[Passed] GVfs: GVfs is running.",
                "[Warning] Sandboxed workers: This check couldn't run.",
            ] {
                assert!(report.contains(expected), "{expected:?} in\n{report}");
            }
            let tree = host.tree();
            let copy = find_by_key(&tree, "settings-diagnostics-copy").expect("copy button");
            assert_eq!(view::text(copy, "name"), tr("Copied"));

            host.runtime().frame(2.0);
            idle(&mut host);
            let tree = host.tree();
            let copy = find_by_key(&tree, "settings-diagnostics-copy").expect("copy button");
            assert_eq!(view::text(copy, "name"), tr("Copy report"));

            // Rerunning replaces previous results cleanly.
            click(&mut host, "settings-diagnostics-run");
            assert!(
                host.run_until(PATIENCE, |h| !h.app().settings_dialog.diagnostics.running),
                "second run finished"
            );
            let diag = &host.app().settings_dialog.diagnostics;
            assert_eq!(diag.results.len(), total);
            assert_eq!(
                diag.results
                    .get(&CheckId::Sandbox)
                    .expect("sandbox")
                    .finding,
                Finding::NotRun
            );
        },
    );
}

#[test]
fn closing_settings_cancels_in_flight_run_and_discards_stale_results() {
    hosted(
        MODULE,
        "closing_settings_cancels_in_flight_run_and_discards_stale_results",
        |mut host| {
            open_diagnostics(&mut host);

            let held_sender: Arc<Mutex<Option<Sender<Check>>>> = Arc::new(Mutex::new(None));
            let capture = Arc::clone(&held_sender);
            host.app_mut().settings_dialog.diagnostics.spawner =
                Some(Rc::new(move |sender: Sender<Check>| {
                    *capture.lock().expect("lock sender") = Some(sender);
                }));

            click(&mut host, "settings-diagnostics-run");
            assert!(host.app().settings_dialog.diagnostics.running);
            let old_gen = host.app().settings_dialog.diagnostics.generation;

            // Close Settings while the run is still pending.
            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().settings_dialog.is_open());
            let diag = &host.app().settings_dialog.diagnostics;
            assert!(!diag.running);
            assert!(diag.generation > old_gen);
            assert!(!diag.has_run);
            assert!(!diag.can_copy());

            // Late result arriving from the stale worker thread is discarded.
            if let Some(sender) = held_sender.lock().expect("lock sender").take() {
                let _ = sender.send(Check::passed(CheckId::Sandbox, Finding::SandboxWorks));
                drop(sender);
            }
            host.wait(Duration::from_millis(120));
            idle(&mut host);

            let diag = &host.app().settings_dialog.diagnostics;
            assert!(diag.results.is_empty());
            assert!(!diag.has_run);
            assert!(!diag.can_copy());
        },
    );
}

#[test]
fn settings_search_for_troubleshooting_reveals_diagnostics_card() {
    hosted(
        MODULE,
        "settings_search_for_troubleshooting_reveals_diagnostics_card",
        |mut host| {
            click(&mut host, "header-settings");
            click(&mut host, "settings-search-field");
            for ch in "troubleshooting".chars() {
                press(&mut host, Key::Text(ch.to_string()), Mods::default());
            }
            host.runtime().frame(0.2);
            idle(&mut host);

            let tree = host.tree();
            assert!(
                find_by_key(&tree, "settings-diagnostics-run").is_some(),
                "searching troubleshooting opens and displays the Diagnostics card"
            );
        },
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
        strata_preview::sandbox_helper::run(&[
            operation.to_owned(),
            input.display().to_string(),
            result.display().to_string(),
            "256".to_owned(),
            "software".to_owned(),
        ])
        .unwrap_or_else(|error| panic!("{operation}: {error}"));
        let png = std::fs::read(&result).expect("rendered thumbnail");
        assert!(sandbox::png_dimensions(&png).is_some(), "{operation}");
    }
}

#[test]
fn install_hints_and_check_evaluators_match_expected_outcomes() {
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
        install_hint(packages::BUBBLEWRAP, None),
        Some(InstallHint::Package("bubblewrap"))
    );
    assert_eq!(install_hint(&[], Some(Pacman)), None);
    assert_eq!(
        install_hint(packages::GVFS_AFC, Some(Zypper)),
        Some(InstallHint::Command(
            "sudo zypper install gvfs-backend-afc".to_owned()
        ))
    );

    let existing = std::env::current_exe().expect("test executable");
    assert_eq!(
        checks::preview_sandbox(false, &existing, || Ok(())).finding,
        Finding::BubblewrapMissing
    );
    assert_eq!(
        checks::preview_sandbox(true, Path::new("/nonexistent/prlimit"), || Ok(())).finding,
        Finding::PrlimitMissing {
            path: "/nonexistent/prlimit".to_owned(),
        }
    );
    assert_eq!(
        checks::workers(|| WorkerSupport::LandlockUnavailable, || Ok(())).finding,
        Finding::LandlockUnavailable
    );
    assert_eq!(
        checks::video_thumbnails(true, false, || Ok(())).finding,
        Finding::SlowVideoThumbnails
    );
    assert_eq!(
        checks::media_previews(false, false, || Ok(())).finding,
        Finding::Missing(vec!["ffmpeg".into(), "ffprobe".into()])
    );

    let running = VfsState::Running(names(&["mtp"]));
    let kde = names(&["org.kde.kmtpd5"]);
    assert_eq!(
        checks::mtp(&running, Some(&kde)).finding,
        Finding::KdeMtpConflict
    );
    assert_eq!(
        checks::folder_monitoring_for(Ok(()), Some(1024)).status,
        Status::Warning
    );
    assert_eq!(
        checks::audio_playback(strata_platform::media::BackendKind::PulseAudio).finding,
        Finding::AudioOutput {
            sink: "PulseAudio".into(),
        }
    );
    assert_eq!(
        checks::audio_playback(strata_platform::media::BackendKind::Alsa).finding,
        Finding::AudioOutput {
            sink: "ALSA".into(),
        }
    );
    assert_eq!(
        checks::audio_playback(strata_platform::media::BackendKind::Silent).status,
        Status::Warning
    );
    assert_eq!(
        checks::custom_actions(0, Vec::new(), Vec::new()).finding,
        Finding::NoActions
    );
}

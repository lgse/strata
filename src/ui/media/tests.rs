// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    sandbox::{MediaPreviewBackend, media::tests::stream},
    test_support::gtk_test,
};
use std::rc::Rc;

pub(crate) fn player(audio: bool, duration_us: u64) -> DecodedMedia {
    let source = SandboxedMedia {
        path: "/synthetic-media".into(),
        size: MediaPreviewSize::new(160, 90),
        backend: MediaPreviewBackend::Software,
    };
    let player = DecodedMedia::new(source);
    use_test_decoder(&player, audio, duration_us);
    player
}

pub(crate) fn use_test_decoder(player: &DecodedMedia, audio: bool, duration_us: u64) {
    player
        .imp()
        .loader
        .replace(Some(Rc::new(move |source, start_tick| {
            stream(Header {
                width: source.size.width as u32,
                height: source.size.height as u32,
                audio,
                duration_us,
                start_tick,
            })
        })));
}

pub(crate) fn wait(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "playback deadline");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn process_identity(pid: u32) -> Option<(u32, String)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    Some((
        pid,
        stat.rsplit_once(") ")?
            .1
            .split_whitespace()
            .nth(19)?
            .to_owned(),
    ))
}

fn descendants(root: u32) -> std::collections::HashSet<(u32, String)> {
    let mut pending = vec![root];
    let mut processes = std::collections::HashSet::new();
    while let Some(pid) = pending.pop() {
        let Some(identity) = process_identity(pid) else {
            continue;
        };
        if pid != root && !processes.insert(identity) {
            continue;
        }
        let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else {
            continue;
        };
        for task in tasks.flatten() {
            if let Ok(children) = std::fs::read_to_string(task.path().join("children")) {
                pending.extend(
                    children
                        .split_whitespace()
                        .filter_map(|pid| pid.parse::<u32>().ok()),
                );
            }
        }
    }
    processes
}

#[test]
fn native_sandbox_workers_play_seek_and_release_video_audio_av_and_gif() {
    gtk_test(
        "ui::media::tests::native_sandbox_workers_play_seek_and_release_video_audio_av_and_gif",
        || {
            let directory = tempfile::tempdir().expect("generated media only");
            for (name, video, audio) in [
                ("video.mkv", true, false),
                ("audio.wav", false, true),
                ("av.mkv", true, true),
                ("loop.gif", true, false),
            ] {
                let input = directory.path().join(name);
                let mut command = std::process::Command::new("/usr/bin/ffmpeg");
                command.args(["-nostdin", "-v", "error"]);
                if video {
                    command.args([
                        "-f",
                        "lavfi",
                        "-i",
                        "testsrc2=size=32x24:rate=30:duration=2",
                    ]);
                }
                if audio {
                    command.args([
                        "-f",
                        "lavfi",
                        "-i",
                        "sine=frequency=660:sample_rate=48000:duration=2",
                    ]);
                }
                if name.ends_with("mkv") {
                    command.args(["-c:v", "ffv1"]);
                }
                if audio {
                    command.args(["-c:a", "pcm_s16le"]);
                }
                assert!(
                    command
                        .args(["-threads", "1"])
                        .arg(&input)
                        .status()
                        .expect("fixture encoder")
                        .success()
                );
                let player = DecodedMedia::new(SandboxedMedia {
                    path: input,
                    size: MediaPreviewSize::new(160, 90),
                    backend: MediaPreviewBackend::Software,
                });
                player.set_muted(true);
                player.play();
                wait(|| {
                    assert!(player.error().is_none(), "{name}: {:?}", player.error());
                    player.timestamp() > 100_000
                });
                let owned = descendants(std::process::id());
                assert!(!owned.is_empty(), "real sandbox descendants must run");
                assert_eq!(player.has_video(), video, "{name}");
                assert_eq!(player.has_audio(), audio, "{name}");
                assert_eq!(player.imp().texture.borrow().is_some(), video, "{name}");
                player.pause();
                player.seek(1_500_000);
                wait(|| {
                    assert!(player.error().is_none(), "{name}: {:?}", player.error());
                    !player.is_seeking()
                });
                assert!(!player.is_playing());
                assert!((player.timestamp() - 1_500_000).abs() < 34_000);
                player.play();
                wait(|| {
                    assert!(player.error().is_none(), "{name}: {:?}", player.error());
                    player.timestamp() > 1_550_000
                });
                if name.ends_with("gif") {
                    wait(|| {
                        assert!(player.error().is_none(), "{name}: {:?}", player.error());
                        player.timestamp() > 2_150_000
                            && player
                                .imp()
                                .frames
                                .borrow()
                                .front()
                                .is_some_and(|frame| media::timestamp(frame.tick) > 2_100_000)
                    });
                    assert!(!player.is_ended());
                    player.seek(29_700_000);
                    wait(|| {
                        assert!(player.error().is_none(), "{name}: {:?}", player.error());
                        !player.is_seeking()
                    });
                }
                wait(|| {
                    assert!(player.error().is_none(), "{name}: {:?}", player.error());
                    player.is_ended()
                });
                player.close();
                wait(|| crate::sandbox::media::tests::active_sessions() == 0);
                wait(|| {
                    owned.iter().all(|(pid, birth)| {
                        process_identity(*pid).as_ref() != Some(&(*pid, birth.clone()))
                    })
                });
            }
            let make_player = || {
                DecodedMedia::new(SandboxedMedia {
                    path: directory.path().join("loop.gif"),
                    size: MediaPreviewSize::new(160, 90),
                    backend: MediaPreviewBackend::Software,
                })
            };
            let first = make_player();
            first.play();
            wait(|| {
                assert!(first.error().is_none(), "{:?}", first.error());
                first.timestamp() > 100_000
            });
            let first_workers = descendants(std::process::id());
            let second = make_player();
            second.play();
            wait(|| {
                assert!(second.error().is_none(), "{:?}", second.error());
                second.timestamp() > 100_000
            });
            let worker = first_workers
                .iter()
                .find(|(pid, _)| {
                    std::fs::read_to_string(format!("/proc/{pid}/comm"))
                        .is_ok_and(|name| name.starts_with("strata-media-"))
                })
                .expect("first parser worker");
            assert_eq!(process_identity(worker.0).as_ref(), Some(worker));
            rustix::process::kill_process(
                rustix::process::Pid::from_raw(worker.0 as i32).expect("worker PID"),
                rustix::process::Signal::KILL,
            )
            .expect("simulate one worker crash");
            wait(|| first.error().is_some());
            let position = second.timestamp();
            wait(|| {
                assert!(
                    second.error().is_none(),
                    "unrelated job survived: {:?}",
                    second.error()
                );
                second.timestamp() > position + 100_000
            });
            first.close();
            second.close();
            wait(|| crate::sandbox::media::tests::active_sessions() == 0);
            wait(|| descendants(std::process::id()).is_empty());
        },
    );
}

#[test]
fn unexpected_application_death_kills_and_reaps_sandbox_descendants() {
    const NAME: &str =
        "ui::media::tests::unexpected_application_death_kills_and_reaps_sandbox_descendants";
    gtk_test(NAME, || {
        if let Some(ready) = std::env::var_os("STRATA_PARENT_DEATH_PROBE") {
            let ready = std::path::PathBuf::from(ready);
            let player = DecodedMedia::new(SandboxedMedia {
                path: ready.with_file_name("loop.gif"),
                size: MediaPreviewSize::new(160, 90),
                backend: MediaPreviewBackend::Software,
            });
            player.play();
            wait(|| {
                assert!(player.error().is_none(), "{:?}", player.error());
                player.timestamp() > 100_000
            });
            let processes = descendants(std::process::id());
            assert!(!processes.is_empty());
            std::fs::write(
                ready.with_extension("staged"),
                serde_json::to_vec(&processes).expect("worker identities"),
            )
            .expect("stage readiness");
            std::fs::rename(ready.with_extension("staged"), &ready).expect("publish readiness");
            loop {
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        // The disposable test process owns/reaps adopted descendants rather than
        // relying on an arbitrary container PID 1 to perform init's duties.
        rustix::process::set_child_subreaper(rustix::process::Pid::from_raw(1))
            .expect("private subreaper");
        let directory = crate::media_helper::private_tempdir().expect("private fixture");
        let input = directory.path().join("loop.gif");
        assert!(
            std::process::Command::new("/usr/bin/ffmpeg")
                .args([
                    "-nostdin",
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc2=size=32x24:rate=30:duration=2",
                    "-threads",
                    "1"
                ])
                .arg(&input)
                .status()
                .expect("fixture")
                .success()
        );
        struct Probe(std::process::Child);
        impl Drop for Probe {
            fn drop(&mut self) {
                let owned = descendants(std::process::id());
                let _ = self.0.kill();
                let _ = self.0.wait();
                for identity in owned {
                    if process_identity(identity.0).as_ref() == Some(&identity) {
                        let _ = rustix::process::kill_process(
                            rustix::process::Pid::from_raw(identity.0 as i32).expect("owned PID"),
                            rustix::process::Signal::KILL,
                        );
                    }
                }
                while let Ok(Some(_)) = rustix::process::wait(rustix::process::WaitOptions::NOHANG)
                {
                }
            }
        }
        let ready = directory.path().join("ready.json");
        let mut probe = Probe(
            std::process::Command::new(std::env::current_exe().expect("test executable"))
                .args(["--exact", NAME, "--nocapture"])
                .env("STRATA_PARENT_DEATH_PROBE", &ready)
                .spawn()
                .expect("application probe"),
        );
        wait(|| ready.is_file());
        let owned: std::collections::HashSet<(u32, String)> =
            serde_json::from_slice(&std::fs::read(&ready).expect("read readiness"))
                .expect("worker identities");
        assert!(!owned.is_empty());
        probe
            .0
            .kill()
            .expect("unexpected application death, not a group kill");
        assert!(!probe.0.wait().expect("reap application").success());
        wait(|| {
            while let Ok(Some(_)) = rustix::process::wait(rustix::process::WaitOptions::NOHANG) {}
            owned
                .iter()
                .all(|identity| process_identity(identity.0).as_ref() != Some(identity))
        });
        assert!(descendants(std::process::id()).is_empty());
    });
}

#[test]
fn raw_texture_play_pause_seek_and_audio_clock_stay_synchronized() {
    gtk_test(
        "ui::media::tests::raw_texture_play_pause_seek_and_audio_clock_stay_synchronized",
        || {
            for audio in [false, true] {
                let player = player(audio, 3_000_000);
                player.play();
                wait(|| {
                    assert!(player.error().is_none(), "{:?}", player.error());
                    player.timestamp() > 100_000
                });
                assert!(player.has_video());
                assert_eq!(player.has_audio(), audio);
                player.pause();
                let paused = player.timestamp();
                std::thread::sleep(Duration::from_millis(50));
                player.tick().expect("paused tick");
                assert_eq!(player.timestamp(), paused);
                player.seek(1_250_000);
                wait(|| {
                    assert!(player.error().is_none(), "{:?}", player.error());
                    !player.is_seeking()
                });
                assert!(!player.is_playing());
                assert!((player.timestamp() - 1_250_000).abs() < 34_000);
                let header = player.imp().header.get().expect("header");
                assert_eq!(header.start_tick, 37);
                assert!(player.imp().frames.borrow().len() <= PRESENTATION_QUEUE);
                player.play();
                wait(|| player.timestamp() > 1_350_000);
                if audio {
                    let audio_position = player
                        .imp()
                        .audio
                        .borrow()
                        .as_ref()
                        .expect("PCM output")
                        .position_us()
                        .expect("audio clock");
                    assert!(
                        (player.timestamp()
                            - (media::timestamp(header.start_tick) + audio_position) as i64)
                            .abs()
                            < 50_000
                    );
                }
                player.close();
            }
        },
    );
}

#[test]
fn paused_idle_releases_worker_and_resumes_at_retained_frame_position() {
    gtk_test(
        "ui::media::tests::paused_idle_releases_worker_and_resumes_at_retained_frame_position",
        || {
            let player = player(true, media::LIMIT_US);
            player.play();
            wait(|| player.timestamp() > 150_000);
            player.pause();
            let position = player.imp().position.get();
            let texture = player
                .imp()
                .texture
                .borrow()
                .as_ref()
                .expect("texture")
                .clone();
            player.imp().paused.set(Some(Instant::now() - PAUSED_IDLE));
            player.tick().expect("idle cleanup");
            assert!(player.imp().dormant.get());
            assert!(player.imp().session.borrow().is_none());
            assert!(player.imp().audio.borrow().is_none());
            assert_eq!(player.imp().texture.borrow().as_ref(), Some(&texture));
            assert_eq!(player.imp().position.get(), position);
            wait(|| player.imp().timer.borrow().is_none());
            player.resize(MediaPreviewSize::for_viewport(100, 80, 2));
            player.play();
            wait(|| player.timestamp() as u64 > position + 100_000);
            assert!(player.error().is_none());
            assert_eq!(player.imp().header.get().expect("resumed").width, 200);
            player.pause();
            player.imp().paused.set(Some(Instant::now() - PAUSED_IDLE));
            player.tick().expect("second idle cleanup");
            wait(|| player.imp().timer.borrow().is_none());
            player.seek(2_000_000);
            wait(|| !player.is_seeking());
            assert!(!player.is_playing());
            assert!(!player.imp().dormant.get());
            assert_eq!(player.timestamp(), 2_000_000);
            player.play();
            wait(|| player.timestamp() as u64 > 2_100_000);
            assert!(player.error().is_none());
            player.close();
        },
    );
}

#[test]
fn resize_is_debounced_and_seek_preserves_pause_and_live_audio_preferences() {
    gtk_test(
        "ui::media::tests::resize_is_debounced_and_seek_preserves_pause_and_live_audio_preferences",
        || {
            let player = player(true, 5_000_000);
            player.set_volume(0.35);
            player.set_muted(true);
            player.play();
            wait(|| player.timestamp() > 100_000);
            player.pause();
            let position = player.timestamp();
            player.resize(MediaPreviewSize::new(320, 180));
            player.resize(MediaPreviewSize::new(480, 270));
            assert_eq!(player.imp().header.get().expect("old header").width, 160);
            player
                .imp()
                .resized
                .set(Some(Instant::now() - RESIZE_DELAY));
            player.tick().expect("resize");
            wait(|| {
                player
                    .imp()
                    .header
                    .get()
                    .is_some_and(|header| header.width == 480)
                    && player.imp().first_frame.get()
            });
            assert!(!player.is_playing());
            assert!((player.timestamp() - position).abs() < 34_000);
            assert!(player.is_muted());
            assert_eq!(player.volume(), 0.35);
            player.set_muted(false);
            player.set_volume(0.8);
            player.seek(2_000_000);
            wait(|| !player.is_seeking());
            assert_eq!(player.volume(), 0.8);
            assert!(!player.is_muted());
            player.close();
        },
    );
}

#[test]
fn unused_pane_space_does_not_restart_an_already_fitted_decode() {
    gtk_test(
        "ui::media::tests::unused_pane_space_does_not_restart_an_already_fitted_decode",
        || {
            let player = player(false, 5_000_000);
            player.play();
            wait(|| player.timestamp() > 0);
            player.pause();
            let loaded = player.imp().loaded_size.get();
            player.resize(MediaPreviewSize::new(160, 180));
            player
                .imp()
                .resized
                .set(Some(Instant::now() - RESIZE_DELAY));
            player.tick().expect("resize");
            assert_eq!(player.imp().loaded_size.get(), loaded);
            assert!(player.imp().first_frame.get());
            assert!(player.imp().restart.get().is_none());
            player.close();
        },
    );
}

#[test]
fn an_early_end_corrects_provisional_duration_and_still_replays() {
    gtk_test(
        "ui::media::tests::an_early_end_corrects_provisional_duration_and_still_replays",
        || {
            for audio in [false, true] {
                let player = player(audio, media::LIMIT_US);
                player
                    .imp()
                    .loader
                    .replace(Some(Rc::new(move |_, start_tick| {
                        crate::sandbox::media::tests::stream_to(
                            Header {
                                width: 16,
                                height: 16,
                                audio,
                                duration_us: media::LIMIT_US,
                                start_tick,
                            },
                            200_000,
                        )
                    })));
                player.play();
                wait(|| {
                    assert!(player.error().is_none());
                    player.is_ended()
                });
                assert_eq!(player.duration(), 200_000);
                assert_eq!(player.timestamp(), 200_000);
                player.play();
                wait(|| player.timestamp() > 0 && player.timestamp() < 200_000);
                assert!(player.error().is_none());
                player.close();
            }
        },
    );
}

#[test]
fn gif_loop_and_end_of_preview_never_extend_the_content_interval() {
    gtk_test(
        "ui::media::tests::gif_loop_and_end_of_preview_never_extend_the_content_interval",
        || {
            let player = player(false, 200_000);
            player.set_loop(true);
            player.play();
            wait(|| player.timestamp() >= 100_000);
            wait(|| player.timestamp() < 100_000);
            assert!(player.is_playing());
            assert!(!player.is_ended());
            player.set_loop(false);
            wait(|| player.is_ended());
            assert!(!player.is_playing());
            assert_eq!(player.timestamp(), 200_000);
            player.play();
            wait(|| player.timestamp() < 200_000 && player.timestamp() > 0);
            player.close();
        },
    );
}

#[test]
fn audio_ends_cleanly_between_sample_boundaries_including_after_a_seek() {
    gtk_test(
        "ui::media::tests::audio_ends_cleanly_between_sample_boundaries_including_after_a_seek",
        || {
            for duration in [200_001, 233_334, 333_334, 999_999] {
                let player = player(true, duration);
                player.play();
                wait(|| player.is_prepared());
                player.seek(66_667);
                wait(|| {
                    assert!(player.error().is_none(), "{:?}", player.error());
                    player.is_ended()
                });
                assert_eq!(player.timestamp() as u64, duration);
                assert!(!player.is_playing());
                assert!(player.imp().session.borrow().is_none());
                assert!(player.imp().audio.borrow().is_none());
                player.close();
            }
        },
    );
}

#[test]
fn full_length_audio_preview_reaches_end_and_releases_resources() {
    gtk_test(
        "ui::media::tests::full_length_audio_preview_reaches_end_and_releases_resources",
        || {
            let player = player(true, media::LIMIT_US);
            player.play();
            let deadline = Instant::now() + Duration::from_secs(40);
            while !player.is_ended() {
                assert!(player.error().is_none(), "{:?}", player.error());
                assert!(Instant::now() < deadline, "full preview did not end");
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(player.timestamp() as u64, media::LIMIT_US);
            assert!(!player.is_playing());
            assert!(player.imp().session.borrow().is_none());
            assert!(player.imp().audio.borrow().is_none());
            player.close();
        },
    );
}

#[test]
fn additional_windows_report_busy_without_interrupting_four_existing_players() {
    gtk_test(
        "ui::media::tests::additional_windows_report_busy_without_interrupting_four_existing_players",
        || {
            let players: Vec<_> = (0..4)
                .map(|_| {
                    let player = player(false, media::LIMIT_US);
                    player.play();
                    player
                })
                .collect();
            wait(|| players.iter().all(|player| player.timestamp() > 0));
            let fifth = player(false, media::LIMIT_US);
            fifth.play();
            wait(|| fifth.error().is_some());
            assert!(fifth.error().expect("busy").message().contains("busy"));
            assert!(
                players
                    .iter()
                    .all(|player| player.is_playing() && player.error().is_none())
            );
            players[0].pause();
            players[0]
                .imp()
                .paused
                .set(Some(Instant::now() - PAUSED_IDLE));
            players[0].tick().expect("idle release");
            let replacement = player(false, media::LIMIT_US);
            replacement.play();
            wait(|| replacement.timestamp() > 0);
            assert!(players[1..].iter().all(|player| player.is_playing()));
            replacement.close();
            fifth.close();
            for player in players {
                player.close();
            }
        },
    );
}

#[test]
fn startup_and_seek_timeout_fail_closed_without_leaving_queued_buffers() {
    gtk_test(
        "ui::media::tests::startup_and_seek_timeout_fail_closed_without_leaving_queued_buffers",
        || {
            let player = player(false, media::LIMIT_US);
            player.imp().starting.set(Some(
                Instant::now() - media::STARTUP_TIMEOUT - Duration::from_millis(1),
            ));
            let error = player.tick().expect_err("startup deadline");
            player.fail(&error);
            assert!(player.error().is_some());
            player.close();
            let player = self::player(false, media::LIMIT_US);
            player.play();
            wait(|| player.timestamp() > 0);
            player.seek(5_000_000);
            player.imp().starting.set(Some(
                Instant::now() - media::STARTUP_TIMEOUT - Duration::from_millis(1),
            ));
            let error = player.tick().expect_err("seek deadline");
            player.fail(&error);
            assert!(!player.is_seeking());
            assert!(!player.is_playing());
            assert!(player.imp().frames.borrow().is_empty());
            player.close();
        },
    );
}

#[test]
fn repeated_teardown_releases_textures_and_does_not_accumulate_fds_or_threads() {
    gtk_test(
        "ui::media::tests::repeated_teardown_releases_textures_and_does_not_accumulate_fds_or_threads",
        || {
            let count = |path| {
                std::fs::read_dir(path)
                    .expect("Linux process resources")
                    .count()
            };
            let mut baseline = None;
            for iteration in 0..25 {
                let player = player(true, 2_000_000);
                let picture = gtk::Picture::for_paintable(&player);
                let window = gtk::Window::builder()
                    .default_width(320)
                    .default_height(180)
                    .child(&picture)
                    .build();
                window.present();
                player.play();
                wait(|| player.timestamp() > 0);
                let texture = player
                    .imp()
                    .texture
                    .borrow()
                    .as_ref()
                    .expect("texture")
                    .downgrade();
                let weak = player.downgrade();
                player.close();
                window.close();
                drop(window);
                drop(picture);
                drop(player);
                wait(|| weak.upgrade().is_none() && texture.upgrade().is_none());
                #[cfg(debug_assertions)]
                wait(|| super::diagnostics::live_bytes() == 0);
                std::thread::sleep(Duration::from_millis(30));
                let resources = (count("/proc/self/fd"), count("/proc/self/task"));
                if iteration == 4 {
                    baseline = Some(resources);
                }
                if iteration == 24 {
                    let baseline = baseline.expect("warm baseline");
                    assert!(
                        resources.0 <= baseline.0 + 2,
                        "FDs: {baseline:?} -> {resources:?}"
                    );
                    assert!(
                        resources.1 <= baseline.1 + 2,
                        "threads: {baseline:?} -> {resources:?}"
                    );
                }
            }
        },
    );
}

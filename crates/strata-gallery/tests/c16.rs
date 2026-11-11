//! C16's page in process: the dock's and the dashboard's timers stepped from wake to wake (no
//! frames between, animations on and off), the live fill and labels moving without commits,
//! and how many commits twenty running cards cost. The scripts in `tests/scripts/c16/` check
//! the rest. Each test names the bullets it covers.

mod c16 {
    use std::time::Instant;

    use rune::{ElementInfo, Env, Key, KeyEvent, Mods, Runtime};
    use strata_gallery::{Gallery, scenes};

    const FRAME: f64 = 1.0 / 60.0;

    fn mount_with(name: &str, change: impl FnOnce(&mut Gallery)) -> Runtime<Gallery> {
        let scene = scenes::all_scenes()
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, f)| f)
            .unwrap_or_else(|| panic!("no scene {name}"));
        let mut g = Gallery::default();
        scene(&mut g);
        change(&mut g);
        let env = Env::new(g.dark, 1.0);
        let mut rt = strata_gallery::runtime(g, env);
        rt.frame(0.0);
        run(&mut rt, 0.2);
        rt
    }

    fn mount(name: &str) -> Runtime<Gallery> {
        mount_with(name, |_| {})
    }

    fn still(g: &mut Gallery) {
        g.look.animations = false;
        g.c16.dock.set_animations(false);
    }

    fn run<S>(rt: &mut Runtime<S>, seconds: f64) {
        for _ in 0..(seconds / FRAME).round() as usize {
            rt.frame(FRAME);
        }
    }

    fn element<S>(rt: &Runtime<S>, key: &str) -> Option<ElementInfo> {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
    }

    fn rect<S>(rt: &Runtime<S>, key: &str) -> [f32; 4] {
        element(rt, key)
            .unwrap_or_else(|| panic!("no element {key}"))
            .rect
    }

    fn centre<S>(rt: &Runtime<S>, key: &str) -> (f32, f32) {
        let r = rect(rt, key);
        (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
    }

    fn tap(rt: &mut Runtime<Gallery>, key: &str) {
        let (x, y) = centre(rt, key);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
        // Off everything the page reacts to, so no hover pauses a countdown by accident.
        rt.pointer_move(5.0, 300.0);
        rt.frame(FRAME);
    }

    /// Steps a still window for at most `limit` seconds: a frame only at each wake the page
    /// asked for, or for queued input, none in between and none for motion. Stops at the first
    /// frame after which `done` holds, and returns the page clock it stopped at.
    fn wake_until(rt: &mut Runtime<Gallery>, limit: f64, done: impl Fn(&Gallery) -> bool) -> f64 {
        let start = rt.state.c16.now;
        for _ in 0..2000 {
            let now = rt.state.c16.now;
            if now - start > limit || done(&rt.state) {
                break;
            }
            if rt.has_pending_input() {
                rt.frame(FRAME);
                continue;
            }
            let Some(deadline) = rt.wake_deadline() else {
                break;
            };
            if deadline - start > limit {
                break;
            }
            rt.frame((deadline - now).max(0.001));
        }
        rt.state.c16.now
    }

    fn card(g: &Gallery, id: u64) -> Option<String> {
        g.c16.dock.op(id).map(|o| o.title().to_owned())
    }

    /// Runs a fresh copy (op 1) to its end with ordinary frames: the worker reports every
    /// frame, as a real one does.
    fn completed_copy(change: impl FnOnce(&mut Gallery)) -> Runtime<Gallery> {
        let mut rt = mount_with("c16-idle-light", change);
        tap(&mut rt, "c16-start-copy");
        for _ in 0..600 {
            if rt.state.c16.dock.op(1).and_then(|o| o.ended()).is_some() {
                break;
            }
            rt.frame(FRAME);
        }
        assert_eq!(card(&rt.state, 1).as_deref(), Some("Copy complete"));
        rt
    }

    // operations/progress/dock: "A completed card counts down 5 seconds with a shrinking bar
    // and 'closing in' text, then fades out and closes."
    #[test]
    fn a_completed_card_closes_after_five_seconds_in_a_resting_window() {
        for animations in [true, false] {
            let mut rt = completed_copy(|g| {
                if !animations {
                    still(g)
                }
            });
            let start = rt.state.c16.now;
            let closing = |g: &Gallery| {
                g.c16
                    .dock
                    .op(1)
                    .and_then(|o| o.shown.ended.clone())
                    .map(|e| e.words)
            };
            wake_until(&mut rt, 1.05, |_| false);
            assert_eq!(closing(&rt.state).as_deref(), Some("closing in 4s"));
            let end = wake_until(&mut rt, 10.0, |g| g.c16.dock.op(1).is_none());
            assert!(rt.state.c16.dock.op(1).is_none(), "animations {animations}");
            let fade = if animations { 0.18 } else { 0.0 };
            let took = end - start;
            assert!(
                (took - 5.0 - fade).abs() < 0.05,
                "closed after {took:.3} s (animations {animations})"
            );
            assert!(element(&rt, "c16-dock").is_none(), "the empty dock closes");
        }
    }

    // operations/progress/dock: "Hovering or focusing a completed card pauses its countdown
    // and shows 'paused'; leaving resumes the remaining time." (the hover half: Rune can't
    // tell a card that focus came in, see the module docs)
    #[test]
    fn hover_pauses_the_countdown_and_leaving_resumes_the_rest() {
        for animations in [true, false] {
            let mut rt = completed_copy(|g| {
                if !animations {
                    still(g)
                }
            });
            wake_until(&mut rt, 2.0, |_| false);
            let (x, y) = centre(&rt, "c16-dock-card-1");
            rt.pointer_move(x, y);
            rt.frame(FRAME);
            let paused_at = rt.state.c16.now;
            wake_until(&mut rt, 30.0, |_| false);
            assert!(card(&rt.state, 1).is_some(), "hovered, it stays");
            let words = rt
                .state
                .c16
                .dock
                .op(1)
                .unwrap()
                .shown
                .ended
                .clone()
                .unwrap()
                .words;
            assert_eq!(words, "paused");
            assert!(
                rt.wake_deadline().is_none(),
                "a paused card asks for no wakes"
            );
            rt.pointer_move(5.0, 300.0);
            rt.frame(FRAME);
            let resumed = rt.state.c16.now;
            let end = wake_until(&mut rt, 10.0, |g| g.c16.dock.op(1).is_none());
            assert!(card(&rt.state, 1).is_none());
            // It paused with about 3 s left (5 s less the 2 s waited), and keeps them.
            let fade = if animations { 0.18 } else { 0.0 };
            let took = end - resumed;
            assert!(
                (took - 3.0 - fade).abs() < 0.1,
                "closed {took:.3} s after resuming (paused at {paused_at:.3})"
            );
        }
    }

    // operations/progress/dock: "Hovering … a completed card pauses its countdown": a card
    // that finishes under a resting pointer hears no new enter (Rune fires on_hover_change on
    // changes only), so it starts its countdown paused, as Strata's does (review 1, B1).
    #[test]
    fn a_card_that_finishes_under_a_resting_pointer_stays_paused() {
        for animations in [true, false] {
            let mut rt = mount_with("c16-idle-light", |g| {
                if !animations {
                    still(g)
                }
            });
            tap(&mut rt, "c16-start-copy");
            run(&mut rt, 0.3);
            let (x, y) = centre(&rt, "c16-dock-card-1");
            rt.pointer_move(x, y);
            for _ in 0..600 {
                if rt.state.c16.dock.op(1).and_then(|o| o.ended()).is_some() {
                    break;
                }
                rt.frame(FRAME);
            }
            assert_eq!(card(&rt.state, 1).as_deref(), Some("Copy complete"));
            wake_until(&mut rt, 30.0, |g| g.c16.dock.op(1).is_none());
            assert!(
                card(&rt.state, 1).is_some(),
                "animations {animations}: it closed"
            );
            let words = rt
                .state
                .c16
                .dock
                .op(1)
                .unwrap()
                .shown
                .ended
                .clone()
                .unwrap()
                .words;
            assert_eq!(words, "paused");
        }
    }

    // operations/progress/dock: "The pin button, 'Keep notification', keeps a completed card
    // open and shows 'pinned' until it is dismissed."
    #[test]
    fn a_pinned_card_stays_until_dismissed() {
        let mut rt = completed_copy(|_| {});
        tap(&mut rt, "c16-dock-card-1-pin");
        wake_until(&mut rt, 60.0, |_| false);
        assert!(card(&rt.state, 1).is_some());
        tap(&mut rt, "c16-dock-card-1-complete");
        assert!(card(&rt.state, 1).is_none());
        assert_eq!(rt.state.c16.heard, "Dismissed 1");
    }

    // operations/progress/dock: "The pin button, 'Keep notification', keeps a completed card
    // open … until it is dismissed": unpinning, by assistive technology so no hover pauses it,
    // resumes the countdown in a resting window from the pin's own wake (review 2, SF3).
    #[test]
    fn unpinning_resumes_the_countdown_in_a_resting_window() {
        let mut rt = completed_copy(|_| {});
        assert!(rt.accessibility_action(
            "c16-dock-card-1-pin",
            rune::accesskit::Action::Click,
            None
        ));
        rt.frame(FRAME);
        wake_until(&mut rt, 30.0, |_| false);
        assert!(card(&rt.state, 1).is_some(), "pinned, it stays");
        assert!(rt.accessibility_action(
            "c16-dock-card-1-pin",
            rune::accesskit::Action::Click,
            None
        ));
        rt.frame(FRAME);
        let start = rt.state.c16.now;
        let end = wake_until(&mut rt, 10.0, |g| g.c16.dock.op(1).is_none());
        assert!(card(&rt.state, 1).is_none(), "unpinned, it closes");
        assert!(end - start < 5.0 + 0.18 + 0.05, "{:.3} s", end - start);
    }

    // The pointer crossing a card changes nothing the view draws (a running card has no
    // countdown, and a finished card's words follow at the tick): the hover is live-only
    // (review 2, nit).
    #[test]
    fn the_pointer_crossing_a_card_commits_nothing() {
        let mut rt = mount("c16-cancelling-light");
        run(&mut rt, 0.5);
        let (x, y) = centre(&rt, "c16-dock-card-1");
        let cross = |rt: &mut Runtime<Gallery>| {
            let c0 = rune::commit_generation();
            rt.pointer_move(x, y);
            rt.frame(FRAME);
            rt.pointer_move(5.0, 300.0);
            rt.frame(FRAME);
            rune::commit_generation() - c0
        };
        // Debug builds re-run the view after a live-only event to check nothing it draws
        // changed: no warning means the hover was truly live-only.
        cross(&mut rt);
        assert!(
            rt.live_only_warnings().is_empty(),
            "{:?}",
            rt.live_only_warnings()
        );
        rt.check_live_only(false);
        assert_eq!(cross(&mut rt), 0);
    }

    // operations/progress: "An operation on fewer than 16 items shows no progress dialog for
    // its first 350 ms, and none at all if it finishes sooner."
    #[test]
    fn a_small_operation_shows_after_350_ms_from_its_own_wake_and_a_fast_one_never() {
        let mut rt = mount("c16-idle-light");
        tap(&mut rt, "c16-start-small-fast");
        let mut seen = false;
        for _ in 0..90 {
            rt.frame(FRAME);
            seen |= element(&rt, "c16-dock").is_some();
        }
        assert!(!seen, "a 0.2 s copy never shows");
        // The slow one: no worker frames in between, only the reveal's wake.
        tap(&mut rt, "c16-start-small-slow");
        let id = rt.state.c16.next_id;
        rt.state.c16.fakes.retain(|f| f.id != id);
        let start = rt.state.c16.now;
        let end = wake_until(&mut rt, 2.0, |g| !g.c16.dock.is_empty());
        assert!(!rt.state.c16.dock.is_empty());
        assert!(
            (end - start - 0.35).abs() < 0.05,
            "showed {:.3} s after it started",
            end - start
        );
    }

    // operations/progress: "If the transfer has not stopped 8 seconds after Cancel, the title
    // becomes 'Device not responding'…": Cancel from the dialog, whose C15 buttons carry no
    // Tx, still stalls on time.
    #[test]
    fn a_stalled_cancel_in_the_dialog_reads_device_not_responding_after_8_s() {
        for animations in [true, false] {
            let mut rt = mount_with("c16-idle-light", |g| {
                if !animations {
                    still(g)
                }
            });
            tap(&mut rt, "c16-start-move-stuck");
            run(&mut rt, 0.5);
            tap(&mut rt, "c16-progress-primary");
            let start = rt.state.c16.now;
            let end = wake_until(&mut rt, 12.0, |g| {
                card(g, 1).as_deref() == Some("Device not responding")
            });
            assert_eq!(card(&rt.state, 1).as_deref(), Some("Device not responding"));
            assert!(
                (end - start - 8.0).abs() < 0.1,
                "stalled {:.3} s after Cancel",
                end - start
            );
            let dialog = element(&rt, "c16-progress-dialog").unwrap();
            assert_eq!(dialog.label.as_deref(), Some("Device not responding"));
        }
    }

    // The same stall reached by Escape with animations off, where no press motion keeps
    // frames coming: the poll after the cancel's commit stamps it (review 1, should-fix 3,
    // which removed the wake beacon no test needed).
    #[test]
    fn escape_in_a_still_dialog_stalls_from_its_own_wake() {
        let mut rt = mount_with("c16-idle-light", still);
        tap(&mut rt, "c16-start-move-stuck");
        run(&mut rt, 1.0);
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        let start = rt.state.c16.now;
        let end = wake_until(&mut rt, 12.0, |g| {
            card(g, 1).as_deref() == Some("Device not responding")
        });
        assert_eq!(card(&rt.state, 1).as_deref(), Some("Device not responding"));
        assert!(
            (end - start - 8.0).abs() < 0.1,
            "stalled {:.3} s after Escape",
            end - start
        );
    }

    /// A blocking move whose subtitle already reads the cancel's words and whose worker has
    /// gone quiet, in a resting window: a cancel then moves no layout and starts no motion of
    /// its own (review 2, SF1's repro).
    fn quiet_dialog_move(change: impl FnOnce(&mut Gallery)) -> Runtime<Gallery> {
        use strata_components::progress_dock::{OpSpec, Placement, Progress, Sample, Transfer};
        use strata_kit::icons::IconName;
        let mut rt = mount_with("c16-idle-light", change);
        rt.update(|g, tx| {
            let mut spec = OpSpec::new(IconName::Folder, "Moving items", "Raw footage");
            spec.transfer = true;
            spec.placement = Placement::Dialog;
            spec.subtitle = Some(g.c16.dock.words().waiting_for_write.clone());
            g.c16.dock.start(77, spec, tx);
            g.c16.dialog = Some(77);
        });
        run(&mut rt, 0.1);
        rt.update(|g, tx| {
            let now = g.c16.now;
            let sample = Sample::new(Progress::Transfer(Transfer {
                completed_items: 0,
                total_items: 1,
                completed_files: 3,
                total_files: Some(30),
                bytes: 10_000_000,
                total_bytes: Some(120_000_000),
            }))
            .busy("Writing to device…")
            .current("clip.mov");
            g.c16.dock.progress(77, sample, now, tx);
        });
        run(&mut rt, 0.5);
        wake_until(&mut rt, 30.0, |_| false);
        rt
    }

    // operations/progress: "If the transfer has not stopped 8 seconds after Cancel, the title
    // becomes 'Device not responding'": a cancel from the dialog (Escape, or Return on the
    // focused Cancel) that moves no layout still stalls on time, stepping only wakes and
    // input, with animations on and off and with the host's clock frozen (time_scale 0). C15's
    // handlers carry no Tx; the dock's wake beacon asks for the poll that stamps the cancel.
    #[test]
    fn a_cancel_that_moves_no_layout_still_stalls_on_time() {
        type Case = (&'static str, fn(&mut Gallery));
        let cases: [Case; 3] = [
            ("animations on", |_| {}),
            ("animations off", still),
            ("time scale 0", |g| g.c16.time_scale = Some(0.0)),
        ];
        for (case, change) in cases {
            for key in [Key::Escape, Key::Enter] {
                let mut rt = quiet_dialog_move(change);
                assert_eq!(card(&rt.state, 77).as_deref(), Some("Moving items"));
                let start = rt.state.c16.now;
                rt.key(KeyEvent {
                    key: key.clone(),
                    mods: Mods::default(),
                });
                let end = wake_until(&mut rt, 20.0, |g| {
                    card(g, 77).as_deref() == Some("Device not responding")
                });
                assert_eq!(
                    card(&rt.state, 77).as_deref(),
                    Some("Device not responding"),
                    "{case}, {key:?}"
                );
                assert!(
                    (end - start - 8.0).abs() < 0.1,
                    "{case}, {key:?}: stalled {:.3} s after the cancel",
                    end - start
                );
            }
        }
    }

    // operations/progress/dock: "When a notice expires and the dock holds no other row, the
    // dock closes."
    #[test]
    fn a_notice_expires_from_its_own_wake_and_the_dock_closes() {
        let mut rt = mount("c16-idle-light");
        tap(&mut rt, "c16-start-notice");
        assert!(element(&rt, "c16-dock").is_some());
        let start = rt.state.c16.now;
        let end = wake_until(&mut rt, 10.0, |g| g.c16.dock.is_empty());
        assert!((end - start - 4.0).abs() < 0.05, "{:.3} s", end - start);
        rt.frame(FRAME);
        assert!(element(&rt, "c16-dock").is_none());
    }

    // operations/progress/jobs: "A row shows … a status with elapsed time ('Running for',
    // …)": the open list moves each second, from wake to wake, and asks nothing once hidden.
    #[test]
    fn the_jobs_list_ticks_each_second_while_open_and_stops_when_hidden() {
        let mut rt = mount("c16-jobs-light");
        let status = |rt: &Runtime<Gallery>| {
            element(rt, "c16-jobs-row-4-status")
                .and_then(|e| e.text)
                .unwrap_or_default()
        };
        let before = status(&rt);
        let deadline = rt
            .wake_deadline()
            .expect("an open list with a running job wakes");
        assert!(deadline - rt.state.c16.now <= 1.0 + 1e-6);
        wake_until(&mut rt, 1.0, |_| false);
        assert_ne!(
            status(&rt),
            before,
            "the elapsed time moved without frames between"
        );
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        rt.frame(FRAME);
        rt.frame(FRAME);
        assert!(!rt.state.c16.jobs_ui.is_open());
        // A wake asked before Escape still comes once; after it, nothing more is asked.
        wake_until(&mut rt, 1.5, |_| false);
        assert!(
            rt.wake_deadline().is_none(),
            "a hidden list asks for no wakes"
        );
    }

    // operations/progress/jobs: "Dismiss removes one finished job; Clear finished removes every
    // finished job": clearing a list of finished jobs leaves it open on "No background jobs.",
    // closing it sends focus to the host's fallback (the cursor row: the button goes with
    // the last job), and a later job doesn't pop the list open (review 1, should-fix 1).
    #[test]
    fn clearing_every_job_leaves_a_note_then_focus_on_the_fallback_and_the_list_shut() {
        let mut rt = mount_with("c16-jobs-light", |g| {
            g.c16.jobs.retain(|r| r.status.is_finished());
        });
        tap(&mut rt, "c16-jobs-clear");
        run(&mut rt, 0.2);
        assert!(rt.state.c16.jobs.is_empty());
        // Clear finished went with the rows; Rune hands focus to the list's next control.
        let focused = rt
            .elements()
            .into_iter()
            .find(|e| e.focused)
            .and_then(|e| e.key);
        assert_eq!(focused.as_deref(), Some("c16-jobs-minimize"));
        let note = element(&rt, "c16-jobs-note").and_then(|e| e.text);
        assert_eq!(note.as_deref(), Some("No background jobs."));
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        run(&mut rt, 0.2);
        assert!(!rt.state.c16.jobs_ui.is_open());
        assert!(element(&rt, "c16-jobs").is_none(), "no jobs: no button");
        let focused = rt
            .elements()
            .into_iter()
            .find(|e| e.focused)
            .and_then(|e| e.key);
        assert_eq!(focused.as_deref(), Some("c16-cursor"));
        rt.state
            .c16
            .jobs
            .push(strata_components::progress_dock::jobs::JobRow::new(
                9,
                "Resize images",
                strata_components::progress_dock::jobs::JobStatus::Running,
                rt.state.c16.now,
            ));
        rt.update(|_, _| {});
        run(&mut rt, 0.2);
        assert!(element(&rt, "c16-jobs").is_some());
        assert!(
            element(&rt, "c16-jobs-list").is_none(),
            "a later job doesn't open the list"
        );
    }

    // operations/progress: "During a copy or move with a known byte total, the bar advances
    // as bytes are written, not only after each item.": the fill and the labels follow every
    // sample through Live values, with no commit between the throttled ones.
    #[test]
    fn the_fill_and_labels_move_every_frame_and_commits_stay_throttled() {
        let mut rt = mount("c16-card-light");
        let widths: Vec<f32> = (0..12)
            .map(|_| {
                rt.frame(FRAME);
                rect(&rt, "c16-dock-card-1-fill")[2]
            })
            .collect();
        assert!(
            widths.windows(2).all(|w| w[1] > w[0]),
            "the fill grows every frame: {widths:?}"
        );
        let c0 = rune::commit_generation();
        run(&mut rt, 1.0);
        let commits = rune::commit_generation() - c0;
        assert!(commits <= 4, "{commits} commits in a second of progress");
        let live = &rt.state.c16.dock.op(1).unwrap().live;
        let percent = element(&rt, "c16-dock-card-1-percent").and_then(|e| e.text);
        assert_eq!(percent.as_deref(), Some(&*live.percent.get()));
    }

    // Scrolling the dock moves every track, and on_layout reports each move: a counting card
    // must not commit for one that didn't change its size (review 1, should-fix 2).
    #[test]
    fn scrolling_the_dock_past_counting_cards_commits_only_for_their_seconds() {
        let mut rt = mount_with("c16-complete-light", |g| {
            let mut tx = rune::Tx::default();
            for _ in 0..6 {
                let id = g.c16.copy();
                g.c16.fakes.retain(|f| f.id != id);
                g.c16.dock.complete(id, "Copy complete", &mut tx);
            }
        });
        run(&mut rt, 0.3);
        // In the dock's padding beside the cards, so no card is hovered as they slide by.
        let r = rect(&rt, "c16-dock");
        let (x, y) = (r[0] + r[2] - 4.0, r[1] + r[3] / 2.0);
        rt.pointer_move(x, y);
        run(&mut rt, 0.1);
        let c0 = rune::commit_generation();
        for i in 0..30 {
            rt.wheel(x, y, if i < 15 { -20.0 } else { 20.0 });
            rt.frame(FRAME);
        }
        let commits = rune::commit_generation() - c0;
        // One for the countdowns' next second, and two Rune makes as the wheel gesture starts
        // and turns (they stay with every card pinned). Before the fix: 29.
        assert!(commits <= 3, "{commits} commits over 30 wheel frames");
    }

    /// Perf: twenty cards whose byte counters move every frame. Prints the commit rate and the
    /// frame cost; asserts the commit rate stays at the throttle's.
    #[test]
    fn twenty_cards_updating_every_frame_commit_at_most_four_times_a_second() {
        let mut rt = mount_with("c16-idle-light", |g| {
            for _ in 0..20 {
                g.c16.copy();
            }
            // Slow enough that none finishes in the measured 10 s.
            for f in &mut g.c16.fakes {
                f.kind = scenes::c16::Kind::Copy {
                    total: 4_000_000_000,
                    files: 1000,
                    rate: 8_000_000.0,
                };
            }
        });
        run(&mut rt, 1.0);
        let seconds = 10.0;
        // Each frame's cost, with the page committing only when the dock flushes, then with a
        // commit forced every frame (what a byte counter in committed text would cost).
        let measure = |rt: &mut Runtime<Gallery>, force: bool| {
            let c0 = rune::commit_generation();
            let mut times = Vec::new();
            for _ in 0..(seconds / FRAME) as usize {
                let t = Instant::now();
                if force {
                    rt.update(|g, _| g.c16.probes = g.c16.probes.wrapping_add(0));
                }
                rt.frame(FRAME);
                times.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            times.sort_by(f64::total_cmp);
            let p = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
            (rune::commit_generation() - c0, p(0.5), p(0.95))
        };
        let (commits, p50, p95) = measure(&mut rt, false);
        let (forced, f50, f95) = measure(&mut rt, true);
        eprintln!(
            "c16 perf: 20 cards, {commits} commits in {seconds} s ({:.1}/s), frame p50 {p50:.2} \
             ms, p95 {p95:.2} ms; committing every frame: {forced} commits, p50 {f50:.2} ms, \
             p95 {f95:.2} ms",
            commits as f64 / seconds
        );
        assert!(
            commits as f64 / seconds <= 4.0,
            "{commits} commits in {seconds} s"
        );
    }
}

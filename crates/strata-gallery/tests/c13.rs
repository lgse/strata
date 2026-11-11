//! C13's page: what the player does in process, beside its goldens and scripts. These cover
//! what a script can't see: commits, wakes, timers stepped from wake to wake with no frames
//! between, intents the host gets, and the frame time of a playing video.

mod c13 {
    use std::time::{Duration, Instant};

    use rune::{ElementInfo, Env, Runtime};
    use strata_components::media_player::{Art, MediaIntent, PlaybackClock, PlaybackState};
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
        rt
    }

    fn mount(name: &str) -> Runtime<Gallery> {
        mount_with(name, |_| {})
    }

    fn run(rt: &mut Runtime<Gallery>, seconds: f64) {
        for _ in 0..(seconds / FRAME).round() as usize {
            rt.frame(FRAME);
        }
    }

    fn element(rt: &Runtime<Gallery>, key: &str) -> Option<ElementInfo> {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
    }

    /// Whether `key` is drawn and not on its way out.
    fn shown(rt: &Runtime<Gallery>, key: &str) -> bool {
        element(rt, key).is_some_and(|e| !e.exiting)
    }

    fn centre(rt: &Runtime<Gallery>, key: &str) -> (f32, f32) {
        let r = element(rt, key).unwrap_or_else(|| panic!("no {key}")).rect;
        (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
    }

    /// Steps from wake to wake, with no frames between, until `done` or `limit` seconds pass
    /// on the page's clock; returns the page clock it stopped at.
    fn wake_until(
        rt: &mut Runtime<Gallery>,
        limit: f64,
        done: impl Fn(&Runtime<Gallery>) -> bool,
    ) -> f64 {
        let start = rt.state.c13.now;
        for _ in 0..5000 {
            let now = rt.state.c13.now;
            if now - start > limit || done(rt) {
                break;
            }
            if rt.has_pending_input() {
                rt.frame(FRAME);
                continue;
            }
            let Some(deadline) = rt.wake_deadline() else {
                break;
            };
            rt.frame((deadline - now).max(0.001));
        }
        rt.state.c13.now
    }

    fn commits() -> u64 {
        rune::commit_generation()
    }

    fn still(g: &mut Gallery) {
        g.look.animations = false;
    }

    // Commits and wakes.

    /// Plan §4 C13 and P7: a playing video updates its frames and its position with no view
    /// commit per frame. 600 frames at 60 fps of a 1280 × 720 video commit once a second (the
    /// elapsed time and the slider's spoken value), and Rune's live-only check stays quiet.
    #[test]
    fn a_playing_video_commits_once_a_second_over_600_frames() {
        let mut rt = mount("c13-perf-light");
        run(&mut rt, 1.0);
        let c0 = commits();
        let mut times = Vec::with_capacity(600);
        for _ in 0..600 {
            let t = Instant::now();
            rt.frame(FRAME);
            times.push(t.elapsed());
        }
        let n = commits() - c0;
        times.sort();
        let p95 = times[times.len() * 95 / 100];
        eprintln!(
            "c13 perf: 600 frames of 1280 × 720 video at 60 fps: {n} commits, frame p95 {p95:?} (median {:?})",
            times[300]
        );
        assert_eq!(rt.state.c13.clock.state(), PlaybackState::Playing);
        assert!(n <= 11, "{n} commits in 10 s");
        assert!(n >= 9, "the elapsed second still commits: {n}");
        assert!(
            rt.live_only_warnings().is_empty(),
            "{:?}",
            rt.live_only_warnings()
        );
    }

    /// The spectrum's bars, caps and reflections are live values: a playing song commits only
    /// as its seconds change.
    #[test]
    fn a_playing_spectrum_commits_only_as_the_seconds_change() {
        let mut rt = mount("c13-audio-record-light");
        run(&mut rt, 0.5);
        assert!(element(&rt, "media-bar-0").is_some());
        let c0 = commits();
        run(&mut rt, 5.0);
        let n = commits() - c0;
        eprintln!("c13 spectrum: 300 frames playing: {n} commits");
        assert!(n <= 6, "{n} commits in 5 s");
        assert!(
            rt.live_only_warnings().is_empty(),
            "{:?}",
            rt.live_only_warnings()
        );
    }

    /// A paused player at rest asks for nothing: no wake, no poll.
    #[test]
    fn a_paused_player_at_rest_asks_for_no_wake() {
        let mut rt = mount("c13-video-paused-light");
        run(&mut rt, 2.0);
        assert_eq!(rt.wake_deadline(), None);
        let mut audio = mount("c13-audio-paused-light");
        run(&mut audio, 2.0);
        assert_eq!(audio.wake_deadline(), None, "the spectrum settles too");
    }

    /// A song paused mid-play: its bars and caps fall to rest from wake to wake, then the
    /// wakes stop.
    #[test]
    fn a_paused_spectrum_falls_to_rest_from_wake_to_wake_then_stops() {
        let mut rt = mount("c13-audio-record-light");
        run(&mut rt, 0.5);
        rt.update(|g, _| g.c13.apply(MediaIntent::Pause));
        let end = wake_until(&mut rt, 5.0, |rt| rt.wake_deadline().is_none());
        assert!(rt.state.c13.media.spectrum().ballistics().is_settled());
        assert!(end - 0.5 < 3.0, "settled by {end:.2}");
        assert_eq!(rt.wake_deadline(), None);
    }

    /// `preview/preview-panel/media/video`: "The first frame fades the stand-in out over
    /// 150 ms, or replaces it at once under reduced motion." From wake to wake, animations
    /// on and off.
    #[test]
    fn the_stand_in_fades_over_150_ms_or_goes_at_once_from_wake_to_wake() {
        for animations in [true, false] {
            let mut rt = mount_with("c13-video-fresh-light", |g| {
                if !animations {
                    still(g);
                }
            });
            assert!(element(&rt, "media-stand-in").is_some());
            let landed = wake_until(&mut rt, 1.0, |rt| rt.state.c13.media.has_frame());
            let gone = wake_until(&mut rt, 1.0, |rt| !shown(rt, "media-stand-in"));
            let took = gone - landed;
            if animations {
                assert!((0.14..0.2).contains(&took), "faded over {took:.3} s");
            } else {
                assert!(took < 0.02, "gone at once: {took:.3} s");
            }
        }
    }

    /// The storyboard is asked for once per clip, when its first frame shows.
    #[test]
    fn the_storyboard_is_asked_once_at_the_first_frame() {
        let mut rt = mount("c13-video-fresh-light");
        run(&mut rt, 2.0);
        let asks = |g: &Gallery| {
            g.c13
                .intents
                .iter()
                .filter(|i| **i == MediaIntent::StoryboardWanted)
                .count()
        };
        assert_eq!(asks(&rt.state), 1);
        assert!(
            rt.state
                .c13
                .media
                .storyboard()
                .is_some_and(|b| b.loaded() > 0)
        );
    }

    /// The waveform's grow-in commits at most 30 times a second (OWNER ruling, D3), even at
    /// 120 frames a second; with animations off the levels land as they come.
    #[test]
    fn the_grow_in_commits_at_most_30_times_a_second() {
        let mut rt = mount("c13-video-fresh-light");
        // Levels arrive from 0.3 s, a chunk each 40 ms.
        for _ in 0..30 {
            rt.frame(0.01);
        }
        let c0 = commits();
        for _ in 0..120 {
            rt.frame(1.0 / 120.0);
        }
        let n = commits() - c0;
        assert!(n <= 32, "{n} commits in 1 s at 120 frames a second");
        assert!(n >= 10, "it still grows: {n}");
    }

    // Behaviour a script can't watch.

    fn covered(rt: &Runtime<Gallery>) -> bool {
        rt.elements().into_iter().any(|e| {
            !e.exiting
                && e.key
                    .as_deref()
                    .is_some_and(|k| k.starts_with("media-cover-"))
        })
    }

    /// `preview/preview-panel/media/video`: "During a seek, the nearest storyboard cell covers
    /// the old frame until the new one is decoded." From wake to wake: the cover shows while
    /// the clock seeks and goes when it lands.
    #[test]
    fn a_seek_shows_the_nearest_cell_until_it_lands() {
        let mut rt = mount("c13-video-paused-light");
        run(&mut rt, 0.3);
        assert!(!covered(&rt));
        rt.update(|g, _| {
            let d = &mut g.c13;
            let mut c =
                strata_components::media_player::FakeClock::new(Some(Duration::from_secs(200)))
                    .latency(0.0, 0.4);
            c.advance(d.now);
            d.clock = c;
            d.apply(MediaIntent::Seek(Duration::from_secs(150)));
        });
        rt.frame(FRAME);
        assert_eq!(rt.state.c13.clock.state(), PlaybackState::Seeking);
        assert!(covered(&rt), "the cell covers the frame");
        let landed = wake_until(&mut rt, 2.0, |rt| !covered(rt));
        assert_eq!(rt.state.c13.clock.state(), PlaybackState::Paused);
        assert!(landed - 0.3 < 0.5, "gone when the seek landed: {landed:.3}");
    }

    /// `preview/preview-panel/media/audio`: "Tracks never advance automatically when one
    /// ends."
    #[test]
    fn a_track_that_ends_stays_put() {
        let mut rt = mount("c13-audio-record-light");
        rt.update(|g, _| {
            g.c13.clock.set_position(Duration::from_secs(244));
            g.c13.clock.play_now();
        });
        run(&mut rt, 4.0);
        assert_eq!(rt.state.c13.clock.state(), PlaybackState::Ended);
        assert_eq!(rt.state.c13.file, 0);
        assert!(
            !rt.state
                .c13
                .intents
                .iter()
                .any(|i| matches!(i, MediaIntent::Step(_)))
        );
        assert_eq!(rt.wake_deadline(), None, "nothing ticks at the end");
    }

    /// `preview/preview-panel/media/audio`: "Resizing the pane does not restart audio
    /// playback." No intent goes out, wide or stacked.
    #[test]
    fn resizing_audio_sends_no_intent() {
        let mut rt = mount("c13-audio-record-light");
        run(&mut rt, 0.3);
        let before = rt.state.c13.intents.len();
        for area in [[760.0, 640.0], [360.0, 360.0], [520.0, 640.0]] {
            rt.update(|g, _| g.c13.area = area);
            run(&mut rt, 0.3);
        }
        assert_eq!(
            rt.state.c13.intents.len(),
            before,
            "{:?}",
            &rt.state.c13.intents[before..]
        );
        assert_eq!(rt.state.c13.clock.state(), PlaybackState::Playing);
    }

    /// `preview/preview-panel/media`: "Video decodes at the pane's size times display scale,
    /// capped at 1280 px on either axis and never enlarged beyond the source", and "Growing the
    /// pane restarts decoding at the current position only when the frame would grow by 8% or
    /// more; shrinking never restarts."
    #[test]
    fn the_decode_size_follows_growth_of_eight_percent_and_never_a_shrink() {
        let mut rt = mount("c13-video-paused-light");
        run(&mut rt, 0.5);
        let sizes = |g: &Gallery| -> Vec<[u32; 2]> {
            g.c13
                .intents
                .iter()
                .filter_map(|i| match i {
                    MediaIntent::DecodeSize(s) => Some(*s),
                    _ => None,
                })
                .collect()
        };
        let first = sizes(&rt.state);
        assert_eq!(
            first.len(),
            1,
            "the first frame came at 320 × 180: {first:?}"
        );
        let n = first.len();
        // 5 % wider: no restart.
        rt.update(|g, _| g.c13.area = [690.0, 640.0]);
        run(&mut rt, 0.3);
        assert_eq!(sizes(&rt.state).len(), n);
        // Narrower: never.
        rt.update(|g, _| g.c13.area = [420.0, 640.0]);
        run(&mut rt, 0.3);
        assert_eq!(sizes(&rt.state).len(), n);
        // Back past 8 % of what the decoder makes: a restart, within 1280.
        rt.update(|g, _| g.c13.area = [960.0, 640.0]);
        run(&mut rt, 0.3);
        let all = sizes(&rt.state);
        assert_eq!(all.len(), n + 1, "{all:?}");
        assert!(all[n].iter().all(|e| *e <= 1280));
    }

    /// `preview/preview-panel/media/audio`: "Stepping to a track with the same art keeps it in
    /// place; only different art crossfades." The same art keeps its element; new art mounts
    /// fresh (D4: without the crossfade).
    #[test]
    fn the_same_art_keeps_its_element_and_new_art_replaces_it() {
        let mut rt = mount("c13-audio-wide-light");
        run(&mut rt, 0.3);
        assert!(element(&rt, "media-art-1").is_some());
        rt.update(|g, _| {
            let px = g.c13.frames.clone();
            g.c13.media.set_art(Some(Art {
                id: 1,
                pixels: px,
                size: [240, 240],
            }));
        });
        run(&mut rt, 0.1);
        assert!(element(&rt, "media-art-1").is_some());
        rt.update(|g, _| {
            let px = g.c13.frames.clone();
            g.c13.media.set_art(Some(Art {
                id: 2,
                pixels: px,
                size: [240, 240],
            }));
        });
        // The old art's exit transition runs first.
        run(&mut rt, 0.1);
        assert!(!shown(&rt, "media-art-1"));
        assert!(element(&rt, "media-art-2").is_some());
    }

    /// Hovering the timeline moves the pointer's line, the tint and the bubble as live
    /// values: within one second of the clip, nothing commits.
    #[test]
    fn hovering_within_a_second_commits_nothing() {
        let mut rt = mount("c13-video-paused-light");
        run(&mut rt, 0.5);
        let r = element(&rt, "media-timeline").unwrap().rect;
        let y = r[1] + r[3] / 2.0;
        // 200 s over the width: the middle of second 31.
        let x = r[0] + r[2] * 31.5 / 200.0;
        rt.pointer_move(x, y);
        run(&mut rt, 0.1);
        assert!(element(&rt, "media-bubble").is_some());
        let c0 = commits();
        for dx in [0.1, 0.2, 0.3, 0.4] {
            rt.pointer_move(x + dx, y);
            rt.frame(FRAME);
        }
        assert_eq!(commits() - c0, 0);
        assert!(
            rt.live_only_warnings().is_empty(),
            "{:?}",
            rt.live_only_warnings()
        );
        rt.pointer_move(r[0] + 160.0, y);
        rt.frame(FRAME);
        assert!(commits() > c0, "a new second shows");
        rt.pointer_move(5.0, 600.0);
        run(&mut rt, 0.1);
        assert!(!shown(&rt, "media-bubble"), "leaving hides the bubble");
    }

    /// The window's media keys reach the player through the host's key handler, from the
    /// listing too.
    #[test]
    fn the_window_keys_work_from_the_listing() {
        let mut rt = mount("c13-video-paused-light");
        run(&mut rt, 0.3);
        let (x, y) = centre(&rt, "files");
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(FRAME);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
        rt.key(rune::KeyEvent {
            key: rune::Key::Text(" ".into()),
            mods: rune::Mods {
                ctrl: true,
                cmd: true,
                alt: true,
                ..rune::Mods::default()
            },
        });
        run(&mut rt, 0.1);
        assert_eq!(rt.state.c13.clock.state(), PlaybackState::Playing);
    }
}

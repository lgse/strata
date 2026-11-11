//! C1d's marquee on the c1 page, in process: what scripts can't do (Ctrl and Shift held through
//! a drag) and what no golden shows (an open band's moves and auto-scroll run no view). A test
//! stand-in for C1b's pointer intent starts the band past the drag threshold, as C1b hands it
//! over, so these tests don't depend on C1b. Each test names the bullets it checks; the scripts
//! in `tests/scripts/c1/marquee-*.json` check the rest, and two tests here run them through the
//! gallery binary: one for Rune's perf warnings (debug builds), one under
//! `RUNE_COMMIT_BUDGET_MS=8` (`--profile perf`). Those two need the page's pointer controller
//! (C1b) to start the band.

#[expect(
    dead_code,
    reason = "the marquee tests use only the gallery command and the scripts folder"
)]
#[path = "goldens/support.rs"]
mod support;

mod c1 {
    mod marquee {
        use rune::{ElementInfo, Env, Mods, PhysicalInput, Runtime};
        use strata_components::collection::element::{
            Ctx, Effect, Flow, PointerController, PointerInput, PointerPhase,
        };
        use strata_components::collection::marquee::view::KEY as BAND;
        use strata_components::contract::{EntryId, EntrySource, Point};
        use strata_gallery::scenes::c1::{WARMED, WARMUP};
        use strata_gallery::{Gallery, scenes};

        use crate::support;

        const FRAME: f64 = 1.0 / 60.0;

        /// Stands in for C1b: a primary press starts a marquee once it travels more than 8 pt
        /// on either axis, at the press with the modifiers held at the press. It changes no
        /// selection itself, as C1b's Ctrl and Shift presses beside an entry don't.
        #[derive(Default)]
        struct Press {
            at: Option<(Point, Mods)>,
        }

        impl PointerController for Press {
            fn pointer(&mut self, input: &PointerInput, cx: &mut Ctx<'_>) -> Flow {
                match input.phase {
                    PointerPhase::Down => self.at = Some((input.point, input.mods)),
                    PointerPhase::Move => {
                        let past = input.travel.x.abs() > 8.0 || input.travel.y.abs() > 8.0;
                        if past && let Some((at, mods)) = self.at.take() {
                            cx.emit(Effect::BeginMarquee { at, mods });
                        }
                    }
                    PointerPhase::Up => self.at = None,
                }
                Flow::Handled
            }
        }

        fn mount(name: &str) -> Runtime<Gallery> {
            let scene = scenes::all_scenes()
                .into_iter()
                .find(|(n, _)| *n == name)
                .map(|(_, f)| f)
                .unwrap_or_else(|| panic!("no scene {name}"));
            let mut g = Gallery::default();
            scene(&mut g);
            g.c1.coll.controllers.pointer = Some(Box::new(Press::default()));
            let env = Env::new(g.dark, 1.0);
            let mut rt = strata_gallery::runtime(g, env);
            rt.frame(0.0);
            run(&mut rt, 0.5);
            rt
        }

        fn run(rt: &mut Runtime<Gallery>, seconds: f64) {
            for _ in 0..(seconds / FRAME).round() as usize {
                rt.frame(FRAME);
            }
        }

        fn element(rt: &Runtime<Gallery>, key: &str) -> Option<ElementInfo> {
            rt.elements()
                .into_iter()
                .find(|e| !e.exiting && e.key.as_deref() == Some(key))
        }

        /// A point in the blank space at the right of row `id`'s row.
        fn beside(rt: &Runtime<Gallery>, id: u64) -> (f32, f32) {
            let [x, y, w, h] = element(rt, &format!("row-{id}"))
                .unwrap_or_else(|| panic!("row-{id} isn't mounted"))
                .visible;
            (x + w - 24.0, y + h / 2.0)
        }

        fn selected(rt: &Runtime<Gallery>) -> Vec<u64> {
            let d = &rt.state.c1;
            d.coll
                .selection
                .selected_ids(&d.listing)
                .into_iter()
                .map(EntryId::get)
                .collect()
        }

        /// Whether any node of the accessibility tree has key `key`.
        fn heard(rt: &Runtime<Gallery>, key: &str) -> bool {
            fn walk(v: &serde_json::Value, key: &str) -> bool {
                v["key"] == key
                    || v["children"]
                        .as_array()
                        .is_some_and(|c| c.iter().any(|c| walk(c, key)))
            }
            walk(&rt.accessibility_json(), key)
        }

        /// Presses beside row `id`, starts the band, and holds it at the viewport's bottom edge
        /// for `seconds`. Returns the pointer's window x and y.
        fn hold_at_the_bottom(rt: &mut Runtime<Gallery>, id: u64, seconds: f64) -> (f32, f32) {
            let (x, y) = beside(rt, id);
            rt.pointer_down(x, y);
            rt.frame(FRAME);
            rt.pointer_move(x, y + 9.0);
            rt.frame(FRAME);
            let [_, vy, _, vh] = element(rt, "viewport").expect("the viewport").visible;
            let at = (x, vy + vh - 2.0);
            rt.pointer_move(at.0, at.1);
            rt.frame(FRAME);
            run(rt, seconds);
            at
        }

        fn select(rt: &mut Runtime<Gallery>, ids: &[u64]) {
            let d = &mut rt.state.c1;
            for (n, id) in ids.iter().enumerate() {
                let i = d.listing.index_of(EntryId::new(*id)).expect("listed");
                if n == 0 {
                    d.coll.selection.select_only(&d.listing, i);
                } else {
                    d.coll.selection.toggle(&d.listing, i);
                }
            }
            run(rt, 0.1);
        }

        /// Presses beside row `from` with `mods` held, lets the modifiers go, and drags to row
        /// `to` in steps, then releases.
        fn band(rt: &mut Runtime<Gallery>, mods: Mods, from: u64, to: u64) {
            let (x, y0) = beside(rt, from);
            let (_, y1) = beside(rt, to);
            rt.physical_input(PhysicalInput::Modifiers(mods));
            rt.pointer_down(x, y0);
            rt.frame(FRAME);
            rt.physical_input(PhysicalInput::Modifiers(Mods::default()));
            for k in 1..=8 {
                rt.pointer_move(x, y0 + (y1 - y0) * k as f32 / 8.0);
                rt.frame(FRAME);
            }
            assert!(element(rt, BAND).is_some(), "the band shows while held");
            rt.pointer_up(x, y1);
            run(rt, 0.1);
            assert!(element(rt, BAND).is_none(), "and goes on release");
        }

        #[test]
        fn a_ctrl_band_toggles_and_a_shift_band_adds_against_the_selection_at_press() {
            // browser/selection/marquee: A Ctrl-marquee toggles each crossed entry relative to the selection at press.
            // browser/selection/marquee: A Shift-marquee adds the crossed entries to the selection at press.
            let ctrl = Mods {
                ctrl: true,
                ..Mods::default()
            };
            let shift = Mods {
                shift: true,
                ..Mods::default()
            };
            let mut rt = mount("c1-ten-rows-light");
            select(&mut rt, &[3, 4, 9]);
            band(&mut rt, ctrl, 2, 5);
            assert_eq!(
                selected(&rt),
                [2, 5, 9],
                "Ctrl flips 2 to 5 against the press"
            );

            band(&mut rt, shift, 6, 7);
            assert_eq!(selected(&rt), [2, 5, 6, 7, 9], "Shift adds 6 and 7");

            let ctrl_shift = Mods {
                shift: true,
                ..ctrl
            };
            band(&mut rt, ctrl_shift, 5, 6);
            assert_eq!(selected(&rt), [2, 7, 9], "Ctrl+Shift toggles (D5)");
        }

        /// What C1e asks of a marquee: an open band's moves, and auto-scroll frames that cross
        /// no new entry, run no view; showing and hiding the band, and selection changes, do.
        /// Twice: with Rune's debug check of `live_only` (it commits once a quarter second after
        /// a commit to compare what the view builds; no warnings), then without it, counting.
        #[test]
        fn an_open_band_runs_no_view_while_it_moves_and_scrolls_at_ten_hertz() {
            // browser/selection/marquee: Holding the pointer near a viewport edge scrolls the listing and extends the selection each frame.
            for check in [true, false] {
                let mut rt = mount("c1-photos-light");
                rt.check_live_only(check);
                let (x, y) = beside(&rt, 10_002);
                rt.pointer_down(x, y);
                rt.frame(FRAME);
                rt.pointer_move(x, y + 9.0);
                rt.frame(FRAME);
                assert!(element(&rt, BAND).is_some());
                assert!(!heard(&rt, BAND), "the band is decoration, not in the tree");
                assert_eq!(selected(&rt), [10_002]);

                // Moves inside one row (the press was mid-row, rows are taller than 22 pt)
                // cross nothing new.
                let before = rt.stats.commits;
                for k in 1..=10 {
                    rt.pointer_move(x - k as f32, y + 9.0 + k as f32 * 0.2);
                    rt.frame(FRAME);
                }
                let after_moves = rt.stats.commits;

                // Held at the bottom edge for a second: the view glides down and the band grows.
                let [_, vy, _, vh] = element(&rt, "viewport").expect("the viewport").visible;
                rt.pointer_move(x, vy + vh - 2.0);
                rt.frame(FRAME);
                let from = rt.state.c1.coll.scroll_offset();
                let commits_from = rt.stats.commits;
                run(&mut rt, 1.0);
                let scrolled = rt.state.c1.coll.scroll_offset() - from;
                let commits = rt.stats.commits - commits_from;
                assert!(scrolled > 500.0, "it glided {scrolled} pt in a second");
                assert!(selected(&rt).len() > 30, "and the band grew with it");
                assert!(
                    rt.live_only_warnings().is_empty(),
                    "{:?}",
                    rt.live_only_warnings()
                );
                if !check {
                    assert_eq!(after_moves, before, "moves within a row ran the view");
                    // D1: the band applies at most every 100 ms while the view scrolls.
                    assert!(
                        (1..=11).contains(&commits),
                        "{commits} commits in a second of auto-scroll"
                    );
                }
                rt.pointer_up(x, vy + vh - 2.0);
                rt.frame(FRAME);
                assert!(element(&rt, BAND).is_none());
                let n = selected(&rt).len();
                let last = rt.state.c1.listing.len();
                assert!(n > 30 && n < last);
            }
        }

        /// Runs script `name` from `tests/scripts/c1/` through the gallery binary with `env`;
        /// its stderr.
        fn gallery_run(name: &str, env: &[(&str, &str)]) -> String {
            let script = support::scripts_dir("c1").join(format!("{name}.json"));
            let spec: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&script).expect("the script"))
                    .expect("JSON");
            let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("c1-marquee");
            std::fs::create_dir_all(&dir).expect("a scratch folder");
            let events = dir.join(format!("{name}-events.json"));
            let body = serde_json::json!({ "events": spec["events"] }).to_string();
            std::fs::write(&events, body).expect("the events");
            let mut cmd = support::gallery();
            cmd.env_remove("RUNE_PERF_WARN")
                .env_remove("RUNE_COMMIT_BUDGET_MS");
            for (k, v) in env {
                cmd.env(k, v);
            }
            cmd.arg("--scene")
                .arg(spec["scene"].as_str().expect("a scene"))
                .arg("--script")
                .arg(&events)
                .arg("--tree")
                .arg(dir.join(format!("{name}-tree.json")));
            support::run(cmd).unwrap_or_else(|e| panic!("{e}"))
        }

        /// D1's check: Rune's debug builds warn, on stderr, when one source commits through
        /// most of a scroll, when an app steps its own glide with instant scrolls, and when a
        /// resting window keeps waking. None of them may fire for a band: dragged, held at the
        /// edge for ten seconds over 100,000 rows, brought back, or wheeled.
        #[test]
        #[cfg_attr(not(debug_assertions), ignore = "Rune's perf warnings are debug-only")]
        fn the_marquee_scripts_keep_runes_perf_warnings_silent() {
            for name in [
                "marquee-drag",
                "marquee-edge-scroll",
                "marquee-return",
                "marquee-wheel",
            ] {
                let stderr = gallery_run(name, &[]);
                let warnings: Vec<&str> = stderr
                    .lines()
                    .filter(|l| l.contains("RUNE_PERF_WARN=0 silences"))
                    .collect();
                assert!(
                    warnings.is_empty(),
                    "{name}: Rune's perf warnings:\n{}",
                    warnings.join("\n")
                );
            }
        }

        /// The edge auto-scroll over 100,000 rows under `RUNE_COMMIT_BUDGET_MS=8`: no commit
        /// over 8 ms once the scene is mounted. The page waits out Rune's one-line-a-second
        /// limit after the mount (U-C1e-5), as C1e's scroll budget does.
        ///
        /// It times release-build commits, so it runs only in an optimized build:
        /// `cargo test --profile perf -p strata-gallery --test c1_marquee budget`.
        #[test]
        #[cfg_attr(debug_assertions, ignore = "perf: run with --profile perf")]
        fn edge_auto_scroll_over_a_hundred_thousand_rows_stays_under_the_commit_budget() {
            let stderr = gallery_run(
                "marquee-edge-scroll",
                &[("RUNE_COMMIT_BUDGET_MS", "8"), (WARMUP, "1")],
            );
            let after = stderr
                .split_once(WARMED)
                .map(|(_, after)| after)
                .unwrap_or_else(|| panic!("the page never warmed up:\n{stderr}"));
            let over: Vec<&str> = after
                .lines()
                .filter(|l| l.starts_with("rune: commit took"))
                .collect();
            assert!(
                over.is_empty(),
                "commits over budget while the band auto-scrolls:\n{}",
                over.join("\n")
            );
        }

        /// S1 (review): the release stops the auto-scroll dead, as Strata's timer stops: the
        /// view stays where it was, and the last row the band took stays under the pointer.
        #[test]
        fn releasing_a_band_mid_scroll_stops_the_view_where_it_is() {
            let mut rt = mount("c1-photos-light");
            let (x, y) = hold_at_the_bottom(&mut rt, 10_002, 1.0);
            let at = rt.state.c1.coll.scroll_offset();
            assert!(at > 500.0, "it scrolled {at} pt");
            rt.pointer_up(x, y);
            run(&mut rt, 1.0);
            let moved = rt.state.c1.coll.scroll_offset() - at;
            assert!(
                moved.abs() < 1.0,
                "the view went on {moved} pt after the release"
            );
            let [_, vy, _, _] = element(&rt, "viewport").expect("the viewport").visible;
            let row = rt.state.look.row.list;
            let under = 10_000 + ((y - vy + at) / row) as u64;
            assert_eq!(
                selected(&rt).last(),
                Some(&under),
                "the row under the pointer"
            );
        }

        /// S2 and review round 2: the page passes its look's animations switch both to Rune
        /// (`Cx::reduced_motion`) and to the marquee, so they agree whatever the system says,
        /// and the edge scroll keeps the curve's speed (at most 1,500 pt/s). Only the look is
        /// flipped here, against the system preference each way: animations off with the
        /// system's reduced motion off, and on with it on.
        #[test]
        fn the_look_alone_decides_reduced_motion_and_the_edge_scroll_keeps_its_speed() {
            for (animations, system) in [(false, false), (true, true)] {
                let mut rt = mount("c1-photos-light");
                rt.set_system_reduced_motion(system);
                rt.update(|g, _| g.look.animations = animations);
                run(&mut rt, 0.1);
                let from = rt.state.c1.coll.scroll_offset();
                hold_at_the_bottom(&mut rt, 10_002, 1.0);
                let speed = rt.state.c1.coll.scroll_offset() - from;
                assert!(
                    (1_000.0..=1_550.0).contains(&speed),
                    "animations {animations}, system reduced motion {system}: \
                     {speed} pt in a second"
                );
            }
        }
    }
}

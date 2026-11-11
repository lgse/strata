//! C7's page in process: timers stepped from wake to wake, the wheel, the views kept mounted
//! under a spring, the title, and, through a host with a binder of its own (as X2's will be),
//! what Rune's binder can't send: drags from this process that open tabs, and content drops.
//! The scripts in `tests/scripts/c7/` check the rest. Each test names the bullets it covers.

mod c7 {
    use std::cell::RefCell;
    use std::time::Instant;

    use rune::{Cx, El, ElementInfo, Env, Key, KeyEvent, Live, Mods, PointerEvent, Runtime, Tx};
    use strata_components::contract::Dnd;
    use strata_components::contract::dnd::{
        DropActions, DropData, DropHover, DropItem, DropOffer, DropZone, Dropped, URI_LIST,
    };
    use strata_components::tabs::{
        Accepts, BrowserTabs, BrowserTabsState, DropSite, Tab, TabId, TabsData, TabsIntent, Via,
    };
    use strata_gallery::{Gallery, scenes};
    use strata_kit::icons::IconName;
    use strata_kit::look::Look;

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
        run(&mut rt, 1.0);
        rt
    }

    fn mount(name: &str) -> Runtime<Gallery> {
        mount_with(name, |_| {})
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
    }

    fn key(rt: &mut Runtime<Gallery>, key: Key, mods: Mods) {
        rt.key(KeyEvent { key, mods });
        rt.frame(FRAME);
    }

    /// Steps a still window for at most `limit` seconds: a frame only at each wake the page
    /// asked for, or for queued input (a layout report), none in between and none for motion
    /// (a held file drag keeps the runtime animating, which would hide a timer that asked for
    /// no wake: review 2). Stops at the first frame after which `done` holds, and returns the
    /// clock it stopped at.
    fn wake_until(rt: &mut Runtime<Gallery>, limit: f64, done: impl Fn(&Gallery) -> bool) -> f64 {
        let start = rt.state.c7.now;
        for _ in 0..400 {
            let now = rt.state.c7.now;
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
            // A deadline is due once the clock passes it: step at least a millisecond.
            rt.frame((deadline - now).max(0.001));
        }
        rt.state.c7.now
    }

    fn tab(id: u64) -> String {
        format!("t-c7-{id}")
    }

    // browser/tabs: "Holding a file or browser-content drag over a tab for 450 ms selects it, so
    // the drop can land on a folder in its listing." In a window at rest, which polls only at
    // the wakes it asked for (C11's and C6's reviews: a timer stamped at the first poll after
    // the hover, with one wake asked from the hover, never ended).
    #[test]
    fn the_hover_selects_a_tab_in_a_resting_window() {
        for animations in [true, false] {
            let mut rt = mount_with("c7-several-light", |g| g.look.animations = animations);
            let at = centre(&rt, &tab(2));
            rt.drag_files(vec!["/home/demo/report.pdf".into()], Some(at));
            rt.frame(FRAME);
            let arrived = rt.state.c7.now;
            // Only at wakes: a held file drag keeps the runtime animating, which a frame per
            // 1/60 s would follow, hiding a timer that asked for no wake (review 2).
            let stopped = wake_until(&mut rt, 3.0, |g| !g.c7.log.is_empty());
            assert_eq!(rt.state.c7.log, "Select Documents (spring)");
            assert_eq!(rt.state.c7.active, Some(TabId(2)));
            let took = stopped - arrived;
            assert!((0.44..0.5).contains(&took), "after {took:.3} s");
        }
    }

    // browser/tabs: "A Trash or Recent tab also refuses file drags: it neither accents nor
    // selects under the drag." And a drag resting on the active tab arms nothing.
    #[test]
    fn a_drag_on_the_active_tab_or_trash_selects_nothing() {
        let mut rt = mount("c7-several-light");
        for id in [3, 5] {
            rt.drag_files(
                vec!["/home/demo/report.pdf".into()],
                Some(centre(&rt, &tab(id))),
            );
            run(&mut rt, 1.0);
            assert_eq!(rt.state.c7.active, Some(TabId(3)));
            assert_eq!(rt.state.c7.log, "");
        }
        assert_eq!(
            rt.drop_hovered().as_deref(),
            Some("c7"),
            "Trash passes to the strip"
        );
        assert_eq!(rt.state.c7.state.drop_hover(), None, "nothing is tinted");
    }

    // P1's rule, the host's part: a spring that shows another tab during a drag leaves the
    // source tab's view mounted at its path and place (styled hidden), so a drag from it keeps
    // its capture.
    #[test]
    fn a_spring_keeps_the_previous_view_mounted_where_it_was() {
        let mut rt = mount("c7-several-light");
        let before = element(&rt, "c7-pane-3").unwrap();
        rt.drag_files(
            vec!["/home/demo/report.pdf".into()],
            Some(centre(&rt, &tab(2))),
        );
        run(&mut rt, 1.0);
        assert_eq!(rt.state.c7.active, Some(TabId(2)));
        let after = element(&rt, "c7-pane-3").expect("the previous view is still mounted");
        assert_eq!(after.path, before.path);
        assert_eq!(after.rect, before.rect);
        assert_eq!(after.opacity, 0.0);
    }

    // browser/tabs: "When the strip is wider than the window, the wheel scrolls it horizontally
    // and no scrollbar is shown." Rune turns no vertical wheel sideways (U-C6-5), so without
    // C7's mapping (`tabs/wheel.rs`) this fails.
    #[test]
    fn a_vertical_wheel_scrolls_an_overflowing_strip_sideways() {
        let mut rt = mount("c7-overflow-light");
        let first = rect(&rt, &tab(1))[0];
        let at = centre(&rt, &tab(22));
        rt.pointer_move(at.0, at.1);
        rt.wheel_event(rune::WheelEvent::new(0.0, 120.0).at(at.0, at.1));
        run(&mut rt, 0.2);
        let back = rect(&rt, &tab(1))[0];
        assert!(
            back > first + 100.0,
            "a wheel up shows the start: {first} -> {back}"
        );
        let at = centre(&rt, &tab(18));
        rt.pointer_move(at.0, at.1);
        rt.wheel_event(rune::WheelEvent::new(0.0, -80.0).at(at.0, at.1));
        run(&mut rt, 0.2);
        let on = rect(&rt, &tab(1))[0];
        assert!(on < back - 60.0, "a wheel down shows more: {back} -> {on}");
    }

    // browser/tabs: "Selecting a tab scrolls it into view", from the keyboard too (Ctrl+Shift+1,
    // the page's stand-in for Strata's key map).
    #[test]
    fn a_tab_selected_from_the_keyboard_scrolls_into_view() {
        let mut rt = mount("c7-overflow-light");
        let viewport = rect(&rt, "c7-scroll");
        assert!(
            rect(&rt, &tab(1))[0] < viewport[0],
            "tab 1 starts scrolled off"
        );
        let mods = Mods {
            ctrl: true,
            shift: true,
            ..Mods::default()
        };
        key(&mut rt, Key::Command('1'), mods);
        run(&mut rt, 1.0);
        assert_eq!(rt.state.c7.active, Some(TabId(1)));
        let t = rect(&rt, &tab(1));
        assert!(t[0] >= viewport[0] - 0.5, "{t:?} in {viewport:?}");
    }

    // browser/tabs: "Selecting a tab scrolls it into view", clear of the strip's edge fades, at
    // either end (review 1: a revealed tab landed under the fade, U-C7-7).
    #[test]
    fn a_revealed_tab_is_clear_of_the_edge_fades() {
        let mut rt = mount("c7-overflow-light");
        let fade = strata_kit::slots::tabs::fade_width(&rt.state.look);
        let viewport = rect(&rt, "c7-scroll");
        let t = rect(&rt, &tab(22));
        assert!(
            t[0] + t[2] <= viewport[0] + viewport[2] - fade + 0.5,
            "tab 22 {t:?} ends clear of the right fade in {viewport:?}"
        );
        let mods = Mods {
            ctrl: true,
            shift: true,
            ..Mods::default()
        };
        // Ctrl+Shift+0: tab 10, off to the left. In a window at rest: a frame while something
        // moves, else only at each wake the strip asked for.
        let start = rect(&rt, &tab(10))[0];
        key(&mut rt, Key::Command('0'), mods);
        let mut xs = vec![rect(&rt, &tab(10))[0]];
        for _ in 0..120 {
            if rt.animating {
                rt.frame(FRAME);
            } else if let Some(deadline) = rt.wake_deadline() {
                rt.frame((deadline - rt.state.c7.now).max(0.001));
            } else {
                break;
            }
            xs.push(rect(&rt, &tab(10))[0]);
        }
        assert_eq!(rt.state.c7.active, Some(TabId(10)));
        let t = rect(&rt, &tab(10));
        assert!(
            t[0] >= viewport[0] + fade - 0.5,
            "tab 10 {t:?} starts clear of the left fade in {viewport:?}"
        );
        // It lands in one step: the first frame that moves it puts it where it stays, with no
        // shift by the fade's width a frame later (review 2).
        let moved = xs.iter().find(|&&x| (x - start).abs() > 0.5).copied();
        assert_eq!(moved, Some(t[0]), "frames: {xs:?}");
    }

    // A host that changes the active tab from a key handler does so after that frame's poll.
    // In a window at rest with nothing moving, the strip asks for the tick that follows it, so
    // the tab is revealed and the window retitled with no other frame coming (review 2).
    #[test]
    fn a_key_selection_reveals_and_retitles_in_a_still_window() {
        let mut rt = mount_with("c7-overflow-light", |g| g.look.animations = false);
        let fade = strata_kit::slots::tabs::fade_width(&rt.state.look);
        let mods = Mods {
            ctrl: true,
            shift: true,
            ..Mods::default()
        };
        rt.key(KeyEvent {
            key: Key::Command('0'),
            mods,
        });
        // The frame the key brings, then only wakes: no frames for motion.
        rt.frame(FRAME);
        wake_until(&mut rt, 2.0, |_| false);
        let viewport = rect(&rt, "c7-scroll");
        let t = rect(&rt, &tab(10));
        assert!(
            t[0] >= viewport[0] + fade - 0.5,
            "tab 10 {t:?} in {viewport:?}"
        );
        assert_eq!(rt.state.c7.title, "Folder 10 — Strata");
    }

    /// Sends `deltas` over the tab under the viewport's middle, a frame after each, phased as a
    /// trackpad's gesture (`Began`, then `Moved`, then `Ended`) or unphased as a wheel's, and
    /// returns how far tab 1 moved on each frame after the first.
    fn gesture(rt: &mut Runtime<Gallery>, deltas: &[(f32, f32)], phased: bool) -> Vec<f32> {
        use rune::GesturePhase;
        let viewport = rect(rt, "c7-scroll");
        let mid = viewport[0] + viewport[2] / 2.0;
        let under = (1..=30)
            .map(|id| rect(rt, &tab(id)))
            .find(|r| (r[0]..r[0] + r[2]).contains(&mid))
            .expect("a tab under the middle");
        let at = (under[0] + under[2] / 2.0, under[1] + under[3] / 2.0);
        rt.pointer_move(at.0, at.1);
        let mut xs = Vec::new();
        for (i, &(dx, dy)) in deltas.iter().enumerate() {
            let mut ev = rune::WheelEvent::new(dx, dy).at(at.0, at.1).precise(true);
            if phased {
                let phase = if i == 0 {
                    GesturePhase::Began
                } else {
                    GesturePhase::Moved
                };
                ev = ev.phase(phase);
            }
            rt.wheel_event(ev);
            rt.frame(FRAME);
            xs.push(rect(rt, &tab(1))[0]);
        }
        if phased {
            let end = rune::WheelEvent::new(0.0, 0.0)
                .at(at.0, at.1)
                .phase(GesturePhase::Ended);
            rt.wheel_event(end);
        }
        run(rt, 0.3);
        xs.windows(2).map(|w| w[1] - w[0]).collect()
    }

    // A sideways gesture whose first delta lands in the frame a reveal's jump goes out: the
    // tab's handler takes it, so Rune hands it the whole gesture, and it scrolls every delta
    // after (review 3: the rest was dropped). Phased and unphased.
    #[test]
    fn a_sideways_gesture_through_the_reveal_frame_scrolls_to_its_end() {
        let mods = Mods {
            ctrl: true,
            shift: true,
            ..Mods::default()
        };
        for phased in [true, false] {
            let mut rt = mount("c7-overflow-light");
            rt.key(KeyEvent {
                key: Key::Command('0'),
                mods,
            });
            rt.frame(FRAME);
            rt.frame(FRAME);
            let moved = gesture(&mut rt, &[(-20.0, 0.0); 10], phased);
            assert!(
                moved.iter().all(|d| (d + 20.0).abs() < 2.0),
                "phased {phased}: {moved:?}"
            );
        }
    }

    // A gesture that starts mostly vertical (the tab's handler takes it) and turns sideways
    // keeps scrolling: its sideways deltas reach no one else. Phased and unphased.
    #[test]
    fn a_gesture_that_turns_sideways_keeps_scrolling() {
        for phased in [true, false] {
            let mut rt = mount("c7-overflow-light");
            let mut deltas = vec![(0.0, -20.0)];
            deltas.extend([(-20.0, 0.0); 8]);
            let moved = gesture(&mut rt, &deltas, phased);
            assert!(
                moved.iter().all(|d| (d + 20.0).abs() < 2.0),
                "phased {phased}: {moved:?}"
            );
        }
    }

    // Wheel deltas that reach a tab within one frame each scroll: the markers report the offset
    // once a frame, so the wheel's jumps add up from where the last one left the viewport.
    #[test]
    fn wheel_deltas_within_one_frame_add_up() {
        let mut rt = mount("c7-overflow-light");
        let at = centre(&rt, &tab(20));
        rt.pointer_move(at.0, at.1);
        let before = rect(&rt, &tab(1))[0];
        for _ in 0..3 {
            rt.wheel_event(rune::WheelEvent::new(0.0, 40.0).at(at.0, at.1));
        }
        run(&mut rt, 0.2);
        let after = rect(&rt, &tab(1))[0];
        assert!((after - before - 120.0).abs() < 2.0, "{before} -> {after}");
    }

    // The active tab renamed by a host's key handler (its folder navigated) retitles the window
    // in a still window too: the change lands after that frame's poll (review 3).
    #[test]
    fn renaming_the_active_tab_retitles_a_still_window() {
        let mut rt = mount_with("c7-several-light", |g| g.look.animations = false);
        assert_eq!(rt.state.c7.title, "Music — Strata");
        // F8: the page's stand-in for the active tab's folder changing.
        key(&mut rt, Key::Function(8), Mods::default());
        wake_until(&mut rt, 2.0, |_| false);
        assert_eq!(rt.state.c7.title, "Music/Albums — Strata");
    }

    // The user's wheel, vertical or sideways, right after a selection wins over its reveal
    // still to land (review 2: the reveal's jump undid it).
    #[test]
    fn a_wheel_right_after_a_selection_beats_its_reveal() {
        let mods = Mods {
            ctrl: true,
            shift: true,
            ..Mods::default()
        };
        for (frames, dx, dy) in [
            (0, 0.0, -200.0),
            (1, 0.0, -200.0),
            (2, 0.0, -200.0),
            (0, -200.0, 0.0),
            (1, -200.0, 0.0),
            (2, -200.0, 0.0),
        ] {
            let mut rt = mount("c7-overflow-light");
            // Ctrl+Shift+0 selects tab 10, off to the left; the wheel then asks for more on
            // the right.
            rt.key(KeyEvent {
                key: Key::Command('0'),
                mods,
            });
            for _ in 0..frames {
                rt.frame(FRAME);
            }
            // Over the tab under the viewport's middle.
            let viewport = rect(&rt, "c7-scroll");
            let mid = viewport[0] + viewport[2] / 2.0;
            let under = (1..=30)
                .map(|id| rect(&rt, &tab(id)))
                .find(|r| (r[0]..r[0] + r[2]).contains(&mid))
                .expect("a tab under the middle");
            let at = (under[0] + under[2] / 2.0, under[1] + under[3] / 2.0);
            rt.pointer_move(at.0, at.1);
            let before = rect(&rt, &tab(1))[0];
            rt.wheel_event(rune::WheelEvent::new(dx, dy).at(at.0, at.1));
            run(&mut rt, 0.5);
            let after = rect(&rt, &tab(1))[0];
            assert!(
                after < before - 100.0,
                "wheel ({dx}, {dy}) {frames} frames after: tab 1 {before} -> {after}"
            );
        }
    }

    /// The width a tab gives its title, from rune-kit's `title` element inside it.
    fn title_width(rt: &Runtime<Gallery>, tab: &str) -> f32 {
        rt.elements()
            .into_iter()
            .find(|e| {
                e.key.as_deref() == Some("title")
                    && !e.exiting
                    && e.path.contains(&format!("/{tab}/"))
            })
            .unwrap_or_else(|| panic!("no title in {tab}"))
            .rect[2]
    }

    // browser/tabs: "Names longer than 22 characters are ellipsized", so names up to there show:
    // at the narrowest, a tab still leaves room for about 10 characters of its label, active
    // (clear of its close button) or at rest (review 1: at 96 points they read "Folder").
    #[test]
    fn the_narrowest_tab_still_shows_about_ten_characters() {
        let rt = mount("c7-overflow-light");
        let ten = 10.0 * rt.state.look.kit.ty.ui_compact.size * 0.6;
        for id in [22, 18] {
            let w = title_width(&rt, &tab(id));
            assert!(w >= ten - 0.5, "tab {id}'s title has {w} of {ten} points");
        }
    }

    fn underline_x(rt: &Runtime<Gallery>) -> f32 {
        rt.elements()
            .into_iter()
            .filter(|e| e.key.as_deref() == Some("underline") && !e.exiting)
            .map(|e| e.rect[0])
            .fold(f32::NAN, f32::max)
    }

    // browser/tabs: "A 3 px accent underline slides to the selected tab, and jumps without
    // sliding when animations are disabled."
    #[test]
    fn the_underline_slides_and_jumps_with_animations_off() {
        for animations in [true, false] {
            let mut rt = mount_with("c7-several-light", |g| g.look.animations = animations);
            let from = underline_x(&rt);
            tap(&mut rt, &tab(5));
            rt.frame(FRAME);
            let next = underline_x(&rt);
            run(&mut rt, 1.0);
            let to = underline_x(&rt);
            assert!(to > from + 100.0);
            if animations {
                assert!(next > from && next < to - 1.0, "slides: {from} {next} {to}");
            } else {
                assert!((next - to).abs() < 0.5, "jumps: {from} {next} {to}");
            }
        }
    }

    // browser/tabs: "Navigating the active tab, or selecting another tab, retitles the window
    // after the active tab's folder", through `Tx::set_title`: the window's accessible name is
    // its title.
    #[test]
    fn selecting_a_tab_retitles_the_window() {
        let mut rt = mount("c7-several-light");
        assert_eq!(rt.accessibility_json()["name"], "Music — Strata");
        tap(&mut rt, &tab(4));
        run(&mut rt, 0.1);
        let long = "Quarterly reports and invoices 2026 — Strata";
        assert_eq!(rt.accessibility_json()["name"], long, "the whole name");
    }

    // ---- A host with a binder of its own, as X2's will be. ----

    /// A zone the binder attached, with where its element drew.
    type Attached = (Live<[f32; 4]>, DropZone<Host>);

    thread_local! {
        /// The zones the binder attached in the last view.
        static ZONES: RefCell<Vec<Attached>> = const { RefCell::new(Vec::new()) };
    }

    fn binder() -> Dnd<Host> {
        Dnd::custom(
            |el: El, zone| {
                let rect = Live::new([0.0; 4]);
                ZONES.with(|z| z.borrow_mut().push((rect.clone(), zone)));
                el.drawn_rect_into(&rect)
            },
            |el, _| el,
        )
    }

    /// The innermost zone at `(x, y)`, as X2's binder finds it by drawn rect.
    fn zone_at(x: f32, y: f32) -> DropZone<Host> {
        ZONES.with(|z| {
            z.borrow()
                .iter()
                .filter(|(r, _)| {
                    let [rx, ry, w, h] = r.get();
                    (rx..=rx + w).contains(&x) && (ry..=ry + h).contains(&y)
                })
                .min_by(|(a, _), (b, _)| {
                    let area = |r: [f32; 4]| r[2] * r[3];
                    area(a.get()).total_cmp(&area(b.get()))
                })
                .map(|(_, zone)| zone.clone())
                .expect("a zone there")
        })
    }

    struct Host {
        look: Look,
        tabs: Vec<(u64, &'static str, Accepts)>,
        active: Option<TabId>,
        tabs_state: BrowserTabsState,
        heard: Vec<TabsIntent>,
        /// Rune's own binder instead of the recording one.
        rune_binder: bool,
        /// Whether the host's own drags open tabs (it sets `opens_tabs`).
        opens: bool,
        /// Whether every drag opens tabs, so one under Rune's binder (from another process,
        /// as Rune sees it) does.
        opens_any: bool,
        /// The clock at the last poll.
        now: f64,
    }

    fn host(n: u64) -> Host {
        let mut tabs: Vec<_> = (1..=n).map(|i| (i, "folder", Accepts::ALL)).collect();
        if let Some(t) = tabs.get_mut(1) {
            t.2 = Accepts::NONE;
        }
        if let Some(t) = tabs.get_mut(2) {
            t.2 = Accepts::FILES;
        }
        Host {
            look: Look::fallback(false),
            tabs,
            active: Some(TabId(1)),
            tabs_state: BrowserTabsState::default(),
            heard: Vec::new(),
            rune_binder: false,
            opens: true,
            opens_any: false,
            now: 0.0,
        }
    }

    fn data_tabs(h: &Host) -> Vec<Tab<'static>> {
        h.tabs
            .iter()
            .map(|&(id, name, accepts)| {
                Tab::new(TabId(id), name, IconName::Folder).accepts(accepts)
            })
            .collect()
    }

    fn host_view(h: &Host, cx: &mut Cx<Host>) -> El {
        ZONES.with(|z| z.borrow_mut().clear());
        let tabs = data_tabs(h);
        let data = TabsData::new(&tabs, h.active);
        let strip = BrowserTabs::new(
            &data,
            &h.tabs_state,
            |h: &mut Host| &mut h.tabs_state,
            |h| &mut h.tabs_state.drag,
        )
        .dnd(if h.rune_binder { Dnd::rune() } else { binder() })
        .content_types(&["text/plain"])
        .on_intent(|h: &mut Host, intent, _| h.heard.push(intent));
        // The host's own drags (from this process) open tabs here.
        let any = h.opens_any;
        let strip = if h.opens {
            strip.opens_tabs(move |_, offer| any || offer.local)
        } else {
            strip
        };
        let strip = strip.build(&h.look, cx);
        rune::col().w_full().h_full().child(strip)
    }

    fn host_poll(h: &mut Host, t: f64, tx: &mut Tx) -> bool {
        h.now = t;
        let tabs = data_tabs(h);
        let polled = h.tabs_state.tick(t, &TabsData::new(&tabs, h.active), tx);
        let changed = !polled.intents.is_empty();
        h.heard.extend(polled.intents);
        changed
    }

    fn mount_host(h: Host) -> Runtime<Host> {
        let mut rt = Runtime::new(h, host_view, (960.0, 200.0), Env::new(false, 1.0));
        rt.poll = Some(host_poll);
        rt.frame(0.0);
        run(&mut rt, 0.5);
        rt
    }

    fn local_folders() -> DropOffer {
        DropOffer::new(vec![URI_LIST.into()], DropActions::COPY | DropActions::MOVE)
            .local(true)
            .serial(7)
    }

    fn point(x: f32, y: f32) -> PointerEvent {
        // Non-exhaustive: X2's binder gets one from P1; only the window point matters here.
        let mut p = PointerEvent::default();
        p.window_x = x;
        p.window_y = y;
        p
    }

    fn opened(h: &Host) -> Vec<usize> {
        h.heard
            .iter()
            .filter_map(|i| match i {
                TabsIntent::OpenTabs { at, .. } => Some(*at),
                _ => None,
            })
            .collect()
    }

    fn springs(h: &Host) -> usize {
        h.heard
            .iter()
            .filter(|i| {
                matches!(
                    i,
                    TabsIntent::Select {
                        via: Via::Spring,
                        ..
                    }
                )
            })
            .count()
    }

    // browser/tabs: "Over the strip, that line moves to the gap nearest the pointer. Releasing
    // opens the dragged location in a new tab at that gap"; "A folder-only drag released over a
    // tab header opens a new tab and moves or copies nothing into that tab"; "A folder-only
    // drag held over a tab does not select it". The gap is rune-kit's incoming gap.
    #[test]
    fn a_drag_that_opens_tabs_places_a_gap_and_opens_there() {
        let mut rt = mount_host(host(5));
        // A quarter into tab 1: before its middle (even once the gap shrinks the tabs), so it
        // opens at 0, not after the tab it dropped on.
        let t1 = rect(&rt, "t-tabs-1");
        let (x, y) = (t1[0] + t1[2] * 0.25, t1[1] + t1[3] / 2.0);
        let zone = zone_at(x, y);
        let offer = local_folders();
        rt.update(|h, tx| {
            zone.hover(h, DropHover::Accepting, &offer, tx);
            zone.moved(h, point(x, y), tx);
        });
        run(&mut rt, 1.0);
        assert_eq!(rt.state.tabs_state.drag.incoming_slot(), Some(0));
        assert!(
            element(&rt, "tabs-list-incoming").is_some(),
            "the gap shows"
        );
        assert_eq!(rt.state.tabs_state.drop_hover(), None, "no tab is tinted");
        assert_eq!(springs(&rt.state), 0, "no tab is selected");
        let dropped = Dropped::new(
            offer.clone(),
            DropData::files(&["/srv/a/".into()]),
            Mods::default(),
        );
        // Both real binders report the zone's leave before they deliver its drop (Rune's
        // `runtime/input/drop.rs`, P1's `dnd/window.rs`): W1's `Left` is "left, moved, or dropped".
        rt.update(|h, tx| {
            zone.hover(h, DropHover::Left, &offer, tx);
            zone.deliver(h, dropped, tx);
        });
        run(&mut rt, 0.5);
        assert_eq!(opened(&rt.state), [0]);
        assert!(
            !rt.state
                .heard
                .iter()
                .any(|i| matches!(i, TabsIntent::DropInto { .. }))
        );
        assert!(
            element(&rt, "tabs-list-incoming").is_none(),
            "the gap closes"
        );
    }

    // The same over the strip's own zone, between two tabs (review 1, B1: the drop's leave
    // closed the gap, and the drop opened at the end).
    #[test]
    fn a_drop_in_the_gap_between_tabs_opens_there_after_its_leave() {
        let mut rt = mount_host(host(5));
        let t2 = rect(&rt, "t-tabs-2");
        let gap = (t2[0] + t2[2] + 2.0, t2[1] + t2[3] / 2.0);
        let strip = zone_at(gap.0, gap.1);
        rt.update(|h, tx| {
            strip.hover(h, DropHover::Accepting, &local_folders(), tx);
            strip.moved(h, point(gap.0, gap.1), tx);
        });
        run(&mut rt, 0.5);
        assert_eq!(rt.state.tabs_state.drag.incoming_slot(), Some(2));
        let dropped = Dropped::new(
            local_folders(),
            DropData::files(&["/srv/a/".into()]),
            Mods::default(),
        );
        rt.update(|h, tx| {
            strip.hover(h, DropHover::Left, &local_folders(), tx);
            strip.deliver(h, dropped, tx);
        });
        run(&mut rt, 0.2);
        assert_eq!(opened(&rt.state), [2]);
    }

    // A drag that leaves a tab's gap for the strip's and drops there, all in one turn: the
    // strip's gap hasn't built, so its slot isn't known, and the drop keeps the tab's rather
    // than falling to the end (review 2).
    #[test]
    fn a_drop_on_a_zone_reached_in_the_same_turn_keeps_the_last_slot() {
        let mut rt = mount_host(host(5));
        let t1 = rect(&rt, "t-tabs-1");
        let (x, y) = (t1[0] + t1[2] * 0.25, t1[1] + t1[3] / 2.0);
        let tab_zone = zone_at(x, y);
        rt.update(|h, tx| {
            tab_zone.hover(h, DropHover::Accepting, &local_folders(), tx);
            tab_zone.moved(h, point(x, y), tx);
        });
        run(&mut rt, 0.5);
        assert_eq!(rt.state.tabs_state.drag.incoming_slot(), Some(0));
        let t2 = rect(&rt, "t-tabs-2");
        let gap = (t2[0] + t2[2] + 2.0, t2[1] + t2[3] / 2.0);
        let strip = zone_at(gap.0, gap.1);
        let dropped = Dropped::new(
            local_folders(),
            DropData::files(&["/srv/a/".into()]),
            Mods::default(),
        );
        rt.update(|h, tx| {
            tab_zone.hover(h, DropHover::Left, &local_folders(), tx);
            strip.hover(h, DropHover::Accepting, &local_folders(), tx);
            strip.moved(h, point(gap.0, gap.1), tx);
            strip.hover(h, DropHover::Left, &local_folders(), tx);
            strip.deliver(h, dropped, tx);
        });
        assert_eq!(opened(&rt.state), [0]);
    }

    // A leave with no drop keeps the gap's slot only briefly, and it expires in a window at
    // rest too: it asks for the wakes that expire it (review 2: it relied on frames that
    // happened to come).
    #[test]
    fn a_kept_slot_expires_from_wake_to_wake() {
        // Rune's binder, so the leave and the drop arrive as input inside frames, as a real
        // binder's do, after that frame's poll. Three tabs keep their widest width with the gap
        // open or shut, so closing it moves nothing that would bring a frame of its own (the
        // active tab's marker reports its moves): only the leave's wakes expire the slot.
        let mut h = host(3);
        h.rune_binder = true;
        h.opens_any = true;
        let mut rt = mount_host(h);
        let t2 = rect(&rt, "t-tabs-2");
        let gap = (t2[0] + t2[2] + 2.0, t2[1] + t2[3] / 2.0);
        let folder = || vec![std::path::PathBuf::from("/srv/a/")];
        rt.drag_files(folder(), Some(gap));
        run(&mut rt, 0.5);
        assert_eq!(rt.state.tabs_state.drag.incoming_slot(), Some(2));
        rt.drag_files_leave();
        // As a still window draws: a frame only for queued input or at a wake, none for motion
        // and none of the test's own (review 3: one stepped here hid the leave's wake).
        for _ in 0..20 {
            if rt.has_pending_input() {
                rt.frame(FRAME);
            } else if let Some(deadline) = rt.wake_deadline() {
                rt.frame((deadline - rt.state.now).max(0.001));
            } else {
                break;
            }
        }
        // A later drop with no hover of its own opens at the end, not in the old gap.
        rt.drop_files(folder(), Some(gap));
        rt.frame(FRAME);
        assert_eq!(opened(&rt.state), [3], "the kept slot expired");
    }

    // browser/tabs: "After an Escape-cancelled drag, the strip shows no insertion line": the
    // binder reports a cancelled drag, or one that left the strip, as a leave with no drop.
    #[test]
    fn a_leave_closes_the_gap() {
        let mut rt = mount_host(host(5));
        let (x, y) = centre(&rt, "t-tabs-4");
        let zone = zone_at(x, y);
        rt.update(|h, tx| {
            zone.hover(h, DropHover::Accepting, &local_folders(), tx);
            zone.moved(h, point(x, y), tx);
        });
        run(&mut rt, 0.3);
        assert!(element(&rt, "tabs-list-incoming").is_some());
        rt.update(|h, tx| zone.hover(h, DropHover::Left, &local_folders(), tx));
        rt.frame(FRAME);
        assert!(
            element(&rt, "tabs-list-incoming").is_none(),
            "it closes at once"
        );
        run(&mut rt, 0.5);
        assert_eq!(rt.state.tabs_state.drag.incoming_slot(), None);
        // The slot the leave kept for a drop expired: a later drop with no hover of its own
        // (only headless runs send one) opens at the end, not in the old gap.
        let t1 = rect(&rt, "t-tabs-1");
        let strip = zone_at(t1[0] + t1[2] + 2.0, t1[1] + t1[3] / 2.0);
        let dropped = Dropped::new(
            local_folders(),
            DropData::files(&["/srv/a/".into()]),
            Mods::default(),
        );
        rt.update(|h, tx| strip.deliver(h, dropped, tx));
        assert_eq!(opened(&rt.state), [5]);
    }

    // Moving from a tab to the strip's gap beside it: the binder may report the strip's arrival
    // before the tab's leave. Only the zone holding the gap closes it, so it never shuts for a
    // frame and opens again.
    #[test]
    fn a_leave_from_a_tab_after_the_strip_took_the_drag_keeps_the_gap() {
        let mut rt = mount_host(host(5));
        let t4 = rect(&rt, "t-tabs-4");
        let (x, y) = (t4[0] + t4[2] - 1.0, t4[1] + t4[3] / 2.0);
        let tab_zone = zone_at(x, y);
        let gap = (t4[0] + t4[2] + 2.0, y);
        let strip = zone_at(gap.0, gap.1);
        rt.update(|h, tx| {
            tab_zone.hover(h, DropHover::Accepting, &local_folders(), tx);
            tab_zone.moved(h, point(x, y), tx);
        });
        run(&mut rt, 0.3);
        rt.update(|h, tx| {
            strip.hover(h, DropHover::Accepting, &local_folders(), tx);
            strip.moved(h, point(gap.0, gap.1), tx);
            tab_zone.hover(h, DropHover::Left, &local_folders(), tx);
        });
        rt.frame(FRAME);
        assert!(element(&rt, "tabs-list-incoming").is_some(), "the gap held");
        assert!(rt.state.tabs_state.drag.incoming_slot().is_some());
    }

    // browser/tabs: "Text, an image, or a web address dropped on a tab from another application
    // goes to that tab's current folder"; "A tab showing Trash, Recent, or a remote folder
    // refuses dropped text and images"; "Text dragged from another application onto the strip
    // is never taken as a tab reorder. The tab header under it receives it as a content drop."
    #[test]
    fn content_goes_into_a_tab_that_takes_it_and_no_other() {
        let rt = mount_host(host(4));
        let text = DropOffer::new(vec!["text/plain".into()], DropActions::COPY);
        let (x, y) = centre(&rt, "t-tabs-4");
        let zone = zone_at(x, y);
        assert!(zone.accepts(&text));
        let mut rt = rt;
        let data = DropData::new(vec![DropItem::new("text/plain", "hello")]);
        let dropped = Dropped::new(text.clone(), data, Mods::default());
        rt.update(|h, tx| zone.deliver(h, dropped, tx));
        let into = rt.state.heard.iter().find_map(|i| match i {
            TabsIntent::DropInto { id, dropped } => Some((*id, dropped.data.get("text/plain"))),
            _ => None,
        });
        assert_eq!(into, Some((TabId(4), Some(&b"hello"[..]))));
        assert!(
            rt.state.tabs_state.drag.phase() == Default::default(),
            "no reorder began"
        );
        // Tab 3 is remote (files only), tab 2 is Trash (nothing: no zone of its own).
        let (x, y) = centre(&rt, "t-tabs-3");
        assert!(!zone_at(x, y).accepts(&text));
        let (x, y) = centre(&rt, "t-tabs-2");
        let behind = zone_at(x, y);
        assert!(!behind.accepts(&text));
        assert!(
            behind
                .choose(&rt.state, &local_folders(), Mods::default())
                .is_some()
        );
        assert_eq!(
            behind.choose(
                &rt.state,
                &DropOffer::new(vec![URI_LIST.into()], DropActions::COPY),
                Mods::default()
            ),
            None,
            "the strip refuses a drag that opens no tabs"
        );
    }

    // browser/tabs: "The New tab button under a sidebar, folder, or tab drag shows an accent
    // tint", and its drop opens at the end.
    #[test]
    fn new_tab_lights_and_opens_a_drag_that_opens_tabs_at_the_end() {
        let mut rt = mount_host(host(3));
        let (x, y) = centre(&rt, "tabs-new");
        let zone = zone_at(x, y);
        rt.update(|h, tx| zone.hover(h, DropHover::Accepting, &local_folders(), tx));
        rt.frame(FRAME);
        assert_eq!(
            rt.state.tabs_state.drop_hover(),
            Some((DropSite::NewTab, DropHover::Accepting))
        );
        let dropped = Dropped::new(
            local_folders(),
            DropData::files(&["/srv/b/".into()]),
            Mods::default(),
        );
        rt.update(|h, tx| zone.deliver(h, dropped, tx));
        rt.frame(FRAME);
        assert_eq!(opened(&rt.state), [3]);
        assert_eq!(rt.state.tabs_state.drop_hover(), None);
    }

    // Review 1: a zone that follows moves asks for a frame on each motion of a drag over it, so
    // only a strip whose host has drags that open tabs (which place the gap) follows them.
    #[test]
    fn without_drags_that_open_tabs_no_zone_follows_moves() {
        let mut h = host(4);
        h.opens = false;
        let rt = mount_host(h);
        ZONES.with(|z| {
            assert!(!z.borrow().is_empty());
            assert!(z.borrow().iter().all(|(_, zone)| !zone.follows_moves()));
        });
        drop(rt);
        let rt = mount_host(host(4));
        let (x, y) = centre(&rt, "t-tabs-4");
        assert!(
            zone_at(x, y).follows_moves(),
            "a tab follows a drag that opens tabs"
        );
        let (x, y) = centre(&rt, "tabs-new");
        assert!(
            !zone_at(x, y).follows_moves(),
            "New tab opens at the end, wherever it is"
        );
    }

    /// The median time, in milliseconds, of a frame of a reorder drag across a strip of `n` tabs.
    fn drag_frames(n: u64, rune_binder: bool) -> f64 {
        let mut h = host(n);
        h.rune_binder = rune_binder;
        for t in &mut h.tabs {
            t.2 = Accepts::ALL;
        }
        let mut rt = mount_host(h);
        let (x, y) = centre(&rt, "t-tabs-2");
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        let mut times = Vec::new();
        for i in 1..=60 {
            rt.pointer_move(x + i as f32 * 4.0, y);
            let start = Instant::now();
            rt.frame(FRAME);
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        rt.pointer_up(x + 240.0, y);
        rt.frame(FRAME);
        assert!(
            rt.state
                .heard
                .iter()
                .any(|i| matches!(i, TabsIntent::Reorder { .. })),
            "the drag reordered"
        );
        times.sort_by(f64::total_cmp);
        times[times.len() / 2]
    }

    // Many tabs, reorder drags: a frame of a drag across 200 tabs stays under the bar (debug
    // build, in process: build, layout and the frame's work, no GPU). 1,000 is reported.
    #[test]
    fn a_reorder_drag_across_many_tabs_stays_quick() {
        let two_hundred = drag_frames(200, true);
        let thousand = drag_frames(1000, true);
        let recording = drag_frames(200, false);
        eprintln!(
            "c7 perf: reorder-drag frame median {two_hundred:.2} ms at 200 tabs, {thousand:.2} ms at 1,000 (Rune's binder); {recording:.2} ms at 200 with a binder that watches each zone's rect"
        );
        assert!(
            two_hundred < 50.0,
            "{two_hundred:.2} ms a frame at 200 tabs"
        );
    }
}

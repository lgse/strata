//! C22's page in process: the peek's waits stepped from wake to wake in a resting window
//! (animations on and off), what the host hears, and what assistive technology hears of a
//! keyboard peek. The scripts in `tests/scripts/c22/` check the bullets' end states.

mod c22 {
    use rune::runtime::ElementInfo;
    use rune::{Env, Key, KeyEvent, Mods, Runtime};
    use strata_components::collection::element::row_key;
    use strata_components::contract::EntryId;
    use strata_components::folder_peek::{Origin, PeekStatus, card_key};
    use strata_gallery::scenes::c22::{ICONS, LIST, Mode};
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
        for _ in 0..60 {
            rt.frame(FRAME);
        }
        rt
    }

    fn element(rt: &Runtime<Gallery>, key: &str) -> Option<ElementInfo> {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
    }

    fn centre(rt: &Runtime<Gallery>, key: &str) -> (f32, f32) {
        let [x, y, w, h] = element(rt, key)
            .unwrap_or_else(|| panic!("no element {key}"))
            .visible;
        (x + w / 2.0, y + h / 2.0)
    }

    /// Moves the pointer to the middle of `key`, and runs the frame that hears it.
    fn point(rt: &mut Runtime<Gallery>, key: &str) {
        let (x, y) = centre(rt, key);
        rt.pointer_move(x, y);
        rt.frame(FRAME);
    }

    /// Moves the pointer to a blank spot below the rows.
    fn point_away(rt: &mut Runtime<Gallery>) {
        rt.pointer_move(200.0, 600.0);
        rt.frame(FRAME);
    }

    fn command(rt: &mut Runtime<Gallery>, c: char) {
        rt.key(KeyEvent {
            key: Key::Command(c),
            mods: Mods::default(),
        });
        rt.frame(FRAME);
    }

    /// Steps a still window for at most `limit` seconds: a frame only at each wake the page or
    /// Rune asked for (or for queued input), none in between and none for motion, which would
    /// hide a wait that asked for no wake (C7's review 2). Stops once `done` holds; returns the
    /// seconds from the start to the frame it stopped at.
    fn wake_until(rt: &mut Runtime<Gallery>, limit: f64, done: impl Fn(&Gallery) -> bool) -> f64 {
        let start = rt.state.c22.clock;
        for _ in 0..400 {
            let now = rt.state.c22.clock;
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
        rt.state.c22.clock - start
    }

    fn open(g: &Gallery) -> Option<EntryId> {
        g.c22.peek.target().map(|t| t.folder)
    }

    /// Home's entry ids: Projects 1, Music 2, Photos 3, Empty 4, Locked 5, Slow 6,
    /// Archive 7, Downloads 8, the long-named folder 13, then the files 9 to 12.
    fn id(n: u64) -> EntryId {
        EntryId::new(n)
    }

    fn with_animations(on: bool) -> impl FnOnce(&mut Gallery) {
        move |g: &mut Gallery| g.look.animations = on
    }

    /// The row (List) or tile (Icons) of entry `n`.
    fn item(n: u64) -> String {
        row_key(id(n))
    }

    // preview/quick-preview/folder-peek: "With Folder peeking on, hovering a folder in Icons or
    // List for 500 ms opens a popover headed by the folder's name." In a resting window, which
    // runs frames only at the wakes asked for (C11's, C6's and C7's timer reviews), in both
    // views, with animations on and off; and the host's listing lands on its own wake.
    #[test]
    fn a_folder_peeks_500_ms_after_the_pointer_rests_on_it_in_a_resting_window() {
        for (scene, scope) in [("c22-list-light", LIST), ("c22-icons-light", ICONS)] {
            for animations in [true, false] {
                let mut rt = mount_with(scene, with_animations(animations));
                point(&mut rt, &item(2));
                let took = wake_until(&mut rt, 2.0, |g| open(g).is_some());
                assert!(
                    (0.48..0.52).contains(&took),
                    "{scene}, animations {animations}: opened after {took:.3} s"
                );
                let t = rt.state.c22.peek.target().unwrap();
                assert_eq!(
                    (t.folder, &*t.collection, t.origin),
                    (id(2), scope, Origin::Hover)
                );
                assert_eq!(rt.state.c22.heard, "Peek Music");
                let took = wake_until(&mut rt, 1.0, |g| g.c22.peeked.status == PeekStatus::Ready);
                assert!((0.09..0.12).contains(&took), "listed after {took:.3} s");
                rt.frame(FRAME);
                let heading = element(&rt, "peek-heading").expect("the card's heading");
                assert_eq!(heading.text.as_deref(), Some("Music"));
                assert!(element(&rt, &card_key(scope)).is_some());
            }
        }
    }

    // preview/quick-preview/folder-peek: "Moving the pointer from the folder onto the popover
    // keeps it open; leaving both closes it after 80 ms."
    #[test]
    fn the_card_holds_the_peek_and_leaving_both_closes_it_80_ms_later() {
        for animations in [true, false] {
            let mut rt = mount_with("c22-icons-light", with_animations(animations));
            point(&mut rt, &item(2));
            wake_until(&mut rt, 1.0, |g| open(g).is_some());
            rt.frame(FRAME);
            let serial = rt.state.c22.peek.target().unwrap().serial;
            point(&mut rt, &card_key(ICONS));
            wake_until(&mut rt, 1.0, |_| false);
            assert_eq!(open(&rt.state), Some(id(2)), "the card holds it");

            // Back onto its folder, then off the card: the same peek stays, never closed.
            point(&mut rt, &item(2));
            wake_until(&mut rt, 1.0, |_| false);
            assert_eq!(open(&rt.state), Some(id(2)), "its folder holds it again");
            assert_eq!(rt.state.c22.peek.target().unwrap().serial, serial);
            assert_eq!(rt.state.c22.events, [format!("open Music #{serial}")]);

            point_away(&mut rt);
            let took = wake_until(&mut rt, 1.0, |g| open(g).is_none());
            assert!(
                (0.06..0.1).contains(&took),
                "animations {animations}: closed after {took:.3} s"
            );
            assert_eq!(rt.state.c22.heard, "Peek closed");
            wake_until(&mut rt, 1.0, |_| false);
            assert_eq!(rt.wake_deadline(), None, "nothing left waiting");
        }
    }

    // preview/quick-preview/folder-peek: "Leaving a folder before the delay elapses opens no
    // peek." Nothing is left waiting either.
    #[test]
    fn a_pass_shorter_than_the_delay_opens_nothing_and_leaves_no_wake() {
        let mut rt = mount_with("c22-list-light", |_| {});
        point(&mut rt, &item(2));
        rt.frame(0.3);
        point(&mut rt, &item(9));
        wake_until(&mut rt, 2.0, |_| false);
        assert_eq!(open(&rt.state), None);
        assert_eq!(rt.wake_deadline(), None);
    }

    // preview/quick-preview/folder-peek: "Turning Folder peeking off closes an open peek and
    // cancels a pending one."
    #[test]
    fn turning_peeking_off_closes_the_peek_and_ends_a_pending_wait() {
        let mut rt = mount_with("c22-list-light", |_| {});
        point(&mut rt, &item(2));
        wake_until(&mut rt, 1.0, |g| open(g).is_some());
        command(&mut rt, 'p');
        assert_eq!(open(&rt.state), None);
        assert!(!rt.state.c22.peek.enabled());
        command(&mut rt, 'p');

        // Pending: off 0.2 s in, then the pointer stays.
        point(&mut rt, &item(3));
        rt.frame(0.2);
        command(&mut rt, 'p');
        wake_until(&mut rt, 2.0, |_| false);
        assert_eq!(open(&rt.state), None);
        // On again with the pointer still there: nothing, as in Strata, whose peek starts only
        // from the pointer arriving on a folder (coordinator ruling; c22_round1 checks the
        // arrival).
        command(&mut rt, 'p');
        wake_until(&mut rt, 2.0, |_| false);
        assert_eq!(open(&rt.state), None);
    }

    // preview/quick-preview/folder-peek: "Columns never shows a folder peek, whatever the saved
    // preference, and switching to Columns closes an open or pending peek."
    #[test]
    fn switching_to_columns_closes_the_peek_and_columns_never_peeks() {
        let mut rt = mount_with("c22-list-light", |_| {});
        point(&mut rt, &item(2));
        wake_until(&mut rt, 1.0, |g| open(g).is_some());
        command(&mut rt, '3');
        // The commit unmounts the rows; Rune runs the leave it queued on the next frame.
        rt.frame(FRAME);
        assert_eq!(rt.state.c22.mode, Mode::Columns);
        assert_eq!(open(&rt.state), None);
        assert_eq!(rt.state.c22.heard, "Peek closed");

        // Pending, then Columns, where nothing waits on a folder.
        command(&mut rt, '1');
        rt.frame(FRAME);
        point(&mut rt, &item(3));
        rt.frame(0.2);
        command(&mut rt, '3');
        wake_until(&mut rt, 2.0, |_| false);
        assert_eq!(open(&rt.state), None);
        point(&mut rt, &item(2));
        wake_until(&mut rt, 2.0, |_| false);
        assert_eq!(open(&rt.state), None, "no peek in Columns");
    }

    // integration/10xer-mode: "In Icons and List, `i` on a directory toggles the folder peek
    // without moving focus" (the key is the page's, as Strata's 10xer keymap): it opens at
    // once, stays when the pointer is elsewhere, and is announced (owner ruling: a keyboard
    // peek is a polite live region). Escape closes it first.
    #[test]
    fn the_keyboard_peek_opens_at_once_is_announced_and_escape_closes_it_first() {
        let mut rt = mount_with("c22-list-light", |g| {
            let d = &mut g.c22;
            d.list.selection.select_only(&d.home, 1);
        });
        let i = KeyEvent {
            key: Key::Text("i".into()),
            mods: Mods::default(),
        };
        rt.key(i.clone());
        rt.frame(FRAME);
        let t = rt.state.c22.peek.target().expect("open at once");
        assert_eq!((t.folder, t.origin), (id(2), Origin::Keyboard));
        wake_until(&mut rt, 1.0, |g| g.c22.peeked.status == PeekStatus::Ready);
        rt.frame(FRAME);
        let tree = rt.accessibility_tree();
        let card = tree
            .nodes
            .iter()
            .map(|(_, n)| n)
            .find(|n| n.label() == Some("Folder peek"))
            .expect("the card's group");
        assert_eq!(
            format!("{:?}", card.live()),
            "Some(Polite)",
            "the card itself"
        );
        point_away(&mut rt);
        wake_until(&mut rt, 1.0, |_| false);
        assert_eq!(open(&rt.state), Some(id(2)), "the pointer doesn't close it");

        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        rt.frame(FRAME);
        assert_eq!(open(&rt.state), None);
        assert_eq!(rt.state.page, Some("c22"), "Escape stopped at the peek");

        // `i` on a file opens nothing.
        rt.update(|g, _| {
            let d = &mut g.c22;
            d.list.selection.select_only(&d.home, 9);
        });
        rt.key(i);
        rt.frame(FRAME);
        assert_eq!(open(&rt.state), None);
    }

    // A click on the card opens its folder (Strata's commit_peek).
    #[test]
    fn a_click_on_the_card_opens_its_folder() {
        let mut rt = mount_with("c22-icons-light", |_| {});
        point(&mut rt, &item(2));
        wake_until(&mut rt, 1.0, |g| g.c22.peeked.status == PeekStatus::Ready);
        rt.frame(FRAME);
        let (x, y) = centre(&rt, "peek-open");
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
        assert_eq!(rt.state.c22.heard, "Open Music");
        assert_eq!(open(&rt.state), None);
    }

    mod round1 {
        //! Review round 1: what assistive technology and the pointer must not see.

        use rune::runtime::ElementInfo;
        use rune::{Env, Key, KeyEvent, Mods, Runtime};
        use strata_components::collection::element::{row_key, viewport_path};
        use strata_components::contract::{EntryId, EntrySource};
        use strata_components::folder_peek::{More, PeekStatus, card_key};
        use strata_gallery::scenes::c22::{ICONS, LIST, lengthen};
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
            for _ in 0..60 {
                rt.frame(FRAME);
            }
            rt
        }

        fn element(rt: &Runtime<Gallery>, key: &str) -> Option<ElementInfo> {
            rt.elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        }

        fn centre(rt: &Runtime<Gallery>, key: &str) -> (f32, f32) {
            let [x, y, w, h] = element(rt, key)
                .unwrap_or_else(|| panic!("no element {key}"))
                .visible;
            (x + w / 2.0, y + h / 2.0)
        }

        fn point(rt: &mut Runtime<Gallery>, key: &str) {
            let (x, y) = centre(rt, key);
            rt.pointer_move(x, y);
            rt.frame(FRAME);
        }

        fn command(rt: &mut Runtime<Gallery>, c: char) {
            rt.key(KeyEvent {
                key: Key::Command(c),
                mods: Mods::default(),
            });
            rt.frame(FRAME);
        }

        /// As `c22::wake_until`: frames only at the wakes asked for.
        fn wake_until(
            rt: &mut Runtime<Gallery>,
            limit: f64,
            done: impl Fn(&Gallery) -> bool,
        ) -> f64 {
            let start = rt.state.c22.clock;
            for _ in 0..400 {
                let now = rt.state.c22.clock;
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
            rt.state.c22.clock - start
        }

        fn open(g: &Gallery) -> Option<EntryId> {
            g.c22.peek.target().map(|t| t.folder)
        }

        fn item(n: u64) -> String {
            row_key(EntryId::new(n))
        }

        // B1. preview/quick-preview/folder-peek: "When neither side fits, the popover overlays the
        // viewport's trailing edge." The viewport is the pane the person sees, not the columns'
        // width C4 scrolls sideways inside it.
        #[test]
        fn a_trailing_edge_card_stays_inside_a_sideways_scrolled_pane() {
            for (pane, scroll) in [(400.0, 150.0), (300.0, 0.0)] {
                let mut rt = mount_with("c22-list-light", |g| {
                    g.c22.pane = Some(pane);
                    g.c22.scroll_x = Some(scroll);
                });
                for _ in 0..30 {
                    rt.frame(FRAME);
                }
                point(&mut rt, &item(2));
                wake_until(&mut rt, 1.0, |g| open(g).is_some());
                rt.frame(FRAME);
                let card = element(&rt, &card_key(LIST)).expect("a card").rect;
                assert!(
                    card[0] >= 0.0 && card[0] + card[2] <= pane,
                    "pane {pane}, scrolled {scroll}: card {card:?}"
                );
            }
        }

        // B1, a keyboard peek: the host opens it with the pane-bounded viewport too.
        #[test]
        fn a_keyboard_peek_in_a_sideways_scrolled_pane_stays_inside_it() {
            let mut rt = mount_with("c22-list-light", |g| {
                g.c22.pane = Some(400.0);
                g.c22.scroll_x = Some(150.0);
                let d = &mut g.c22;
                d.list.selection.select_only(&d.home, 1);
            });
            for _ in 0..30 {
                rt.frame(FRAME);
            }
            rt.key(KeyEvent {
                key: Key::Text("i".into()),
                mods: Mods::default(),
            });
            rt.frame(FRAME);
            rt.frame(FRAME);
            assert_eq!(open(&rt.state), Some(EntryId::new(2)));
            let card = element(&rt, &card_key(LIST)).expect("a card").rect;
            assert!(
                card[0] >= 0.0 && card[0] + card[2] <= 400.0,
                "card {card:?}"
            );
        }

        /// Whether any card is drawn (visible at all) past the pane `0..pane` in this frame.
        fn card_outside(rt: &Runtime<Gallery>, pane: f32) -> Option<[f32; 4]> {
            rt.elements()
                .into_iter()
                .filter(|e| e.key.as_deref() == Some(&card_key(LIST)) && e.opacity > 0.01)
                .map(|e| e.rect)
                .find(|r| r[0] < -0.5 || r[0] + r[2] > pane + 0.5)
        }

        // B1-r2 (coordinator ruling): a sideways scroll after the peek opened closes it, and the
        // card is never drawn outside the pane on the way, by the wheel or from code.
        #[test]
        fn a_sideways_scroll_closes_the_peek_and_never_draws_it_outside_the_pane() {
            for by_wheel in [true, false] {
                let mut rt = mount_with("c22-list-light", |g| {
                    g.c22.pane = Some(400.0);
                    g.c22.scroll_x = Some(300.0);
                });
                for _ in 0..30 {
                    rt.frame(FRAME);
                }
                point(&mut rt, &item(2));
                wake_until(&mut rt, 1.0, |g| open(g).is_some());
                rt.frame(FRAME);
                assert_eq!(card_outside(&rt, 400.0), None);
                let (x, y) = centre(&rt, &item(2));
                let x = x.min(150.0);
                rt.pointer_move(x, y);
                rt.frame(FRAME);
                for i in 0..40 {
                    if by_wheel && i < 10 {
                        rt.wheel_xy(x, y, 40.0, 0.0);
                    } else if !by_wheel && i == 0 {
                        rt.update(|_, tx| tx.scroll_to_x(format!("{LIST}-scroll"), 0.0));
                    }
                    rt.frame(FRAME);
                    if let Some(r) = card_outside(&rt, 400.0) {
                        panic!("wheel {by_wheel}, frame {i}: card drawn at {r:?}");
                    }
                }
                assert_eq!(open(&rt.state), None, "wheel {by_wheel}");
                assert!(rt.state.c22.events.iter().any(|e| e == "closed"));
            }
        }

        // C24b: C4 keeps its sideways scroll's one `on_scroll_offset` (Rune's replaces) and
        // calls the peek's follower from it after its own bars, so neither drops the other: in
        // the frame a sideways scroll moves the peeked folder, the card is hidden (the peek's
        // follower) and the table's sideways bar shows (C4's own).
        #[test]
        fn one_sideways_frame_hides_the_card_and_shows_the_tables_sideways_bar() {
            let mut rt = mount_with("c22-list-light", |g| {
                g.c22.pane = Some(400.0);
                g.c22.scroll_x = Some(300.0);
            });
            for _ in 0..90 {
                rt.frame(FRAME);
            }
            point(&mut rt, &item(2));
            wake_until(&mut rt, 1.0, |g| open(g).is_some());
            rt.frame(FRAME);
            let shown =
                |rt: &Runtime<Gallery>, key: &str| element(rt, key).map_or(0.0, |e| e.opacity);
            let thumb = format!("{LIST}-sideways-bar/trough/thumb");
            let thumb_x = |rt: &Runtime<Gallery>| {
                rt.elements()
                    .into_iter()
                    .find(|e| !e.exiting && e.path.ends_with(&thumb))
                    .expect("the sideways bar's thumb")
                    .rect[0]
            };
            assert!(shown(&rt, &card_key(LIST)) > 0.5, "the card is up");
            let before = thumb_x(&rt);
            let (x, y) = centre(&rt, &item(2));
            rt.wheel_xy(x.min(150.0), y, 40.0, 0.0);
            rt.frame(FRAME);
            assert_eq!(
                shown(&rt, &card_key(LIST)),
                0.0,
                "the card hid in that frame"
            );
            assert!(
                thumb_x(&rt) < before,
                "the table's sideways bar followed in it"
            );
        }

        // B1. preview/quick-preview/folder-peek: "A viewport narrower than 256 px opens no peek."
        #[test]
        fn a_pane_narrower_than_the_card_peeks_nothing() {
            let mut rt = mount_with("c22-narrow-light", |_| {});
            point(&mut rt, &item(2));
            wake_until(&mut rt, 1.5, |_| false);
            assert_eq!(open(&rt.state), None);
        }

        // B2. A folder under a menu's scrim, or under the menu, never peeks.
        #[test]
        fn rows_under_a_menu_and_its_scrim_never_peek() {
            let mut rt = mount_with("c22-list-light", |_| {});
            command(&mut rt, 'm');
            point(&mut rt, &item(9));
            point(&mut rt, &item(3));
            wake_until(&mut rt, 1.5, |_| false);
            assert_eq!(open(&rt.state), None, "through the scrim");
            point(&mut rt, "c22-menu");
            wake_until(&mut rt, 1.5, |_| false);
            assert_eq!(open(&rt.state), None, "through the menu");
        }

        // B3. preview/quick-preview/folder-peek: "Starting or ending a drag cancels a pending peek
        // and closes an open one": a drag from outside the collection, dropped on a folder.
        #[test]
        fn a_drag_from_elsewhere_dropped_on_a_folder_peeks_nothing() {
            let mut rt = mount_with("c22-list-light", |_| {});
            let (sx, sy) = centre(&rt, "c22-drag-source");
            let (tx, ty) = centre(&rt, &item(3));
            rt.pointer_move(sx, sy);
            rt.frame(FRAME);
            rt.pointer_down(sx, sy);
            rt.frame(FRAME);
            for i in 1..=20 {
                let t = i as f32 / 20.0;
                rt.pointer_move(sx + (tx - sx) * t, sy + (ty - sy) * t);
                rt.frame(FRAME);
            }
            rt.pointer_up(tx, ty);
            rt.frame(FRAME);
            // Moving inside the folder it was dropped on, within its wait, is no arrival.
            rt.frame(0.1);
            rt.pointer_move(tx + 3.0, ty);
            rt.frame(FRAME);
            wake_until(&mut rt, 1.5, |_| false);
            assert_eq!(rt.state.c22.heard, "Dropped");
            assert_eq!(open(&rt.state), None);
        }

        // A press of the collection's own on a folder, then a move inside it within its wait:
        // no arrival either.
        #[test]
        fn a_click_on_a_folder_then_a_move_inside_it_peeks_nothing() {
            let mut rt = mount_with("c22-list-light", |_| {});
            let (x, y) = centre(&rt, &item(3));
            rt.pointer_move(x, y);
            rt.frame(FRAME);
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(FRAME);
            rt.frame(0.1);
            rt.pointer_move(x + 3.0, y);
            rt.frame(FRAME);
            wake_until(&mut rt, 1.5, |_| false);
            assert_eq!(open(&rt.state), None);
        }

        // SF1. The card's own close delay: from the card straight to blank space.
        #[test]
        fn leaving_the_card_for_blank_space_closes_80_ms_later() {
            for animations in [true, false] {
                let mut rt = mount_with("c22-icons-light", |g| g.look.animations = animations);
                point(&mut rt, &item(2));
                wake_until(&mut rt, 1.0, |g| open(g).is_some());
                rt.frame(FRAME);
                point(&mut rt, &card_key(ICONS));
                wake_until(&mut rt, 1.0, |_| false);
                rt.pointer_move(200.0, 600.0);
                rt.frame(FRAME);
                let took = wake_until(&mut rt, 1.0, |g| open(g).is_none());
                assert!((0.06..0.1).contains(&took), "closed after {took:.3} s");
            }
        }

        // SF2 (coordinator ruling: Strata). A peek starts only from the pointer arriving on a
        // folder: peeking on again with the pointer parked shows nothing, moving inside the row
        // doesn't count, and leaving and coming back does.
        #[test]
        fn peeking_on_again_with_the_pointer_parked_waits_for_an_arrival() {
            let mut rt = mount_with("c22-list-light", |_| {});
            command(&mut rt, 'p');
            point(&mut rt, &item(3));
            wake_until(&mut rt, 1.0, |_| false);
            command(&mut rt, 'p');
            wake_until(&mut rt, 2.0, |_| false);
            assert_eq!(open(&rt.state), None, "parked");
            let (x, y) = centre(&rt, &item(3));
            rt.pointer_move(x + 4.0, y);
            rt.frame(FRAME);
            wake_until(&mut rt, 2.0, |_| false);
            assert_eq!(open(&rt.state), None, "a move inside the row");
            // Off it for longer than its 80 ms leave delay (a quicker out and back is no leave to
            // Rune), then back.
            point(&mut rt, &item(9));
            wake_until(&mut rt, 0.3, |_| false);
            point(&mut rt, &item(3));
            let took = wake_until(&mut rt, 2.0, |g| open(g).is_some());
            assert_eq!(open(&rt.state), Some(EntryId::new(3)), "an arrival");
            assert!((0.48..0.52).contains(&took), "after {took:.3} s");
        }

        // Owner item 3: peeking turned on (or off and on) within a folder's wait never peeks it,
        // even with a move inside the row before the wait ends.
        #[test]
        fn turning_peeking_on_within_a_wait_never_peeks_that_folder() {
            for off_first in [true, false] {
                let mut rt = mount_with("c22-list-light", |_| {});
                if off_first {
                    command(&mut rt, 'p');
                }
                // Arrive on Photos; peeking changes 0.2 s into its wait.
                point(&mut rt, &item(9));
                wake_until(&mut rt, 0.3, |_| false);
                let (x, y) = centre(&rt, &item(3));
                rt.pointer_move(x, y);
                rt.frame(FRAME);
                rt.frame(0.2);
                if !off_first {
                    command(&mut rt, 'p');
                }
                command(&mut rt, 'p');
                rt.frame(0.1);
                rt.pointer_move(x + 3.0, y);
                rt.frame(FRAME);
                wake_until(&mut rt, 1.5, |_| false);
                assert!(rt.state.c22.peek.enabled());
                assert_eq!(open(&rt.state), None, "off first {off_first}");
            }
        }

        // SF3. A peek whose folder's row scrolls out of view closes, with the pointer on the card
        // (the row's own leave came long before).
        #[test]
        fn a_peek_closes_when_its_row_scrolls_out_of_view() {
            let mut rt = mount_with("c22-list-light", |g| lengthen(&mut g.c22, 400));
            point(&mut rt, &item(2));
            wake_until(&mut rt, 1.0, |g| open(g).is_some());
            rt.frame(FRAME);
            point(&mut rt, &card_key(LIST));
            wake_until(&mut rt, 0.5, |_| false);
            assert_eq!(open(&rt.state), Some(EntryId::new(2)));
            // Out of view, still mounted (Rune keeps rows past the viewport).
            rt.update(|_, tx| tx.scroll_to_instant(viewport_path(LIST), 0.0, 150.0));
            rt.frame(FRAME);
            rt.frame(FRAME);
            assert!(element(&rt, &item(2)).is_some(), "still mounted");
            assert_eq!(open(&rt.state), None, "clipped out of view");
            assert_eq!(
                rt.state.c22.events.last().map(String::as_str),
                Some("closed")
            );
            assert_eq!(rt.state.c22.heard, "Peek closed");
        }

        // SF3. A keyboard peek too, when the row leaves the tree.
        #[test]
        fn a_keyboard_peek_closes_when_its_row_is_scrolled_away() {
            let mut rt = mount_with("c22-list-light", |g| {
                lengthen(&mut g.c22, 400);
                let d = &mut g.c22;
                d.list.selection.select_only(&d.home, 1);
            });
            rt.key(KeyEvent {
                key: Key::Text("i".into()),
                mods: Mods::default(),
            });
            rt.frame(FRAME);
            assert_eq!(open(&rt.state), Some(EntryId::new(2)));
            rt.update(|_, tx| tx.scroll_to_instant(viewport_path(LIST), 0.0, 6_000.0));
            rt.frame(FRAME);
            rt.frame(FRAME);
            assert!(element(&rt, &item(2)).is_none(), "unmounted");
            assert_eq!(open(&rt.state), None);
            assert_eq!(
                rt.state.c22.events.last().map(String::as_str),
                Some("closed")
            );
        }

        // A result for a peek no longer open is dropped by its serial.
        #[test]
        fn a_listing_for_a_closed_peek_is_dropped_by_its_serial() {
            let mut rt = mount_with("c22-list-light", |_| {});
            point(&mut rt, &item(6));
            wake_until(&mut rt, 1.0, |g| open(g).is_some());
            point(&mut rt, &item(2));
            wake_until(&mut rt, 1.0, |g| open(g) == Some(EntryId::new(2)));
            wake_until(&mut rt, 3.0, |g| g.c22.stale > 0);
            let d = &rt.state.c22;
            assert_eq!(d.stale, 1, "Slow's result came for a closed peek");
            assert_eq!(d.peeked.name, "Music");
            assert_eq!(d.peeked.status, PeekStatus::Ready);
            assert_eq!(d.peeked.rows.len(), 3);
        }

        // A host that says there are none more gets no "and 0 more" line.
        #[test]
        fn no_more_line_when_the_host_says_none_more() {
            let mut rt = mount_with("c22-list-light", |_| {});
            point(&mut rt, &item(8));
            wake_until(&mut rt, 1.0, |g| g.c22.peeked.status == PeekStatus::Ready);
            rt.frame(FRAME);
            assert_eq!(rt.state.c22.peeked.more, Some(More::Exactly(0)));
            assert!(element(&rt, "peek-rows").is_some());
            assert!(element(&rt, "peek-more").is_none());
        }

        // C3: a tile a peek is open from shows no name tooltip, so the card and a tooltip
        // never cover each other.
        #[test]
        fn a_peeked_tile_shows_no_name_tooltip() {
            let mut rt = mount_with("c22-icons-light", |_| {});
            let label = rt
                .elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some("label") && e.path.contains("/row-13/"))
                .expect("the tile's name")
                .visible;
            let (x, y) = (label[0] + label[2] / 2.0, label[1] + label[3] / 2.0);
            rt.pointer_move(x, y);
            for _ in 0..90 {
                rt.frame(FRAME);
            }
            assert_eq!(open(&rt.state), Some(EntryId::new(13)));
            assert_eq!(rt.help_tooltip().map(|(t, _)| t.to_owned()), None);
        }
    }
}

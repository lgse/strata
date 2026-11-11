//! C18's page in process: what scripts can't reach (an input method composing, the wakes of a
//! window at rest, whether a row is scrolled into view). The scripts in `tests/scripts/c18/`
//! check the rest. Each test names the bullets it covers.

mod c18 {
    use rune::{ElementInfo, Env, Ime, Key, KeyEvent, Mods, Runtime, WheelEvent};
    use strata_components::collection::selection::AnchorRule;
    use strata_components::contract::{EntryId, EntryKind, ListEntry, ListSource};
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
        run(&mut rt, 0.5);
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

    fn key(rt: &mut Runtime<Gallery>, key: Key, mods: Mods) {
        rt.key(KeyEvent { key, mods });
        rt.frame(FRAME);
    }

    fn typ(rt: &mut Runtime<Gallery>, text: &str) {
        for c in text.chars() {
            key(rt, Key::Text(c.to_string()), Mods::default());
        }
    }

    fn element(rt: &Runtime<Gallery>, key: &str) -> Option<ElementInfo> {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
    }

    fn centre(rt: &Runtime<Gallery>, key: &str) -> (f32, f32) {
        let r = element(rt, key)
            .unwrap_or_else(|| panic!("no element {key}"))
            .rect;
        (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
    }

    /// Steps from wake to wake, as a window at rest does, until no wake is due within
    /// `limit` seconds of the start. Returns the clock it stopped at.
    fn rest(rt: &mut Runtime<Gallery>, limit: f64) -> f64 {
        let start = rt.state.c18.now;
        for _ in 0..200 {
            let Some(deadline) = rt.wake_deadline() else {
                break;
            };
            let now = rt.state.c18.now;
            if deadline - start > limit {
                break;
            }
            // A deadline is due once the clock passes it: step at least a millisecond.
            rt.frame((deadline - now).max(0.001));
        }
        rt.state.c18.now
    }

    /// A long folder: the filter's results scroll.
    fn long(g: &mut Gallery) {
        let disk: Vec<ListEntry> = (0..300u64)
            .map(|i| {
                ListEntry::new(
                    EntryId::new(1000 + i),
                    format!("file-{i:03}.txt"),
                    EntryKind::File,
                    "file_text",
                )
            })
            .collect();
        g.c18.listing = ListSource::new(disk.clone());
        g.c18.disk = disk;
    }

    fn top_shown(rt: &Runtime<Gallery>) -> bool {
        rt.state.c18.coll.visible_range().contains(&0)
    }

    // browser/search/filter: "With a pinyin input method composing in the filter field, Escape
    // cancels only the composition and the filter stays open." Rune gives Escape to the
    // composition first; the bar never hears it.
    #[test]
    fn escape_while_composing_keeps_the_filter_open() {
        let mut rt = mount("c18-filter-target-light");
        rt.ime(Ime::Preedit("か".into(), Some((3, 3))));
        rt.frame(FRAME);
        key(&mut rt, Key::Escape, Mods::default());
        run(&mut rt, 0.2);
        assert!(rt.state.c18.filter.is_open());
        assert!(rt.state.c18.filter.field_focused());
    }

    // integration/10xer-mode: "An Enter that an input method consumes while composing never
    // submits a footer prompt; an Enter it passes back submits it."
    #[test]
    fn an_enter_the_input_method_takes_never_submits_a_prompt() {
        let mut rt = mount("c18-tenxer-light");
        typ(&mut rt, "/");
        assert!(rt.state.c18.prompt.focused());
        rt.ime(Ime::Preedit("ぱ".into(), Some((3, 3))));
        rt.frame(FRAME);
        key(&mut rt, Key::Enter, Mods::default());
        run(&mut rt, 0.1);
        assert!(rt.state.c18.prompt.is_open(), "the composition took Enter");
        rt.ime(Ime::Commit("p".into()));
        rt.frame(FRAME);
        key(&mut rt, Key::Enter, Mods::default());
        run(&mut rt, 0.1);
        assert!(!rt.state.c18.prompt.is_open(), "the next Enter submits");
        assert_eq!(rt.state.c18.log, "Found Projects");
    }

    // browser/search/filter/enter-target: "While the field has focus, the list keeps the top
    // result scrolled into view as results arrive. A button press, wheel scroll, touch, or drag
    // hover on the results stops that scrolling; the next query edit scrolls back to the top
    // result." Whether a row shows is what the person sees; the visible range says it.
    #[test]
    fn the_top_result_follows_the_results_until_they_are_scrolled() {
        let mut rt = mount_with("c18-filter-open-light", long);
        typ(&mut rt, "file");
        run(&mut rt, 0.3);
        assert!(top_shown(&rt));
        // The person wheels down. A host can't hear a wheel over a C1e collection yet (Rune's
        // `on_wheel` stops at the nearest scroll view, and `Collection::on_scroll` reads no
        // state: a C1e seam follow-up), so the test calls what that hook would.
        let (x, y) = centre(&rt, "c18-list");
        rt.pointer_move(x, y);
        for _ in 0..20 {
            rt.wheel_event(WheelEvent::new(0.0, -120.0).at(x, y).precise(false));
            rt.frame(FRAME);
        }
        run(&mut rt, 0.5);
        assert!(!top_shown(&rt), "the wheel scrolled the results");
        rt.state.c18.filter.results_scrolled();
        // A same-query update: the results stay where the person put them.
        key(&mut rt, Key::Function(6), Mods::default());
        run(&mut rt, 0.5);
        assert!(!top_shown(&rt), "no reveal after the person scrolled");
        // A query edit brings the top result back.
        typ(&mut rt, "-");
        run(&mut rt, 0.5);
        assert!(top_shown(&rt));
    }

    // integration/10xer-mode/places: "Esc cancels an armed chord, and any other unknown second
    // key cancels it with `Unknown chord`." The flash goes 2 s after it shows, in a window at
    // rest, which polls only at the wakes it asked for.
    #[test]
    fn the_unknown_chord_flash_ends_in_a_resting_window() {
        let mut rt = mount("c18-tenxer-light");
        typ(&mut rt, "gq");
        let shown = rt.state.c18.now;
        assert_eq!(rt.state.c18.flash.text(), Some("Unknown chord"));
        assert!(element(&rt, "c18-flash").is_some());
        let stopped = rest(&mut rt, 5.0);
        assert_eq!(rt.state.c18.flash.text(), None, "the flash ended");
        let took = stopped - shown;
        assert!((1.9..2.2).contains(&took), "after {took:.3} s");
        assert!(element(&rt, "c18-flash").is_none());
    }

    // A second flash restarts the 2 s.
    #[test]
    fn a_second_flash_restarts_its_time() {
        let mut rt = mount("c18-tenxer-light");
        typ(&mut rt, "gq");
        run(&mut rt, 1.5);
        typ(&mut rt, "gq");
        let second = rt.state.c18.now;
        let stopped = rest(&mut rt, 5.0);
        let took = stopped - second;
        assert!((1.9..2.2).contains(&took), "after {took:.3} s");
    }

    // integration/10xer-mode: "Arming a chord… shows its pill… with a panel above it", and
    // nothing times it out: an armed chord asks for no wake, and is still armed at rest.
    #[test]
    fn an_armed_chord_stays_armed_at_rest() {
        let mut rt = mount("c18-tenxer-light");
        typ(&mut rt, "g");
        assert!(
            element(&rt, "c18-chord-panel").is_some(),
            "the panel shows at once"
        );
        let start = rt.state.c18.now;
        let stopped = rest(&mut rt, 30.0);
        assert_eq!(stopped, start, "nothing asked for a wake");
        rt.frame(30.0);
        assert!(rt.state.c18.chord.is_armed());
        assert!(element(&rt, "c18-chord-pill").is_some());
        typ(&mut rt, "g");
        assert!(!rt.state.c18.chord.is_armed());
    }

    // integration/10xer-mode/places: "Enter on a typed path that exists acts on it at once;
    // otherwise Enter waits for the search to finish." The page's picker answers 0.3 s after
    // it is asked; an Enter before then waits for it, in a window at rest.
    #[test]
    fn an_early_enter_in_go_waits_for_the_picker_in_a_resting_window() {
        let mut rt = mount("c18-tenxer-light");
        typ(&mut rt, "g ");
        assert!(rt.state.c18.prompt.focused(), "g Space opens go ›");
        run(&mut rt, 0.5);
        typ(&mut rt, "~/do");
        key(&mut rt, Key::Enter, Mods::default());
        let asked = rt.state.c18.now;
        assert_eq!(rt.state.c18.log, "Waiting for the folders");
        let stopped = rest(&mut rt, 2.0);
        assert_eq!(rt.state.c18.log, "Go ~/Documents");
        assert!(!rt.state.c18.prompt.is_open());
        assert!((0.25..0.4).contains(&(stopped - asked)));
    }

    // integration/10xer-mode/search: "When another program deletes the cursor's file while
    // the `f` or `s` prompt has focus, focus stays in the prompt"; browser/search/filter: "When
    // another program deletes the listing's cursor entry in any view, a focused filter field or
    // result keeps focus."
    #[test]
    fn a_deleted_cursor_entry_leaves_the_field_and_the_prompt_focused() {
        let cursor_on_top = |g: &mut Gallery| {
            let d = &mut g.c18;
            let _ = d
                .coll
                .selection
                .move_cursor(&d.listing, 0, AnchorRule::Move);
        };
        let mut rt = mount_with("c18-filter-target-light", cursor_on_top);
        key(&mut rt, Key::Function(7), Mods::default());
        run(&mut rt, 0.2);
        assert_eq!(rt.state.c18.log, "Deleted reports");
        let fk = "c18-filter-field";
        assert!(element(&rt, fk).is_some_and(|e| e.focused));
        assert!(rt.state.c18.filter.field_focused());

        let mut rt = mount_with("c18-prompt-filter-light", cursor_on_top);
        key(&mut rt, Key::Function(7), Mods::default());
        run(&mut rt, 0.2);
        assert!(rt.state.c18.log.starts_with("Deleted"));
        assert!(rt.state.c18.prompt.focused());
    }

    // browser/search/filter: "With **Type to search** on, typing a printable character in a
    // file view opens the filter with that character as the query." Keys typed in one frame,
    // before the field takes the keyboard, all reach the query.
    #[test]
    fn a_burst_of_type_to_search_keys_all_reach_the_query() {
        let mut rt = mount("c18-listing-light");
        for c in ["r", "e", "p"] {
            rt.key(KeyEvent {
                key: Key::Text(c.into()),
                mods: Mods::default(),
            });
        }
        run(&mut rt, 0.3);
        assert_eq!(rt.state.c18.filter.text(), "rep");
        assert_eq!(rt.state.c18.log, "Query “rep”, 1 shown");
    }

    // A held Enter in the field opens its target once.
    #[test]
    fn a_held_enter_opens_once() {
        let mut rt = mount("c18-filter-target-light");
        let enter = KeyEvent {
            key: Key::Enter,
            mods: Mods::default(),
        };
        rt.key(enter.clone());
        rt.frame(FRAME);
        assert_eq!(rt.state.c18.log, "Opened reports");
        rt.state.c18.log.clear();
        for _ in 0..3 {
            rt.repeat_key(enter.clone());
            rt.frame(FRAME);
        }
        assert_eq!(rt.state.c18.log, "", "the repeats open nothing");
    }

    // While an input method composes, Down and Space in the filter field are the
    // composition's: Down neither moves to the results nor picks, Space previews nothing.
    #[test]
    fn down_and_space_while_composing_stay_with_the_composition() {
        let mut rt = mount("c18-filter-picked-light");
        rt.ime(Ime::Preedit("か".into(), Some((3, 3))));
        rt.frame(FRAME);
        key(&mut rt, Key::Down, Mods::default());
        key(&mut rt, Key::Text(" ".into()), Mods::default());
        run(&mut rt, 0.2);
        assert!(
            !rt.state.c18.log.starts_with("Down to"),
            "{}",
            rt.state.c18.log
        );
        assert_ne!(rt.state.c18.log, "Quick preview");
        assert!(rt.state.c18.filter.field_focused());
    }

    // The same in a prompt: Down chooses no candidate.
    #[test]
    fn down_while_composing_chooses_no_candidate() {
        let mut rt = mount("c18-prompt-go-light");
        run(&mut rt, 0.5);
        assert_eq!(rt.state.c18.prompt.chosen_index(), Some(0));
        rt.ime(Ime::Preedit("か".into(), Some((3, 3))));
        rt.frame(FRAME);
        key(&mut rt, Key::Down, Mods::default());
        run(&mut rt, 0.2);
        assert_eq!(rt.state.c18.prompt.chosen_index(), Some(0));
        assert!(!rt.state.c18.prompt.stepped());
        assert!(rt.state.c18.prompt.is_open());
    }

    // Review round 2, SF3: a field menu whose prompt closes from code (a submit, leaving the
    // mode) closes at the next poll (`FieldMenu::check`), and the prompt's hold goes with it.
    #[test]
    fn a_prompt_closed_under_its_field_menu_closes_the_menu() {
        let mut rt = mount("c18-tenxer-light");
        typ(&mut rt, "/");
        typ(&mut rt, "rep");
        key(
            &mut rt,
            Key::Function(10),
            Mods {
                shift: true,
                ..Mods::default()
            },
        );
        run(&mut rt, 0.2);
        assert!(rt.state.c18.fields.is_open(), "the menu opened");
        assert!(rt.state.c18.prompt.is_held());
        let _ = rt.state.c18.prompt.close();
        run(&mut rt, 0.1);
        assert!(
            !rt.state.c18.fields.is_open(),
            "the menu closed with its field"
        );
        assert!(!rt.state.c18.prompt.is_held());
        assert_eq!(rt.state.c18.log, "Prompt menu closed");
    }
}

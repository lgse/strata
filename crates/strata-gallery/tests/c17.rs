//! C17's page in process: what scripts can't reach (an input method composing, every frame of
//! a superseded query, a window at rest stepped from wake to wake, a tooltip, the new-tab
//! chords and the middle click, the states with no feature-map bullet). The scripts in
//! `tests/scripts/c17/` check the rest. Each test names the bullets it covers, or says it
//! covers none.

mod c17 {
    use rune::{ElementInfo, Env, Ime, Key, KeyEvent, Mods, PointerButton, Runtime};
    use strata_components::contract::fuzzy;
    use strata_components::search_palette::{Answer, PaletteItem, Phase, option_key, results_key};
    use strata_gallery::scenes::c17::KEY;
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

    fn press(rt: &mut Runtime<Gallery>, key: Key, mods: Mods) {
        rt.key(KeyEvent { key, mods });
        rt.frame(FRAME);
    }

    fn ctrl(shift: bool) -> Mods {
        Mods {
            ctrl: true,
            cmd: true,
            shift,
            ..Mods::default()
        }
    }

    fn typ(rt: &mut Runtime<Gallery>, text: &str) {
        for c in text.chars() {
            press(rt, Key::Text(c.to_string()), Mods::default());
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

    fn focused(rt: &Runtime<Gallery>) -> Option<String> {
        rt.elements()
            .into_iter()
            .find(|e| e.focused && !e.exiting)
            .and_then(|e| e.key)
    }

    /// The accessible name of the node keyed `key`.
    fn name(rt: &Runtime<Gallery>, key: &str) -> Option<String> {
        fn find(v: &serde_json::Value, key: &str) -> Option<String> {
            if v["key"] == key {
                return v["name"].as_str().map(str::to_owned);
            }
            v["children"].as_array()?.iter().find_map(|c| find(c, key))
        }
        find(&rt.accessibility_json(), key)
    }

    fn status(rt: &Runtime<Gallery>) -> Option<String> {
        name(rt, &format!("{KEY}-status"))
    }

    fn ids(rt: &Runtime<Gallery>) -> Vec<String> {
        rt.state
            .c17
            .palette
            .items()
            .iter()
            .map(|it| it.id.clone())
            .collect()
    }

    /// Steps a resting window: a frame only at each requested wake, none in between, for at
    /// most `limit` seconds, calling `each` after every frame. Reduced motion holds the
    /// spinner still, so the window rests between wakes.
    fn rest_with(rt: &mut Runtime<Gallery>, limit: f64, mut each: impl FnMut(&Runtime<Gallery>)) {
        let start = rt.state.c17.now;
        for _ in 0..400 {
            let Some(deadline) = rt.wake_deadline() else {
                break;
            };
            let now = rt.state.c17.now;
            if deadline - start > limit {
                break;
            }
            rt.frame((deadline - now).max(0.001));
            each(rt);
        }
    }

    fn resting(name: &str) -> Runtime<Gallery> {
        let mut rt = mount(name);
        rt.set_system_reduced_motion(true);
        run(&mut rt, 0.5);
        rt
    }

    // No bullet names it (the WP's "a superseded query's results never show"): a query typed
    // over another before its answers land shows none of them, in any frame, in a window at
    // rest stepped from wake to wake.
    #[test]
    fn a_superseded_query_s_answers_never_show() {
        let mut rt = resting("c17-closed-light");
        press(&mut rt, Key::Command('k'), ctrl(false));
        rest_with(&mut rt, 1.0, |_| {});
        typ(&mut rt, "re");
        // "rep" supersedes "re" (and "r") before their first batch lands.
        typ(&mut rt, "p");
        let mut saw = 0;
        rest_with(&mut rt, 2.0, |rt| {
            for it in rt.state.c17.palette.items() {
                assert!(
                    fuzzy("rep", &it.label).is_some() || fuzzy("rep", &it.detail).is_some(),
                    "{} answers a superseded query",
                    it.id
                );
            }
            saw = saw.max(rt.state.c17.palette.items().len());
        });
        assert!(saw > 0, "the current query's answers landed");
        assert_eq!(rt.state.c17.palette.phase(), Some(Phase::Results));
        assert!(
            !rt.state.c17.palette.busy(),
            "the last batch ended the walk"
        );
    }

    // browser/search: "A spinner shows in the search bar while indexing runs." In a window at
    // rest the three batches land at the wakes the fake worker asked for, and the count is
    // announced at the first results and when the query is done (owner ruling).
    #[test]
    fn streamed_batches_land_in_a_resting_window() {
        let mut rt = resting("c17-closed-light");
        press(&mut rt, Key::Command('k'), ctrl(false));
        typ(&mut rt, "notes");
        let busy = format!("{KEY}-busy");
        assert!(element(&rt, &busy).is_some(), "the spinner while it runs");
        let mut counts = Vec::new();
        rest_with(&mut rt, 1.0, |rt| {
            let n = rt.state.c17.palette.items().len();
            if counts.last() != Some(&n) {
                counts.push(n);
            }
        });
        assert!(counts.len() >= 3, "batches grew the list: {counts:?}");
        assert!(
            element(&rt, &busy).is_none(),
            "the spinner goes with the walk"
        );
        let n = rt.state.c17.palette.items().len();
        assert_eq!(status(&rt), Some(format!("{n} results")));
    }

    // No bullet names the loading or error states (the WP's "each state is distinct"): each
    // state's status reads its own words, results read a count.
    #[test]
    fn each_state_reads_its_own_words() {
        let at_one = |name: &str| {
            let mut rt = mount(name);
            run(&mut rt, 1.0);
            status(&rt).unwrap_or_default()
        };
        let idle = at_one("c17-idle-light");
        let loading = at_one("c17-loading-light");
        let results = at_one("c17-results-light");
        let none = at_one("c17-no-results-light");
        let unavailable = at_one("c17-error-light");
        let mut rt = mount("c17-loading-light");
        rt.update(|g, _| {
            let generation = g.c17.palette.generation();
            assert!(
                g.c17
                    .palette
                    .fail(generation, "The search index could not be read.")
            );
        });
        run(&mut rt, 0.1);
        let failed = status(&rt).unwrap_or_default();
        assert_eq!(idle, "Type to search Home and mounted local drives");
        assert_eq!(loading, "Searching…");
        assert!(results.ends_with(" results"), "{results}");
        assert_eq!(none, "No matching files or folders");
        assert_eq!(unavailable, "No local search locations available.");
        assert_eq!(failed, "The search index could not be read.");
        let mut all = vec![&idle, &loading, &results, &none, &unavailable, &failed];
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 6, "distinct");
    }

    // browser/search: "With a pinyin input method composing in the query field, Escape
    // cancels only the composition and search stays open."
    #[test]
    fn escape_while_composing_keeps_the_palette_open() {
        let mut rt = mount("c17-idle-light");
        run(&mut rt, 0.5);
        rt.ime(Ime::Preedit("ni".into(), None));
        rt.frame(FRAME);
        press(&mut rt, Key::Escape, Mods::default());
        assert!(rt.state.c17.palette.is_open());
        assert_eq!(
            focused(&rt).as_deref(),
            Some(format!("{KEY}-field").as_str())
        );
    }

    // browser/search: "With an input method composing in the query field, Enter that the input
    // method passes back after committing opens the highlighted result."
    #[test]
    fn an_enter_after_the_composition_commits_opens_the_highlighted_result() {
        let mut rt = mount("c17-results-light");
        run(&mut rt, 1.0);
        let first = ids(&rt)[0].clone();
        rt.ime(Ime::Preedit("ni".into(), None));
        rt.frame(FRAME);
        press(&mut rt, Key::Enter, Mods::default());
        assert!(rt.state.c17.palette.is_open(), "the composition took Enter");
        rt.ime(Ime::Commit(String::new()));
        rt.frame(FRAME);
        press(&mut rt, Key::Enter, Mods::default());
        assert_eq!(rt.state.c17.log, format!("Opened {first}"));
        assert!(!rt.state.c17.palette.is_open());
    }

    // No bullet: Strata's palette has no new-tab key (owner ruling, a deviation). The host's
    // open_in_new_tab chord (Ctrl+Enter, Shift ignored) and a middle click open in a new tab.
    #[test]
    fn ctrl_enter_and_a_middle_click_open_in_a_new_tab() {
        for mods in [ctrl(false), ctrl(true)] {
            let mut rt = mount("c17-results-light");
            run(&mut rt, 1.0);
            let first = ids(&rt)[0].clone();
            press(&mut rt, Key::Enter, mods);
            assert_eq!(rt.state.c17.log, format!("Opened {first} in a new tab"));
        }
        let mut rt = mount("c17-results-light");
        run(&mut rt, 1.0);
        let second = ids(&rt)[1].clone();
        let (x, y) = centre(&rt, &row_key(&rt, &second));
        rt.pointer_move(x, y);
        rt.button_down(x, y, PointerButton::Middle);
        rt.frame(FRAME);
        rt.button_up(x, y, PointerButton::Middle);
        rt.frame(FRAME);
        assert_eq!(rt.state.c17.log, format!("Opened {second} in a new tab"));
        // Alt+Ctrl+Enter is neither.
        let mut rt = mount("c17-results-light");
        run(&mut rt, 1.0);
        press(
            &mut rt,
            Key::Enter,
            Mods {
                alt: true,
                ..ctrl(false)
            },
        );
        assert!(rt.state.c17.palette.is_open());
    }

    // browser/search: "Hovering Partial results shows a tooltip with the same causes as its
    // description; complete results clear the tooltip."
    #[test]
    fn the_partial_notice_shows_its_causes_on_hover_until_results_are_complete() {
        let mut rt = mount("c17-partial-light");
        run(&mut rt, 1.0);
        let partial = format!("{KEY}-partial");
        let (x, y) = centre(&rt, &partial);
        rt.pointer_move(x, y);
        run(&mut rt, 1.0);
        let causes = "Some folders could not be read: ~/Private";
        assert_eq!(rt.help_tooltip().map(|(t, _)| t), Some(causes));
        // A rescan that completes: the same query's answer, complete.
        rt.update(|g, tx| {
            let generation = g.c17.palette.generation();
            let items = g.c17.palette.items().to_vec();
            let answer = strata_components::search_palette::Answer::new(generation, items).done();
            assert!(g.c17.palette.deliver(answer, tx).changed);
        });
        run(&mut rt, 1.0);
        assert!(element(&rt, &partial).is_none());
        assert_eq!(rt.help_tooltip(), None);
    }

    // browser/search: "Up and Down move the highlight while the caret stays in the query
    // field". The pointer only tints: a pointer resting on a row never moves the highlight,
    // even as the rows scroll under it (C6's review round 1).
    #[test]
    fn a_resting_pointer_never_moves_the_highlight() {
        let mut rt = mount("c17-results-light");
        run(&mut rt, 1.0);
        let third = ids(&rt)[3].clone();
        let (x, y) = centre(&rt, &row_key(&rt, &third));
        rt.pointer_move(x, y);
        run(&mut rt, 0.2);
        assert_eq!(rt.state.c17.palette.highlighted(), Some(0));
        for i in 1..12 {
            press(&mut rt, Key::Down, Mods::default());
            run(&mut rt, 0.2);
            assert_eq!(rt.state.c17.palette.highlighted(), Some(i));
        }
    }

    // browser/search: "Opening a result, or Open containing folder, hands focus to the
    // revealed item in the browser rather than the control that opened search." In a window
    // at rest: the leaving frame ends at the wake the palette asked for (probe P2).
    #[test]
    fn opening_a_result_hands_focus_to_the_view_in_a_resting_window() {
        let mut rt = resting("c17-closed-light");
        let (x, y) = centre(&rt, "c17-search");
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
        typ(&mut rt, "notes");
        rest_with(&mut rt, 1.0, |_| {});
        press(&mut rt, Key::Enter, Mods::default());
        rest_with(&mut rt, 1.0, |_| {});
        assert!(!rt.state.c17.palette.is_open());
        assert!(element(&rt, &format!("{KEY}-dialog")).is_none(), "gone");
        assert_eq!(focused(&rt).as_deref(), Some("c17-files"));
        assert!(rt.state.c17.log.starts_with("Opened "));
    }

    // browser/search: "Each result shows its name, its full path, and a thumbnail for images."
    // The thumbnails land through the C25 store at the wakes the fake worker asks for.
    #[test]
    fn image_results_get_their_thumbnails_in_a_resting_window() {
        let mut rt = resting("c17-closed-light");
        press(&mut rt, Key::Command('k'), ctrl(false));
        typ(&mut rt, "trip-photo");
        rest_with(&mut rt, 2.0, |_| {});
        let d = &rt.state.c17;
        let key = d.palette.items()[0].thumb.expect("an image");
        assert!(
            matches!(
                d.thumbs.state(key),
                strata_components::thumbnails::State::Ready { .. }
            ),
            "{:?}",
            d.thumbs.stats()
        );
    }

    /// Whether the row of result `id` is drawn wholly inside the list's viewport.
    fn visible(rt: &Runtime<Gallery>, id: &str) -> bool {
        let Some(vp) = element(rt, &format!("{KEY}-results")).map(|e| e.rect) else {
            return false;
        };
        element(rt, &row_key(rt, id)).is_some_and(|e| {
            e.rect[1] >= vp[1] - 0.5 && e.rect[1] + e.rect[3] <= vp[1] + vp[3] + 0.5
        })
    }

    fn item(id: &str) -> PaletteItem {
        PaletteItem::new(id, id, format!("/x/{id}"))
    }

    fn named(prefix: &str, n: usize) -> Vec<PaletteItem> {
        (0..n).map(|i| item(&format!("{prefix}{i:02}"))).collect()
    }

    /// The key of result `id`'s row: rows are keyed by place.
    fn row_key(rt: &Runtime<Gallery>, id: &str) -> String {
        let items = rt.state.c17.palette.items();
        let i = items.iter().position(|it| it.id == id);
        i.map_or_else(|| format!("{KEY}-no-row"), |i| option_key(KEY, i))
    }

    /// A resting palette on a query whose answers the test delivers itself.
    fn synthetic() -> Runtime<Gallery> {
        synthetic_with(true)
    }

    /// The same, with reduced motion `reduced` (off, a fling decays over frames).
    fn synthetic_with(reduced: bool) -> Runtime<Gallery> {
        let mut rt = mount("c17-closed-light");
        rt.set_system_reduced_motion(reduced);
        run(&mut rt, 0.5);
        press(&mut rt, Key::Command('k'), ctrl(false));
        rt.update(|g, _| {
            let _ = g.c17.palette.set_query("zz-probe");
            g.c17.worker.clear();
        });
        rt.frame(FRAME);
        rt
    }

    /// Delivers `items` for the current query, then steps the resting window frame by frame
    /// until it rests, calling `each` after every frame, the first the batch's commit.
    fn land(
        rt: &mut Runtime<Gallery>,
        items: Vec<PaletteItem>,
        mut each: impl FnMut(&Runtime<Gallery>),
    ) {
        rt.update(move |g, tx| {
            let generation = g.c17.palette.generation();
            let _ = g.c17.palette.deliver(Answer::new(generation, items), tx);
        });
        for _ in 0..30 {
            rt.frame(FRAME);
            each(rt);
        }
    }

    // browser/search: "While indexing adds results, the highlighted result stays selected by
    // path…". Review round 1, B1: until the user moves the highlight, a batch keeps the list's
    // pixel offset (Strata's restore_scroll with navigation_started false), so better results
    // a batch puts above the first row show at the top; Rune's anchoring would keep the old
    // first row there and hide them.
    #[test]
    fn a_batch_above_the_first_row_shows_at_the_top_until_the_user_moves() {
        let mut rt = synthetic();
        land(&mut rt, named("b", 30), |_| {});
        assert!(visible(&rt, "b00"));
        let mut more = vec![item("a0"), item("a1")];
        more.extend(named("b", 30));
        land(&mut rt, more, |rt| {
            assert!(
                visible(rt, "a0"),
                "the new best result shows, in every frame"
            );
        });
        assert_eq!(
            rt.state
                .c17
                .palette
                .highlighted_item()
                .map(|i| i.id.as_str()),
            Some("b00")
        );
    }

    // The same, through the page's own streamed batches: type "rs" and let the walk finish;
    // the first result shows in every frame.
    #[test]
    fn the_streamed_best_result_shows_in_every_frame() {
        let mut rt = resting("c17-closed-light");
        press(&mut rt, Key::Command('k'), ctrl(false));
        typ(&mut rt, "rs");
        rest_with(&mut rt, 1.5, |rt| {
            if let Some(first) = rt.state.c17.palette.items().first() {
                assert!(visible(rt, &first.id), "{} hidden above the list", first.id);
            }
        });
        assert!(!rt.state.c17.palette.busy());
    }

    // browser/search: "When the first result is preselected…" with the review round 1 B2
    // ruling: a new query scrolls to the top, so its highlighted first result shows, even after
    // the user arrowed far down the old one.
    #[test]
    fn a_new_query_s_highlighted_first_result_shows_after_arrowing_down() {
        let mut rt = resting("c17-closed-light");
        press(&mut rt, Key::Command('k'), ctrl(false));
        typ(&mut rt, "o");
        rest_with(&mut rt, 1.0, |_| {});
        for _ in 0..20 {
            press(&mut rt, Key::Down, Mods::default());
        }
        rest_with(&mut rt, 0.5, |_| {});
        typ(&mut rt, "t");
        let asked = rt.state.c17.palette.generation();
        rest_with(&mut rt, 1.5, |rt| {
            let p = &rt.state.c17.palette;
            if p.generation() == asked && p.highlighted() == Some(0) {
                let id = &p.items()[0].id;
                assert!(
                    visible(rt, id),
                    "the new query's first result {id} is hidden"
                );
            }
        });
        assert_eq!(rt.state.c17.palette.highlighted(), Some(0));
        let first = rt.state.c17.palette.items()[0].id.clone();
        assert!(visible(&rt, &first));
    }

    // Review round 1, SF1: once the user has moved the highlight, a batch that moves the
    // highlighted row keeps it in view, so Enter never opens a row they can't see.
    #[test]
    fn a_moved_highlight_stays_in_view_when_a_batch_pushes_it_down() {
        let mut rt = synthetic();
        land(&mut rt, named("b", 30), |_| {});
        for _ in 0..3 {
            press(&mut rt, Key::Down, Mods::default());
        }
        rest_with(&mut rt, 0.5, |_| {});
        assert_eq!(
            rt.state
                .c17
                .palette
                .highlighted_item()
                .map(|i| i.id.as_str()),
            Some("b03")
        );
        let base = named("b", 30);
        let mut pushed = base[..3].to_vec();
        pushed.extend(named("m", 12));
        pushed.extend(base[3..].iter().cloned());
        land(&mut rt, pushed, |rt| {
            assert!(visible(rt, "b03"), "the highlighted row is out of view");
        });
        assert_eq!(
            rt.state
                .c17
                .palette
                .highlighted_item()
                .map(|i| i.id.as_str()),
            Some("b03")
        );
    }

    // browser/search: "Up and Down move the highlight while the caret stays in the query
    // field" (review round 1, SF2): the caret doesn't move.
    #[test]
    fn up_and_down_leave_the_caret_where_it_was() {
        let mut rt = mount("c17-results-light");
        run(&mut rt, 1.0);
        press(&mut rt, Key::Left, Mods::default());
        let caret = |rt: &Runtime<Gallery>| {
            let e = &rt.state.c17.palette.edit;
            (e.anchor, e.focus)
        };
        assert_eq!(caret(&rt), (1, 1));
        for k in [Key::Down, Key::Down, Key::Up] {
            let name = format!("{k:?}");
            press(&mut rt, k, Mods::default());
            assert_eq!(caret(&rt), (1, 1), "{name} moved the caret");
        }
        assert_eq!(rt.state.c17.palette.highlighted(), Some(1));
    }

    // No bullet (review round 1, SF3): a held Enter opens nothing, even one held from before
    // the results landed; a fresh press then opens.
    #[test]
    fn a_held_enter_opens_nothing_even_through_loading() {
        let mut rt = resting("c17-closed-light");
        press(&mut rt, Key::Command('k'), ctrl(false));
        typ(&mut rt, "notes");
        press(&mut rt, Key::Enter, Mods::default());
        assert!(
            rt.state.c17.palette.items().is_empty(),
            "pressed while loading"
        );
        for _ in 0..60 {
            rt.repeat_key(KeyEvent {
                key: Key::Enter,
                mods: Mods::default(),
            });
            rt.frame(FRAME);
        }
        assert!(
            !rt.state.c17.palette.items().is_empty(),
            "results landed while held"
        );
        assert!(
            rt.state.c17.palette.is_open(),
            "a held Enter opened a result"
        );
        assert_eq!(rt.state.c17.log, "", "nothing opened");
        press(&mut rt, Key::Enter, Mods::default());
        assert!(rt.state.c17.log.starts_with("Opened "));
    }

    // Review round 1, SF4: a new query announces nothing until its first answer; the old
    // list's size would be the wrong count.
    #[test]
    fn a_new_query_announces_no_count_until_its_first_answer() {
        let mut rt = mount("c17-results-light");
        run(&mut rt, 1.0);
        assert!(status(&rt).is_some_and(|s| s.ends_with(" results")));
        typ(&mut rt, "p");
        assert!(
            !rt.state.c17.palette.items().is_empty(),
            "the old results stay"
        );
        let heard = status(&rt).unwrap_or_default();
        assert!(heard.is_empty(), "announced {heard:?} for the stale list");
        run(&mut rt, 0.5);
        let n = rt
            .state
            .c17
            .palette
            .announced()
            .expect("the new query's count");
        assert_eq!(status(&rt), Some(format!("{n} results")));
    }

    // browser/search: "Escape, or a click outside the dialog panel, closes search." Strata's
    // ESCAPE_ANY ignores the modifiers, so Shift+Escape closes too (review round 1, a nit).
    #[test]
    fn shift_escape_closes_as_escape_does() {
        let mut rt = mount("c17-results-light");
        run(&mut rt, 1.0);
        press(
            &mut rt,
            Key::Escape,
            Mods {
                shift: true,
                ..Mods::default()
            },
        );
        assert!(!rt.state.c17.palette.is_open());
        assert_eq!(rt.state.c17.log, "Closed (Escape)");
    }

    // Review round 1, a nit: the jump answers its query in the commit that opens it, as
    // Strata's history search does, so it never shows "Searching…".
    #[test]
    fn the_jump_lists_its_history_in_the_frame_it_opens() {
        let mut rt = resting("c17-closed-light");
        press(&mut rt, Key::Command('k'), ctrl(true));
        assert_eq!(rt.state.c17.palette.phase(), Some(Phase::Results));
        assert_eq!(status(&rt), Some("10 results".into()));
    }

    /// The list's scroll offset, from the first mounted row (rows are one height).
    fn offset(rt: &Runtime<Gallery>) -> f32 {
        let vp = element(rt, &format!("{KEY}-results"))
            .expect("the list")
            .rect;
        for (i, it) in rt.state.c17.palette.items().iter().enumerate() {
            if let Some(e) = element(rt, &row_key(rt, &it.id)) {
                return vp[1] - e.rect[1] + i as f32 * e.rect[3];
            }
        }
        f32::NAN
    }

    /// Moves the pointer over the list and draws a frame, ready to wheel or fling there.
    fn over_list(rt: &mut Runtime<Gallery>) -> (f32, f32) {
        let r = element(rt, &format!("{KEY}-results"))
            .expect("the list")
            .rect;
        let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        rt.pointer_move(x, y);
        rt.frame(FRAME);
        (x, y)
    }

    fn settle(rt: &mut Runtime<Gallery>) {
        for _ in 0..30 {
            rt.frame(FRAME);
        }
    }

    fn highlighted(rt: &Runtime<Gallery>) -> Option<String> {
        rt.state
            .c17
            .palette
            .highlighted_item()
            .map(|i| i.id.clone())
    }

    // Review round 2, B1 repro 1: a wheel and a same-query batch in one frame. The batch never
    // undoes the wheel.
    #[test]
    fn a_batch_in_a_wheel_s_frame_keeps_the_wheel_s_scroll() {
        for reduced in [true, false] {
            let mut rt = synthetic_with(reduced);
            land(&mut rt, named("b", 60), |_| {});
            let (x, y) = over_list(&mut rt);
            rt.wheel(x, y, -200.0);
            let mut more = named("b", 60);
            more.push(item("z0"));
            land(&mut rt, more, |_| {});
            assert!(
                (offset(&rt) - 200.0).abs() < 1.0,
                "reduced motion {reduced}: the batch undid the wheel, offset {}",
                offset(&rt)
            );
        }
    }

    // Review round 2, B1 repro 2: a batch and a Down in one frame, after wheeling the highlight
    // off screen: the arrow's reveal wins, in every frame.
    #[test]
    fn an_arrow_in_a_batch_s_frame_reveals_its_row() {
        let mut rt = synthetic();
        land(&mut rt, named("b", 60), |_| {});
        let (x, y) = over_list(&mut rt);
        rt.wheel(x, y, -800.0);
        settle(&mut rt);
        let mut more = named("b", 60);
        more.push(item("zz"));
        rt.update(move |g, tx| {
            let generation = g.c17.palette.generation();
            let _ = g.c17.palette.deliver(Answer::new(generation, more), tx);
        });
        rt.key(KeyEvent {
            key: Key::Down,
            mods: Mods::default(),
        });
        for _ in 0..30 {
            rt.frame(FRAME);
            assert_eq!(highlighted(&rt).as_deref(), Some("b01"));
            assert!(
                visible(&rt, "b01"),
                "b01 out of view at offset {}",
                offset(&rt)
            );
        }
    }

    // Review round 2, B1 repro 3: a new query's answer and a second batch in one poll, after a
    // wheel: the list goes to the top, its first result shown.
    #[test]
    fn a_new_query_s_answers_in_one_poll_scroll_to_the_top() {
        let mut rt = synthetic();
        land(&mut rt, named("b", 60), |_| {});
        let (x, y) = over_list(&mut rt);
        rt.wheel(x, y, -600.0);
        settle(&mut rt);
        rt.update(|g, tx| {
            let _ = g.c17.palette.set_query("zz-new");
            g.c17.worker.clear();
            let generation = g.c17.palette.generation();
            let _ = g
                .c17
                .palette
                .deliver(Answer::new(generation, named("c", 40)), tx);
            let _ = g
                .c17
                .palette
                .deliver(Answer::new(generation, named("c", 50)), tx);
        });
        for _ in 0..30 {
            rt.frame(FRAME);
            assert!(visible(&rt, "c00"), "c00 hidden at offset {}", offset(&rt));
        }
        assert!(offset(&rt).abs() < 1.0);
    }

    // Review round 2, B1: a batch mid-fling neither stops nor moves the fling, before the user
    // moved the highlight and after (a batch's reveal waits for the list to stop).
    #[test]
    fn a_batch_mid_fling_leaves_the_fling_alone() {
        for moved in [false, true] {
            let (mid, alone) = fling(false, moved);
            let (_, batched) = fling(true, moved);
            assert!(
                alone > mid + 100.0,
                "moved {moved}: the fling ran on: {mid} -> {alone}"
            );
            assert!(
                (batched - alone).abs() < 1.0,
                "moved {moved}: a batch changed the fling: {batched} against {alone}"
            );
        }
    }

    /// A fling over 200 rows, with a batch five frames in if `with_batch`, after a Down if
    /// `moved`: the offsets five frames in and when it stops.
    fn fling(with_batch: bool, moved: bool) -> (f32, f32) {
        {
            let mut rt = synthetic_with(false);
            land(&mut rt, named("b", 200), |_| {});
            if moved {
                press(&mut rt, Key::Down, Mods::default());
                settle(&mut rt);
            }
            let (x, y) = over_list(&mut rt);
            rt.fling(x, y, -3000.0);
            for _ in 0..5 {
                rt.frame(FRAME);
            }
            let mid = offset(&rt);
            if with_batch {
                // Rows added above: a moved highlight's row moves, so its reveal is due.
                let mut more = vec![item("a0")];
                more.extend(named("b", 200));
                rt.update(move |g, tx| {
                    let generation = g.c17.palette.generation();
                    let _ = g.c17.palette.deliver(Answer::new(generation, more), tx);
                });
            }
            for _ in 0..90 {
                rt.frame(FRAME);
            }
            (mid, offset(&rt))
        }
    }

    // Review round 2, SF1: an arrow reveals its row, in every frame (reduced motion: no glide).
    #[test]
    fn arrows_keep_the_highlighted_row_in_view() {
        let mut rt = synthetic();
        land(&mut rt, named("b", 60), |_| {});
        for i in 1..=14 {
            press(&mut rt, Key::Down, Mods::default());
            let id = format!("b{i:02}");
            assert!(
                visible(&rt, &id),
                "{id} out of view at offset {}",
                offset(&rt)
            );
        }
    }

    // Review round 2, SF2: before the user moves the highlight, a batch adding rows above keeps
    // a non-zero offset (Strata's restore_scroll).
    #[test]
    fn a_batch_keeps_a_wheeled_offset_until_the_user_moves() {
        let mut rt = synthetic();
        land(&mut rt, named("b", 60), |_| {});
        let (x, y) = over_list(&mut rt);
        rt.wheel(x, y, -200.0);
        rt.frame(FRAME);
        let before = offset(&rt);
        assert!(
            (before - 200.0).abs() < 1.0,
            "the wheel scrolled to {before}"
        );
        let mut more = vec![item("a0"), item("a1")];
        more.extend(named("b", 60));
        land(&mut rt, more, |rt| {
            assert!(
                (offset(rt) - before).abs() < 1.0,
                "offset {} not kept",
                offset(rt)
            );
        });
    }

    // Review round 2, SF3 (ruling: follow Strata): after the user moved the highlight, any
    // changed batch reveals the highlighted row, even one that leaves its index alone.
    #[test]
    fn after_moving_any_changed_batch_reveals_the_highlighted_row() {
        let mut rt = synthetic();
        land(&mut rt, named("b", 60), |_| {});
        for _ in 0..3 {
            press(&mut rt, Key::Down, Mods::default());
        }
        let (x, y) = over_list(&mut rt);
        rt.wheel(x, y, -800.0);
        settle(&mut rt);
        assert!(!visible(&rt, "b03"), "wheeled away");
        let mut more = named("b", 60);
        more.push(item("zz"));
        land(&mut rt, more, |_| {});
        assert!(
            visible(&rt, "b03"),
            "b03 out of view at offset {}",
            offset(&rt)
        );
    }

    // Review round 2, SF4 (ruling: follow Strata): after the user moved the highlight, rows a
    // batch adds above at offset 0 stay visible while the highlighted row would stay in view.
    #[test]
    fn after_moving_rows_added_above_show_while_the_highlight_stays_in_view() {
        let mut rt = synthetic();
        land(&mut rt, named("b", 30), |_| {});
        press(&mut rt, Key::Down, Mods::default());
        settle(&mut rt);
        let mut more = vec![item("a0"), item("a1")];
        more.extend(named("b", 30));
        land(&mut rt, more, |rt| {
            assert!(visible(rt, "a0"), "a0 hidden at offset {}", offset(rt));
            assert!(visible(rt, "b01"), "the highlight left the view");
        });
    }

    // Review round 2, a nit: while an input method composes, Up and Down are its own; the
    // highlight stays.
    #[test]
    fn arrows_while_composing_leave_the_highlight() {
        let mut rt = mount("c17-results-light");
        run(&mut rt, 1.0);
        rt.ime(Ime::Preedit("ni".into(), None));
        rt.frame(FRAME);
        press(&mut rt, Key::Down, Mods::default());
        assert_eq!(rt.state.c17.palette.highlighted(), Some(0));
    }

    // Review round 2, B1: after the user moved the highlight, a batch the page's poll delivers
    // while a wheel's scroll is still landing leaves it alone: the tick sees the list move and
    // the batch's reveal waits (Strata's interaction_revision). A batch handed over in the
    // wheel's own frame, before the wheel is applied, still reveals (see the module docs).
    #[test]
    fn after_moving_a_batch_during_a_wheel_keeps_the_wheel_s_scroll() {
        let mut rt = synthetic();
        land(&mut rt, named("b", 60), |_| {});
        press(&mut rt, Key::Down, Mods::default());
        settle(&mut rt);
        let (x, y) = over_list(&mut rt);
        rt.wheel(x, y, -400.0);
        rt.frame(FRAME);
        let mut more = vec![item("a0")];
        more.extend(named("b", 60));
        rt.update(move |g, _| {
            let generation = g.c17.palette.generation();
            g.c17
                .worker
                .push((g.c17.now, Answer::new(generation, more)));
        });
        settle(&mut rt);
        assert_eq!(rt.state.c17.palette.items()[0].id, "a0", "the batch landed");
        assert!(
            (offset(&rt) - 400.0).abs() < 1.0,
            "the batch undid the wheel: offset {}",
            offset(&rt)
        );
    }

    // Review round 2 (mutant S7): a new query starts unmoved, as Strata's begin_query clears
    // navigation_started. After arrowing in the old query, the new one's batches keep the
    // offset, so results added above its first rows show rather than the list chasing its
    // highlight.
    #[test]
    fn a_new_query_starts_unmoved_after_arrowing_in_the_old_one() {
        let mut rt = synthetic();
        land(&mut rt, named("b", 30), |_| {});
        for _ in 0..3 {
            press(&mut rt, Key::Down, Mods::default());
        }
        rt.update(|g, _| {
            let _ = g.c17.palette.set_query("zz-other");
            g.c17.worker.clear();
        });
        rt.frame(FRAME);
        land(&mut rt, named("c", 30), |_| {});
        let mut more = named("a", 12);
        more.extend(named("c", 30));
        land(&mut rt, more, |rt| {
            assert!(visible(rt, "a00"), "a00 hidden at offset {}", offset(rt));
        });
        assert_eq!(highlighted(&rt).as_deref(), Some("c00"));
    }

    /// Queues `items` for the current query on the page's fake worker: its poll delivers them
    /// at the start of the next frame.
    fn queue(rt: &mut Runtime<Gallery>, items: Vec<PaletteItem>) {
        rt.update(move |g, _| {
            let generation = g.c17.palette.generation();
            g.c17
                .worker
                .push((g.c17.now, Answer::new(generation, items)));
        });
    }

    fn two_above() -> Vec<PaletteItem> {
        let mut more = vec![item("a0"), item("a1")];
        more.extend(named("b", 30));
        more
    }

    // Review round 3, B1: a click held while a batch moves another result into its row opens
    // nothing, as GTK's ListBox activates only when press and release land on the same row.
    // Left and middle.
    #[test]
    fn a_press_held_while_a_batch_moves_its_row_opens_nothing() {
        for button in [PointerButton::Primary, PointerButton::Middle] {
            let mut rt = synthetic();
            land(&mut rt, named("b", 30), |_| {});
            let (x, y) = centre(&rt, &row_key(&rt, "b03"));
            rt.pointer_move(x, y);
            rt.frame(FRAME);
            rt.button_down(x, y, button);
            rt.frame(FRAME);
            queue(&mut rt, two_above());
            rt.frame(FRAME);
            rt.frame(FRAME);
            assert_eq!(
                rt.state.c17.palette.items()[3].id,
                "b01",
                "{button:?}: b01 under it"
            );
            rt.button_up(x, y, button);
            rt.frame(FRAME);
            assert_eq!(rt.state.c17.log, "", "{button:?} opened something");
            assert!(rt.state.c17.palette.is_open(), "{button:?}");
        }
    }

    // Review round 3, B1: the same for a right-click: no menu, and the highlight stays.
    #[test]
    fn a_right_press_held_while_a_batch_moves_its_row_asks_for_no_menu() {
        let mut rt = synthetic();
        land(&mut rt, named("b", 30), |_| {});
        let (x, y) = centre(&rt, &row_key(&rt, "b03"));
        rt.pointer_move(x, y);
        rt.frame(FRAME);
        rt.button_down(x, y, PointerButton::Secondary);
        rt.frame(FRAME);
        queue(&mut rt, two_above());
        rt.frame(FRAME);
        rt.frame(FRAME);
        rt.button_up(x, y, PointerButton::Secondary);
        rt.frame(FRAME);
        assert_eq!(rt.state.c17.menu.as_ref().map(|m| m.0.as_str()), None);
        assert_eq!(highlighted(&rt).as_deref(), Some("b00"));
    }

    // Review round 3, B1: the guard leaves ordinary clicks alone, a batch landing before the
    // press or after the release included.
    #[test]
    fn a_click_between_batches_still_acts_on_its_row() {
        for button in [
            PointerButton::Primary,
            PointerButton::Middle,
            PointerButton::Secondary,
        ] {
            let mut rt = synthetic();
            land(&mut rt, named("b", 30), |_| {});
            queue(&mut rt, two_above());
            rt.frame(FRAME);
            rt.frame(FRAME);
            let (x, y) = centre(&rt, &row_key(&rt, "b03"));
            rt.pointer_move(x, y);
            rt.frame(FRAME);
            rt.button_down(x, y, button);
            rt.frame(FRAME);
            rt.button_up(x, y, button);
            rt.frame(FRAME);
            match button {
                PointerButton::Primary => assert_eq!(rt.state.c17.log, "Opened b03"),
                PointerButton::Middle => assert_eq!(rt.state.c17.log, "Opened b03 in a new tab"),
                _ => assert_eq!(
                    rt.state.c17.menu.as_ref().map(|m| m.0.as_str()),
                    Some("b03")
                ),
            }
        }
    }

    // Review round 3, B2: right after an arrow scrolls the list, a batch in the next frame
    // still reveals the highlighted row: the palette's own scroll isn't the user scrolling.
    // With and without reduced motion (a reveal glides over frames without it), and with up to
    // three frames between.
    #[test]
    fn a_batch_right_after_an_arrow_scroll_keeps_the_highlight_in_view() {
        for reduced in [true, false] {
            for gap in 0..4 {
                let mut rt = synthetic_with(reduced);
                land(&mut rt, named("b", 60), |_| {});
                let top =
                    |rt: &Runtime<Gallery>| element(rt, &option_key(KEY, 0)).map(|e| e.rect[1]);
                let mut n = 0;
                loop {
                    let before = top(&rt);
                    press(&mut rt, Key::Down, Mods::default());
                    n += 1;
                    if top(&rt) != before || n > 30 {
                        break;
                    }
                }
                for _ in 0..gap {
                    rt.frame(FRAME);
                }
                let mut more = named("a", 4);
                more.extend(named("b", 60));
                queue(&mut rt, more);
                let id = format!("b{n:02}");
                for f in 0..60 {
                    rt.frame(FRAME);
                    if f >= 20 || reduced {
                        assert!(
                            visible(&rt, &id),
                            "reduced {reduced}, gap {gap}, frame {f}: {id} out of view at {}",
                            offset(&rt)
                        );
                    }
                }
                press(&mut rt, Key::Enter, Mods::default());
                assert_eq!(rt.state.c17.log, format!("Opened {id}"));
            }
        }
    }

    // Review round 3 (mutant T5): an arrow scrolls only as far as it takes: the row it reaches
    // past the bottom lands at the bottom, not in the middle.
    #[test]
    fn an_arrow_scrolls_only_as_far_as_it_takes() {
        let mut rt = synthetic();
        land(&mut rt, named("b", 60), |_| {});
        let vp = element(&rt, &results_key(KEY)).expect("the list").rect;
        let mut i = 0;
        while offset(&rt) < 1.0 {
            press(&mut rt, Key::Down, Mods::default());
            i += 1;
            assert!(i < 30);
        }
        let row = element(&rt, &row_key(&rt, &format!("b{i:02}")))
            .expect("the row")
            .rect;
        assert!(
            (row[1] + row[3] - (vp[1] + vp[3])).abs() < 1.0,
            "b{i:02} ends at {} where the list ends at {}",
            row[1] + row[3],
            vp[1] + vp[3]
        );
    }

    // Review round 3, B2 (the batch's side): after the user moved the highlight, a batch whose
    // reveal scrolls the list and another batch in the next frame: the second still reveals,
    // since the first's scroll was the palette's own, not the user's.
    #[test]
    fn back_to_back_batches_keep_a_moved_highlight_in_view() {
        let mut rt = synthetic();
        land(&mut rt, named("b", 60), |_| {});
        for _ in 0..3 {
            press(&mut rt, Key::Down, Mods::default());
        }
        settle(&mut rt);
        let mut first = named("a", 12);
        first.extend(named("b", 60));
        queue(&mut rt, first);
        rt.frame(FRAME);
        assert!(visible(&rt, "b03"), "the first batch revealed b03");
        let mut second = named("m", 12);
        second.extend(named("a", 12));
        second.extend(named("b", 60));
        queue(&mut rt, second);
        for f in 0..30 {
            rt.frame(FRAME);
            assert!(
                visible(&rt, "b03"),
                "frame {f}: b03 out of view at {}",
                offset(&rt)
            );
        }
    }
}

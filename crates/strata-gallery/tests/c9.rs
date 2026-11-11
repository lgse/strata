//! C9's page: what the drawer does in process, beside its goldens and scripts. Each test that
//! checks a feature-map Behavior bullet names it, as a script would, where a script can't:
//! states in the middle of a slide or a drag, and paint.

mod c9 {
    use rune::{Env, Key, KeyEvent, Mods, Runtime};
    use serde_json::Value;
    use strata_gallery::{Gallery, scenes};

    const DT: f64 = 1.0 / 60.0;

    fn runtime(name: &str, scale: f32) -> Runtime<Gallery> {
        let f = scenes::all_scenes()
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, f)| f)
            .unwrap_or_else(|| panic!("no scene {name}"));
        let mut g = Gallery::default();
        f(&mut g);
        let env = Env::new(g.dark, scale);
        let mut rt = strata_gallery::runtime(g, env);
        rt.frame(0.0);
        rt
    }

    fn scene(name: &str) -> Runtime<Gallery> {
        let mut rt = runtime(name, 1.0);
        settle(&mut rt, 10);
        rt
    }

    fn settle(rt: &mut Runtime<Gallery>, frames: usize) {
        for _ in 0..frames {
            rt.frame(DT);
        }
    }

    /// A key, then one frame: what the next frame shows.
    fn key1(rt: &mut Runtime<Gallery>, key: Key, mods: Mods) {
        rt.key(KeyEvent { key, mods });
        rt.frame(DT);
    }

    fn key(rt: &mut Runtime<Gallery>, key: Key) {
        key1(rt, key, Mods::default());
        rt.frame(DT);
    }

    fn space(rt: &mut Runtime<Gallery>) {
        key(rt, Key::Text(" ".into()));
    }

    fn at(rt: &Runtime<Gallery>, key: &str) -> (f32, f32) {
        rt.find(key).unwrap_or_else(|| panic!("no element {key}"))
    }

    /// A click on `key`, then one frame.
    fn tap1(rt: &mut Runtime<Gallery>, key: &str) {
        let (x, y) = at(rt, key);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(DT);
        rt.pointer_up(x, y);
        rt.frame(DT);
    }

    fn tap(rt: &mut Runtime<Gallery>, key: &str) {
        tap1(rt, key);
        rt.frame(DT);
    }

    fn typed(rt: &mut Runtime<Gallery>, s: &str) {
        for c in s.chars() {
            key(rt, Key::Text(c.to_string()));
        }
    }

    fn info(rt: &Runtime<Gallery>, key: &str) -> Option<rune::ElementInfo> {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
    }

    fn focused(rt: &Runtime<Gallery>) -> Option<String> {
        rt.elements()
            .into_iter()
            .find(|e| e.focused)
            .and_then(|e| e.key)
    }

    /// Whether focus is in the find bar's query field (its own focusable is rune-kit's).
    fn in_find_field(rt: &Runtime<Gallery>) -> bool {
        rt.elements()
            .into_iter()
            .any(|e| e.focused && e.path.contains("preview-find-field"))
    }

    fn exiting(rt: &Runtime<Gallery>, key: &str) -> Option<bool> {
        info(rt, key).map(|e| e.exiting)
    }

    /// The accessibility node keyed `key`, as assistive technology reads it.
    fn node(rt: &Runtime<Gallery>, key: &str) -> Option<Value> {
        fn find(v: &Value, key: &str) -> Option<Value> {
            if v["key"] == key {
                return Some(v.clone());
            }
            v["children"].as_array()?.iter().find_map(|c| find(c, key))
        }
        find(&rt.accessibility_json(), key)
    }

    /// The divider's value: the width assistive technology hears.
    fn value(rt: &Runtime<Gallery>) -> String {
        node(rt, "preview-grip").expect("a grip")["value"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    fn status(rt: &Runtime<Gallery>) -> Option<String> {
        node(rt, "preview-find-status").map(|n| n["name"].as_str().unwrap_or_default().to_owned())
    }

    #[test]
    fn left_from_the_content_returns_focus_to_the_cursor_row() {
        let mut rt = scene("c9-list-open-light");
        key(&mut rt, Key::Tab);
        key(&mut rt, Key::Right);
        assert_eq!(focused(&rt).as_deref(), Some("fake-content"));
        key(&mut rt, Key::Left);
        assert_eq!(focused(&rt).as_deref(), Some("row-1"));
    }

    #[test]
    fn the_find_keys_open_find_only_while_the_view_is_searchable() {
        let mut rt = scene("c9-list-open-light");
        key(&mut rt, Key::Tab);
        key(&mut rt, Key::Right);
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        key1(&mut rt, Key::Command('f'), ctrl);
        assert!(rt.state.c9.drawer.is_finding());
        assert!(in_find_field(&rt), "the query field takes focus");
        typed(&mut rt, "drawer");
        key(&mut rt, Key::Escape);
        assert!(!rt.state.c9.drawer.is_finding());
        assert_eq!(
            focused(&rt).as_deref(),
            Some("fake-content"),
            "keys go back to the document"
        );
        assert_eq!(
            rt.state.c9.drawer.find_query(),
            "drawer",
            "the query outlives the bar"
        );

        // Rendered, the fake document is table-only: no find, and the keys pass on.
        tap(&mut rt, "preview-view-switch");
        tap(&mut rt, "fake-content");
        key1(&mut rt, Key::Command('f'), ctrl);
        assert!(!rt.state.c9.drawer.is_finding());
    }

    /// `preview/preview-panel/find`: The status reads "Match 1 of N", or "No matches", and is
    /// hidden while the query is empty.
    #[test]
    fn the_find_status_reads_the_match_or_no_matches_and_hides_while_the_query_is_empty() {
        let mut rt = scene("c9-list-open-light");
        tap(&mut rt, "preview-find");
        assert_eq!(status(&rt), None, "hidden while the query is empty");
        typed(&mut rt, "preview");
        assert_eq!(status(&rt).as_deref(), Some("Match 1 of 5"));
        typed(&mut rt, "zzz");
        assert_eq!(status(&rt).as_deref(), Some("No matches"));
        for _ in 0.."previewzzz".len() {
            key(&mut rt, Key::Backspace);
        }
        assert_eq!(status(&rt), None);
    }

    #[test]
    fn the_status_waits_for_the_view_to_answer() {
        let mut rt = scene("c9-list-open-light");
        rt.update(|g, _| g.c9.view.pending = true);
        tap(&mut rt, "preview-find");
        typed(&mut rt, "preview");
        assert_eq!(
            status(&rt),
            None,
            "no \"No matches\" before the view answers"
        );
        rt.update(|g, _| g.c9.view.pending = false);
        settle(&mut rt, 2);
        assert_eq!(status(&rt).as_deref(), Some("Match 1 of 5"));
    }

    #[test]
    fn reopening_find_runs_its_query_again() {
        let mut rt = scene("c9-list-open-light");
        tap(&mut rt, "preview-find");
        typed(&mut rt, "preview");
        key(&mut rt, Key::Escape);
        assert_eq!(
            rt.state.c9.view_status().map(|s| s.total),
            Some(0),
            "the view dropped its matches"
        );
        tap(&mut rt, "preview-find");
        assert_eq!(rt.state.c9.view_status().map(|s| s.total), Some(5));
        assert_eq!(status(&rt).as_deref(), Some("Match 1 of 5"));
    }

    #[test]
    fn the_find_bar_goes_with_the_views_searchable_text() {
        let mut rt = scene("c9-find-open-light");
        assert!(node(&rt, "preview-find-bar").is_some());
        tap(&mut rt, "preview-view-switch");
        assert!(
            node(&rt, "preview-find-bar").is_none(),
            "the table-only form has nothing to search"
        );
        tap(&mut rt, "preview-view-switch");
        assert!(node(&rt, "preview-find-bar").is_some());
    }

    #[test]
    fn the_close_button_closes_reports_it_and_ends_find() {
        let mut rt = scene("c9-list-open-light");
        tap(&mut rt, "preview-find");
        typed(&mut rt, "preview");
        assert_eq!(rt.state.c9.view_status().map(|s| s.total), Some(5));
        tap(&mut rt, "preview-close");
        let d = &rt.state.c9;
        assert!(!d.drawer.is_open());
        assert!(
            d.dismissed,
            "Strata hears the close and marks the preview dismissed"
        );
        assert_eq!(
            d.view_status().map(|s| s.total),
            Some(0),
            "the view drops its matches"
        );
        assert_eq!(d.drawer.find_query(), "");
    }

    #[test]
    fn a_host_close_ends_find_on_the_view_too() {
        let mut rt = scene("c9-list-open-light");
        tap(&mut rt, "preview-find");
        typed(&mut rt, "preview");
        tap(&mut rt, "row-2");
        space(&mut rt);
        assert!(
            !rt.state.c9.drawer.is_open(),
            "Space on the listing closed the preview"
        );
        assert_eq!(rt.state.c9.view_status().map(|s| s.total), Some(0));
    }

    #[test]
    fn the_wrap_and_view_controls_report_their_intents() {
        let mut rt = scene("c9-list-open-light");
        let checked = |rt: &Runtime<Gallery>| {
            node(rt, "preview-wrap").is_some_and(|n| {
                n["states"]
                    .as_array()
                    .is_some_and(|s| s.iter().any(|x| x == "checked"))
            })
        };
        assert!(checked(&rt));
        tap(&mut rt, "preview-wrap");
        assert!(!rt.state.c9.view.wrap);
        assert!(!checked(&rt));
        tap(&mut rt, "preview-view-switch");
        assert_eq!(
            node(&rt, "preview-view-switch").unwrap()["name"],
            "View source"
        );
    }

    /// `preview/preview-panel/layout`: In Columns with an automatic width, widening or
    /// narrowing the window resizes the preview or empty slot in one frame, without the
    /// boundary jumping back and forth.
    #[test]
    fn a_window_resize_lands_the_automatic_width_in_one_frame() {
        let mut rt = scene("c9-columns-open-light");
        assert_eq!(value(&rt), "359");
        rt.set_viewport((1100.0, 640.0), 1.0);
        rt.frame(DT);
        assert_eq!(value(&rt), "499", "the frame the window resizes");
        assert!(!rt.animating, "with nothing left moving");
        settle(&mut rt, 20);
        assert_eq!(value(&rt), "499", "and it stays");
    }

    /// `preview/preview-panel/layout`: At 4× display scaling the preview stays in place
    /// without alternating between layouts.
    #[test]
    fn at_four_times_scale_the_preview_holds_its_layout() {
        let mut rt = runtime("c9-columns-open-light", 4.0);
        settle(&mut rt, 5);
        let first = value(&rt);
        let commits = rt.stats.commits;
        settle(&mut rt, 60);
        assert_eq!(value(&rt), first);
        assert_eq!(
            rt.stats.commits, commits,
            "no layout alternates with another"
        );
    }

    /// `preview/preview-panel/layout`: Reopening the preview during its slide-out, such as
    /// Space twice quickly in List, turns the drawer back from its current position.
    #[test]
    fn closing_keeps_the_content_inert_until_the_slide_ends_and_reopening_turns_back() {
        let mut rt = scene("c9-list-open-light");
        space(&mut rt);
        assert_eq!(
            exiting(&rt, "preview-drawer"),
            Some(true),
            "the content leaves with the slide"
        );
        settle(&mut rt, 40);
        assert_eq!(
            exiting(&rt, "preview-drawer"),
            None,
            "and goes when it ends"
        );

        space(&mut rt);
        settle(&mut rt, 40);
        space(&mut rt);
        settle(&mut rt, 4);
        space(&mut rt);
        assert_eq!(
            exiting(&rt, "preview-drawer"),
            Some(false),
            "reopened mid-slide, it stays"
        );
        settle(&mut rt, 40);
        assert!(node(&rt, "preview-drawer").is_some());
    }

    /// `preview/preview-panel/layout`: Closing the preview mid-slide returns keyboard focus to
    /// the files at once; only content teardown waits for the slide.
    #[test]
    fn closing_mid_slide_returns_focus_to_the_files_at_once() {
        let mut rt = scene("c9-list-open-light");
        space(&mut rt);
        settle(&mut rt, 40);
        key(&mut rt, Key::Tab);
        assert_eq!(focused(&rt).as_deref(), Some("row-1"));
        // Space opens; Right moves into the content while the drawer still slides.
        key1(&mut rt, Key::Text(" ".into()), Mods::default());
        key1(&mut rt, Key::Right, Mods::default());
        assert_eq!(focused(&rt).as_deref(), Some("fake-content"));
        assert!(rt.animating, "still sliding open");
        key1(&mut rt, Key::Escape, Mods::default());
        assert_eq!(
            focused(&rt).as_deref(),
            Some("row-1"),
            "focus is back the next frame"
        );
        assert_eq!(
            exiting(&rt, "preview-drawer"),
            Some(true),
            "while the content slides away"
        );
    }

    /// `preview/preview-panel/layout`: A reserved Columns slot never slides.
    #[test]
    fn a_reserved_columns_slot_never_slides() {
        let mut rt = scene("c9-columns-reserved-light");
        key1(&mut rt, Key::Text(" ".into()), Mods::default());
        assert!(node(&rt, "preview-drawer").is_some());
        assert!(!rt.animating, "content appears in the slot at once");
        settle(&mut rt, 2);
        key1(&mut rt, Key::Text(" ".into()), Mods::default());
        assert!(!rt.animating, "and leaves it in place");
    }

    /// `preview/preview-panel/layout`: Space, **i**, Esc, and the close button dismiss the
    /// preview content and keep the slot; **Appearance → Preview panel** off releases it.
    /// (`i` is a Strata key binding; the gallery's Space and Escape stand in for its keys.)
    #[test]
    fn dismissing_in_columns_keeps_the_slot_until_released() {
        let mut rt = scene("c9-columns-open-light");
        // The files' laid-out width: what the slot leaves them, compared, never a pixel value.
        let files = |rt: &Runtime<Gallery>| info(rt, "preview-files").expect("the files").target[2];
        let open = files(&rt);
        let kept = |rt: &Runtime<Gallery>, how: &str| {
            let d = &rt.state.c9.drawer;
            assert!(
                d.is_reserved() && !d.is_open(),
                "{how}: dismissed, still reserved"
            );
            assert!(node(rt, "preview-grip").is_none(), "{how}: no grip");
            assert!(!rt.animating, "{how}: nothing slides");
            assert_eq!(files(rt), open, "{how}: the columns don't move");
        };
        tap1(&mut rt, "preview-close");
        kept(&rt, "the close button");
        space(&mut rt);
        assert_eq!(files(&rt), open);
        key1(&mut rt, Key::Escape, Mods::default());
        kept(&rt, "Escape");
        space(&mut rt);
        key1(&mut rt, Key::Text(" ".into()), Mods::default());
        kept(&rt, "Space");
        rt.update(|g, _| g.c9.drawer.reserve(false));
        settle(&mut rt, 2);
        assert!(!rt.state.c9.drawer.is_reserved());
        assert!(
            files(&rt) > open,
            "Appearance → Preview panel off releases the space"
        );
    }

    #[test]
    fn home_and_end_take_the_divider_to_its_ends() {
        let mut rt = scene("c9-list-open-light");
        key(&mut rt, Key::Tab);
        key(&mut rt, Key::Function(8));
        assert_eq!(focused(&rt).as_deref(), Some("preview-grip"));
        key(&mut rt, Key::End);
        assert_eq!(value(&rt), "659");
        key(&mut rt, Key::Home);
        assert_eq!(value(&rt), "300");
        key(&mut rt, Key::Up);
        assert_eq!(value(&rt), "308", "Up widens a step");
    }

    /// `browser/view-modes/columns`: Hovering the edge of the column that meets the preview
    /// also lights the preview divider line (the drawer's half: the host lights it).
    #[test]
    fn the_host_lights_the_divider_line() {
        let mut rt = scene("c9-columns-open-light");
        let lit = |rt: &Runtime<Gallery>| info(rt, "preview-divider-lit").map(|e| e.opacity);
        assert_eq!(lit(&rt), Some(0.0));
        rt.update(|g, _| g.c9.drawer.light_line(true));
        settle(&mut rt, 30);
        assert_eq!(lit(&rt), Some(1.0), "lit with no hover");
        assert_eq!(
            info(&rt, "preview-divider-strip").map(|e| e.opacity),
            Some(0.0),
            "the grip keeps its own hover"
        );
        rt.update(|g, _| g.c9.drawer.light_line(false));
        settle(&mut rt, 30);
        assert_eq!(lit(&rt), Some(0.0));
    }

    /// `preview/preview-panel/layout`: Hovering the grip or divider line lights the line and
    /// tints the grip; after 400 ms, "Preview panel minimum width" appears. And: Crossing
    /// between the divider line and the grip keeps the caption; pressing hides it in favor of
    /// the outline.
    #[test]
    fn the_caption_follows_hover_and_gives_way_to_the_outline() {
        let mut rt = scene("c9-list-open-light");
        let lit = |rt: &Runtime<Gallery>, key: &str| info(rt, key).map(|e| e.opacity);
        assert_eq!(lit(&rt, "preview-divider-lit"), Some(0.0));
        let (lx, ly) = at(&rt, "preview-divider-line");
        rt.pointer_move(lx, ly);
        settle(&mut rt, 6);
        assert_eq!(
            lit(&rt, "preview-divider-lit"),
            Some(1.0),
            "the line lights"
        );
        assert_eq!(
            lit(&rt, "preview-divider-strip"),
            Some(1.0),
            "the grip tints"
        );
        assert!(
            node(&rt, "preview-divider-caption").is_none(),
            "not before 400 ms"
        );
        settle(&mut rt, 24);
        assert_eq!(
            node(&rt, "preview-divider-caption").map(|n| n["name"].clone()),
            Some(Value::from("Preview panel minimum width"))
        );
        let (gx, gy) = at(&rt, "preview-divider-strip");
        rt.pointer_move(gx, gy);
        settle(&mut rt, 2);
        assert!(
            node(&rt, "preview-divider-caption").is_some(),
            "crossing keeps it"
        );
        rt.pointer_down(gx, gy);
        rt.pointer_move(gx - 40.0, gy);
        settle(&mut rt, 2);
        assert!(
            node(&rt, "preview-divider-caption").is_none(),
            "pressing hides it"
        );
        assert!(
            node(&rt, "preview-minimum-outline").is_some(),
            "for the outline"
        );
        rt.pointer_up(gx - 40.0, gy);
    }

    /// `preview/preview-panel/layout`: Pointer-dragging the divider shows a dashed outline …
    /// captioned "Preview panel minimum width", which fades on release. (The dashes and the
    /// outline's span are paint and geometry, left to the eye.)
    #[test]
    fn the_outline_shows_while_dragging_and_fades_on_release() {
        let mut rt = scene("c9-list-open-light");
        let (x, y) = at(&rt, "preview-grip");
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_move(x - 60.0, y);
        settle(&mut rt, 2);
        assert_eq!(
            node(&rt, "preview-minimum-outline").map(|n| n["name"].clone()),
            Some(Value::from("Preview panel minimum width"))
        );
        rt.pointer_up(x - 60.0, y);
        rt.frame(DT);
        assert_eq!(
            exiting(&rt, "preview-minimum-outline"),
            Some(true),
            "it fades"
        );
        settle(&mut rt, 40);
        assert_eq!(exiting(&rt, "preview-minimum-outline"), None, "and goes");
        assert_eq!(value(&rt), "492");
    }

    #[test]
    fn escape_cancels_a_drag_and_puts_the_width_back() {
        let mut rt = scene("c9-list-open-light");
        assert_eq!(value(&rt), "432");
        let (x, y) = at(&rt, "preview-grip");
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_move(x - 100.0, y);
        settle(&mut rt, 2);
        assert_eq!(value(&rt), "532");
        key(&mut rt, Key::Escape);
        rt.pointer_up(x - 100.0, y);
        settle(&mut rt, 30);
        assert_eq!(value(&rt), "432", "back to the automatic width");
        assert!(node(&rt, "preview-minimum-outline").is_none());
        assert!(
            rt.state.c9.drawer.is_open(),
            "Escape ended the drag, not the preview"
        );
    }

    #[test]
    fn a_drag_cut_short_by_a_narrow_window_leaves_no_outline() {
        let mut rt = scene("c9-list-open-light");
        let (x, y) = at(&rt, "preview-grip");
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_move(x - 40.0, y);
        settle(&mut rt, 2);
        assert!(node(&rt, "preview-minimum-outline").is_some());
        rt.set_viewport((510.0, 640.0), 1.0);
        settle(&mut rt, 4);
        assert!(
            node(&rt, "preview-drawer").is_none(),
            "too narrow: the pane hid mid-drag"
        );
        rt.pointer_up(100.0, y);
        rt.set_viewport((960.0, 640.0), 1.0);
        settle(&mut rt, 30);
        assert!(node(&rt, "preview-drawer").is_some());
        assert!(
            node(&rt, "preview-minimum-outline").is_none(),
            "no drag outlives the pane"
        );
    }

    #[test]
    fn a_narrow_window_drops_the_find_fields_focus() {
        let mut rt = scene("c9-list-open-light");
        tap(&mut rt, "preview-find");
        typed(&mut rt, "pre");
        assert!(in_find_field(&rt));
        rt.set_viewport((510.0, 640.0), 1.0);
        settle(&mut rt, 4);
        key(&mut rt, Key::Down);
        rt.set_viewport((960.0, 640.0), 1.0);
        settle(&mut rt, 4);
        assert!(
            node(&rt, "preview-find-bar").is_some(),
            "find is still open"
        );
        assert!(!in_find_field(&rt), "but widening doesn't take focus back");
    }

    #[test]
    fn escape_goes_to_the_host_while_the_find_bar_is_hidden() {
        let mut rt = scene("c9-find-open-light");
        // Rendered, the fake document has nothing to search: find stays open, its bar goes.
        tap(&mut rt, "preview-view-switch");
        assert!(rt.state.c9.drawer.is_finding() && node(&rt, "preview-find-bar").is_none());
        tap(&mut rt, "fake-content");
        key(&mut rt, Key::Escape);
        assert!(
            !rt.state.c9.drawer.is_open(),
            "the first Escape reaches the host"
        );
    }

    #[test]
    fn the_find_field_doesnt_take_focus_back_when_the_view_is_searchable_again() {
        let mut rt = scene("c9-find-open-light");
        assert!(in_find_field(&rt));
        rt.update(|g, _| {
            g.c9.view.form = Some(strata_components::preview_drawer::DocumentView::Rendered)
        });
        settle(&mut rt, 4);
        tap(&mut rt, "row-2");
        rt.update(|g, _| {
            g.c9.view.form = Some(strata_components::preview_drawer::DocumentView::Source)
        });
        settle(&mut rt, 4);
        assert!(node(&rt, "preview-find-bar").is_some());
        assert!(!in_find_field(&rt), "focus stays on the listing");
        assert_eq!(focused(&rt).as_deref(), Some("row-2"));
    }

    #[test]
    fn restarting_find_in_one_handler_keeps_the_hosts_search() {
        use strata_components::preview_drawer::HostedPreview;
        let mut rt = scene("c9-list-open-light");
        tap(&mut rt, "preview-find");
        typed(&mut rt, "preview");
        rt.update(|g, _| {
            g.c9.drawer.reset_find();
            g.c9.drawer.start_find("preview");
            g.c9.view.find("preview");
        });
        settle(&mut rt, 4);
        assert_eq!(
            rt.state.c9.view_status().map(|s| s.total),
            Some(5),
            "no stale find_end lands after it"
        );
    }

    #[test]
    fn closing_the_drawer_after_find_closed_ends_find_once() {
        use strata_components::preview_drawer::HostedPreview;
        let mut rt = scene("c9-list-open-light");
        tap(&mut rt, "preview-find");
        typed(&mut rt, "preview");
        key(&mut rt, Key::Escape);
        assert_eq!(
            rt.state.c9.view_status().map(|s| s.total),
            Some(0),
            "closing the bar ended find"
        );
        // The host searches on its own afterwards.
        rt.update(|g, _| g.c9.view.find("drawer"));
        key(&mut rt, Key::Escape);
        settle(&mut rt, 2);
        assert!(!rt.state.c9.drawer.is_open());
        assert_eq!(
            rt.state.c9.view_status().map(|s| s.total),
            Some(2),
            "and the close doesn't end it again"
        );
    }
}

//! C10b's page: what the code view and the script editor do in process, beside their goldens
//! and scripts. Each test that checks a feature-map Behavior bullet names it, as a script
//! would, where a script can't: positions in the middle of a drag or a scroll, values that
//! assistive technology sets, and the editor's keys and colours.

mod c10 {
    use std::time::{Duration, Instant};

    use rune::accesskit::{Action, ActionData};
    use rune::{Env, Key, KeyEvent, Mods, Runtime, clipboard};
    use serde_json::Value;
    use strata_components::code_view::{CodeState, EditorState};
    use strata_components::preview_drawer::HostedPreview;
    use strata_components::scroll::BarLook;
    use strata_gallery::{Gallery, scenes};

    const DT: f64 = 1.0 / 60.0;

    fn scene(name: &str) -> Runtime<Gallery> {
        let f = scenes::all_scenes()
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, f)| f)
            .unwrap_or_else(|| panic!("no scene {name}"));
        let mut g = Gallery::default();
        f(&mut g);
        let env = Env::new(g.dark, 1.0);
        let mut rt = strata_gallery::runtime(g, env);
        rt.frame(0.0);
        settle(&mut rt, 10);
        rt
    }

    fn settle(rt: &mut Runtime<Gallery>, frames: usize) {
        for _ in 0..frames {
            rt.frame(DT);
        }
    }

    fn key_with(rt: &mut Runtime<Gallery>, key: Key, mods: Mods) {
        rt.key(KeyEvent { key, mods });
        settle(rt, 2);
    }

    fn key(rt: &mut Runtime<Gallery>, key: Key) {
        key_with(rt, key, Mods::default());
    }

    fn ctrl() -> Mods {
        Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        }
    }

    fn shift() -> Mods {
        Mods {
            shift: true,
            ..Mods::default()
        }
    }

    fn info(rt: &Runtime<Gallery>, key: &str) -> Option<rune::ElementInfo> {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
    }

    /// The element whose path ends in `suffix`.
    fn at_path(rt: &Runtime<Gallery>, suffix: &str) -> Option<rune::ElementInfo> {
        rt.elements().into_iter().find(|e| e.path.ends_with(suffix))
    }

    /// The code viewport's vertical offset.
    fn offset(rt: &Runtime<Gallery>) -> f32 {
        rt.state.c10.code.scroll_offset()
    }

    /// Row `r`'s text.
    fn row_text(rt: &Runtime<Gallery>, r: usize) -> Option<rune::ElementInfo> {
        let seg = format!("/code-l{r}/");
        rt.elements()
            .into_iter()
            .find(|e| e.path.contains(&seg) && e.path.ends_with("/t"))
    }

    /// The thumb's top and length, as drawn.
    fn thumb(rt: &Runtime<Gallery>) -> (f32, f32) {
        let t = at_path(rt, "code-position/trough/thumb").expect("a thumb");
        (t.rect[1], t.rect[3])
    }

    fn bar(rt: &Runtime<Gallery>) -> [f32; 4] {
        info(rt, "code-position").expect("the position bar").rect
    }

    fn wheel(rt: &mut Runtime<Gallery>, dy: f32) {
        let r = info(rt, "code-scroll").expect("the viewport").rect;
        rt.wheel(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, dy);
        rt.frame(DT);
    }

    /// The topmost row text drawn in the viewport: its path and its top.
    fn top_row(rt: &Runtime<Gallery>) -> (String, f32) {
        let vp = info(rt, "code-scroll").expect("the viewport").rect;
        rt.elements()
            .into_iter()
            .filter(|e| e.path.contains("/code-l") && e.path.ends_with("/t"))
            .filter(|e| e.rect[1] + e.rect[3] > vp[1] + 0.5)
            .min_by(|a, b| a.rect[1].total_cmp(&b.rect[1]))
            .map(|e| (e.path, e.rect[1]))
            .expect("a row on screen")
    }

    fn shown_rows(rt: &Runtime<Gallery>) -> Vec<usize> {
        let vp = info(rt, "code-scroll").expect("the viewport").rect;
        rt.elements()
            .into_iter()
            .filter(|e| e.path.ends_with("/t") && e.visible[3] > 0.0)
            .filter(|e| e.rect[1] < vp[1] + vp[3] && e.rect[1] + e.rect[3] > vp[1])
            .filter_map(|e| {
                let seg = e.path.split('/').find(|s| s.starts_with("code-l"))?;
                seg["code-l".len()..].parse().ok()
            })
            .collect()
    }

    fn node(rt: &Runtime<Gallery>, key: &str) -> Option<Value> {
        fn find(v: &Value, key: &str) -> Option<Value> {
            if v["key"] == key {
                return Some(v.clone());
            }
            v["children"].as_array()?.iter().find_map(|c| find(c, key))
        }
        find(&rt.accessibility_json(), key)
    }

    /// Presses at `(x, y)` and holds, a frame.
    fn press(rt: &mut Runtime<Gallery>, x: f32, y: f32) {
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(DT);
    }

    fn drag_to(rt: &mut Runtime<Gallery>, x: f32, y: f32) {
        rt.pointer_move(x, y);
        rt.frame(DT);
    }

    fn release(rt: &mut Runtime<Gallery>, x: f32, y: f32) {
        rt.pointer_up(x, y);
        rt.frame(DT);
    }

    /// Frames, sleeping between them, until `done` holds or 60 s pass: the highlighter
    /// thread answers in real time.
    fn until(rt: &mut Runtime<Gallery>, done: impl Fn(&Gallery) -> bool) {
        let start = Instant::now();
        while !done(&rt.state) {
            assert!(start.elapsed() < Duration::from_secs(60), "timed out");
            std::thread::sleep(Duration::from_millis(5));
            rt.frame(DT);
        }
        rt.frame(DT);
    }

    // Sanctioned exception to "no geometry assertions" (AGENTS.md), by the coordinator's
    // ruling for C10b: the work package's claim is that nothing moves when colours land, and
    // the offset and the top row's place are that claim.
    #[test]
    fn colours_landing_move_nothing() {
        // The text draws plain (no highlighter yet), scrolled into its middle.
        let mut rt = scene("c10-large-light");
        rt.update(|g, _| {
            let text = g.c10.code.text().clone();
            g.c10.code = CodeState::new(text);
        });
        settle(&mut rt, 4);
        for _ in 0..30 {
            wheel(&mut rt, -400.0);
        }
        settle(&mut rt, 30);
        let (row, y) = top_row(&rt);
        let line: usize = row
            .split('/')
            .find_map(|s| s.strip_prefix("code-l"))
            .and_then(|n| n.parse().ok())
            .expect("a row number");
        let before = offset(&rt);
        assert!(!rt.state.c10.code.is_coloured(line));
        // Then the colours land.
        rt.update(|g, _| {
            let hl = g
                .c10
                .live
                .as_ref()
                .expect("a highlighter")
                .highlighter
                .clone();
            let st = std::mem::take(&mut g.c10.code);
            g.c10.code = st.highlighter(&hl, Some("generated.rs"), None);
        });
        until(&mut rt, |g| {
            g.c10.code.is_coloured(line) && g.c10.code.is_coloured(line + 20)
        });
        assert_eq!(offset(&rt), before, "the viewport stays put");
        let after = rt
            .elements()
            .into_iter()
            .find(|e| e.path == row)
            .expect("the top row stays mounted")
            .rect[1];
        assert_eq!(after, y, "the top row stays put");
    }

    /// The list's widths while scrolling, wrap off, through six 200-character ASCII rows, then
    /// `wide` at lines 406 on, then short lines.
    fn widths_while_scrolling(wide: &[String]) -> Vec<f32> {
        use std::fmt::Write as _;
        let mut rt = scene("c10-wrap-off-light");
        let wide = wide.to_vec();
        rt.update(move |g, _| {
            let mut text = String::new();
            for _ in 0..6 {
                let _ = writeln!(text, "{}", "a".repeat(200));
            }
            for i in 6..406 {
                let _ = writeln!(text, "line {i}");
            }
            for w in &wide {
                let _ = writeln!(text, "{w}");
            }
            for i in 406 + wide.len()..800 {
                let _ = writeln!(text, "line {i}");
            }
            let mut st = CodeState::new(text);
            st.set_wrap(false);
            g.c10.code = st;
        });
        settle(&mut rt, 60);
        let width = |rt: &Runtime<Gallery>| info(rt, "code-rows").expect("the list").rect[2];
        let mut seen = vec![width(&rt)];
        for _ in 0..60 {
            wheel(&mut rt, -400.0);
            seen.push(width(&rt));
        }
        assert!(offset(&rt) > 410.0 * 18.0, "scrolled past the wide rows");
        seen.dedup();
        seen
    }

    /// With wrap off the list is as wide as its widest row before that row scrolls into
    /// view, so it neither widens nor narrows as rows mount (review 1's repro: a CJK row and
    /// an emoji row, each shorter in characters than the six longest rows, draw wider).
    #[test]
    fn the_unwrapped_width_holds_while_scrolling() {
        let seen = widths_while_scrolling(&["漢".repeat(150), "😀".repeat(150)]);
        assert_eq!(
            seen.len(),
            1,
            "the list's width moved while scrolling: {seen:?}"
        );
    }

    /// The same for characters `unicode-width` rates one column that a fallback face draws
    /// wider (review 2's repro): a dingbat and an Indic script.
    #[test]
    fn the_unwrapped_width_holds_for_narrow_rated_wide_glyphs() {
        for wide in ["❤".repeat(190), "क".repeat(190)] {
            let seen = widths_while_scrolling(std::slice::from_ref(&wide));
            assert_eq!(seen.len(), 1, "{}: the width moved: {seen:?}", &wide[..3]);
        }
    }

    // The position-bar tests below read the thumb's and the bar's drawn places. Like
    // `colours_landing_move_nothing`, that is a sanctioned exception to "no geometry
    // assertions" (AGENTS.md): each bullet they check is a claim about where the thumb or
    // the bar is (named in each test's doc comment), and the place is the claim.

    /// preview/preview-panel/documents/scrollbar: "Wheel or touchpad scrolling forward over
    /// the content never moves the thumb backward; scrolling backward never moves it forward;
    /// stopping leaves it still."
    #[test]
    fn the_wheel_never_moves_the_thumb_against_it() {
        let mut rt = scene("c10-large-light");
        let mut last = thumb(&rt).0;
        for _ in 0..80 {
            wheel(&mut rt, -240.0);
            let t = thumb(&rt).0;
            assert!(t >= last - 0.01, "forward: {t} after {last}");
            last = t;
        }
        assert!(last > bar(&rt)[1], "the thumb moved down");
        // A wheel step glides: stopping is once the glide has landed.
        settle(&mut rt, 60);
        let still = thumb(&rt).0;
        assert!(still >= last, "the glide's end moved it back");
        settle(&mut rt, 60);
        assert_eq!(thumb(&rt).0, still, "stopping leaves it still");
        last = still;
        for _ in 0..40 {
            wheel(&mut rt, 240.0);
            let t = thumb(&rt).0;
            assert!(t <= last + 0.01, "backward: {t} after {last}");
            last = t;
        }
    }

    /// preview/preview-panel/documents/scrollbar: "In a rendered document or a large
    /// plain-text Source preview, holding the scrollbar thumb still keeps the content at one
    /// position", "Dragging the thumb downward never moves the content backward, and the
    /// content passes through interior positions on the way", "Releasing the thumb, even
    /// with the pointer over the content, leaves the content in place. The next wheel input
    /// scrolls it and the thumb follows."
    #[test]
    fn a_thumb_drag_moves_forward_holds_and_lets_go() {
        let mut rt = scene("c10-large-light");
        let b = bar(&rt);
        let (top, len) = thumb(&rt);
        let x = b[0] + b[2] / 2.0;
        let mut y = top + len / 2.0;
        press(&mut rt, x, y);
        let mut last = offset(&rt);
        let mut moved = 0;
        for _ in 0..40 {
            y += 6.0;
            drag_to(&mut rt, x, y);
            rt.frame(DT);
            let o = offset(&rt);
            assert!(
                o >= last,
                "dragging down moved the content back: {o} after {last}"
            );
            if o > last {
                moved += 1;
            }
            last = o;
        }
        assert!(
            moved > 20,
            "the content passed through interior positions ({moved} moves)"
        );
        // Held still: no movement over a second.
        let held = offset(&rt);
        settle(&mut rt, 60);
        assert_eq!(
            offset(&rt),
            held,
            "holding the thumb keeps the content still"
        );
        // The pointer drifts over the content, still level with the thumb, and lets go
        // there: the content stays.
        let vp = info(&rt, "code-scroll").expect("the viewport").rect;
        drag_to(&mut rt, vp[0] + 40.0, y);
        settle(&mut rt, 10);
        let there = offset(&rt);
        release(&mut rt, vp[0] + 40.0, y);
        settle(&mut rt, 20);
        assert_eq!(offset(&rt), there, "releasing leaves the content in place");
        let t = thumb(&rt).0;
        wheel(&mut rt, -240.0);
        settle(&mut rt, 2);
        assert!(offset(&rt) > there, "the next wheel scrolls");
        assert!(thumb(&rt).0 > t, "and the thumb follows");
    }

    /// preview/preview-panel/documents/scrollbar: "Dragging the thumb to the top or bottom of
    /// the track shows the document's first or last line, even while new rows are measured."
    /// and "With the first row's top in view, the thumb sits at the top of the track. With
    /// the last row's end in view, it sits at the bottom."
    #[test]
    fn the_thumb_at_either_end_shows_that_end() {
        let mut rt = scene("c10-large-light");
        let b = bar(&rt);
        let (top, len) = thumb(&rt);
        assert_eq!(
            top, b[1],
            "at the first row's top the thumb is at the track's top"
        );
        let x = b[0] + b[2] / 2.0;
        press(&mut rt, x, top + len / 2.0);
        for i in 1..=10 {
            drag_to(&mut rt, x, top + len / 2.0 + b[3] * i as f32 / 10.0);
        }
        drag_to(&mut rt, x, b[1] + b[3] + 50.0);
        settle(&mut rt, 10);
        let last = rt.state.c10.code.text().lines().count() - 1;
        assert!(
            shown_rows(&rt).contains(&last),
            "the last line shows: {:?}",
            shown_rows(&rt)
        );
        let (t, l) = thumb(&rt);
        assert!(
            (t + l - (b[1] + b[3])).abs() < 0.5,
            "at the end the thumb is at the track's bottom"
        );
        drag_to(&mut rt, x, b[1] - 50.0);
        settle(&mut rt, 10);
        release(&mut rt, x, b[1] - 50.0);
        assert_eq!(offset(&rt), 0.0);
        assert!(shown_rows(&rt).contains(&0), "the first line shows");
    }

    /// preview/preview-panel/documents/scrollbar: "Setting the scrollbar to its minimum or
    /// maximum from the keyboard or an accessibility tool shows the document's exact start or
    /// end."
    #[test]
    fn the_bar_s_ends_from_the_keyboard_and_assistive_technology() {
        let mut rt = scene("c10-large-light");
        let last = rt.state.c10.code.text().lines().count() - 1;
        // `range` is [value, min, max, step].
        let max = node(&rt, "code-position").expect("the bar")["range"][2]
            .as_f64()
            .unwrap_or(0.0);
        assert!(rt.accessibility_action(
            "code-position",
            Action::SetValue,
            Some(ActionData::NumericValue(max))
        ));
        settle(&mut rt, 10);
        assert!(shown_rows(&rt).contains(&last), "the maximum shows the end");
        assert!(rt.accessibility_action(
            "code-position",
            Action::SetValue,
            Some(ActionData::NumericValue(0.0))
        ));
        settle(&mut rt, 10);
        assert_eq!(offset(&rt), 0.0, "the minimum shows the start");
        // The keyboard: the bar is a Tab stop; End and Home go to the ends, Down a row.
        assert!(rt.accessibility_action("code-position", Action::Focus, None));
        settle(&mut rt, 2);
        assert!(info(&rt, "code-position").is_some_and(|e| e.focused));
        key(&mut rt, Key::End);
        settle(&mut rt, 10);
        assert!(shown_rows(&rt).contains(&last), "End shows the end");
        key(&mut rt, Key::Home);
        settle(&mut rt, 10);
        assert_eq!(offset(&rt), 0.0, "Home shows the start");
        key(&mut rt, Key::Down);
        settle(&mut rt, 4);
        assert!(offset(&rt) > 0.0, "Down steps forward");
        let value = node(&rt, "code-position").expect("the bar")["value"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert_eq!(value, "Line 2 of 50000");
    }

    /// preview/preview-panel/documents/scrollbar: "The vertical scrollbar sits beside the
    /// content instead of overlaying it, and hides when the whole document fits."
    #[test]
    fn the_bar_sits_beside_the_text_and_hides_when_it_fits() {
        let rt = scene("c10-rust-light");
        let b = bar(&rt);
        let vp = info(&rt, "code-scroll").expect("the viewport").rect;
        assert!(
            b[0] >= vp[0] + vp[2] - 1.0,
            "beside the viewport, not over it"
        );
        let rt = scene("c10-plain-light");
        assert!(
            info(&rt, "code-position").is_none(),
            "hidden when the text fits"
        );
    }

    /// preview/preview-panel/documents/scrollbar: "Closing the preview while holding the
    /// thumb ends the drag; after reopening, the thumb follows scrolling again."
    #[test]
    fn closing_mid_drag_then_reopening_follows_the_scroll() {
        let mut rt = scene("c10-large-light");
        let b = bar(&rt);
        let (top, len) = thumb(&rt);
        press(&mut rt, b[0] + b[2] / 2.0, top + len / 2.0);
        drag_to(&mut rt, b[0] + b[2] / 2.0, top + len / 2.0 + 40.0);
        let look = BarLook::app(&rt.state.look);
        let trough =
            |rt: &Runtime<Gallery>| at_path(rt, "code-position/trough").expect("a trough").fill;
        settle(&mut rt, 30);
        assert_eq!(
            trough(&rt),
            look.trough_hover,
            "a held thumb tints the trough"
        );
        rt.pointer_move(10.0, 10.0);
        rt.update(|g, _| g.c10.drawer = Default::default());
        settle(&mut rt, 30);
        assert!(info(&rt, "code-position").is_none(), "closed");
        rt.update(|g, _| g.c10.drawer.open_at_once());
        settle(&mut rt, 30);
        // Rune sends the gone bar no Up: the drag ended with it, so the trough is at rest.
        assert_eq!(trough(&rt), look.trough, "the drag ended with the preview");
        let t = thumb(&rt).0;
        for _ in 0..10 {
            wheel(&mut rt, -400.0);
        }
        settle(&mut rt, 2);
        assert!(thumb(&rt).0 > t, "the thumb follows the scroll");
    }

    /// The value the bar reads to assistive technology: "Line N of M".
    fn bar_line(rt: &Runtime<Gallery>) -> usize {
        let v = node(rt, "code-position").expect("the bar")["value"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        v.split(' ')
            .nth(1)
            .and_then(|n| n.parse().ok())
            .expect("a line number")
    }

    /// The bar's Page Down and Page Up step a page of rows, as a scroll bar's do.
    #[test]
    fn the_bar_s_page_keys_step_a_page() {
        let mut rt = scene("c10-large-light");
        assert!(rt.accessibility_action("code-position", Action::Focus, None));
        settle(&mut rt, 2);
        key(&mut rt, Key::PageDown);
        settle(&mut rt, 4);
        let shown = shown_rows(&rt).len();
        let line = bar_line(&rt);
        assert!(
            line > shown / 2,
            "Page Down went a page, to line {line} ({shown} rows show)"
        );
        key(&mut rt, Key::PageUp);
        settle(&mut rt, 4);
        assert_eq!(bar_line(&rt), 1);
    }

    /// A press on the thumb holds it where it was caught: a move of a point moves the
    /// content a point's worth of rows, not by the thumb jumping to centre on the pointer.
    #[test]
    fn a_thumb_press_holds_it_where_caught() {
        let mut rt = scene("c10-large-light");
        let b = bar(&rt);
        let (top, len) = thumb(&rt);
        let x = b[0] + b[2] / 2.0;
        press(&mut rt, x, top + len - 3.0);
        drag_to(&mut rt, x, top + len - 2.0);
        settle(&mut rt, 4);
        // One point of the thumb's travel is about 1,600 points of a 50,000-line text.
        let o = offset(&rt);
        assert!(o < 6000.0, "the content jumped to {o}");
        release(&mut rt, x, top + len - 2.0);
    }

    /// A press on the trough away from the thumb puts the thumb's middle there (GTK's warp).
    #[test]
    fn a_trough_press_warps_the_thumb() {
        let mut rt = scene("c10-large-light");
        let b = bar(&rt);
        let x = b[0] + b[2] / 2.0;
        let y = b[1] + b[3] / 2.0;
        press(&mut rt, x, y);
        settle(&mut rt, 6);
        release(&mut rt, x, y);
        let lines = rt.state.c10.code.text().lines().count();
        let line = bar_line(&rt);
        assert!(
            line > lines * 3 / 10 && line < lines * 7 / 10,
            "the middle of the track went to line {line} of {lines}"
        );
    }

    /// With wrap off the bar sets the exact row: rows have one height, so an offset is a row.
    #[test]
    fn the_bar_sets_a_row_with_wrap_off() {
        let mut rt = scene("c10-large-light");
        rt.update(|g, _| g.c10.code.set_wrap(false));
        settle(&mut rt, 10);
        assert!(rt.accessibility_action(
            "code-position",
            Action::SetValue,
            Some(ActionData::NumericValue(1000.0))
        ));
        settle(&mut rt, 10);
        let first = shown_rows(&rt).into_iter().min();
        assert_eq!(first, Some(1000));
    }

    /// With wrap off the line numbers stay at the left while the code scrolls sideways
    /// (`sticky_left`). The number's drawn place is the claim, as for the bar.
    #[test]
    fn the_numbers_stay_while_the_code_scrolls_sideways() {
        let mut rt = scene("c10-wrap-off-light");
        let number = |rt: &Runtime<Gallery>| at_path(rt, "code-l3/n").expect("a number").rect[0];
        let text = |rt: &Runtime<Gallery>| row_text(rt, 3).expect("a text").rect[0];
        let (n0, t0) = (number(&rt), text(&rt));
        rt.update(|_, tx| tx.scroll_to_x("code-scroll", 500.0));
        settle(&mut rt, 4);
        assert!(text(&rt) < t0 - 400.0, "the code scrolled sideways");
        assert_eq!(number(&rt), n0, "the number stayed");
    }

    /// The current match's highlight is keyed `current`, which find centres.
    #[test]
    fn the_current_match_is_keyed() {
        let mut rt = scene("c10-rust-light");
        rt.update(|g, _| {
            g.c10.drawer.start_find("fn");
            g.c10.code.find("fn");
            g.c10.code.find_step(true);
        });
        settle(&mut rt, 10);
        let (_, y) = rt.find("current").expect("a highlight keyed current");
        let row = row_text(&rt, 28)
            .expect("the second match's row (line 29's comment)")
            .rect;
        assert!(
            y >= row[1] && y <= row[1] + row[3],
            "on the second match's row"
        );
    }

    /// Ctrl+A is the plain chord only: with Shift it selects nothing (Rune's rule, and
    /// Strata's: Ctrl+Shift+A isn't select-all).
    #[test]
    fn ctrl_shift_a_selects_nothing() {
        let mut rt = scene("c10-rust-light");
        let t = row_text(&rt, 3).expect("row 3").rect;
        press(&mut rt, t[0] + 4.0, t[1] + 4.0);
        release(&mut rt, t[0] + 4.0, t[1] + 4.0);
        key_with(
            &mut rt,
            Key::Command('a'),
            Mods {
                shift: true,
                ..ctrl()
            },
        );
        settle(&mut rt, 2);
        assert_eq!(rt.state.c10.selection, "");
        assert!(!rt.state.c10.code.copy_all().is_empty());
    }

    /// preview/preview-panel/documents: "With focus in a rendered or large source document,
    /// Ctrl+A selects the whole loaded document." Ctrl+C after it copies all 50,000 lines,
    /// past Rune's 10,000-row collection cap.
    #[test]
    fn select_all_copies_the_whole_large_text() {
        let mut rt = scene("c10-large-light");
        let clip = clipboard::memory();
        rt.use_clipboard(&clip);
        let t = row_text(&rt, 3).expect("row 3").rect;
        press(&mut rt, t[0] + 4.0, t[1] + 4.0);
        release(&mut rt, t[0] + 4.0, t[1] + 4.0);
        key_with(&mut rt, Key::Command('a'), ctrl());
        key_with(&mut rt, Key::Command('c'), ctrl());
        let want = rt.state.c10.code.text().len();
        assert_eq!(clip.text().map(|t| t.len()), Some(want));
    }

    /// preview/preview-panel/find: "Find searches small source buffers, large virtualized
    /// source, …" A match near the end of 50,000 lines is counted, selected and revealed.
    #[test]
    fn find_reaches_the_end_of_a_large_text() {
        let mut rt = scene("c10-large-light");
        rt.update(|g, _| {
            g.c10.drawer.start_find("item 49994");
            g.c10.code.find("item 49994");
        });
        settle(&mut rt, 60);
        let status = rt.state.c10.code.find_status().expect("a status");
        assert_eq!((status.current, status.total), (Some(1), 1));
        assert!(
            shown_rows(&rt).contains(&49994),
            "the match is revealed: {:?} at {}",
            shown_rows(&rt),
            offset(&rt)
        );
        assert_eq!(rt.state.c10.selection, "item 49994");
    }

    fn editor() -> Runtime<Gallery> {
        let mut rt = scene("c10-editor-light");
        assert!(rt.accessibility_action("field", Action::Focus, None));
        settle(&mut rt, 2);
        rt
    }

    fn caret_to(rt: &mut Runtime<Gallery>, at: usize) {
        rt.update(|g, _| {
            if let Some(ed) = &mut g.c10.editor {
                ed.edit.set_caret(at);
            }
        });
        rt.frame(DT);
    }

    fn script(rt: &Runtime<Gallery>) -> String {
        rt.state
            .c10
            .editor
            .as_ref()
            .expect("an editor")
            .text()
            .to_owned()
    }

    /// Return keeps the line's indentation (Strata's `auto_indent(true)`).
    #[test]
    fn return_keeps_the_indent() {
        let mut rt = editor();
        let text = script(&rt);
        let at = text.find("stamp=").expect("the stamp line");
        let end = at + text[at..].find('\n').expect("its end");
        caret_to(&mut rt, end);
        key(&mut rt, Key::Enter);
        assert!(
            script(&rt).contains("+%Y-%m-%d)\n    \n    mv"),
            "{}",
            script(&rt)
        );
    }

    /// Tab types spaces to the next four-column stop; over lines it indents each, and
    /// Shift+Tab takes the indent off (Strata's `tab_width(4)`,
    /// `insert_spaces_instead_of_tabs(true)`, GtkSourceView's `indent-on-tab`).
    #[test]
    fn tab_and_shift_tab_indent_by_four_columns() {
        let mut rt = editor();
        caret_to(&mut rt, 1);
        key(&mut rt, Key::Tab);
        assert!(script(&rt).starts_with("#   !/usr"), "{}", script(&rt));
        let text = script(&rt);
        let a = text.find("dir=").expect("dir");
        let b = text.find("for f").expect("for");
        rt.update(|g, _| {
            if let Some(ed) = &mut g.c10.editor {
                ed.edit.set_selection(a, b + 2);
            }
        });
        rt.frame(DT);
        key(&mut rt, Key::Tab);
        let t = script(&rt);
        assert!(
            t.contains("\n    dir=\"${1")
                && t.contains("\n    count=0")
                && t.contains("\n    for f"),
            "{t}"
        );
        key_with(&mut rt, Key::Tab, shift());
        assert!(t.len() > script(&rt).len());
        let t = script(&rt);
        assert!(
            t.contains("\ndir=") && t.contains("\ncount=0") && t.contains("\nfor f"),
            "{t}"
        );
        // One undo step brings the indent back.
        key_with(&mut rt, Key::Command('z'), ctrl());
        assert!(script(&rt).contains("\n    count=0"));
    }

    fn focused_key(rt: &Runtime<Gallery>) -> Option<String> {
        rt.elements()
            .into_iter()
            .find(|e| e.focused)
            .and_then(|e| e.key)
    }

    /// Ctrl+Tab and Ctrl+Shift+Tab leave the editor, forward and back, as they leave a
    /// GtkTextView, whose Tab types: the text stays as it was. The page's stops are the
    /// runtime button, the script, then the Library button.
    #[test]
    fn ctrl_tab_leaves_the_editor() {
        let mut rt = editor();
        let before = script(&rt);
        key_with(&mut rt, Key::Tab, ctrl());
        assert_eq!(focused_key(&rt).as_deref(), Some("c10-library"));
        assert!(rt.accessibility_action("field", Action::Focus, None));
        settle(&mut rt, 2);
        key_with(
            &mut rt,
            Key::Tab,
            Mods {
                shift: true,
                ..ctrl()
            },
        );
        assert_eq!(focused_key(&rt).as_deref(), Some("c10-runtime"));
        assert_eq!(script(&rt), before, "nothing typed");
        // Plain Tab still types.
        assert!(rt.accessibility_action("field", Action::Focus, None));
        settle(&mut rt, 2);
        key(&mut rt, Key::Tab);
        assert_eq!(focused_key(&rt).as_deref(), Some("field"));
        assert_ne!(script(&rt), before);
    }

    /// Two Ctrl+Tabs in one frame move focus twice, typing nothing (review 2).
    #[test]
    fn two_ctrl_tabs_in_a_frame_move_focus_twice() {
        let mut rt = editor();
        let before = script(&rt);
        for _ in 0..2 {
            rt.key(KeyEvent {
                key: Key::Tab,
                mods: ctrl(),
            });
        }
        settle(&mut rt, 2);
        assert_eq!(script(&rt), before, "nothing typed");
        assert_eq!(
            focused_key(&rt).as_deref(),
            Some("c10-runtime"),
            "past the Library button"
        );
    }

    /// Typing re-highlights the edited line: its colours change from a comment's to code's.
    #[test]
    fn an_edit_recolours_its_line() {
        let mut rt = editor();
        let text = script(&rt);
        let at = text.find("# Rename").expect("the comment");
        let line = text[..at].matches('\n').count();
        let spans = |g: &Gallery| {
            let ed = g.c10.editor.as_ref().expect("an editor");
            let line_start = ed
                .text()
                .split('\n')
                .take(line)
                .map(|l| l.len() + 1)
                .sum::<usize>();
            ed.spans()
                .into_iter()
                .filter(|s| s.start as usize >= line_start && (s.start as usize) < line_start + 6)
                .map(|s| s.color)
                .collect::<Vec<_>>()
        };
        let before = spans(&rt.state);
        caret_to(&mut rt, at);
        key(&mut rt, Key::Delete);
        until(&mut rt, |g| {
            let ed = g.c10.editor.as_ref().expect("an editor");
            !ed.is_provisional(line) && spans(g) != before
        });
        assert!(script(&rt).contains("\n Rename"));
    }

    /// The editor colours the lines it shows wherever it is scrolled: its span window moves
    /// with the viewport.
    #[test]
    fn the_editor_colours_the_lines_in_view_as_it_scrolls() {
        use std::fmt::Write as _;
        let mut rt = editor();
        rt.update(|g, _| {
            let mut text = String::from("#!/usr/bin/env bash\n");
            for i in 1..3000 {
                let _ = writeln!(text, "echo \"line {i}\" # note {i}");
            }
            let hl = g
                .c10
                .live
                .as_ref()
                .expect("a highlighter")
                .highlighter
                .clone();
            g.c10.editor = Some(EditorState::new(text, &hl, Some("long.sh"), None));
        });
        settle(&mut rt, 4);
        // Whether every line in view has a colour span starting in it.
        let coloured = |g: &Gallery| {
            let ed = g.c10.editor.as_ref().expect("an editor");
            let lines = ed.shown_lines();
            if lines.is_empty() {
                return false;
            }
            let starts: Vec<usize> = std::iter::once(0)
                .chain(ed.text().match_indices('\n').map(|(i, _)| i + 1))
                .collect();
            let spans = ed.spans();
            lines.clone().all(|l| {
                let (a, b) = (starts[l], starts.get(l + 1).copied().unwrap_or(usize::MAX));
                // The empty line after the last line break has nothing to colour.
                a == ed.text().len() || spans.iter().any(|s| (a..b).contains(&(s.start as usize)))
            })
        };
        let vp = info(&rt, "script-scroll")
            .expect("the editor's viewport")
            .rect;
        let mut tops = Vec::new();
        for _ in 0..4 {
            for _ in 0..12 {
                rt.wheel(vp[0] + vp[2] / 2.0, vp[1] + vp[3] / 2.0, -600.0);
                rt.frame(DT);
            }
            settle(&mut rt, 30);
            until(&mut rt, |g| coloured(g));
            tops.push(
                rt.state
                    .c10
                    .editor
                    .as_ref()
                    .expect("an editor")
                    .shown_lines()
                    .start,
            );
        }
        assert!(
            tops.windows(2).all(|w| w[1] > w[0]),
            "scrolled down each time: {tops:?}"
        );
        assert!(tops[3] > 1000, "far from the start: {tops:?}");
    }

    /// The field's right-click opens C21's field menu (Rune's own is empty on Linux).
    #[test]
    fn the_field_menu_opens_on_a_right_click() {
        let mut rt = editor();
        let f = info(&rt, "field").expect("the field").rect;
        rt.pointer_move(f[0] + 20.0, f[1] + 10.0);
        rt.secondary_down(f[0] + 20.0, f[1] + 10.0);
        rt.secondary_up(f[0] + 20.0, f[1] + 10.0);
        settle(&mut rt, 10);
        assert_eq!(
            rt.state.c10.fields.field(),
            Some(scenes::c10::Field::Script)
        );
        assert!(node(&rt, "c10-field-menu").is_some() || rt.state.c10.fields.is_open());
    }
}

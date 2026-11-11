//! C24's scroll extras on the c24 page, in process: what scripts can't see (scroll offsets,
//! cursors, opacities, wake-ups, a held Alt) and what no golden shows (motion over time). Each
//! test names the feature-map bullets it checks; a partial check says which half. The scripts
//! in `tests/scripts/c24/` check the rest, and two tests here run them through the gallery
//! binary: one for Rune's perf warnings (debug builds), one under `RUNE_COMMIT_BUDGET_MS=8`
//! (`--profile perf`).
//!
//! The scripts, and which half of a bullet each checks where it checks one half:
//!
//! | Script | Checks |
//! | --- | --- |
//! | `autoscroll-start` | a middle click in the list's margin starts it (the shield, "Autoscroll") |
//! | `autoscroll-row` | a middle click on a folder row opens it in a new tab, starts nothing |
//! | `autoscroll-move`, `-long`, `-reduced` | it scrolls toward the pointer (rows scroll away); `-move` also checks Escape stops it (the Escape half of the stop bullet) |
//! | `autoscroll-click` | a click outside the list stops it (the click half of the stop bullet) |
//! | `autoscroll-click-row` | the click on a row that stops it opens and selects nothing |
//! | `autoscroll-fits` | a list that fits starts none |
//! | `autoscroll-rename` | a middle click on C5's rename field inside the listing starts none, and opens nothing (not the paste half, Rune draft U-C24b-3) |
//! | `autoscroll-margin-fits` | in a list that fits, a middle click on a row's margin starts none and opens nothing |
//! | `panel-wheel-outside` | a tick outside the stand-in Sort by panel closes it and scrolls the list under the pointer |
//! | `panel-wheel-inside` | a tick inside it keeps it open and scrolls nothing under it |
//! | `panel-wheel-chrome` | a tick over the sidebar only closes it |
//! | `history-back`, `history-forward` | buttons 8 and 9 reach the host |
//! | `autoscroll-navigate` | another folder stops it (the navigating half; hiding is in process) |
//! | `bar-drag` | a thumb drag scrolls and starts no band (Alt held: in process) |
//! | `bar-press` | a trough press is the bar's: it jumps, no band (the scroll-bar half of the bullet) |
//! | `nested-wheel` | a vertical wheel over a column scrolls only it (the vertical half; Shift + wheel can't be sent headless, U-C24-7) |

#[expect(
    dead_code,
    reason = "these tests use only the gallery command and the scripts folder"
)]
#[path = "goldens/support.rs"]
mod support;

mod c24 {
    use rune::{
        Cursor, ElementInfo, Env, GesturePhase, Key, KeyEvent, Mods, PhysicalInput, PointerButton,
        Runtime, WheelEvent,
    };
    use strata_components::collection::marquee::view::KEY as BAND;
    use strata_components::contract::{EntryId, Rect};
    use strata_components::scroll::autoscroll::ANCHOR;
    use strata_gallery::scenes::c24::{BAR, COLUMNS, WARMED, WARMUP, column_key};
    use strata_gallery::{Gallery, scenes};

    use crate::support;

    const FRAME: f64 = 1.0 / 60.0;

    fn mount(name: &str) -> Runtime<Gallery> {
        let scene = scenes::all_scenes()
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, f)| f)
            .unwrap_or_else(|| panic!("no scene {name}"));
        let mut g = Gallery::default();
        scene(&mut g);
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

    /// The element at key path `path` (`a/b`), by its path's end.
    fn under(rt: &Runtime<Gallery>, parent: &str, key: &str) -> Option<ElementInfo> {
        rt.elements().into_iter().find(|e| {
            !e.exiting && e.key.as_deref() == Some(key) && e.path.contains(&format!("/{parent}/"))
        })
    }

    fn offset(rt: &Runtime<Gallery>) -> f32 {
        rt.state.c24.feed.get().offset[1]
    }

    fn viewport(rt: &Runtime<Gallery>) -> Rect {
        rt.state.c24.coll.viewport()
    }

    fn running(rt: &Runtime<Gallery>) -> bool {
        rt.state.c24.auto.active()
    }

    fn selected(rt: &Runtime<Gallery>) -> Vec<u64> {
        let d = &rt.state.c24;
        d.coll
            .selection
            .selected_ids(&d.listing)
            .into_iter()
            .map(EntryId::get)
            .collect()
    }

    fn press(rt: &mut Runtime<Gallery>, x: f32, y: f32, button: PointerButton) {
        rt.pointer_move(x, y);
        rt.frame(FRAME);
        match button {
            PointerButton::Primary => {
                rt.pointer_down(x, y);
                rt.frame(FRAME);
                rt.pointer_up(x, y);
            }
            PointerButton::Secondary => {
                rt.secondary_down(x, y);
                rt.frame(FRAME);
                rt.secondary_up(x, y);
            }
            b => {
                rt.button_down(x, y, b);
                rt.frame(FRAME);
                rt.button_up(x, y, b);
            }
        }
        rt.frame(FRAME);
    }

    /// A middle click in the list's left margin, halfway down: where autoscroll anchors.
    fn anchor_at(rt: &Runtime<Gallery>) -> (f32, f32) {
        let v = viewport(rt);
        (v.x + 2.0, v.y + v.h / 2.0)
    }

    /// Starts an autoscroll from the list's margin; returns the anchor.
    fn start(rt: &mut Runtime<Gallery>) -> (f32, f32) {
        let (x, y) = anchor_at(rt);
        press(rt, x, y, PointerButton::Middle);
        assert!(
            running(rt),
            "a middle click in the margin starts autoscroll"
        );
        (x, y)
    }

    /// How far the list scrolled in `seconds` with the pointer `dy` below the anchor.
    fn travel(name: &str, dy: f32, seconds: f64) -> f32 {
        let mut rt = mount(name);
        let (x, y) = start(&mut rt);
        rt.pointer_move(x + 40.0, y + dy);
        run(&mut rt, 0.25);
        let from = offset(&rt);
        run(&mut rt, seconds);
        offset(&rt) - from
    }

    // Starting.

    #[test]
    fn a_middle_click_on_the_lists_background_anchors_autoscroll_at_the_press() {
        // browser/scrolling: Middle-clicking the empty background or gutter of a scrollable Columns, Icons, or List listing starts autoscroll. A round anchor marks the press point.
        let mut rt = mount("c24-ten-k-light");
        let (x, y) = start(&mut rt);
        // Its whole rect: in the margin it is clipped at the list's edge.
        let [ax, ay, aw, ah] = element(&rt, ANCHOR).expect("the anchor shows").rect;
        assert!((ax + aw / 2.0 - x).abs() < 1.0 && (ay + ah / 2.0 - y).abs() < 1.0);
        assert!((aw - ah).abs() < 0.5, "round");
    }

    #[test]
    fn a_middle_click_on_a_folder_row_opens_it_in_a_new_tab_and_starts_nothing() {
        // browser/scrolling: Middle-clicking an entry row starts no autoscroll; the row's open-in-new-tab or reveal action runs instead.
        let mut rt = mount("c24-ten-k-light");
        let [x, y, _, h] = element(&rt, "row-11").expect("row 11").visible;
        press(&mut rt, x + 120.0, y + h / 2.0, PointerButton::Middle);
        assert!(!running(&rt));
        assert_eq!(rt.state.c24.intent, "Opened item folder 00010 in a new tab");
    }

    #[test]
    fn a_middle_click_on_the_bar_starts_nothing() {
        // browser/scrolling: Middle-clicking the empty background or gutter of a scrollable Columns, Icons, or List listing starts autoscroll. A round anchor marks the press point.
        // (The other half: a scroll bar isn't background, as in Strata's `is_background`.)
        let mut rt = mount("c24-ten-k-bar-light");
        let [x, y, w, h] = element(&rt, BAR).expect("the bar").visible;
        press(&mut rt, x + w / 2.0, y + h / 2.0, PointerButton::Middle);
        assert!(!running(&rt));
    }

    #[test]
    fn a_listing_that_fits_starts_no_autoscroll() {
        // browser/scrolling: Middle-clicking a listing that fits its viewport starts no autoscroll.
        let mut rt = mount("c24-fits-light");
        let v = viewport(&rt);
        press(
            &mut rt,
            v.x + 300.0,
            v.y + v.h - 20.0,
            PointerButton::Middle,
        );
        assert!(!running(&rt));
        assert!(element(&rt, ANCHOR).is_none());
    }

    #[test]
    fn a_middle_click_on_a_text_field_inside_the_list_starts_no_autoscroll() {
        // browser/scrolling: Middle-clicking a text field, such as the filter field, pastes as usual and starts no autoscroll.
        // (Half: no autoscroll, and nothing else, for a field inside the listing: C5's rename
        // editor on a folder's row. The paste is Rune's to add: it has no middle-click paste,
        // draft U-C24b-3. Review 2 SF1: the old stand-in field let the press reach the row.)
        let mut rt = mount("c24-rename-light");
        assert!(rt.state.c24.rename.has_keyboard());
        let [x, y, w, h] = element(&rt, "rename-field")
            .expect("C5's rename field")
            .visible;
        press(&mut rt, x + w / 2.0, y + h / 2.0, PointerButton::Middle);
        assert!(!running(&rt));
        assert_eq!(
            rt.state.c24.intent, "Nothing yet",
            "the row under it opened nothing"
        );
        assert!(rt.state.c24.rename.has_keyboard(), "the edit goes on");
        // Not vacuous: on the same row, the margin starts autoscroll, so the field is what
        // keeps its press from the listing.
        let [rx, ry, _, rh] = element(&rt, "row-11").expect("row 11").visible;
        press(&mut rt, rx + 2.0, ry + rh / 2.0, PointerButton::Middle);
        assert!(running(&rt), "the row's margin is background");
    }

    #[test]
    fn after_autoscroll_stops_the_keyboard_is_back_in_the_list() {
        // browser/scrolling: Escape, or another click with any button anywhere in the window, stops autoscroll.
        // (Deviation, review 2 SF3: the shield, a modal, takes the keyboard while it runs; Rune
        // gives it back to the list as it goes, so keys move the cursor again.)
        let mut rt = mount("c24-ten-k-light");
        let [x, y, _, h] = element(&rt, "row-3").expect("row 3").visible;
        press(&mut rt, x + 120.0, y + h / 2.0, PointerButton::Primary);
        let before = focused(&rt);
        assert!(before.is_some(), "the list has the keyboard");
        start(&mut rt);
        assert_ne!(focused(&rt), before, "the shield took it");
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        run(&mut rt, 0.2);
        assert!(!running(&rt));
        assert_eq!(focused(&rt), before);
        rt.key(KeyEvent {
            key: Key::Down,
            mods: Mods::default(),
        });
        run(&mut rt, 0.1);
        assert_eq!(selected(&rt), vec![4], "Down moves the cursor");
    }

    #[test]
    fn back_and_forward_buttons_still_reach_the_host() {
        // browser/scrolling: Mouse buttons 8 and 9 still go back and forward in history.
        let mut rt = mount("c24-ten-k-light");
        let v = viewport(&rt);
        press(&mut rt, v.x + 400.0, v.y + 100.0, PointerButton::Back);
        assert_eq!(rt.state.c24.history, "Back");
        press(&mut rt, v.x + 400.0, v.y + 100.0, PointerButton::Forward);
        assert_eq!(rt.state.c24.history, "Forward");
        assert!(!running(&rt));
    }

    // Steering.

    #[test]
    fn the_further_the_pointer_from_the_anchor_the_faster_the_list_scrolls_toward_it() {
        // browser/scrolling: Moving the pointer away from the anchor scrolls toward it, faster the further away, on both axes the listing can scroll.
        let near = travel("c24-ten-k-light", 60.0, 1.0);
        let far = travel("c24-ten-k-light", 200.0, 1.0);
        assert!(near > 0.0 && far > 2.0 * near, "near {near}, far {far}");

        // Back up past the anchor: it scrolls up.
        let mut rt = mount("c24-ten-k-light");
        let (x, y) = start(&mut rt);
        rt.pointer_move(x, y + 200.0);
        run(&mut rt, 1.0);
        let down = offset(&rt);
        rt.pointer_move(x, y - 200.0);
        run(&mut rt, 0.5);
        assert!(offset(&rt) < down);
    }

    #[test]
    fn autoscroll_moves_a_plain_viewport_on_both_axes() {
        // browser/scrolling: Moving the pointer away from the anchor scrolls toward it, faster the further away, on both axes the listing can scroll.
        let mut rt = mount("c24-bars-light");
        let r = rt.state.c24.grid_rect;
        let (x, y) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        press(&mut rt, x, y, PointerButton::Middle);
        assert!(running(&rt));
        rt.pointer_move(x + 150.0, y + 150.0);
        run(&mut rt, 1.0);
        let [ox, oy] = rt.state.c24.grid_feed.get().offset;
        assert!(ox > 50.0 && oy > 50.0, "{ox}, {oy}");
    }

    #[test]
    fn within_12_points_of_the_anchor_nothing_scrolls_and_nothing_wakes() {
        // browser/scrolling: The pointer must move more than 12 px from the anchor before scrolling starts; speed reaches 32 px per 16 ms frame at 232 px.
        // (Half: the dead zone. The top speed is the next test's.)
        let mut rt = mount("c24-ten-k-light");
        let (x, y) = start(&mut rt);
        rt.pointer_move(x + 6.0, y + 10.0);
        run(&mut rt, 1.0);
        assert_eq!(offset(&rt), 0.0);
        assert_eq!(
            rt.wake_deadline(),
            None,
            "an idle autoscroll asks for no frames"
        );
    }

    #[test]
    fn far_from_the_anchor_the_list_scrolls_32_points_a_16_ms_frame() {
        // browser/scrolling: The pointer must move more than 12 px from the anchor before scrolling starts; speed reaches 32 px per 16 ms frame at 232 px.
        // (Half: the top speed, 2,000 pt/s, within 5 %.)
        let speed = travel("c24-ten-k-light", 280.0, 1.0);
        assert!((speed - 2000.0).abs() < 100.0, "{speed} pt/s");
    }

    #[test]
    fn with_animations_off_autoscroll_still_moves_at_its_speed() {
        // browser/scrolling: Moving the pointer away from the anchor scrolls toward it, faster the further away, on both axes the listing can scroll.
        let speed = travel("c24-reduced-light", 280.0, 1.0);
        assert!((speed - 2000.0).abs() < 100.0, "{speed} pt/s");
    }

    #[test]
    fn the_pointer_over_the_list_shows_the_scroll_cursor_while_autoscroll_runs() {
        // browser/scrolling: While autoscroll runs, the pointer over that listing shows the all-scroll cursor.
        // (Partial: Rune has no all-scroll cursor; it shows Cursor::Move, U-C24-5.)
        let mut rt = mount("c24-ten-k-light");
        let (x, y) = start(&mut rt);
        rt.pointer_move(x + 300.0, y + 4.0);
        rt.frame(FRAME);
        assert_eq!(rt.cursor(), Cursor::Move);
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        run(&mut rt, 0.1);
        rt.pointer_move(x + 300.0, y + 6.0);
        rt.frame(FRAME);
        assert_eq!(rt.cursor(), Cursor::Default);
    }

    // Stopping.

    #[test]
    fn a_press_of_any_button_anywhere_stops_autoscroll_where_it_is() {
        // browser/scrolling: Escape, or another click with any button anywhere in the window, stops autoscroll.
        // (Half: a click with each button, outside the list. Escape is the next test's.)
        for button in [
            PointerButton::Primary,
            PointerButton::Secondary,
            PointerButton::Middle,
            PointerButton::Back,
            PointerButton::Forward,
            PointerButton::Other(0),
        ] {
            let mut rt = mount("c24-ten-k-light");
            let (x, y) = start(&mut rt);
            rt.pointer_move(x, y + 200.0);
            run(&mut rt, 0.5);
            let [tx, ty, tw, th] = element(&rt, "c24-title").expect("title").visible;
            press(&mut rt, tx + tw / 2.0, ty + th / 2.0, button);
            assert!(!running(&rt), "{button:?} stops it");
            let at = offset(&rt);
            run(&mut rt, 0.5);
            assert!(
                (offset(&rt) - at).abs() < 0.5,
                "{button:?}: the list stays put"
            );
            assert!(element(&rt, ANCHOR).is_none());
            assert_eq!(rt.state.c24.history, "No history button", "{button:?}");
        }
    }

    #[test]
    fn escape_stops_only_the_autoscroll_and_cancels_the_armed_chord() {
        // browser/scrolling: Escape while autoscroll runs stops only the autoscroll and cancels an armed 10xer chord.
        // (The chord is the page's stand-in: C18/C19 own the real one. Also checks the Escape half of "Escape, or another click with any button anywhere in the window, stops autoscroll.")
        let mut rt = mount("c24-ten-k-light");
        let [x, y, _, h] = element(&rt, "row-3").expect("row 3").visible;
        press(&mut rt, x + 120.0, y + h / 2.0, PointerButton::Primary);
        assert_eq!(selected(&rt), vec![3]);
        rt.key(KeyEvent {
            key: Key::Text("g".into()),
            mods: Mods::default(),
        });
        rt.frame(FRAME);
        assert!(rt.state.c24.chord);
        let (x, y) = start(&mut rt);
        rt.pointer_move(x, y + 200.0);
        run(&mut rt, 0.5);
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        run(&mut rt, 0.1);
        assert!(!running(&rt));
        let at = offset(&rt);
        run(&mut rt, 0.5);
        assert!((offset(&rt) - at).abs() < 0.5, "the list stays put");
        assert_eq!(selected(&rt), vec![3], "Escape cleared no selection");
        assert!(!rt.state.c24.chord, "the armed chord is cancelled");
        assert_eq!(rt.state.c24.stopped, "Stopped by Escape");
    }

    #[test]
    fn the_click_that_stops_autoscroll_selects_opens_and_bands_nothing() {
        // browser/scrolling: The click that stops autoscroll does not also select, open, or start a marquee.
        let mut rt = mount("c24-ten-k-light");
        let (x, y) = start(&mut rt);
        let [rx, ry, _, rh] = element(&rt, "row-4").expect("row 4").visible;
        press(&mut rt, rx + 120.0, ry + rh / 2.0, PointerButton::Primary);
        assert!(!running(&rt));
        assert!(selected(&rt).is_empty());
        assert_eq!(rt.state.c24.intent, "Nothing yet");

        // A drag that stops it draws no band.
        start(&mut rt);
        rt.pointer_move(x + 300.0, y);
        rt.frame(FRAME);
        rt.pointer_down(x + 300.0, y);
        rt.frame(FRAME);
        rt.pointer_move(x + 300.0, y + 80.0);
        rt.frame(FRAME);
        assert!(element(&rt, BAND).is_none());
        rt.pointer_up(x + 300.0, y + 80.0);
        rt.frame(FRAME);
        assert!(selected(&rt).is_empty());
    }

    #[test]
    fn leaving_or_hiding_the_list_stops_its_autoscroll_and_hides_the_anchor() {
        // browser/scrolling: Navigating away from, hiding, or closing a listing while it autoscrolls stops the autoscroll and hides the anchor.
        // (Navigating away and hiding; closing is the host's, which calls the same `stop_gone`.)
        let alt_left = KeyEvent {
            key: Key::Left,
            mods: Mods {
                alt: true,
                ..Mods::default()
            },
        };
        let mut rt = mount("c24-ten-k-light");
        let (x, y) = start(&mut rt);
        rt.pointer_move(x, y + 200.0);
        run(&mut rt, 0.3);
        rt.key(alt_left);
        run(&mut rt, 0.2);
        assert!(!running(&rt));
        assert!(element(&rt, ANCHOR).is_none());
        assert_eq!(rt.state.c24.folder, 1);
        assert_eq!(offset(&rt), 0.0, "the new folder starts at its top");

        let mut rt = mount("c24-ten-k-light");
        start(&mut rt);
        rt.key(KeyEvent {
            key: Key::Command('h'),
            mods: Mods::default(),
        });
        run(&mut rt, 0.2);
        assert!(rt.state.c24.hidden);
        assert!(!running(&rt));
        assert!(element(&rt, ANCHOR).is_none());
    }

    #[test]
    fn a_stopped_autoscroll_leaves_the_window_asleep() {
        // browser/scrolling: Escape, or another click with any button anywhere in the window, stops autoscroll.
        let mut rt = mount("c24-ten-k-light");
        let (x, y) = start(&mut rt);
        rt.pointer_move(x, y + 200.0);
        run(&mut rt, 0.5);
        assert!(
            rt.wake_deadline().is_some(),
            "a moving autoscroll asks for frames"
        );
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        run(&mut rt, 1.5);
        assert_eq!(rt.wake_deadline(), None);
    }

    // The bar.

    fn opacity(rt: &Runtime<Gallery>) -> f32 {
        element(rt, BAR).map_or(0.0, |e| e.opacity)
    }

    #[test]
    fn the_bar_is_hidden_at_rest_and_shows_while_the_list_scrolls_or_the_pointer_moves_over_it() {
        // browser/scrolling: Scrollbars fade in on hover or scrolling and stay hidden at rest; the trough strengthens on hover and while dragging.
        // (Half: showing and hiding. The trough is the next test's.)
        let mut rt = mount("c24-ten-k-light");
        assert_eq!(opacity(&rt), 0.0, "hidden at rest");
        let v = viewport(&rt);
        rt.wheel_xy(v.x + 300.0, v.y + 200.0, 0.0, -300.0);
        run(&mut rt, 0.3);
        assert!(opacity(&rt) > 0.9, "shown while it scrolls");
        run(&mut rt, 2.0);
        assert_eq!(opacity(&rt), 0.0, "faded after the scroll");
        rt.pointer_move(v.x + 300.0, v.y + 210.0);
        run(&mut rt, 0.3);
        assert!(opacity(&rt) > 0.9, "shown by the pointer over the list");
        run(&mut rt, 2.0);
        assert_eq!(opacity(&rt), 0.0, "faded once the pointer rests");
    }

    #[test]
    fn the_trough_strengthens_under_the_pointer_and_while_dragged() {
        // browser/scrolling: Scrollbars fade in on hover or scrolling and stay hidden at rest; the trough strengthens on hover and while dragging.
        // (Half: the trough.)
        let alpha = |rt: &Runtime<Gallery>| under(rt, BAR, "trough").map_or(0.0, |e| e.fill.a);
        let mut rt = mount("c24-ten-k-bar-light");
        let rest = alpha(&rt);
        let [x, y, w, h] = element(&rt, BAR).expect("the bar").visible;
        let (bx, by) = (x + w / 2.0, y + h - 10.0);
        rt.pointer_move(bx, by);
        run(&mut rt, 0.5);
        let hover = alpha(&rt);
        assert!(hover > rest, "{rest} -> {hover}");
        rt.pointer_down(bx, by);
        rt.frame(FRAME);
        rt.pointer_move(bx - 200.0, by - 40.0);
        run(&mut rt, 0.5);
        assert!(alpha(&rt) > rest, "held while dragging, away from the bar");
        rt.pointer_up(bx - 200.0, by - 40.0);
        run(&mut rt, 0.5);
        assert!((alpha(&rt) - rest).abs() < 1e-3, "back at rest");
    }

    #[test]
    fn dragging_the_thumb_scrolls_the_list_and_selects_nothing_even_with_alt() {
        // browser/selection/marquee: In List, Icons, Columns, or inline search results, dragging a view's own scrollbar handle scrolls the view and selects nothing, even with Alt held. This includes an overlay scrollbar over blank row space.
        let mut rt = mount("c24-ten-k-bar-light");
        let thumb = under(&rt, BAR, "thumb").expect("the thumb").visible;
        let (x, y) = (thumb[0] + thumb[2] / 2.0, thumb[1] + thumb[3] / 2.0);
        let before = offset(&rt);
        let alt = Mods {
            alt: true,
            ..Mods::default()
        };
        rt.physical_input(PhysicalInput::Modifiers(alt));
        rt.frame(FRAME);
        rt.pointer_move(x, y);
        rt.frame(FRAME);
        rt.pointer_down(x, y);
        rt.frame(FRAME);
        for step in 1..=10 {
            rt.pointer_move(x, y + 20.0 * step as f32);
            rt.frame(FRAME);
            assert!(element(&rt, BAND).is_none(), "no band");
        }
        rt.pointer_up(x, y + 200.0);
        rt.frame(FRAME);
        rt.physical_input(PhysicalInput::Modifiers(Mods::default()));
        run(&mut rt, 0.2);
        assert!(offset(&rt) > before + 1000.0, "{before} -> {}", offset(&rt));
        assert!(selected(&rt).is_empty());
        assert_eq!(rt.state.c24.intent, "Nothing yet");
    }

    fn focused(rt: &Runtime<Gallery>) -> Option<String> {
        rt.elements()
            .into_iter()
            .find(|e| e.focused)
            .and_then(|e| e.key)
    }

    #[test]
    fn a_press_or_drag_on_the_bar_leaves_the_keyboard_focus_where_it_was() {
        // browser/selection/marquee: A press on a button, text entry, label, image, slider, scrollbar, or sidebar place keeps that control's own behavior and starts no marquee.
        // (Review round 1: the scroll bar's own behaviour takes no focus, as GTK's scrollbar doesn't.)
        let mut rt = mount("c24-ten-k-bar-light");
        let [fx, fy, _, fh] = element(&rt, "c24-filter").expect("the field").visible;
        press(&mut rt, fx + 5.0, fy + fh / 2.0, PointerButton::Primary);
        assert_eq!(focused(&rt).as_deref(), Some("c24-filter"));
        let [x, y, w, h] = element(&rt, BAR).expect("the bar").visible;
        press(&mut rt, x + w / 2.0, y + h - 10.0, PointerButton::Primary);
        assert_eq!(
            focused(&rt).as_deref(),
            Some("c24-filter"),
            "after a trough press"
        );
        let thumb = under(&rt, BAR, "thumb").expect("the thumb").visible;
        let (tx, ty) = (thumb[0] + thumb[2] / 2.0, thumb[1] + thumb[3] / 2.0);
        rt.pointer_move(tx, ty);
        rt.frame(FRAME);
        rt.pointer_down(tx, ty);
        rt.frame(FRAME);
        rt.pointer_move(tx, ty - 100.0);
        rt.frame(FRAME);
        rt.pointer_up(tx, ty - 100.0);
        rt.frame(FRAME);
        assert_eq!(
            focused(&rt).as_deref(),
            Some("c24-filter"),
            "after a thumb drag"
        );
    }

    #[test]
    fn a_secondary_press_on_the_bar_does_nothing() {
        // browser/selection/marquee: A press on a button, text entry, label, image, slider, scrollbar, or sidebar place keeps that control's own behavior and starts no marquee.
        // (Review round 1: a secondary press on the bar reaches no row: no menu, no selection.)
        let mut rt = mount("c24-ten-k-bar-light");
        let [x, y, w, h] = element(&rt, BAR).expect("the bar").visible;
        press(&mut rt, x + w / 2.0, y + h / 2.0, PointerButton::Secondary);
        assert_eq!(rt.state.c24.intent, "Nothing yet");
        assert!(selected(&rt).is_empty());
    }

    #[test]
    fn an_escape_in_the_dead_zone_is_over_at_once_and_the_next_click_reaches_the_row() {
        // browser/scrolling: Escape, or another click with any button anywhere in the window, stops autoscroll.
        // (Review round 1: an Escape with nothing moving stops at once, so the next click isn't taken as a stop.)
        let mut rt = mount("c24-ten-k-light");
        start(&mut rt);
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        // One frame: the shield goes, and with nothing moving the autoscroll is over in the
        // same frame, with no further frame needed (a resting window may run none): what the
        // view drew then says so. (A click in the same frame as the Escape still lands on the
        // shield: a known gap, in the module docs.)
        rt.frame(FRAME);
        assert!(element(&rt, ANCHOR).is_none());
        assert!(!running(&rt), "idle, not stopping");
        assert_eq!(
            element(&rt, "c24-status").and_then(|e| e.text).as_deref(),
            Some("Stopped by Escape")
        );
        let [x, y, _, h] = element(&rt, "row-4").expect("row 4").visible;
        rt.pointer_down(x + 120.0, y + h / 2.0);
        rt.frame(FRAME);
        rt.pointer_up(x + 120.0, y + h / 2.0);
        rt.frame(FRAME);
        assert!(!running(&rt));
        assert_eq!(selected(&rt), vec![4]);
    }

    #[test]
    fn a_press_on_the_trough_jumps_there_and_starts_no_marquee() {
        // browser/selection/marquee: A press on a button, text entry, label, image, slider, scrollbar, or sidebar place keeps that control's own behavior and starts no marquee.
        // (Half: the scroll bar.)
        let mut rt = mount("c24-ten-k-bar-light");
        let [x, y, w, h] = element(&rt, BAR).expect("the bar").visible;
        let before = offset(&rt);
        let (bx, by) = (x + w / 2.0, y + h - 4.0);
        rt.pointer_move(bx, by);
        rt.frame(FRAME);
        rt.pointer_down(bx, by);
        rt.frame(FRAME);
        rt.pointer_move(bx, by - 20.0);
        rt.frame(FRAME);
        assert!(element(&rt, BAND).is_none());
        rt.pointer_up(bx, by - 20.0);
        run(&mut rt, 0.2);
        assert!(offset(&rt) > before + 50_000.0, "jumped toward the end");
        assert!(selected(&rt).is_empty());
    }

    // Nested scrollers.

    fn strip_x(rt: &Runtime<Gallery>) -> f32 {
        rt.state.c24.strip_feed.get().offset[0]
    }

    fn column_y(rt: &Runtime<Gallery>, i: usize) -> f32 {
        rt.state.c24.columns[i].feed.get().offset[1]
    }

    fn column_point(rt: &Runtime<Gallery>, i: usize) -> (f32, f32) {
        let v = rt.state.c24.columns[i].coll.viewport();
        (v.x + v.w / 2.0, v.y + v.h / 2.0)
    }

    #[test]
    fn a_vertical_wheel_over_a_column_scrolls_only_that_column() {
        // browser/view-modes/columns: Shift+wheel over a listing scrolls the column strip horizontally; vertical wheel ticks scroll only the listing.
        // (Half: vertical ticks. Rune turns Shift+wheel sideways in its window host only, so headless can't send it: U-C24-7.)
        let mut rt = mount("c24-nested-light");
        let (x, y) = column_point(&rt, 1);
        rt.wheel_xy(x, y, 0.0, -120.0);
        run(&mut rt, 0.2);
        assert_eq!(column_y(&rt, 1), 120.0);
        assert_eq!(column_y(&rt, 0), 0.0);
        assert_eq!(strip_x(&rt), 0.0);
    }

    #[test]
    fn a_sideways_swipe_over_a_column_scrolls_the_strip() {
        // browser/scrolling: A two-finger horizontal touchpad swipe over Columns scrolls the column strip.
        // Also: browser/view-modes/columns: Horizontal touchpad gestures over a listing scroll the column strip.
        let mut rt = mount("c24-nested-light");
        let (x, y) = column_point(&rt, 0);
        rt.wheel_event(
            WheelEvent::new(-40.0, -3.0)
                .at(x, y)
                .phase(GesturePhase::Began),
        );
        rt.frame(FRAME);
        for _ in 0..3 {
            rt.wheel_event(
                WheelEvent::new(-40.0, -6.0)
                    .at(x, y)
                    .phase(GesturePhase::Moved),
            );
            rt.frame(FRAME);
        }
        rt.wheel_event(
            WheelEvent::new(0.0, 0.0)
                .at(x, y)
                .phase(GesturePhase::Ended),
        );
        run(&mut rt, 0.2);
        assert_eq!(strip_x(&rt), 160.0);
        for i in 0..COLUMNS {
            assert_eq!(column_y(&rt, i), 0.0, "column {i} kept still");
        }
    }

    #[test]
    fn touchpad_scrolling_in_a_listing_moves_by_its_own_small_deltas() {
        // browser/scrolling: Vertical touchpad scrolling in listings moves smoothly instead of in wheel-sized steps.
        let mut rt = mount("c24-nested-light");
        let (x, y) = column_point(&rt, 0);
        rt.wheel_event(
            WheelEvent::new(0.0, -3.0)
                .at(x, y)
                .phase(GesturePhase::Began),
        );
        rt.frame(FRAME);
        assert_eq!(column_y(&rt, 0), 3.0);
        rt.wheel_event(
            WheelEvent::new(0.0, -2.5)
                .at(x, y)
                .phase(GesturePhase::Moved),
        );
        rt.frame(FRAME);
        assert_eq!(column_y(&rt, 0), 5.5);
    }

    #[test]
    fn one_autoscroll_runs_for_the_window_across_columns() {
        // browser/scrolling: Middle-clicking the empty background or gutter of a scrollable Columns, Icons, or List listing starts autoscroll. A round anchor marks the press point.
        // (The Columns case: a column's own collection starts it; a second middle click elsewhere stops it rather than starting another.)
        let mut rt = mount("c24-nested-light");
        let v = rt.state.c24.columns[1].coll.viewport();
        let (x, y) = (v.x + 2.0, v.y + v.h / 2.0);
        press(&mut rt, x, y, PointerButton::Middle);
        assert_eq!(rt.state.c24.auto.owner(), Some(column_key(1)));
        rt.pointer_move(x + 10.0, y + 200.0);
        run(&mut rt, 0.5);
        assert!(column_y(&rt, 1) > 100.0);
        assert_eq!(column_y(&rt, 0), 0.0);
        let w = rt.state.c24.columns[0].coll.viewport();
        press(&mut rt, w.x + 2.0, w.y + 50.0, PointerButton::Middle);
        assert!(!running(&rt));
    }

    // Rune's perf meters, through the gallery binary.

    /// Runs script `name` from `tests/scripts/c24/` through the gallery binary with `env`;
    /// its stderr.
    fn gallery_run(name: &str, env: &[(&str, &str)]) -> String {
        let script = support::scripts_dir("c24").join(format!("{name}.json"));
        let spec: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&script).expect("the script"))
                .expect("JSON");
        let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("c24");
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

    fn perf_warnings(stderr: &str) -> Vec<&str> {
        stderr
            .lines()
            .filter(|l| l.contains("RUNE_PERF_WARN=0 silences"))
            .collect()
    }

    /// Rune's debug builds warn, on stderr, when one source commits through most of a scroll,
    /// when an app steps a viewport with instant scrolls on committing frames, and when a
    /// resting window keeps waking. None may fire for an autoscroll (gliding, or stepping with
    /// animations off) or a press on the trough.
    #[test]
    #[cfg_attr(not(debug_assertions), ignore = "Rune's perf warnings are debug-only")]
    fn the_autoscroll_scripts_keep_runes_perf_warnings_silent() {
        for name in [
            "autoscroll-move",
            "autoscroll-long",
            "autoscroll-reduced",
            "bar-press",
        ] {
            let stderr = gallery_run(name, &[]);
            let warnings = perf_warnings(&stderr);
            assert!(
                warnings.is_empty(),
                "{name}: Rune's perf warnings:\n{}",
                warnings.join("\n")
            );
        }
    }

    /// A thumb dragged over 10,000 rows moves the list past its mounted rows on every frame,
    /// and Rune then commits about 4 times and runs the view about 3 times a frame (a wheel
    /// moving as far runs it none), so its "stepping its own glide" warning fires, as it does
    /// for rune-kit's own `ScrollBar` (Rune draft U-C24-8). That one warning, on the list's
    /// viewport, is the only one allowed.
    #[test]
    #[cfg_attr(not(debug_assertions), ignore = "Rune's perf warnings are debug-only")]
    fn a_thumb_drag_trips_only_runes_instant_scroll_warning() {
        let stderr = gallery_run("bar-drag", &[]);
        let others: Vec<&str> = perf_warnings(&stderr)
            .into_iter()
            .filter(|l| {
                !(l.starts_with("rune: an instant scroll") && l.contains("\"c24-files/viewport\""))
            })
            .collect();
        assert!(others.is_empty(), "bar-drag: {}", others.join("\n"));
    }

    /// Five seconds of full-speed autoscroll over 10,000 rows, then a thumb drag, under
    /// `RUNE_COMMIT_BUDGET_MS=8`: no commit over 8 ms once the scene is mounted.
    ///
    /// Release-build commits only: `cargo test --profile perf -p strata-gallery --test c24
    /// budget`.
    #[test]
    #[cfg_attr(debug_assertions, ignore = "perf: run with --profile perf")]
    fn autoscroll_and_thumb_drags_over_ten_thousand_rows_stay_under_the_commit_budget() {
        for name in ["autoscroll-long", "bar-drag"] {
            let stderr = gallery_run(name, &[("RUNE_COMMIT_BUDGET_MS", "8"), (WARMUP, "1")]);
            let after = stderr
                .split_once(WARMED)
                .map(|(_, after)| after)
                .unwrap_or_else(|| panic!("{name}: the page never warmed up:\n{stderr}"));
            let over: Vec<&str> = after
                .lines()
                .filter(|l| l.starts_with("rune: commit took"))
                .collect();
            assert!(
                over.is_empty(),
                "{name}: commits over budget:\n{}",
                over.join("\n")
            );
        }
    }

    // The stand-in Sort by panel and C24's OutsideWheel.

    use strata_gallery::scenes::c24::{PANEL, SIDEBAR};

    fn panel_open(rt: &Runtime<Gallery>) -> bool {
        element(rt, PANEL).is_some()
    }

    fn side_offset(rt: &Runtime<Gallery>) -> f32 {
        rt.state.c24.side_feed.get().offset[1]
    }

    fn tick(rt: &mut Runtime<Gallery>, x: f32, y: f32, ev: WheelEvent) {
        rt.pointer_move(x, y);
        rt.frame(FRAME);
        rt.wheel_event(ev.at(x, y));
        run(rt, 0.1);
    }

    fn notch() -> WheelEvent {
        WheelEvent::new(0.0, -120.0).precise(false)
    }

    /// A point over the list, clear of the panel.
    fn over_list(rt: &Runtime<Gallery>) -> (f32, f32) {
        let v = viewport(rt);
        (v.x + 100.0, v.y + v.h - 60.0)
    }

    #[test]
    fn a_wheel_tick_outside_the_panel_closes_it_and_scrolls_the_list_under_the_pointer() {
        // browser/scrolling: A wheel tick outside an open Sort by, Appearance, or Icons Thumbnail size panel closes it.
        // browser/scrolling: The tick that closes a panel also scrolls the listing under the pointer, and no other; over the sidebar or chrome it only closes the panel.
        let mut rt = mount("c24-panel-light");
        assert!(panel_open(&rt));
        let (x, y) = over_list(&rt);
        tick(&mut rt, x, y, notch());
        assert!(!panel_open(&rt));
        assert!(offset(&rt) > 0.0, "the list scrolled");
        assert_eq!(side_offset(&rt), 0.0, "the sidebar didn't");
    }

    #[test]
    fn the_forwarded_tick_moves_the_list_as_far_as_a_plain_tick() {
        // browser/scrolling: That forwarded tick moves the listing by the step a wheel notch normally moves it.
        let mut rt = mount("c24-panel-light");
        let (x, y) = over_list(&rt);
        tick(&mut rt, x, y, notch());
        let forwarded = offset(&rt);
        tick(&mut rt, x, y, notch());
        let plain = offset(&rt) - forwarded;
        assert!(forwarded > 0.0);
        assert!((forwarded - plain).abs() < 0.5, "{forwarded} then {plain}");
    }

    #[test]
    fn over_the_sidebar_or_the_header_a_tick_only_closes_the_panel() {
        // browser/scrolling: The tick that closes a panel also scrolls the listing under the pointer, and no other; over the sidebar or chrome it only closes the panel.
        for over in [SIDEBAR, "c24-title"] {
            let mut rt = mount("c24-panel-light");
            let [x, y, w, h] = element(&rt, over).expect(over).visible;
            tick(&mut rt, x + w / 2.0, y + h.min(200.0) / 2.0, notch());
            assert!(!panel_open(&rt), "{over}");
            assert_eq!(offset(&rt), 0.0, "{over}: the list stays");
            assert_eq!(side_offset(&rt), 0.0, "{over}: the sidebar stays");
        }
    }

    #[test]
    fn a_press_outside_the_panel_closes_it_and_selects_nothing_under_it() {
        // Review round 1, SF3: the scrim is the panel's outside press too, in place of the
        // popover's own: a press there over a row closes the panel and leaves the row alone.
        use strata_gallery::scenes::c24::PANEL_SCRIM;
        let mut rt = mount("c24-panel-light");
        assert!(element(&rt, PANEL_SCRIM).is_some());
        let [rx, ry, rw, rh] = element(&rt, "row-3").expect("row 3").visible;
        let (x, y) = (viewport(&rt).x + 100.0, ry + rh / 2.0);
        assert!((rx..rx + rw).contains(&x), "the press lands on row 3");
        press(&mut rt, x, y, PointerButton::Primary);
        run(&mut rt, 0.3);
        assert!(!panel_open(&rt));
        assert!(element(&rt, PANEL_SCRIM).is_none(), "the scrim is gone");
        assert_eq!(selected(&rt), Vec::<u64>::new(), "nothing is selected");
    }

    #[test]
    fn a_wheel_inside_the_panel_keeps_it_open_and_scrolls_nothing_under_it() {
        // browser/scrolling: Wheel scrolling inside an open panel keeps it open.
        let mut rt = mount("c24-panel-light");
        let [x, y, w, h] = element(&rt, PANEL).expect("the panel").visible;
        tick(&mut rt, x + w / 2.0, y + h / 2.0, notch());
        assert!(panel_open(&rt));
        assert_eq!(offset(&rt), 0.0, "the list under the panel stays put");
    }
}

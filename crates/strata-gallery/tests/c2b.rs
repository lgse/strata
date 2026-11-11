//! C2b on C2's page: the size badge, the width polish, the resize line's accent and the
//! reveal of a cursor the view places. Each test names the feature-map Behavior bullets it
//! checks, where a script can't see them: paint opacity over time, widths and scroll offsets.

mod c2b {
    use rune::{ElementInfo, Env, Key, KeyEvent, Mods, Runtime};
    use serde_json::Value;
    use strata_components::collection::selection::{Listing, Position};
    use strata_components::columns::{Focus, Opening, keys};
    use strata_components::contract::{EntrySource, ListSource};
    use strata_gallery::Gallery;
    use strata_gallery::scenes::{self, c2::Listed, c2::id_in};
    use strata_kit::look::{Look, LookParams};

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
        settle(&mut rt, 40);
        rt
    }

    fn settle(rt: &mut Runtime<Gallery>, frames: usize) {
        for _ in 0..frames {
            rt.frame(DT);
        }
    }

    fn info(rt: &Runtime<Gallery>, key: &str) -> Option<ElementInfo> {
        rt.elements()
            .into_iter()
            .find(|e| !e.exiting && e.key.as_deref() == Some(key))
    }

    fn at(rt: &Runtime<Gallery>, key: &str) -> (f32, f32) {
        let [x, y, w, h] = info(rt, key)
            .unwrap_or_else(|| panic!("no element {key}"))
            .visible;
        (x + w / 2.0, y + h / 2.0)
    }

    fn click_at(rt: &mut Runtime<Gallery>, x: f32, y: f32) {
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(DT);
        rt.pointer_up(x, y);
        rt.frame(DT);
    }

    fn tap(rt: &mut Runtime<Gallery>, key: &str) {
        let (x, y) = at(rt, key);
        click_at(rt, x, y);
        settle(rt, 2);
    }

    /// A press on `key`'s name, near the start of its row.
    fn tap_name(rt: &mut Runtime<Gallery>, key: &str) {
        let [x, y, _, h] = info(rt, key).expect("row").visible;
        click_at(rt, x + 40.0, y + h / 2.0);
        settle(rt, 2);
    }

    fn key(rt: &mut Runtime<Gallery>, k: Key) {
        rt.key(KeyEvent {
            key: k,
            mods: Mods::default(),
        });
        rt.frame(DT);
        rt.frame(DT);
    }

    fn row(parent: &str, name: &str) -> String {
        format!("row-{}", id_in(parent, name).get())
    }

    /// The opacity row `row`'s badge draws at (its hover group's fade included), or `None`
    /// when the row has no badge.
    fn badge(rt: &Runtime<Gallery>, row: &str) -> Option<f32> {
        let tail = format!("/{row}/");
        rt.elements()
            .into_iter()
            .find(|e| {
                !e.exiting
                    && e.key.as_deref() == Some(keys::BADGE)
                    && format!("{}/", e.path).contains(&tail)
            })
            .map(|e| e.opacity)
    }

    /// The accessibility node keyed `key`.
    fn node(rt: &Runtime<Gallery>, key: &str) -> Option<Value> {
        fn find(v: &Value, key: &str) -> Option<Value> {
            if v["key"] == key {
                return Some(v.clone());
            }
            v["children"].as_array()?.iter().find_map(|c| find(c, key))
        }
        find(&rt.accessibility_json(), key)
    }

    fn description(rt: &Runtime<Gallery>, row: &str) -> Option<String> {
        node(rt, row)
            .unwrap_or_else(|| panic!("no node {row}"))
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    /// Column `depth`'s drawn width (its shell keyed by its slot, the depth plus one in these
    /// scenes).
    fn width(rt: &Runtime<Gallery>, depth: usize) -> f32 {
        info(rt, &format!("c2-slot-{}", depth + 1))
            .expect("shell")
            .rect[2]
    }

    /// Presses column `serial`'s edge and moves `dx` in four steps, holding the button.
    fn drag_edge(rt: &mut Runtime<Gallery>, serial: u64, dx: f32) {
        let (x, y) = at(rt, &keys::edge(&format!("c2-col-{serial}")));
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(DT);
        for k in 1..=4 {
            rt.pointer_move(x + dx * k as f32 / 4.0, y);
            rt.frame(DT);
        }
    }

    fn release(rt: &mut Runtime<Gallery>) {
        let (x, y) = rt.pointer().expect("pointer");
        rt.pointer_up(x, y);
        rt.frame(DT);
    }

    /// A click on column `serial`'s edge, moving nothing.
    fn click_edge(rt: &mut Runtime<Gallery>, serial: u64) {
        let (x, y) = at(rt, &keys::edge(&format!("c2-col-{serial}")));
        click_at(rt, x, y);
    }

    fn text_size(rt: &mut Runtime<Gallery>, size: u16) {
        rt.update(|g, _| {
            g.look = Look::fallback(g.dark).with_params(&LookParams {
                text_size: size,
                ..LookParams::default()
            });
        });
    }

    fn set_cap(rt: &mut Runtime<Gallery>, cap: f32) {
        rt.update(|g, _| g.c2.columns.set_width_cap(Some(cap)));
    }

    // The size badge.

    #[test]
    fn selected_and_open_path_rows_show_their_badge_and_assistive_technology_hears_it() {
        // browser/view-modes/columns: Hovering or selecting a folder row, or a folder on the
        // open path, shows "No items" or "N items" in the right-side badge. And: Files show
        // their size in the same badge.
        let rt = scene("c2-badges-light");
        let documents = row("Home", "Documents");
        let taxes = row("Documents", "Taxes");
        let cv = row("Documents", "cv.pdf");
        let letters = row("Documents", "Letters");
        assert_eq!(badge(&rt, &documents), Some(1.0), "on the open path");
        assert_eq!(badge(&rt, &taxes), Some(1.0), "a selected folder");
        assert_eq!(badge(&rt, &cv), Some(1.0), "a selected file");
        assert_eq!(
            badge(&rt, &letters),
            Some(0.0),
            "neither: hidden till hovered"
        );
        assert_eq!(description(&rt, &documents).as_deref(), Some("27 items"));
        assert_eq!(description(&rt, &taxes).as_deref(), Some("5 items"));
        let size = description(&rt, &cv).expect("a size");
        assert!(size.ends_with(" kB") || size.ends_with(" MB"), "{size}");
        // Assistive technology hears it whether or not it shows, as Strata's label is there.
        assert_eq!(description(&rt, &letters).as_deref(), Some("6 items"));
    }

    #[test]
    fn a_folder_whose_count_is_unknown_shows_no_badge() {
        // browser/view-modes/columns: A row with no size or count to show, such as a folder
        // whose count is unknown, shows no badge.
        let rt = scene("c2-home-light");
        let slow = row("Home", "Slow");
        assert_eq!(badge(&rt, &slow), None);
        assert_eq!(description(&rt, &slow), None);
        assert!(badge(&rt, &row("Home", "Empty")).is_some());
        assert_eq!(
            description(&rt, &row("Home", "Empty")).as_deref(),
            Some("No items")
        );
    }

    #[test]
    fn with_sizes_off_no_row_shows_a_badge_and_a_late_count_stays_hidden() {
        // browser/view-modes/columns: With Show sizes in Columns view off, the badge stays
        // hidden for files and folders in every open window. And: A count that arrives after
        // the setting is turned off leaves the badge hidden.
        let mut rt = scene("c2-sizes-off-light");
        for r in [
            row("Home", "Documents"),
            row("Documents", "Taxes"),
            row("Documents", "cv.pdf"),
        ] {
            assert_eq!(badge(&rt, &r), None, "{r}");
            assert_eq!(description(&rt, &r), None, "{r}");
        }
        // Slow's count fills in once it has been listed, with the setting off.
        let slow = row("Home", "Slow");
        tap(&mut rt, &keys::title("c2-col-1"));
        tap_name(&mut rt, &slow);
        settle(&mut rt, 150);
        assert!(rt.state.c2.listings.1, "Slow is listed");
        assert_eq!(badge(&rt, &slow), None);
        assert_eq!(description(&rt, &slow), None);
        // Turned on: the count already there shows.
        rt.update(|g, _| g.c2.show_sizes = true);
        settle(&mut rt, 2);
        assert_eq!(badge(&rt, &slow), Some(1.0), "selected");
        assert_eq!(description(&rt, &slow).as_deref(), Some("10 items"));
    }

    #[test]
    fn the_pointer_fades_a_badge_in_over_120_ms_and_leaving_hides_it_at_once() {
        // browser/view-modes/columns: Moving the pointer onto a row fades its badge in over 120
        // ms. And: Moving the pointer off a row hides its badge at once, without a fade.
        let mut rt = scene("c2-badges-light");
        let letters = row("Documents", "Letters");
        let (x, y) = at(&rt, &letters);
        rt.pointer_move(x, y);
        let mut fade = Vec::new();
        for _ in 0..10 {
            rt.frame(DT);
            fade.push(badge(&rt, &letters).expect("badge"));
        }
        let partway = fade.iter().filter(|o| **o > 0.05 && **o < 0.95).count();
        assert!(partway >= 3, "it fades in: {fade:?}");
        let full = fade.iter().position(|o| *o >= 0.999).expect("lands");
        assert!(
            (full as f64 + 1.0) * DT <= 0.12 + 2.0 * DT,
            "within about 120 ms: {fade:?}"
        );
        // Off the row, onto the next: hidden in the next frame.
        let (x2, y2) = at(&rt, &row("Documents", "Reports"));
        rt.pointer_move(x2, y2);
        rt.frame(DT);
        assert_eq!(badge(&rt, &letters), Some(0.0));
    }

    #[test]
    fn selecting_a_row_shows_its_badge_at_once_and_moving_off_hides_it_at_once() {
        // browser/view-modes/columns: Selecting a row, or opening a folder onto the open path,
        // shows its badge at once. And: Moving the selection off a row hides that row's badge
        // at once.
        let mut rt = scene("c2-badges-light");
        // The pointer on the title, over no row.
        tap(&mut rt, &keys::title("c2-col-3"));
        tap(&mut rt, &keys::title("c2-col-2"));
        let cv = row("Documents", "cv.pdf");
        let budget = row("Documents", "budget-2026.ods");
        assert_eq!(badge(&rt, &budget), Some(0.0));
        key(&mut rt, Key::Down);
        assert_eq!(badge(&rt, &budget), Some(1.0), "selected: at once");
        assert_eq!(badge(&rt, &cv), Some(0.0), "the selection left it: at once");
        assert_eq!(badge(&rt, &row("Documents", "Taxes")), Some(0.0));
        // A folder opened onto the open path by a click shows its badge in the frame after.
        let letters = row("Documents", "Letters");
        let [x, y, _, h] = info(&rt, &letters).expect("row").visible;
        rt.pointer_move(x + 40.0, y + h / 2.0);
        rt.pointer_down(x + 40.0, y + h / 2.0);
        rt.frame(DT);
        rt.pointer_up(x + 40.0, y + h / 2.0);
        // The pointer leaves for the header before the frame, so no hover holds it up.
        let (tx, ty) = at(&rt, &keys::title("c2-col-1"));
        rt.pointer_move(tx, ty);
        rt.frame(DT);
        assert_eq!(rt.state.c2.listings.0[2].name, "Letters");
        assert_eq!(badge(&rt, &letters), Some(1.0));
    }

    #[test]
    fn arrow_keys_hide_a_hovered_rows_badge_and_the_next_move_fades_it_back() {
        // browser/view-modes/columns: With the pointer resting on an unselected row, arrow keys
        // that move the selection hide that row's badge at once. And: Moving the pointer then
        // fades that badge in again.
        let mut rt = scene("c2-badges-light");
        tap(&mut rt, &keys::title("c2-col-2"));
        let letters = row("Documents", "Letters");
        let (x, y) = at(&rt, &letters);
        rt.pointer_move(x, y);
        settle(&mut rt, 12);
        assert_eq!(badge(&rt, &letters), Some(1.0));
        key(&mut rt, Key::Down);
        assert_eq!(badge(&rt, &letters), Some(0.0), "hidden at once");
        rt.pointer_move(x + 4.0, y);
        rt.frame(DT);
        rt.frame(DT);
        rt.frame(DT);
        let o = badge(&rt, &letters).expect("badge");
        assert!(o > 0.0 && o < 1.0, "fading in again: {o}");
        settle(&mut rt, 10);
        assert_eq!(badge(&rt, &letters), Some(1.0));
    }

    #[test]
    fn a_renamed_row_hides_its_badge_and_it_returns_only_with_the_setting_on() {
        // browser/view-modes/columns: While a row is being renamed its badge stays hidden;
        // after the rename ends it returns only with the setting on.
        let mut rt = scene("c2-badges-light");
        let cv = row("Documents", "cv.pdf");
        let id = id_in("Documents", "cv.pdf");
        rt.update(|g, _| g.c2.renaming = Some((1, id)));
        settle(&mut rt, 2);
        assert_eq!(badge(&rt, &cv), None);
        assert_eq!(description(&rt, &cv), None);
        rt.update(|g, _| g.c2.renaming = None);
        settle(&mut rt, 2);
        assert_eq!(badge(&rt, &cv), Some(1.0));
        assert!(description(&rt, &cv).is_some());
        rt.update(|g, _| g.c2.renaming = Some((1, id)));
        settle(&mut rt, 2);
        rt.update(|g, _| {
            g.c2.show_sizes = false;
            g.c2.renaming = None;
        });
        settle(&mut rt, 2);
        assert_eq!(badge(&rt, &cv), None, "the setting went off meanwhile");
        assert_eq!(description(&rt, &cv), None);
    }

    /// The chip of row `row`, as drawn.
    fn chip(rt: &Runtime<Gallery>, row: &str) -> ElementInfo {
        let tail = format!("/{row}/");
        rt.elements()
            .into_iter()
            .find(|e| {
                !e.exiting
                    && e.key.as_deref() == Some(keys::BADGE)
                    && format!("{}/", e.path).contains(&tail)
            })
            .expect("a chip")
    }

    #[test]
    fn turning_animations_off_at_runtime_reaches_rows_already_built() {
        // browser/view-modes/columns: Moving the pointer onto a row fades its badge in over 120
        // ms (with animations off, at once; rows are memoised, so the change must reach rows
        // built before it).
        let mut rt = scene("c2-badges-light");
        let letters = row("Documents", "Letters");
        let (x, y) = at(&rt, &letters);
        let (ox, oy) = at(&rt, &keys::title("c2-col-1"));
        rt.update(|g, _| g.look.animations = false);
        settle(&mut rt, 2);
        rt.pointer_move(x, y);
        rt.frame(DT);
        rt.frame(DT);
        assert_eq!(
            badge(&rt, &letters),
            Some(1.0),
            "at once with animations off"
        );
        rt.pointer_move(ox, oy);
        settle(&mut rt, 2);
        rt.update(|g, _| g.look.animations = true);
        settle(&mut rt, 2);
        rt.pointer_move(x, y);
        rt.frame(DT);
        rt.frame(DT);
        let o = badge(&rt, &letters).expect("badge");
        assert!(o > 0.0 && o < 1.0, "fading again with them on: {o}");
    }

    #[test]
    fn the_badge_tokens_and_the_highlight_reach_rows_already_built() {
        // browser/view-modes/columns: The badge is an opaque chip in the theme's surface color
        // with 5 px rounded corners; on an open-path folder row it takes the highlight color
        // (overrides of them reach rows built before the change).
        let mut rt = scene("c2-badges-light");
        let documents = row("Home", "Documents");
        assert_eq!(chip(&rt, &documents).radius, 5.0);
        let highlight = rune::Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        rt.update(|g, _| {
            g.look.slots.columns.badge_radius = Some(2.0);
            g.look.fill.highlight = highlight;
        });
        // A fill eases to its new colour.
        settle(&mut rt, 30);
        let c = chip(&rt, &documents);
        assert_eq!(c.radius, 2.0);
        assert_eq!(c.fill, highlight);
    }

    // Widths.

    #[test]
    fn the_release_eases_the_other_columns_to_the_dragged_width_once() {
        // browser/view-modes/columns: On release, the window's other open columns ease to the
        // dragged width once; other windows keep theirs.
        let mut rt = scene("c2-three-columns-light");
        drag_edge(&mut rt, 1, 100.0);
        assert_eq!(width(&rt, 1), 300.0, "not while it moves");
        release(&mut rt);
        rt.frame(DT);
        let mid = width(&rt, 1);
        assert!(mid > 300.0 && mid < 400.0, "easing: {mid}");
        settle(&mut rt, 40);
        assert_eq!(
            [width(&rt, 0), width(&rt, 1), width(&rt, 2)],
            [400.0; 3],
            "all at the dragged width"
        );
        assert_eq!(rt.state.c2.intent, "Width 400");
        // Once: a new column opens at the width handed over, as any does; the open ones keep
        // what they have.
        assert_eq!(rt.state.c2.columns.column(2).expect("open").width(), 400.0);
    }

    #[test]
    fn a_click_on_an_edge_spreads_its_width_and_hands_it_over() {
        // browser/view-modes/columns: On release, the window's other open columns ease to the
        // dragged width once ... A click on an edge without dragging also spreads.
        let mut rt = scene("c2-three-columns-light");
        // Letters, opened in Reports' place after the saved width became 500.
        rt.update(|g, _| g.c2.columns.set_default_width(500.0));
        tap_name(&mut rt, &row("Documents", "Letters"));
        settle(&mut rt, 40);
        assert_eq!([width(&rt, 1), width(&rt, 2)], [300.0, 500.0]);
        click_edge(&mut rt, 4);
        settle(&mut rt, 40);
        assert_eq!(rt.state.c2.intent, "Width 500");
        assert_eq!([width(&rt, 0), width(&rt, 1), width(&rt, 2)], [500.0; 3]);
    }

    /// Fits column `serial` by two presses on its edge `gap` seconds apart.
    fn two_presses(rt: &mut Runtime<Gallery>, serial: u64, gap: f64) {
        click_edge(rt, serial);
        settle(rt, ((gap / DT).round() as usize).saturating_sub(2));
        click_edge(rt, serial);
    }

    #[test]
    fn two_presses_on_an_edge_within_400_ms_fit_that_column_alone() {
        // browser/view-modes/columns: Double-clicking a column's right edge eases that column
        // alone to its widest content over 220 ms, never below 300 px.
        let mut rt = scene("c2-three-columns-light");
        two_presses(&mut rt, 2, 0.3);
        let mid = width(&rt, 1);
        settle(&mut rt, 40);
        let fit = width(&rt, 1);
        // Documents holds "Quarterly report for the infrastructure committee, …" (90 characters).
        assert!(fit > 600.0, "fits the long name: {fit}");
        assert!(mid > 300.0 && mid < fit, "eases there: {mid} of {fit}");
        assert_eq!([width(&rt, 0), width(&rt, 2)], [300.0, 300.0], "alone");
        assert_eq!(rt.state.c2.intent, format!("Width {fit:.0}"));
        // Reports' names are short: never below one standard column.
        two_presses(&mut rt, 3, 0.2);
        settle(&mut rt, 40);
        assert_eq!(width(&rt, 2), 300.0);
        // Presses half a second apart fit nothing (each is a click, which spreads).
        let mut rt = scene("c2-three-columns-light");
        two_presses(&mut rt, 2, 0.5);
        settle(&mut rt, 40);
        assert_eq!(width(&rt, 1), 300.0);
    }

    #[test]
    fn an_autofit_stops_at_the_area_and_hands_over_its_capped_width() {
        // browser/view-modes/columns: An autofit saves its capped width. And: Autofit and edge
        // drags stop at the visible browser area (the autofit half).
        let mut rt = scene("c2-three-columns-light");
        set_cap(&mut rt, 500.0);
        two_presses(&mut rt, 2, 0.2);
        settle(&mut rt, 40);
        assert_eq!(width(&rt, 1), 500.0);
        assert_eq!(rt.state.c2.intent, "Width 500");
    }

    #[test]
    fn a_text_size_change_narrows_wider_columns_and_the_earlier_size_restores_them() {
        // browser/view-modes/columns: After a text-size change, open columns wider than the
        // area are narrowed; returning to the earlier text size restores the dragged width.
        let mut rt = scene("c2-three-columns-light");
        set_cap(&mut rt, 700.0);
        drag_edge(&mut rt, 1, 300.0);
        release(&mut rt);
        settle(&mut rt, 40);
        assert_eq!(width(&rt, 1), 600.0);
        // 20 px text draws 600 base points at 923: past the area, so narrowed to it once the
        // area comes again (the host publishes it on each layout).
        text_size(&mut rt, 20);
        rt.frame(DT);
        set_cap(&mut rt, 700.0);
        settle(&mut rt, 3);
        assert_eq!([width(&rt, 0), width(&rt, 1), width(&rt, 2)], [700.0; 3]);
        assert_eq!(
            rt.state.c2.columns.column(1).expect("open").width(),
            600.0,
            "the dragged width stays"
        );
        // Back to 13 px: the dragged width draws again.
        text_size(&mut rt, 13);
        settle(&mut rt, 40);
        assert_eq!([width(&rt, 0), width(&rt, 1), width(&rt, 2)], [600.0; 3]);
        // An area that doesn't move (no new one set) still narrows them, two frames on.
        text_size(&mut rt, 20);
        settle(&mut rt, 4);
        assert_eq!(width(&rt, 1), 700.0);
    }

    #[test]
    fn a_window_shrink_narrows_no_open_column_and_an_edge_drag_snaps_it_to_the_area() {
        // browser/view-modes/columns: Shrinking the window does not narrow open columns;
        // dragging the edge of such a column snaps it to the area.
        let mut rt = scene("c2-three-columns-light");
        set_cap(&mut rt, 1_000.0);
        drag_edge(&mut rt, 1, 300.0);
        release(&mut rt);
        settle(&mut rt, 40);
        assert_eq!(width(&rt, 1), 600.0);
        set_cap(&mut rt, 450.0);
        settle(&mut rt, 10);
        assert_eq!([width(&rt, 0), width(&rt, 1), width(&rt, 2)], [600.0; 3]);
        drag_edge(&mut rt, 1, 10.0);
        assert_eq!(width(&rt, 0), 450.0, "snaps to the area");
        release(&mut rt);
    }

    #[test]
    fn a_press_on_a_narrowed_columns_edge_starts_from_the_width_drawn() {
        // browser/view-modes/columns: Dragging a column's right edge resizes only that column
        // while the pointer moves (from the width drawn: a column narrowed to the area, which
        // grew since, follows the pointer from there, not from its saved width).
        let mut rt = scene("c2-home-light");
        rt.update(|g, _| {
            g.c2.columns.set_default_width(1_729.0);
            g.c2.columns.set_width_cap(Some(500.0));
        });
        tap_name(&mut rt, &row("Home", "Music"));
        settle(&mut rt, 40);
        assert_eq!(width(&rt, 1), 500.0, "opened narrowed to the area");
        set_cap(&mut rt, 640.0);
        settle(&mut rt, 2);
        assert_eq!(width(&rt, 1), 500.0, "a wider area doesn't widen it");
        drag_edge(&mut rt, 2, 10.0);
        assert_eq!(width(&rt, 1), 510.0, "from where it is drawn");
        release(&mut rt);
    }

    // The resize line.

    #[test]
    fn the_resize_line_turns_the_accent_under_the_pointer_and_while_it_resizes() {
        // C2 review: Strata's `.column-resize-handle.resize-hover` turns the line the accent
        // while the pointer is within its reach, at once.
        let mut rt = scene("c2-three-columns-light");
        let accent = keys::edge_accent("c2-col-1");
        let lit = |rt: &Runtime<Gallery>| rt.presented(&accent).map(|p| p.1);
        assert_eq!(lit(&rt), Some(0.0));
        let (x, y) = at(&rt, &keys::edge("c2-col-1"));
        rt.pointer_move(x, y);
        rt.frame(DT);
        assert_eq!(lit(&rt), Some(1.0), "at once");
        rt.pointer_down(x, y);
        rt.frame(DT);
        // A drag that the cap stops leaves the pointer far past the line: still lit.
        rt.update(|g, _| g.c2.columns.set_width_cap(Some(320.0)));
        for k in 1..=4 {
            rt.pointer_move(x + 50.0 * k as f32, y);
            rt.frame(DT);
        }
        assert_eq!(lit(&rt), Some(1.0), "while it resizes");
        rt.pointer_up(x + 200.0, y);
        settle(&mut rt, 2);
        assert_eq!(lit(&rt), Some(0.0), "released away from it");
    }

    #[test]
    fn the_edge_beside_the_preview_lights_the_divider_line_at_once() {
        // browser/view-modes/columns: Hovering the edge of the column that meets the preview
        // also lights the preview divider line (at once, as Strata's motion controller does;
        // only the caption waits 400 ms).
        let mut rt = scene("c2-preview-slot-light");
        rt.update(|g, _| {
            if let Some(d) = g.c2.drawer.as_mut() {
                d.open_at_once();
            }
        });
        settle(&mut rt, 10);
        rt.update(|g, tx| {
            let look = g.look.clone();
            g.c2.columns.set_active(2, &look, tx);
        });
        settle(&mut rt, 40);
        let (x, y) = at(&rt, &keys::edge("c2-col-3"));
        rt.pointer_move(x - 3.0, y);
        settle(&mut rt, 3);
        assert!(rt.state.c2.columns.last_edge_lit(), "lit in 50 ms");
        assert!(
            node(&rt, &keys::caption("c2-col-3")).is_none(),
            "the caption still waits"
        );
        settle(&mut rt, 30);
        assert!(node(&rt, &keys::caption("c2-col-3")).is_some());
    }

    // A cursor the view places is revealed (C2 review 3, mutant X13).

    /// Home / Big / the folder at Big's index 50,000, the last column active, Big with no
    /// cursor and scrolled to its top: the open folder's row is far out of view.
    fn big_open_far_down(rt: &mut Runtime<Gallery>, big_loading: bool) -> String {
        let child = strata_components::contract::EntryId::new(13_000_000 + 50_000 + 1);
        rt.update(|g, tx| {
            let look = g.look.clone();
            let d = &mut g.c2;
            let mut big = Listed {
                name: "Big".into(),
                source: scenes::c2::listing_of("Big"),
                version: 1,
                due: None,
                from: Some(id_in("Home", "Big")),
            };
            if big_loading {
                big.source = ListSource::new(Vec::new());
                big.due = Some(d.clock + 0.5);
            }
            d.listings.0.push(big);
            let listing = if big_loading {
                Listing::Loading
            } else {
                Listing::Complete
            };
            d.columns.push(
                Opening::new(&d.listings.0[1].source, listing),
                Focus::No,
                &look,
                tx,
            );
            d.listings.0.push(Listed {
                name: "entry-050001".into(),
                source: scenes::c2::listing_of("entry-050001"),
                version: 1,
                due: None,
                from: Some(child),
            });
            d.columns.push(
                Opening::new(&d.listings.0[2].source, Listing::Complete),
                Focus::Column,
                &look,
                tx,
            );
            let c = &mut d.columns.column_mut(1).expect("Big").collection;
            c.selection.restore(&Position::default());
            c.note_selection(&d.listings.0[1].source);
        });
        settle(rt, 40);
        // The keys go to the strip: a press on the last column's title focuses it.
        let last = rt.state.c2.columns.column_key(2).expect("open");
        tap(rt, &keys::title(&last));
        // A cursor in it, so Left reaches its edge.
        key(rt, Key::Down);
        format!("row-{}", child.get())
    }

    fn shown_in_listing(rt: &Runtime<Gallery>, depth: usize, row: &str) -> bool {
        let Some(r) = info(rt, row) else {
            return false;
        };
        let v = rt
            .state
            .c2
            .columns
            .column(depth)
            .expect("open")
            .collection
            .viewport();
        r.rect[1] >= v.y - 0.5 && r.rect[1] + r.rect[3] <= v.y + v.h + 0.5
    }

    #[test]
    fn left_into_a_column_with_no_cursor_reveals_the_open_folders_row() {
        // browser/view-modes/columns: When Back leaves ancestor columns with no cursor, Left
        // into an ancestor column puts the cursor on the folder that column has open (and
        // scrolls its row into view, however far down).
        let mut rt = scene("c2-home-light");
        let open = big_open_far_down(&mut rt, false);
        assert!(
            !shown_in_listing(&rt, 1, &open),
            "out of view to start with"
        );
        assert_eq!(rt.state.c2.columns.active(), 2);
        key(&mut rt, Key::Left);
        settle(&mut rt, 40);
        assert_eq!(rt.state.c2.columns.active(), 1);
        assert!(shown_in_listing(&rt, 1, &open), "revealed");
    }

    #[test]
    fn a_late_listing_reveals_the_cursor_it_places() {
        // browser/view-modes/columns: If that column is still loading, the cursor lands when it
        // finishes (and its row is scrolled into view).
        let mut rt = scene("c2-home-light");
        let open = big_open_far_down(&mut rt, true);
        key(&mut rt, Key::Left);
        assert_eq!(rt.state.c2.columns.active(), 1);
        settle(&mut rt, 60);
        let source = &rt.state.c2.listings.0[1].source;
        assert_eq!(source.len(), 100_000, "listed");
        assert!(shown_in_listing(&rt, 1, &open), "revealed");
    }
}

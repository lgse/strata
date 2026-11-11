//! C3's page in process: what the accessibility tree doesn't show. Tooltips (a pointer resting
//! for the page's 900 ms) and hover aren't in the tree; held thumbnails across a zoom need the
//! store; the paths X2 finds tiles by need the element tree. Each test names the bullets it checks; the scripts in `tests/scripts/c3/`
//! and `strata-components`' `icons_grid` tests check the rest.

#[expect(
    dead_code,
    reason = "c3 uses only the gallery command and the scripts folder"
)]
#[path = "goldens/support.rs"]
mod support;

mod c3 {
    use rune::{ElementInfo, Env, Key, KeyEvent, Mods, PhysicalInput, Runtime};
    use strata_components::contract::{EntryId, EntrySource};
    use strata_components::thumbnails::State;
    use strata_gallery::scenes::c3::{HELP_DELAY, grid};
    use strata_gallery::{Gallery, scenes};

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
        run(&mut rt, 1.0);
        rt
    }

    fn run(rt: &mut Runtime<Gallery>, seconds: f64) {
        for _ in 0..(seconds / FRAME).round() as usize {
            rt.frame(FRAME);
        }
    }

    /// The element keyed as `path`'s last part, under an element keyed as each part before it.
    fn element(rt: &Runtime<Gallery>, path: &str) -> Option<ElementInfo> {
        let parts: Vec<&str> = path.split('/').collect();
        let (last, above) = parts.split_last()?;
        rt.elements().into_iter().find(|e| {
            !e.exiting
                && e.key.as_deref() == Some(*last)
                && above.iter().all(|k| e.path.split('/').any(|seg| seg == *k))
        })
    }

    fn center(rt: &Runtime<Gallery>, path: &str) -> (f32, f32) {
        let [x, y, w, h] = element(rt, path)
            .unwrap_or_else(|| panic!("no element {path}"))
            .visible;
        (x + w / 2.0, y + h / 2.0)
    }

    /// Presses and releases at the middle of `path`, with `mods` held.
    fn click(rt: &mut Runtime<Gallery>, path: &str, mods: Mods) {
        let (x, y) = center(rt, path);
        rt.physical_input(PhysicalInput::Modifiers(mods));
        rt.pointer_down(x, y);
        rt.frame(FRAME);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
        rt.physical_input(PhysicalInput::Modifiers(Mods::default()));
        run(rt, 0.6);
    }

    fn key(rt: &mut Runtime<Gallery>, key: Key, mods: Mods) {
        rt.key(KeyEvent { key, mods });
        run(rt, 0.1);
    }

    /// A point on the blank card space between tile `id`'s frame and its name: 2 pt under
    /// the frame's drawn bottom, centred under it (the caption gap is 4 pt).
    fn under_frame(rt: &Runtime<Gallery>, id: u64) -> (f32, f32) {
        let [x, y, w, h] = element(rt, &format!("row-{id}/icon"))
            .unwrap_or_else(|| panic!("no tile {id}"))
            .visible;
        (x + w / 2.0, y + h + 2.0)
    }

    /// Presses at `from`, moves to `to` over six frames, releases there and settles.
    fn drag(rt: &mut Runtime<Gallery>, (x0, y0): (f32, f32), (x1, y1): (f32, f32)) {
        rt.pointer_down(x0, y0);
        rt.frame(FRAME);
        for i in 1..=6 {
            let f = i as f32 / 6.0;
            rt.pointer_move(x0 + (x1 - x0) * f, y0 + (y1 - y0) * f);
            rt.frame(FRAME);
        }
        rt.pointer_up(x1, y1);
        run(rt, 0.3);
    }

    /// Moves the pointer onto `path` and rests it there past the page's tooltip delay.
    fn rest_on(rt: &mut Runtime<Gallery>, path: &str) -> Option<String> {
        let (x, y) = center(rt, path);
        rest_at(rt, x, y)
    }

    /// Moves the pointer to `x`, `y` and rests it there past the page's tooltip delay.
    fn rest_at(rt: &mut Runtime<Gallery>, x: f32, y: f32) -> Option<String> {
        rt.pointer_move(x, y);
        run(rt, HELP_DELAY + 0.2);
        rt.help_tooltip().map(|(t, _)| t.to_owned())
    }

    fn selected(rt: &Runtime<Gallery>) -> Vec<u64> {
        let d = &rt.state.c3;
        d.coll
            .selection
            .selected_ids(&d.listing)
            .into_iter()
            .map(EntryId::get)
            .collect()
    }

    const LONG: &str = "2026-10-07-release-candidate-build-final-v3-with-a-very-long-tail.tar.gz";

    /// icons: "Resting the pointer about a second (900 ms) on a card whose name is
    /// middle-ellipsized shows the full name in a tooltip", "A name that fits in its two caption
    /// lines shows no tooltip", "The tooltip appears only over the card's thumbnail or caption,
    /// not its padding" and "Over the caption, only the name's text box and the details text
    /// count".
    #[test]
    fn a_cut_name_rests_into_a_tooltip_over_its_frame_name_and_details_only() {
        let mut rt = mount("c3-default-light");
        // Entry 11 is the long name.
        assert_eq!(rest_on(&mut rt, "row-11/label").as_deref(), Some(LONG));
        rt.pointer_move(1.0, 1.0);
        run(&mut rt, 0.2);
        assert_eq!(rest_on(&mut rt, "row-11/icon").as_deref(), Some(LONG));
        rt.pointer_move(1.0, 1.0);
        run(&mut rt, 0.2);
        assert_eq!(rest_on(&mut rt, "row-11/details").as_deref(), Some(LONG));
        rt.pointer_move(1.0, 1.0);
        run(&mut rt, 0.2);
        let (x, y) = under_frame(&rt, 11);
        assert_eq!(rest_at(&mut rt, x, y), None, "the card's padding");
        rt.pointer_move(1.0, 1.0);
        run(&mut rt, 0.2);
        assert_eq!(rest_on(&mut rt, "row-7/label"), None, "beach.jpg fits");
    }

    /// icons: "Resting the pointer about a second (900 ms)…": the page's delay, not Rune's
    /// 0.5 s, holds it back.
    #[test]
    fn the_tooltip_waits_for_the_pages_delay() {
        let mut rt = mount("c3-default-light");
        let (x, y) = center(&rt, "row-11/label");
        rt.pointer_move(x, y);
        run(&mut rt, 0.7);
        assert_eq!(rt.help_tooltip(), None, "not yet at 0.7 s");
        run(&mut rt, 0.4);
        assert!(rt.help_tooltip().is_some(), "shown past 0.9 s");
    }

    /// icons: "After a rename commits, the tooltip shows the new name; while the rename field
    /// is open, none appears."
    #[test]
    fn a_tile_being_renamed_has_no_tooltip_and_a_renamed_one_shows_its_new_name() {
        let mut rt = mount("c3-default-light");
        rt.update(|g, _| g.c3.renaming = Some(EntryId::new(11)));
        run(&mut rt, 0.1);
        assert_eq!(rest_on(&mut rt, "row-11/icon"), None);
        rt.pointer_move(1.0, 1.0);
        let renamed = "renamed-candidate-build-with-another-quite-long-tail-for-the-cut.zip";
        rt.update(|g, tx| {
            g.c3.renaming = None;
            let mut entries = g.c3.listing.entries().to_vec();
            entries[10].name = renamed.to_owned();
            g.c3.listing.replace(entries);
            g.c3.refresh(tx);
        });
        run(&mut rt, 0.2);
        assert_eq!(rest_on(&mut rt, "row-11/label").as_deref(), Some(renamed));
    }

    /// icons: "Keyboard focus on a card shows no name tooltip."
    #[test]
    fn a_key_moving_onto_a_cut_tile_shows_no_tooltip() {
        let mut rt = mount("c3-default-light");
        click(&mut rt, "row-10/icon", Mods::default());
        rt.pointer_move(1.0, 1.0);
        key(&mut rt, Key::Right, Mods::default());
        run(&mut rt, HELP_DELAY + 0.3);
        assert_eq!(selected(&rt), vec![11]);
        assert_eq!(rt.help_tooltip(), None);
    }

    /// icons: "Hover feedback appears only over a tile's thumbnail and caption, not the gutters
    /// between tiles."
    #[test]
    fn hover_lights_the_frame_or_the_name_and_not_the_cards_blank_space() {
        let mut rt = mount("c3-default-light");
        let hover = |rt: &Runtime<Gallery>, path: &str| element(rt, path).map_or(0.0, |e| e.hover);
        let (x, y) = under_frame(&rt, 13);
        rt.pointer_move(x, y);
        run(&mut rt, 0.3);
        assert_eq!(hover(&rt, "row-13/icon"), 0.0);
        assert_eq!(hover(&rt, "row-13/label"), 0.0);
        let (x, y) = center(&rt, "row-13/icon");
        rt.pointer_move(x, y);
        run(&mut rt, 0.3);
        assert!(hover(&rt, "row-13/icon") > 0.0);
        let (x, y) = center(&rt, "row-13/label");
        rt.pointer_move(x, y);
        run(&mut rt, 0.3);
        assert!(hover(&rt, "row-13/label") > 0.0);
        assert_eq!(hover(&rt, "row-13/icon"), 0.0);
    }

    /// icons: "Moving the slider scales existing thumbnails in place without flashing generic
    /// icons." A size step keeps every landed image held, its icon hidden, until the new
    /// step's image replaces it.
    #[test]
    fn a_size_step_keeps_landed_thumbnails_drawn_while_the_new_step_loads() {
        let mut rt = mount("c3-default-light");
        let shown: Vec<_> = {
            let d = &rt.state.c3;
            d.coll
                .visible_range()
                .filter_map(|i| d.listing.entry(i).thumbnail)
                .collect()
        };
        assert!(!shown.is_empty());
        let held = |rt: &Runtime<Gallery>| {
            shown.iter().all(|&k| {
                let d = &rt.state.c3;
                matches!(d.store.state(k), State::Ready { .. })
                    && d.store.thumb(k).icon_alpha().get() == 0.0
            })
        };
        assert!(held(&rt), "all landed before the step");
        let before = rt.state.c3.grid.synced_step();
        rt.update(|g, tx| {
            grid().apply_size(g, strata_components::icons_grid::SizeCommand::Step(9), tx);
        });
        for _ in 0..20 {
            rt.frame(FRAME);
            assert!(held(&rt), "an icon never shows in a landed tile's place");
        }
        assert_ne!(
            rt.state.c3.grid.synced_step(),
            before,
            "the store was asked for the new step"
        );
    }

    /// view-modes (folder-views, the host's part): a size the host changes itself (another
    /// folder's, a slider) lays out at once, and the cursor tile stays in view.
    #[test]
    fn a_size_the_host_sets_keeps_the_cursor_tile_in_view() {
        let mut rt = mount("c3-default-light");
        click(&mut rt, "row-120/icon", Mods::default());
        rt.update(|g, tx| {
            g.c3.size = 160.0;
            grid().size_changed(g, tx);
        });
        run(&mut rt, 0.5);
        let cursor = element(&rt, "row-120").expect("still mounted");
        let page = element(&rt, "c3-grid").expect("the grid");
        let [_, y, _, h] = cursor.visible;
        assert!(h > 0.0 && y >= page.visible[1], "the cursor tile shows");
    }

    /// X2's drop zones: a folder tile's path follows the grid's count a row.
    #[test]
    fn a_tiles_path_finds_it_at_any_width() {
        let mut rt = mount("c3-default-light");
        // A key path names keyed elements only, as Rune resolves one (`#n` is unkeyed).
        let finds = |rt: &Runtime<Gallery>, path: &str| {
            rt.elements().iter().any(|e| {
                let keyed: Vec<&str> = e.path.split('/').filter(|s| !s.starts_with('#')).collect();
                keyed.join("/").ends_with(path) && e.key.as_deref() == Some("row-13")
            })
        };
        let path = grid()
            .tile_path(&rt.state, EntryId::new(13))
            .expect("listed");
        assert!(finds(&rt, &path), "{path}");
        rt.update(|g, _| g.c3.pane = Some(400.0));
        run(&mut rt, 0.5);
        let narrow = grid()
            .tile_path(&rt.state, EntryId::new(13))
            .expect("listed");
        assert_ne!(narrow, path, "a reflow moves it to another line");
        assert!(finds(&rt, &narrow), "{narrow}");
    }

    /// pointer-intent: "In Icons, the gutter beside the thumbnail inside a card is blank space
    /// for marquee." A drag from the blank card space under an unselected tile's frame draws a
    /// band (two columns by two rows here), not a file drag.
    #[test]
    fn a_drag_from_a_cards_blank_space_draws_a_band() {
        let mut rt = mount("c3-default-light");
        let (from, to) = (under_frame(&rt, 1), under_frame(&rt, 10));
        drag(&mut rt, from, to);
        assert_eq!(selected(&rt), vec![1, 2, 9, 10]);
        assert!(
            !rt.state.c3.intent.starts_with("Dragging"),
            "{}",
            rt.state.c3.intent
        );
    }

    /// The grid draws each tile where its hit geometry says, a short last row included (its
    /// cells share the row with empty filler cells): a press lands on the tile drawn there.
    #[test]
    fn every_tile_is_drawn_centred_in_its_geometry_rect() {
        let rt = mount("c3-short-rows-light");
        let collection = grid().collection(&rt.state);
        for (index, id) in (200..210).enumerate() {
            let rect = collection.window_rect_of(&rt.state, index);
            let [x, _, w, _] = element(&rt, &format!("row-{id}/icon"))
                .unwrap_or_else(|| panic!("tile {id}"))
                .rect;
            let drawn = x + w / 2.0;
            let laid = rect.x + rect.w / 2.0;
            assert!(
                (drawn - laid).abs() < 0.5,
                "tile {id}: drawn at {drawn}, laid out at {laid}"
            );
        }
    }

    /// thumbnail-size: "The size steps through 32, 36, 40, 44, 48, 56, 64, 72, 80, 96, 112,
    /// 128, 144, 160, 192, 224, and 256 px." From 32, sixteen steps up land on each size in
    /// turn, a seventeenth stays at 256, and sixteen down walk back the same sizes.
    #[test]
    fn a_step_at_a_time_walks_strata_s_whole_ladder_both_ways() {
        const LADDER: [f32; 17] = [
            32.0, 36.0, 40.0, 44.0, 48.0, 56.0, 64.0, 72.0, 80.0, 96.0, 112.0, 128.0, 144.0, 160.0,
            192.0, 224.0, 256.0,
        ];
        let mut rt = mount("c3-size-32-light");
        let mut sizes = vec![rt.state.c3.size];
        for dir in [1, -1] {
            for _ in 0..17 {
                rt.update(|g, tx| {
                    grid().apply_size(g, strata_components::icons_grid::SizeCommand::Step(dir), tx);
                });
                run(&mut rt, 0.05);
                sizes.push(rt.state.c3.size);
            }
        }
        let up: Vec<f32> = LADDER.iter().copied().chain([256.0]).collect();
        let down: Vec<f32> = LADDER.iter().rev().copied().skip(1).chain([32.0]).collect();
        assert_eq!(&sizes[..18], &up[..], "up");
        assert_eq!(&sizes[18..], &down[..], "down");
    }

    /// A size step that keeps the count a row (one tile a row here) still moves every row: a
    /// cursor tile that showed stays in view, as a reflow keeps it.
    #[test]
    fn a_size_step_that_keeps_the_count_keeps_the_cursor_tile_in_view() {
        let mut rt = mount("c3-narrow-light");
        assert_eq!(rt.state.c3.coll.per_line(), 1);
        click(&mut rt, "row-203/icon", Mods::default());
        assert!(rt.state.c3.coll.visible_range().contains(&3));
        rt.update(|g, tx| {
            grid().apply_size(g, strata_components::icons_grid::SizeCommand::Step(6), tx);
        });
        run(&mut rt, 0.5);
        assert_eq!(rt.state.c3.coll.per_line(), 1, "the count stayed");
        assert_eq!(rt.state.c3.size, 144.0);
        assert!(
            rt.state.c3.coll.visible_range().contains(&3),
            "the cursor tile shows: {:?}",
            rt.state.c3.coll.visible_range()
        );
    }

    /// Turning thumbnails off draws every tile with its icon alone, and on again with its
    /// image: the tile builds again on each (C25's `version` changes with the switch).
    #[test]
    fn thumbnails_off_and_on_rebuild_the_tiles() {
        let mut rt = mount("c3-default-light");
        let thumb = |rt: &Runtime<Gallery>| element(rt, "row-7/icon/thumb").is_some();
        assert!(thumb(&rt), "an image part while on");
        rt.update(|g, _| {
            g.c3.store.set_enabled(false);
        });
        run(&mut rt, 0.2);
        assert!(!thumb(&rt), "the icon alone while off");
        rt.update(|g, _| {
            g.c3.store.set_enabled(true);
        });
        run(&mut rt, 0.2);
        assert!(thumb(&rt), "the image part again");
    }

    /// A cut tile and a dragged tile draw faded (`collection.cut.opacity`,
    /// `collection.drag.opacity`), the whole tile, its frame and name with it; an unmarked tile
    /// beside them doesn't.
    #[test]
    fn cut_and_dragged_tiles_draw_faded() {
        let rt = mount("c3-marks-light");
        let look = rt.state.look();
        let opacity = |path: &str| {
            element(&rt, path)
                .unwrap_or_else(|| panic!("{path}"))
                .opacity
        };
        let cut = strata_kit::slots::collection::cut_opacity(look);
        let drag = strata_kit::slots::collection::drag_opacity(look);
        assert!(cut < 1.0 && drag < 1.0);
        for path in ["row-3", "row-3/icon", "row-3/label"] {
            assert!(
                (opacity(path) - cut).abs() < 0.01,
                "{path}: {}",
                opacity(path)
            );
        }
        for path in ["row-4", "row-4/icon", "row-4/label"] {
            assert!(
                (opacity(path) - drag).abs() < 0.01,
                "{path}: {}",
                opacity(path)
            );
        }
        for path in ["row-2", "row-5", "row-6/label"] {
            assert_eq!(opacity(path), 1.0, "{path}");
        }
    }

    /// pointer-intent: "On an unselected entry, a drag from blank row space beside the filename
    /// starts a marquee", and "a drag from the … rendered filename starts a file drag": 2 pt
    /// left of `beach.jpg`'s drawn name box a drag draws a band; 2 pt inside it, it drags the
    /// file. The name's hit box is its drawn box, neither the cell's width nor narrower.
    #[test]
    fn beside_a_short_name_is_blank_space_and_its_drawn_box_is_the_name() {
        for (dx, band) in [(-2.0f32, true), (2.0, false)] {
            let mut rt = mount("c3-default-light");
            let [x, y, _, h] = element(&rt, "row-7/label").expect("beach.jpg's name").rect;
            let [tx, ty, ..] = element(&rt, "row-15/icon").expect("a tile below").visible;
            drag(&mut rt, (x + dx, y + h / 2.0), (tx + 5.0, ty + 5.0));
            let intent = rt.state.c3.intent.clone();
            if band {
                assert!(!intent.starts_with("Dragging"), "beside: {intent}");
                assert!(
                    selected(&rt).len() > 1,
                    "beside: a band, {:?}",
                    selected(&rt)
                );
            } else {
                assert_eq!(intent, "Dragging 1 items", "inside");
                assert_eq!(selected(&rt), vec![7], "inside");
            }
        }
    }

    /// icons: "Over the caption, only the name's text box and the details text count": a rest
    /// 2 pt beside a cut name's drawn box, or beside the details text, shows no tooltip; 2 pt
    /// inside either, it does.
    #[test]
    fn a_rest_beside_a_cut_names_box_or_its_details_shows_no_tooltip() {
        let mut rt = mount("c3-default-light");
        let card = element(&rt, "row-11").expect("the long name's tile").rect;
        for path in ["row-11/label", "row-11/details"] {
            let [x, y, _, h] = element(&rt, path).unwrap_or_else(|| panic!("{path}")).rect;
            assert!(x - 2.0 > card[0], "{path} leaves card space beside it");
            for (dx, tip) in [(-2.0f32, None), (2.0, Some(LONG))] {
                rt.pointer_move(1.0, 1.0);
                run(&mut rt, 0.2);
                assert_eq!(
                    rest_at(&mut rt, x + dx, y + h / 2.0).as_deref(),
                    tip,
                    "{path} {dx}"
                );
            }
        }
    }

    /// browser/scrolling: "Scrollbars fade in on hover or scrolling and stay hidden at rest; the
    /// trough strengthens on hover and while dragging." On the grid (C24b): its bar is hidden at
    /// rest, shows as the grid scrolls, and its thumb drags the grid.
    #[test]
    fn the_grids_bar_shows_as_it_scrolls_and_its_thumb_drags_it() {
        let mut rt = mount("c3-default-light");
        run(&mut rt, 1.5);
        let bar = |rt: &Runtime<Gallery>| {
            rt.elements()
                .into_iter()
                .find(|e| !e.exiting && e.key.as_deref() == Some("c3-grid-bar"))
                .expect("the grid's bar")
        };
        assert_eq!(bar(&rt).opacity, 0.0, "hidden at rest");
        let v = rt.state.c3.coll.viewport();
        rt.pointer_move(v.x + v.w / 2.0, v.y + v.h / 2.0);
        rt.frame(FRAME);
        rt.wheel(v.x + v.w / 2.0, v.y + v.h / 2.0, -100.0);
        run(&mut rt, 0.1);
        let scrolled = rt.state.c3.grid.feed().get().offset[1];
        assert!(scrolled > 0.0);
        assert!(bar(&rt).opacity > 0.0, "shown as it scrolls");
        let thumb = rt
            .elements()
            .into_iter()
            .find(|e| !e.exiting && e.path.ends_with("c3-grid-bar/trough/thumb"))
            .expect("the thumb")
            .visible;
        let (tx, ty) = (thumb[0] + thumb[2] / 2.0, thumb[1] + thumb[3] / 2.0);
        rt.pointer_move(tx, ty);
        rt.frame(FRAME);
        rt.pointer_down(tx, ty);
        rt.frame(FRAME);
        // Pictures barely overflows: the wheel took it to its end, so drag back up.
        rt.pointer_move(tx, ty - 60.0);
        rt.frame(FRAME);
        rt.pointer_up(tx, ty - 60.0);
        run(&mut rt, 0.1);
        assert!(
            rt.state.c3.grid.feed().get().offset[1] < scrolled,
            "the drag scrolled back"
        );
        assert!(
            rt.state.c3.coll.selection.is_empty(),
            "and selected nothing"
        );
    }
}

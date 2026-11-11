//! C11b's page: what the sheet grid does in process, beside its goldens and scripts, where a
//! script can't look: a drag held while the preview closes, and a sheet arriving from the
//! page's fake reader. Each test that checks a feature-map Behavior bullet names it.

mod c11b {
    use rune::{Env, Key, KeyEvent, Mods, Runtime};
    use strata_components::preview_drawer::HostedPreview;
    use strata_components::sheet_grid::{CellPos, CellRange};
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
        settle(&mut rt, 30);
        rt
    }

    fn settle(rt: &mut Runtime<Gallery>, frames: usize) {
        for _ in 0..frames {
            rt.frame(DT);
        }
    }

    fn mid(rt: &Runtime<Gallery>, end: &str) -> (f32, f32) {
        let r = rt
            .elements()
            .into_iter()
            .find(|e| e.path.ends_with(end))
            .unwrap_or_else(|| panic!("no element ending {end}"))
            .rect;
        (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
    }

    fn tap(rt: &mut Runtime<Gallery>, at: (f32, f32)) {
        rt.pointer_move(at.0, at.1);
        rt.pointer_down(at.0, at.1);
        rt.frame(DT);
        rt.pointer_up(at.0, at.1);
        settle(rt, 5);
    }

    /// preview/preview-panel/documents: "Closing the preview during a table cell drag ends the
    /// drag and its autoscroll."
    #[test]
    fn closing_the_preview_during_a_cell_drag_ends_the_drag_and_its_autoscroll() {
        let mut rt = scene("c11b-csv-light");
        let from = mid(&rt, "sheet-r0-c1");
        rt.pointer_move(from.0, from.1);
        rt.pointer_down(from.0, from.1);
        rt.frame(DT);
        // Down to the drawer's bottom edge, and held there.
        let bottom = rt
            .elements()
            .into_iter()
            .find(|e| e.path.ends_with("sheet-s0-scroll"))
            .expect("the rows' viewport")
            .rect;
        rt.pointer_move(from.0, bottom[1] + bottom[3] - 2.0);
        settle(&mut rt, 20);
        let moving = rt.state.c11b.grid.cursor().unwrap().row;
        assert!(
            moving > 10,
            "the range follows the autoscroll: row {moving}"
        );
        rt.key(KeyEvent {
            key: Key::Function(9),
            mods: Mods::default(),
        });
        settle(&mut rt, 3);
        assert_eq!(rt.state.c11b.last, "Closed");
        assert!(!rt.state.c11b.grid.dragging(), "the drag ended");
        let at = rt.state.c11b.grid.cursor();
        settle(&mut rt, 30);
        assert_eq!(
            rt.state.c11b.grid.cursor(),
            at,
            "nothing scrolls the range on"
        );
        rt.pointer_up(from.0, bottom[1] + bottom[3] - 2.0);
    }

    /// A workbook's other sheets come from Strata's reader when their tab is picked: the
    /// drawer shows it working meanwhile (no bullet: Strata previews the first sheet only, D4).
    #[test]
    fn a_picked_sheet_arrives_from_the_reader_while_the_drawer_shows_it_loading() {
        let mut rt = scene("c11b-book-light");
        let at = mid(&rt, "sheet-sheet1");
        tap(&mut rt, at);
        assert_eq!(rt.state.c11b.last, "Load 1");
        assert_eq!(rt.state.c11b.grid.busy(), Some("Loading sheet…"));
        assert!(rt.state.c11b.grid.sheet(1).is_none());
        settle(&mut rt, 30);
        let g = &rt.state.c11b.grid;
        assert_eq!(g.busy(), None);
        assert_eq!(g.sheet(1).map(|s| s.name.as_ref()), Some("Q1 sales"));
        assert!(rt.elements().iter().any(|e| e.path.ends_with("sheet-h4")));
    }

    /// A sheet the reader can't read says so in the grid's place, the drawer isn't busy, and
    /// Try again asks the reader once more (the page's reader fails Inventory's first read).
    #[test]
    fn a_sheet_the_reader_fails_says_why_and_try_again_reads_it_again() {
        let mut rt = scene("c11b-book-light");
        // Inventory's tab sits past the strip's edge: Ctrl+Page Down gets there (D8).
        let at = mid(&rt, "sheet-r0-c0");
        tap(&mut rt, at);
        for _ in 0..3 {
            rt.key(KeyEvent {
                key: Key::PageDown,
                mods: Mods {
                    cmd: true,
                    ctrl: true,
                    ..Mods::default()
                },
            });
            settle(&mut rt, 2);
        }
        assert_eq!(rt.state.c11b.grid.active(), 3);
        assert_eq!(rt.state.c11b.last, "Load 3");
        settle(&mut rt, 30);
        assert_eq!(rt.state.c11b.grid.busy(), None);
        assert!(rt.state.c11b.grid.sheet(3).is_none());
        let failed = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("sheet-failed"))
            .expect("the failure in the grid's place");
        assert!(failed.rect[2] > 0.0);
        rt.state.c11b.last.clear();
        let at = mid(&rt, "sheet-retry");
        tap(&mut rt, at);
        assert_eq!(rt.state.c11b.last, "Load 3");
        assert_eq!(rt.state.c11b.grid.busy(), Some("Loading sheet…"));
        settle(&mut rt, 30);
        assert_eq!(rt.state.c11b.grid.busy(), None);
        assert!(rt.state.c11b.grid.sheet(3).is_some());
    }

    /// preview/preview-panel/documents: "With a cell range selected, Escape, an arrow key,
    /// Home, or End also clears it", with the modifiers Strata ignores on them: Escape with
    /// any, the arrows, Home and End with Ctrl, Alt or Super. The grid takes each key that
    /// clears a range, so the page stays open; Shift with an arrow still grows it (D6).
    #[test]
    fn modified_escape_arrows_home_and_end_clear_the_range_as_stratas_do() {
        let mut rt = scene("c11b-csv-light");
        let at = mid(&rt, "sheet-r1-c1");
        tap(&mut rt, at);
        let ctrl = Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        };
        let alt = Mods {
            alt: true,
            ..Mods::default()
        };
        let logo = Mods {
            logo: true,
            ..Mods::default()
        };
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        let range = |rt: &mut Runtime<Gallery>| {
            rt.update(|g, _| g.c11b.grid.select(CellPos::new(1, 1), CellPos::new(3, 2)));
            settle(rt, 2);
        };
        let cases = [
            (Key::Escape, ctrl),
            (Key::Escape, shift),
            (
                Key::Escape,
                Mods {
                    shift: true,
                    ..ctrl
                },
            ),
            (Key::Escape, alt),
            (Key::Left, ctrl),
            (Key::Right, ctrl),
            (Key::Up, ctrl),
            (Key::Down, ctrl),
            (Key::Left, alt),
            (Key::Down, alt),
            (Key::Home, alt),
            (Key::End, alt),
            (Key::Up, logo),
            (Key::Right, logo),
            (Key::Home, logo),
            (Key::End, logo),
            (
                Key::Left,
                Mods {
                    shift: true,
                    ..ctrl
                },
            ),
        ];
        for (key, mods) in cases {
            range(&mut rt);
            assert!(rt.state.c11b.grid.range().is_some());
            rt.key(KeyEvent {
                key: key.clone(),
                mods,
            });
            settle(&mut rt, 2);
            assert_eq!(rt.state.c11b.grid.range(), None, "{key:?} {mods:?}");
            assert!(
                rt.state.page.is_some(),
                "{key:?} {mods:?} stayed the grid's"
            );
        }
        range(&mut rt);
        rt.key(KeyEvent {
            key: Key::Down,
            mods: shift,
        });
        settle(&mut rt, 2);
        assert_eq!(
            rt.state.c11b.grid.range(),
            Some(CellRange::spanning(CellPos::new(1, 1), CellPos::new(4, 2))),
            "Shift+Down grows it"
        );
    }

    fn rect(rt: &Runtime<Gallery>, end: &str) -> [f32; 4] {
        rt.elements()
            .into_iter()
            .find(|e| e.path.ends_with(end))
            .unwrap_or_else(|| panic!("no element ending {end}"))
            .rect
    }

    /// Whether the active sheet's tab lies wholly inside the strip's viewport. (Rune's reveal
    /// stops at the edge, under the edge fade: C7's draft U-C7-7.)
    fn active_tab_shows(rt: &Runtime<Gallery>) -> Result<(), String> {
        let strip = rect(rt, "sheet-tabs-scroll");
        let i = rt.state.c11b.grid.active();
        let tab = rect(rt, &format!("t-sheet-sheet{i}"));
        let inside = tab[0] >= strip[0] - 0.5 && tab[0] + tab[2] <= strip[0] + strip[2] + 0.5;
        inside.then_some(()).ok_or(format!(
            "sheet {i}'s tab {tab:?} is outside the strip {strip:?}"
        ))
    }

    /// The tabs leave revealing the active one to the app (as C7 does): each sheet switch,
    /// by keys or by the host, scrolls its tab into the strip.
    #[test]
    fn every_sheet_switch_scrolls_the_active_tab_into_the_strip() {
        let mut rt = scene("c11b-book-light");
        let at = mid(&rt, "sheet-r0-c0");
        tap(&mut rt, at);
        let ctrl = Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        };
        for key in [Key::PageDown, Key::PageDown, Key::PageDown] {
            rt.key(KeyEvent { key, mods: ctrl });
            settle(&mut rt, 30);
        }
        assert_eq!(rt.state.c11b.grid.active(), 3);
        active_tab_shows(&rt).unwrap();
        for _ in 0..3 {
            rt.key(KeyEvent {
                key: Key::PageUp,
                mods: ctrl,
            });
            settle(&mut rt, 30);
        }
        assert_eq!(rt.state.c11b.grid.active(), 0);
        active_tab_shows(&rt).unwrap();
        // A host restoring a preview on its last sheet.
        rt.update(|g, _| g.c11b.grid.set_active(3));
        settle(&mut rt, 30);
        active_tab_shows(&rt).unwrap();
    }

    /// The failed-sheet scene opens on Inventory, the last tab, past the strip's edge.
    #[test]
    fn a_preview_restored_on_a_sheet_past_the_strips_edge_shows_its_tab() {
        let rt = scene("c11b-failed-light");
        assert_eq!(rt.state.c11b.grid.active(), 3);
        active_tab_shows(&rt).unwrap();
    }

    fn heard(g: &mut Gallery, key: &KeyEvent, _tx: &mut rune::Tx) -> bool {
        g.c11b.last = format!("Heard {:?} {:?}", key.key, key.mods);
        true
    }

    /// Without a range, the keys that would clear one (D16) aren't the grid's: Ctrl+Left and
    /// Alt+Home reach the app's own listener, and the cursor stays put.
    #[test]
    fn ctrl_left_and_alt_home_without_a_range_pass_on_and_leave_the_cursor() {
        let mut rt = scene("c11b-csv-light");
        rt.on_key = Some(heard);
        let at = mid(&rt, "sheet-r1-c1");
        tap(&mut rt, at);
        let cursor = rt.state.c11b.grid.cursor();
        assert_eq!(cursor, Some(CellPos::new(1, 1)));
        let ctrl = Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        };
        let alt = Mods {
            alt: true,
            ..Mods::default()
        };
        for (key, mods) in [(Key::Left, ctrl), (Key::Home, alt)] {
            rt.state.c11b.last.clear();
            rt.key(KeyEvent {
                key: key.clone(),
                mods,
            });
            settle(&mut rt, 2);
            assert_eq!(
                rt.state.c11b.last,
                format!("Heard {key:?} {mods:?}"),
                "the app heard it"
            );
            assert_eq!(rt.state.c11b.grid.cursor(), cursor, "{key:?} {mods:?}");
        }
    }

    /// Escape is the grid's only while a range is selected: without one, the page hears it.
    #[test]
    fn escape_without_a_range_passes_on_past_the_grid() {
        let mut rt = scene("c11b-csv-light");
        let at = mid(&rt, "sheet-r1-c0");
        tap(&mut rt, at);
        assert!(rt.state.c11b.grid.cursor().is_some());
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        settle(&mut rt, 2);
        assert_eq!(
            rt.state.page, None,
            "the gallery's Escape went back to the index"
        );
    }
}

//! C20c's page in process: what the accessibility tree a script ends on doesn't show (the jump's
//! highlight and its timing in a window at rest, a reveal's scroll). The scripts in
//! `tests/scripts/c20c/` check the rest, and strata-components' own tests the matcher and the
//! form's kept and rebuilt pages. Each test names what it covers.
//!
//! Coverage notes for the scripts:
//!
//! - No feature-map bullet covers the tabs' keyboard (arrows, Home and End, Ctrl+Page Up and
//!   Down). `tabs-arrows`, `tabs-wrap`, `tabs-ctrl-page` and `tabs-ctrl-page-rebuilt` cite the
//!   bullet the tabs exist for ("New action… opens an editor with General, Script, and
//!   Behavior tabs") and check GtkNotebook's keys and its `real_switch_page` focus rule.
//! - `error-mark` checks the switch to the invalid field's tab, and the edits on the page that
//!   shows; the edit on the hidden page and "writes nothing" are checked here
//!   (`a_save_with_an_invalid_field_keeps_every_edit_and_writes_nothing`).
//! - `select-all` takes "any keyboard layout" as US and Russian: the Russian Ctrl+A is the
//!   physical A with the base "ф", as a window sends it off macOS (`Key::Command('a')`).

mod c20c {
    use rune::{ElementInfo, Env, Key, KeyEvent, Mods, Runtime, TextEdit};
    use strata_components::controls::settings_search::{IndexPage, IndexRow, SearchIndex};
    use strata_gallery::{Gallery, scenes};

    const FRAME: f64 = 1.0 / 60.0;
    const FIELD: &str = "c20c-search-field";

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

    fn run(rt: &mut Runtime<Gallery>, seconds: f64) {
        for _ in 0..(seconds / FRAME).round() as usize {
            rt.frame(FRAME);
        }
    }

    /// Steps from wake to wake, as a window at rest does, with no frames between, until no
    /// wake is due within `limit` seconds. Returns the clock it stopped at.
    fn rest(rt: &mut Runtime<Gallery>, limit: f64) -> f64 {
        let start = rt.state.c20c.now;
        for _ in 0..200 {
            let Some(deadline) = rt.wake_deadline() else {
                break;
            };
            if deadline - start > limit {
                break;
            }
            let now = rt.state.c20c.now;
            rt.frame((deadline - now).max(0.001));
        }
        rt.state.c20c.now
    }

    fn element(rt: &Runtime<Gallery>, key: &str) -> Option<ElementInfo> {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
    }

    fn focused(rt: &Runtime<Gallery>) -> Option<String> {
        rt.elements()
            .into_iter()
            .find(|e| e.focused)
            .and_then(|e| e.key)
    }

    fn flash(rt: &Runtime<Gallery>, row: &str) -> bool {
        let prefix = format!("c20c-search-row-{row}-flash-");
        rt.elements()
            .iter()
            .any(|e| !e.exiting && e.key.as_deref().is_some_and(|k| k.starts_with(&prefix)))
    }

    fn tap(rt: &mut Runtime<Gallery>, key: &str) {
        let r = element(rt, key).unwrap_or_else(|| panic!("no {key}")).rect;
        let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        run(rt, 0.1);
    }

    fn typ(rt: &mut Runtime<Gallery>, text: &str) {
        for c in text.chars() {
            rt.key(KeyEvent {
                key: Key::Text(c.to_string()),
                mods: Mods::default(),
            });
            rt.frame(FRAME);
        }
        run(rt, 0.1);
    }

    fn enter(rt: &mut Runtime<Gallery>) {
        rt.key(KeyEvent {
            key: Key::Enter,
            mods: Mods::default(),
        });
        rt.frame(FRAME);
    }

    /// The jump's highlight shows for 1.5 s, then goes, in a window at rest that polls only at
    /// the wakes it asked for: statically with animations off, and with its fade on.
    #[test]
    fn the_highlight_goes_after_a_second_and_a_half_at_rest() {
        for (scene, animations) in [
            ("c20c-highlight-light", false),
            ("c20c-highlight-animated-light", true),
        ] {
            let mut rt = mount_with(scene, |_| {});
            assert_eq!(rt.state.look.animations, animations);
            assert_eq!(
                rt.state.c20c.search.highlighted(),
                Some("thumbnail-workers"),
                "{scene}"
            );
            let shown = rt.state.c20c.now;
            assert!(
                flash(&rt, "thumbnail-workers"),
                "{scene}: the row is highlighted"
            );
            let stopped = rest(&mut rt, 5.0);
            let took = stopped - shown;
            assert_eq!(rt.state.c20c.search.highlighted(), None, "{scene}");
            assert!(
                (1.45..1.6).contains(&took),
                "{scene}: went after {took:.3} s"
            );
            assert!(
                !flash(&rt, "thumbnail-workers"),
                "{scene}: the highlight is gone"
            );
        }
    }

    /// A new query drops a highlight still showing.
    #[test]
    fn typing_again_drops_the_highlight() {
        let mut rt = mount_with("c20c-settings-light", |_| {});
        tap(&mut rt, FIELD);
        typ(&mut rt, "license");
        enter(&mut rt);
        run(&mut rt, 0.2);
        assert_eq!(rt.state.c20c.search.highlighted(), Some("license"));
        assert_eq!(
            rt.state.c20c.heard.last().map(String::as_str),
            Some("about > license")
        );
        tap(&mut rt, FIELD);
        typ(&mut rt, "s");
        assert_eq!(rt.state.c20c.search.highlighted(), None);
    }

    /// With no match Enter goes nowhere: the field keeps the keyboard and nothing navigates.
    #[test]
    fn enter_with_no_match_does_nothing() {
        let mut rt = mount_with("c20c-settings-light", |_| {});
        tap(&mut rt, FIELD);
        typ(&mut rt, "zzzz");
        let heard = rt.state.c20c.heard.clone();
        enter(&mut rt);
        run(&mut rt, 0.2);
        assert_eq!(rt.state.c20c.heard, heard);
        assert_eq!(rt.state.c20c.search.highlighted(), None);
        assert_eq!(focused(&rt).as_deref(), Some(FIELD));
    }

    /// Forty rows match and the best is the last, below the fold: Enter scrolls it into view
    /// and moves the keyboard to its switch.
    #[test]
    fn a_jump_reveals_a_row_below_the_fold() {
        let mut rt = mount_with("c20c-settings-light", |g| {
            let mut rows: Vec<IndexRow> = (0..40)
                .map(|i| {
                    IndexRow::new(format!("r{i}"), "general", format!("Row {i}")).keywords("alpha")
                })
                .collect();
            rows.push(IndexRow::new("best", "general", "Alpha"));
            g.c20c.index = SearchIndex {
                revision: 9,
                pages: vec![IndexPage::new("general", "General")],
                rows,
            };
        });
        run(&mut rt, 0.3);
        tap(&mut rt, FIELD);
        typ(&mut rt, "alpha");
        let page = element(&rt, "c20c-page").expect("the page").rect;
        let row = |rt: &Runtime<Gallery>| element(rt, "c20c-search-row-best").map(|e| e.rect);
        assert!(
            row(&rt).is_none_or(|r| r[1] >= page[1] + page[3]),
            "the best row starts below the fold"
        );
        enter(&mut rt);
        run(&mut rt, 1.0);
        let r = row(&rt).expect("the row is built");
        assert!(
            r[1] >= page[1] && r[1] + r[3] <= page[1] + page[3] + 0.5,
            "row {r:?} in page {page:?}"
        );
        assert_eq!(
            focused(&rt).as_deref(),
            Some("c20c-switch-best"),
            "the row's switch has the keyboard"
        );
    }

    /// Strata resets each page's scroll on every change of query (`reset_scroll`): with forty
    /// rows still matching, the page goes back to its top.
    #[test]
    fn a_new_query_scrolls_the_page_to_its_top() {
        let mut rt = mount_with("c20c-settings-light", |g| {
            g.c20c.index = SearchIndex {
                revision: 9,
                pages: vec![IndexPage::new("general", "General")],
                rows: (0..40)
                    .map(|i| {
                        IndexRow::new(format!("r{i}"), "general", format!("Row {i}"))
                            .keywords("alpha")
                    })
                    .collect(),
            };
        });
        run(&mut rt, 0.3);
        let page = element(&rt, "c20c-page").expect("the page").rect;
        rt.pointer_move(page[0] + 50.0, page[1] + 50.0);
        rt.frame(FRAME);
        rt.wheel(page[0] + 50.0, page[1] + 50.0, -400.0);
        run(&mut rt, 1.0);
        let offset = |rt: &Runtime<Gallery>| {
            element(rt, "c20c-search-row-r0").map_or(f32::NAN, |e| e.scrolled[1])
        };
        assert!(offset(&rt) > 0.0, "the wheel scrolled the page");
        tap(&mut rt, FIELD);
        typ(&mut rt, "a");
        assert_eq!(
            rt.state.c20c.search.count(),
            Some(40),
            "every row still shows"
        );
        assert_eq!(offset(&rt), 0.0, "the query scrolled the page back");
    }

    /// Only a plain Enter jumps: with Ctrl, Shift or Alt it goes on.
    #[test]
    fn a_modified_enter_does_not_jump() {
        let mut rt = mount_with("c20c-settings-light", |_| {});
        tap(&mut rt, FIELD);
        typ(&mut rt, "license");
        for mods in [
            Mods {
                ctrl: true,
                cmd: true,
                ..Mods::default()
            },
            Mods {
                shift: true,
                ..Mods::default()
            },
            Mods {
                alt: true,
                ..Mods::default()
            },
        ] {
            rt.key(KeyEvent {
                key: Key::Enter,
                mods,
            });
            run(&mut rt, 0.1);
        }
        assert_eq!(rt.state.c20c.search.highlighted(), None);
        assert!(
            !rt.state.c20c.heard.iter().any(|h| h.contains('>')),
            "no jump: {:?}",
            rt.state.c20c.heard
        );
    }

    /// `integration/custom-actions`: "If a field is invalid, the editor switches to that
    /// field's tab, keeps the edits, and writes nothing". The tree shows the switch and the
    /// fields on the page that shows; the hidden page's switch and the write it would make are
    /// in the state.
    #[test]
    fn a_save_with_an_invalid_field_keeps_every_edit_and_writes_nothing() {
        let mut rt = mount_with("c20c-editor-light", |g| g.c20c.name = TextEdit::new(""));
        tap(&mut rt, "c20c-editor-tab-2");
        assert!(!rt.state.c20c.folders);
        tap(&mut rt, "c20c-folders");
        tap(&mut rt, "c20c-save");
        run(&mut rt, 0.1);
        let d = &rt.state.c20c;
        assert_eq!(d.tabs.selected(), 0, "General, the name's page, shows");
        assert_eq!(focused(&rt).as_deref(), Some("c20c-name"));
        assert_eq!(d.saved, 0, "nothing written");
        assert!(d.folders, "the Folders edit on the hidden page is kept");
        assert_eq!(d.description.text, "Scale pictures to a width");
        assert_eq!(d.error, "Enter a name");
        typ(&mut rt, "Resize");
        tap(&mut rt, "c20c-save");
        assert_eq!(rt.state.c20c.saved, 1, "a valid Save writes");
    }
}

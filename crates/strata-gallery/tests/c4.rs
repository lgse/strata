//! C4's page in process: what the accessibility tree doesn't show or a script can't hold. The
//! stuck group header is hidden from assistive technology, so this file is its check (with the
//! `c4-grouped-stuck` PNG golden); modified presses can't be scripted at the pinned Rune; and
//! the 100k scroll runs under `RUNE_COMMIT_BUDGET_MS=8`. Each test names the bullets it checks;
//! the scripts in `tests/scripts/c4/` and `strata-components`' `list_table` tests check the rest.

#[expect(
    dead_code,
    reason = "c4 uses only the gallery command and the scripts folder"
)]
#[path = "goldens/support.rs"]
mod support;

mod c4 {
    use rune::{ElementInfo, Env, Key, KeyEvent, Mods, PhysicalInput, Runtime};
    use strata_components::collection::element::{STUCK, viewport_path};
    use strata_components::contract::EntrySource;
    use strata_gallery::scenes::c4::{SCOPE, WARMED, WARMUP};
    use strata_gallery::{Gallery, scenes};

    use super::support;

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

    /// The element keyed as `path`'s last part, under an element keyed as each part before it
    /// (`row-3/cell-type`), unkeyed parts between them allowed.
    fn element(rt: &Runtime<Gallery>, path: &str) -> Option<ElementInfo> {
        let parts: Vec<&str> = path.split('/').collect();
        let (last, above) = parts.split_last()?;
        rt.elements().into_iter().find(|e| {
            !e.exiting
                && e.key.as_deref() == Some(*last)
                && above.iter().all(|k| e.path.split('/').any(|seg| seg == *k))
        })
    }

    /// Presses and releases at the middle of `path`'s visible part, with `mods` held.
    fn click(rt: &mut Runtime<Gallery>, path: &str, mods: Mods) {
        let [x, y, w, h] = element(rt, path)
            .unwrap_or_else(|| panic!("no element {path}"))
            .visible;
        // Handlers run at the next frame and read the modifiers then.
        rt.physical_input(PhysicalInput::Modifiers(mods));
        rt.pointer_down(x + w / 2.0, y + h / 2.0);
        rt.frame(FRAME);
        rt.pointer_up(x + w / 2.0, y + h / 2.0);
        rt.frame(FRAME);
        rt.physical_input(PhysicalInput::Modifiers(Mods::default()));
        run(rt, 0.6);
    }

    fn selected(rt: &Runtime<Gallery>) -> Vec<String> {
        let d = &rt.state.c4;
        d.coll
            .selection
            .selected_ids(&d.listing)
            .into_iter()
            .map(|id| {
                let i = d.listing.index_of(id).expect("listed");
                d.listing.entry(i).name.to_owned()
            })
            .collect()
    }

    /// view-modes: "Click the first of five entries and Ctrl+click the third. Shift+Down in
    /// List or Shift+Right in Icons then selects only the third and fourth, as in Columns" (the
    /// List half).
    #[test]
    fn a_ctrl_click_then_shift_down_selects_only_the_third_and_fourth() {
        let mut rt = mount("c4-default-light");
        // Music, Projects, Receipts, backup.tar.gz, budget.ods: the first five rows.
        click(&mut rt, "row-3/cell-type", Mods::default());
        // Ctrl on Linux, which Rune also reports as its command key.
        let ctrl = Mods {
            ctrl: true,
            cmd: true,
            ..Mods::default()
        };
        click(&mut rt, "row-2/cell-type", ctrl);
        assert_eq!(selected(&rt), ["Music", "Receipts"]);
        rt.key(KeyEvent {
            key: Key::Down,
            mods: Mods {
                shift: true,
                ..Mods::default()
            },
        });
        run(&mut rt, 0.3);
        assert_eq!(selected(&rt), ["Receipts", "backup.tar.gz"]);
    }

    /// The stuck header's label and its visible height.
    fn stuck(rt: &Runtime<Gallery>) -> (Option<String>, f32) {
        let part = element(rt, &format!("{SCOPE}/{STUCK}")).expect("a stuck header");
        let label = element(rt, &format!("{STUCK}/label")).and_then(|e| e.text);
        (label, part.visible[3])
    }

    /// The plan's sticky file-type group headers (C4 work package: "sticky header while
    /// scrolling"; no feature-map bullet names it, as Strata's GTK headers don't stick):
    /// scrolling a grouped list keeps the top group's header stuck over the rows, and a scroll
    /// into the next group sticks that group's header.
    #[test]
    fn the_top_groups_header_stays_stuck_while_the_list_scrolls() {
        let mut rt = mount("c4-grouped-light");
        let h = strata_kit::slots::list::row_height(rt.state.look());
        // Folder (header line 0), Archive (line 9), then JPEG image (line 40, rows to 206):
        // into JPEG image by the wheel.
        for _ in 0..12 {
            rt.wheel(480.0, 400.0, -h * 4.0);
            run(&mut rt, FRAME * 2.0);
        }
        run(&mut rt, 0.3);
        let (label, shown) = stuck(&rt);
        assert_eq!(label.as_deref(), Some("JPEG image"));
        assert!(
            shown > h * 0.9,
            "whole while its group fills the top: {shown}"
        );
        // On to the next group: Spreadsheet (header line 207).
        rt.update(|_, tx| tx.scroll_to_instant(viewport_path(SCOPE), 0.0, 230.0 * h));
        run(&mut rt, 0.3);
        assert_eq!(stuck(&rt).0.as_deref(), Some("Spreadsheet"));
    }

    /// The scripted 100k scroll (`tests/scripts/c4/scroll-100k.json`) under
    /// `RUNE_COMMIT_BUDGET_MS=8`: no commit over 8 ms once the scene is mounted, with five
    /// columns of cells. As c1's, it waits out Rune's one-line-a-second limit after the mount
    /// (U-C1e-5), and runs only in an optimized build:
    /// `cargo test --profile perf -p strata-gallery --test c4 budget`.
    #[test]
    #[cfg_attr(debug_assertions, ignore = "perf: run with --profile perf")]
    fn scrolling_a_hundred_thousand_entries_stays_under_the_commit_budget() {
        let script = support::scripts_dir("c4").join("scroll-100k.json");
        let spec: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&script).expect("the script"))
                .expect("JSON");
        let events = serde_json::json!({ "events": spec["events"] });
        let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("c4");
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        let file = dir.join("scroll-100k-events.json");
        std::fs::write(&file, events.to_string()).expect("the events");

        let mut cmd = support::gallery();
        cmd.env("RUNE_COMMIT_BUDGET_MS", "8")
            .env(WARMUP, "1")
            .arg("--scene")
            .arg("c4-hundred-k-light")
            .arg("--script")
            .arg(&file)
            .arg("--tree")
            .arg(dir.join("scroll-100k-tree.json"));
        let stderr = match support::run(cmd) {
            Ok(stderr) => stderr,
            Err(e) => panic!("{e}"),
        };
        let after = stderr
            .split_once(WARMED)
            .map(|(_, after)| after)
            .unwrap_or_else(|| panic!("the page never warmed up:\n{stderr}"));
        let over: Vec<&str> = after
            .lines()
            .filter(|l| l.starts_with("rune: commit took"))
            .collect();
        assert!(
            over.is_empty(),
            "commits over budget while scrolling:\n{}",
            over.join("\n")
        );
    }

    /// browser/scrolling: "Moving the pointer away from the anchor scrolls toward it, faster the
    /// further away, on both axes the listing can scroll." In List the axes are the rows' and
    /// the table's sideways scroll (`AutoscrollPointer::sideways`, which this drives end to end).
    #[test]
    fn autoscroll_in_a_narrow_list_moves_the_rows_and_the_columns() {
        use rune::PointerButton;
        let mut rt = mount("c4-bars-light");
        rt.update(|_, tx| tx.scroll_to_x(format!("{SCOPE}-scroll"), 0.0));
        run(&mut rt, 0.2);
        let at = |rt: &Runtime<Gallery>| {
            let t = &rt.state.c4.table;
            (t.sideways_feed().get().offset[0], t.feed().get().offset[1])
        };
        let (x0, y0) = at(&rt);
        assert_eq!(x0, 0.0);
        let v = rt.state.c4.coll.viewport();
        let (ax, ay) = (v.x + 2.0, v.y + v.h / 2.0);
        rt.pointer_move(ax, ay);
        rt.frame(FRAME);
        rt.button_down(ax, ay, PointerButton::Middle);
        rt.frame(FRAME);
        rt.button_up(ax, ay, PointerButton::Middle);
        rt.frame(FRAME);
        assert!(
            rt.state.c4.auto.active(),
            "the row's margin starts autoscroll"
        );
        rt.pointer_move(ax + 200.0, ay + 200.0);
        run(&mut rt, 0.5);
        let (x1, y1) = at(&rt);
        assert!(
            x1 > x0 && y1 > y0,
            "both axes moved: ({x0}, {y0}) to ({x1}, {y1})"
        );
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        run(&mut rt, 0.2);
        assert!(!rt.state.c4.auto.active());
    }

    /// The rows' bar's band, as drawn.
    fn rows_bar(rt: &Runtime<Gallery>) -> ElementInfo {
        element(rt, &format!("{SCOPE}-bar")).expect("the rows' bar")
    }

    /// browser/scrolling: "Scrollbars fade in on hover or scrolling and stay hidden at rest".
    /// A thumb drag cut off by a view switch (Ctrl+1 hides the table) leaves the bar hidden at
    /// rest once the table is back: Rune tells the removed bar nothing (Rune draft R-C4-2), so
    /// `ListTableState::reset` lets go of it (review round 1, SF1).
    #[test]
    fn a_bar_drag_cut_off_by_a_view_switch_leaves_the_bar_hidden_at_rest() {
        let ctrl_1 = KeyEvent {
            key: Key::Command('1'),
            mods: Mods {
                cmd: true,
                ..Mods::default()
            },
        };
        let mut rt = mount("c4-hundred-k-light");
        let v = rt.state.c4.coll.viewport();
        rt.pointer_move(v.x + v.w / 2.0, v.y + v.h / 2.0);
        rt.frame(FRAME);
        rt.wheel(v.x + v.w / 2.0, v.y + v.h / 2.0, -100.0);
        run(&mut rt, 0.1);
        let [x, y, w, h] = element(&rt, &format!("{SCOPE}-bar/trough/thumb"))
            .expect("the thumb")
            .visible;
        let (tx, ty) = (x + w / 2.0, y + h / 2.0);
        rt.pointer_move(tx, ty);
        rt.frame(FRAME);
        rt.pointer_down(tx, ty);
        rt.frame(FRAME);
        rt.pointer_move(tx, ty + 30.0);
        rt.frame(FRAME);
        rt.key(ctrl_1.clone());
        run(&mut rt, 0.2);
        assert!(!rt.state.c4.shown);
        rt.pointer_up(tx, ty + 30.0);
        run(&mut rt, 0.2);
        rt.pointer_move(5.0, 5.0);
        rt.key(ctrl_1);
        run(&mut rt, 3.0);
        assert!(rt.state.c4.shown);
        assert_eq!(
            rows_bar(&rt).opacity,
            0.0,
            "the rows' bar is hidden at rest"
        );
    }
}

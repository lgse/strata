// SPDX-License-Identifier: MIT

mod x1 {
    use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, TitlebarAction};
    use strata_gallery::scenes::x1::shell::{self, Hooks, ShellWindow, Volumes};
    use strata_gallery::scenes::x1::{VOLUMES_AFTER, reload};
    use strata_gallery::{Gallery, SIZE, scenes};
    use strata_kit::look::Look;

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
        rt
    }

    fn run(rt: &mut Runtime<Gallery>, seconds: f64) {
        for _ in 0..(seconds / FRAME).round() as usize {
            rt.frame(FRAME);
        }
    }

    fn press(rt: &mut Runtime<Gallery>, text: &str) {
        rt.key(KeyEvent {
            key: Key::Text(text.into()),
            mods: Mods::default(),
        });
        rt.frame(FRAME);
    }

    fn tap(rt: &mut Runtime<Gallery>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no element {key}"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
    }

    fn click(rt: &mut Runtime<Gallery>, (x, y): (f32, f32)) -> Option<TitlebarAction> {
        rt.frame(0.01);
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        let action = rt.take_titlebar_action();
        rt.pointer_up(x, y);
        rt.frame(0.01);
        action
    }

    fn fill(rt: &Runtime<Gallery>, key: &str) -> rune::Color {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("no element {key}"))
            .fill
    }

    #[test]
    fn the_header_moves_the_window_and_its_buttons_keep_their_presses() {
        let mut rt = mount("x1-shell-light");
        // The empty middle of the bar, right of the location and left of the actions.
        let (_, y) = rt.find("x1-header").expect("the header");
        let bare = (SIZE.0 / 2.0, y);
        assert_eq!(click(&mut rt, bare), Some(TitlebarAction::Drag));
        assert_eq!(click(&mut rt, bare), Some(TitlebarAction::DoubleClick));
        rt.frame(1.0);
        assert_eq!(click(&mut rt, bare), Some(TitlebarAction::Drag));
        rt.frame(0.6);
        assert_eq!(
            click(&mut rt, bare),
            Some(TitlebarAction::Drag),
            "too slow to pair"
        );
        // So does the location, which has no handlers of its own.
        let location = rt.find("x1-location").expect("the location");
        rt.frame(1.0);
        assert_eq!(click(&mut rt, location), Some(TitlebarAction::Drag));

        let close = rt.find("x1-header-close").expect("Close");
        rt.frame(1.0);
        assert_eq!(click(&mut rt, close), None, "Close keeps its press");
        assert_eq!(
            rt.state.x1.window.last.as_deref(),
            Some("Close"),
            "and reports it to the host"
        );
    }

    struct Host {
        window: ShellWindow,
        look: Look,
    }

    fn host_window(h: &mut Host) -> &mut ShellWindow {
        &mut h.window
    }

    fn host_view(h: &Host, cx: &mut Cx<Host>) -> El {
        let hooks = Hooks {
            at: host_window,
            on_action: |_, _, _| {},
            on_volumes: |_| {},
        };
        shell::view(&h.window, &h.look, cx, hooks, |bar| bar, |window| window)
    }

    #[test]
    fn the_window_s_focus_reaches_the_header_s_chrome() {
        let host = Host {
            window: ShellWindow::default(),
            look: Look::fallback(false),
        };
        let mut rt = Runtime::new(host, host_view, SIZE, Env::new(false, 1.0));
        rt.frame(0.0);
        rt.window_focus(true);
        rt.frame(FRAME);
        assert!(rt.state.window.chrome.active);
        rt.window_focus(false);
        rt.frame(FRAME);
        assert!(!rt.state.window.chrome.active);
        rt.window_focus(true);
        rt.frame(FRAME);
        assert!(rt.state.window.chrome.active);
    }

    #[test]
    fn a_text_scale_change_rescales_the_look() {
        let mut rt = mount("x1-shell-light");
        let before = (rt.state.look.kit.ty.ui.size, rt.state.look.icon_size.md);
        press(&mut rt, "s");
        let after = (rt.state.look.kit.ty.ui.size, rt.state.look.icon_size.md);
        assert!(after.0 > before.0, "type {before:?} → {after:?}");
        assert!(after.1 >= before.1, "icons {before:?} → {after:?}");
        press(&mut rt, "s");
        let back = (rt.state.look.kit.ty.ui.size, rt.state.look.icon_size.md);
        assert_eq!(back, before);
    }

    #[test]
    fn with_the_desktop_s_animations_off_the_dialog_shows_at_once() {
        let mut rt = mount("x1-shell-dark");
        press(&mut rt, "a");
        assert!(!rt.state.look.animations);
        tap(&mut rt, "x1-mounts");
        rt.frame(FRAME);
        let layer = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("x1-volumes-layer"))
            .expect("the dialog's layer");
        assert_eq!((layer.opacity, layer.scale), (1.0, 1.0));
    }

    #[test]
    fn a_reload_moves_the_colours_as_a_theme_switch_does() {
        let mut rt = mount("x1-shell-dark");
        let overridden = fill(&rt, "x1-header");
        press(&mut rt, "d");
        run(&mut rt, reload::SETTLE + 2.0 * FRAME);
        let theme = rt.state.look.kit.c.bg_deep;
        assert_ne!(overridden, theme, "the file set the header's fill");
        assert_eq!(rt.state.x1.window.overrides.reloads, 1);
        run(&mut rt, 0.05);
        let moving = fill(&rt, "x1-header");
        assert!(
            moving != overridden && moving != theme,
            "mid-switch the fill is between {overridden:?} and {theme:?}, not {moving:?}"
        );
        run(&mut rt, 1.0);
        assert_eq!(fill(&rt, "x1-header"), theme);
    }

    #[test]
    fn an_edit_re_applies_the_file_to_the_look() {
        let mut rt = mount("x1-shell-light");
        assert_eq!(rt.state.look.frame.inset, 3.0);
        press(&mut rt, "e");
        run(&mut rt, reload::SETTLE + 2.0 * FRAME);
        assert_eq!(rt.state.look.frame.inset, 4.0, "the edited file's inset");
        assert_eq!(rt.state.x1.window.overrides.problems, 1);
    }

    #[test]
    fn a_read_that_finds_nothing_new_changes_nothing() {
        let mut rt = mount("x1-shell-dark");
        press(&mut rt, "t");
        run(&mut rt, reload::SETTLE + 2.0 * FRAME);
        assert_eq!(rt.state.x1.window.overrides.reloads, 0, "the same bytes");

        press(&mut rt, "d");
        run(&mut rt, reload::SETTLE + 2.0 * FRAME);
        assert_eq!(rt.state.x1.window.overrides.reloads, 1);
        assert!(!rt.state.x1.window.overrides.present);
        run(&mut rt, 1.0);
        let deleted = fill(&rt, "x1-header");

        // An empty file: no rows, as no file, but there.
        press(&mut rt, "n");
        run(&mut rt, reload::SETTLE + 2.0 * FRAME);
        let status = &rt.state.x1.window.overrides;
        assert!(status.present, "the readout follows the file");
        assert_eq!(status.reloads, 1, "the look didn't change, so no reload");
        assert_eq!(fill(&rt, "x1-header"), deleted);
    }

    #[test]
    fn the_volumes_arrive_after_the_dialog_asks_and_again_on_refresh() {
        let mut rt = mount("x1-shell-light");
        tap(&mut rt, "x1-mounts");
        assert!(rt.state.x1.window.volumes_open);
        assert_eq!(rt.state.x1.window.volumes, Volumes::Loading);
        run(&mut rt, VOLUMES_AFTER + 2.0 * FRAME);
        let Volumes::Listed(list) = &rt.state.x1.window.volumes else {
            panic!("listed: {:?}", rt.state.x1.window.volumes);
        };
        assert_eq!(list.len(), 3);
        assert!(rt.find("x1-volume-2").is_some(), "every volume is a row");

        tap(&mut rt, "x1-volumes-secondary");
        assert_eq!(rt.state.x1.window.volumes, Volumes::Loading, "Refresh");
        run(&mut rt, VOLUMES_AFTER + 2.0 * FRAME);
        assert!(matches!(rt.state.x1.window.volumes, Volumes::Listed(_)));
    }

    /// Without a GLib bridge the dialog says so, rather than listing forever.
    #[test]
    fn without_a_bridge_the_dialog_shows_the_failure_not_a_spinner() {
        let mut rt = mount("x1-shell-no-bridge-light");
        tap(&mut rt, "x1-mounts");
        run(&mut rt, VOLUMES_AFTER + 2.0 * FRAME);
        assert!(
            matches!(rt.state.x1.window.volumes, Volumes::Failed(_)),
            "{:?}",
            rt.state.x1.window.volumes
        );
        let error = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("x1-volumes-error"))
            .expect("the dialog's error");
        assert_eq!(
            error.text.as_deref(),
            Some("No GLib bridge: the GLib thread couldn't start")
        );
        assert!(
            !rt.elements().iter().any(|e| e.role == "progressbar"),
            "no spinner"
        );
    }

    #[test]
    fn each_golden_scene_builds_its_look_with_the_file_last() {
        let page = scenes::all().find(|p| p.id == "x1").expect("the x1 page");
        for &(name, scene) in page.scenes {
            let mut g = Gallery::default();
            scene(&mut g);
            assert_eq!(g.look.kit.dark, name.ends_with("-dark"), "{name}");
            assert_eq!(g.look.frame.inset, 3.0, "{name}: frame.inset");
        }
    }
}

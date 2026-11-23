// SPDX-License-Identifier: MIT

mod c14b {
    use rune::{ElementInfo, Env, Runtime};
    use strata_components::viewers::model::CameraView;
    use strata_gallery::{Gallery, scenes};
    use strata_kit::look::Look;

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
        frames(&mut rt, 30);
        rt
    }

    fn frames(rt: &mut Runtime<Gallery>, n: usize) {
        for _ in 0..n {
            rt.frame(DT);
        }
    }

    fn element(rt: &Runtime<Gallery>, key: &str) -> ElementInfo {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap_or_else(|| panic!("no element {key}"))
    }

    /// The accessible description of the node keyed `key`.
    fn description(rt: &Runtime<Gallery>, key: &str) -> Option<String> {
        fn find<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
            if v.get("key").and_then(|k| k.as_str()) == Some(key) {
                return Some(v);
            }
            v.get("children")?
                .as_array()?
                .iter()
                .find_map(|c| find(c, key))
        }
        let tree = rt.accessibility_json();
        let node = find(&tree, key).unwrap_or_else(|| panic!("no node {key}"));
        node.get("description")
            .and_then(|d| d.as_str())
            .map(str::to_owned)
    }

    fn click(rt: &mut Runtime<Gallery>, x: f32, y: f32) {
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.frame(DT);
        rt.pointer_up(x, y);
        rt.frame(DT);
    }

    fn click_key(rt: &mut Runtime<Gallery>, key: &str) {
        let r = element(rt, key).rect;
        click(rt, r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    }

    fn next_sweeps(rt: &mut Runtime<Gallery>) -> bool {
        click_key(rt, "c14b-model-next");
        frames(rt, 5);
        let moving = element(rt, "c14b-model-camera-dot").moving;
        frames(rt, 40);
        moving
    }

    #[test]
    fn the_indicator_sweeps_to_the_new_view() {
        let mut rt = scene("c14b-model-light");
        assert!(next_sweeps(&mut rt), "the dot turns");
        assert_eq!(rt.state.c14b.model.camera(), CameraView::BackLeft);
        assert!(
            !element(&rt, "c14b-model-camera-dot").moving,
            "and comes to rest"
        );
    }

    #[test]
    fn the_indicator_jumps_without_motion() {
        let mut rt = scene("c14b-model-light");
        rt.set_system_reduced_motion(true);
        assert!(!next_sweeps(&mut rt), "reduced motion: the dot jumps");

        let mut rt = scene("c14b-model-light");
        rt.state.look.animations = false;
        assert!(!next_sweeps(&mut rt), "animations off: the dot jumps");

        // A new preview mounts its indicator at the view, with nothing to sweep from.
        let rt = scene("c14b-model-rendering-light");
        assert!(!element(&rt, "c14b-model-camera-dot").moving);
    }

    #[test]
    fn a_new_file_puts_the_dot_at_front_left_at_once() {
        let mut rt = scene("c14b-model-light");
        next_sweeps(&mut rt);
        next_sweeps(&mut rt);
        assert_eq!(rt.state.c14b.model.camera(), CameraView::BackRight);
        // The page's `n` stands in for selecting the next model file.
        rt.key(rune::KeyEvent {
            key: rune::Key::Text("n".into()),
            mods: rune::Mods::default(),
        });
        frames(&mut rt, 3);
        assert_eq!(rt.state.c14b.model.file().name, "gear.stl");
        assert_eq!(rt.state.c14b.model.camera(), CameraView::FrontLeft);
        assert!(
            !element(&rt, "c14b-model-camera-dot").moving,
            "the dot lands at Front left without a sweep"
        );
    }

    #[test]
    fn a_theme_change_keeps_the_camera_view() {
        let mut rt = scene("c14b-model-light");
        next_sweeps(&mut rt);
        rt.update(|g, _| g.look = Look::fallback(true));
        frames(&mut rt, 30);
        assert_eq!(rt.state.c14b.model.camera(), CameraView::BackLeft);
        assert_eq!(
            description(&rt, "c14b-model-camera").as_deref(),
            Some("Back left")
        );
    }

    #[test]
    fn a_click_at_a_rows_top_edge_opens_its_folder() {
        let mut rt = scene("c14b-archive-light");
        let docs = element(&rt, "row-8000000006").rect;
        click(&mut rt, docs[0] + docs[2] / 2.0, docs[1] + 0.5);
        frames(&mut rt, 5);
        let a = rt.state.c14b.archive.as_ref().expect("the archive");
        assert_eq!(a.listing().crumbs(), ["docs"]);
    }

    #[test]
    fn buttons_named_by_their_tooltip_have_no_description() {
        let rt = scene("c14b-model-light");
        assert_eq!(description(&rt, "c14b-model-next"), None);
        assert_eq!(description(&rt, "c14b-model-previous"), None);
        let mut rt = scene("c14b-archive-light");
        click_key(&mut rt, "row-8000000006");
        frames(&mut rt, 5);
        assert_eq!(description(&rt, "c14b-archive-up"), None);
    }
}

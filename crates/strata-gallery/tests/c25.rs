//! C25's page in process: thumbnails fill the list while it scrolls with no commit of their
//! own, the on/off switch reaches every view at once, and a filtered list's rows draw their
//! own entries' thumbnails.

mod c25 {
    use rune::{Env, Key, KeyEvent, Mods, Runtime};
    use strata_components::contract::EntrySource;
    use strata_components::thumbnails::{State, THUMB};
    use strata_gallery::{Gallery, scenes};

    const DT: f64 = 1.0 / 60.0;

    fn scene(name: &str, set: impl FnOnce(&mut Gallery)) -> Runtime<Gallery> {
        let f = scenes::all_scenes()
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, f)| f)
            .unwrap_or_else(|| panic!("no scene {name}"));
        let mut g = Gallery::default();
        f(&mut g);
        set(&mut g);
        let env = Env::new(g.dark, 1.0);
        let mut rt = strata_gallery::runtime(g, env);
        rt.frame(0.0);
        for _ in 0..60 {
            rt.frame(DT);
        }
        rt
    }

    fn press(rt: &mut Runtime<Gallery>, c: &str) {
        rt.key(KeyEvent {
            key: Key::Text(c.into()),
            mods: Mods::default(),
        });
        rt.frame(DT);
    }

    fn thumbs_under(rt: &Runtime<Gallery>, scope: &str) -> usize {
        rt.elements()
            .iter()
            .filter(|e| !e.exiting && e.key.as_deref() == Some(THUMB))
            .filter(|e| {
                e.path.starts_with("gallery/c25-page/") && e.path.contains(&format!("/{scope}/"))
            })
            .count()
    }

    /// Commits and view runs over 240 frames of scrolling, with the worker on or paused, and
    /// the thumbnails that landed meanwhile.
    fn scroll(work: bool) -> (u64, usize, usize) {
        let mut rt = scene("c25-filled-light", |g| {
            g.c25.status_on_idle = false;
        });
        if !work {
            rt.state.c25.lanes = 0;
        }
        let (commits, views) = (rt.stats.commits, rt.state.c25.views.get());
        let before = rt.state.c25.store.stats().ready;
        for _ in 0..240 {
            rt.wheel(300.0, 400.0, -12.0);
            rt.frame(DT);
        }
        let landed = rt.state.c25.store.stats().ready - before;
        (
            rt.stats.commits - commits,
            rt.state.c25.views.get() - views,
            landed,
        )
    }

    #[test]
    fn scrolling_fills_the_list_without_a_commit_per_thumbnail() {
        let (commits, views, landed) = scroll(true);
        let (base_commits, base_views, base_landed) = scroll(false);
        assert_eq!(base_landed, 0, "the paused worker lands nothing");
        assert!(landed > 40, "{landed} thumbnails landed");
        assert_eq!(views, 0, "the view never ran");
        assert_eq!(base_views, 0);
        assert_eq!(
            commits, base_commits,
            "only the scroll's own window shifts commit"
        );
    }

    #[test]
    fn turning_thumbnails_off_reaches_both_views_at_once_and_on_restores_them() {
        let mut rt = scene("c25-two-views-light", |_| {});
        let (a, b) = (
            thumbs_under(&rt, scenes::c25::SCOPE),
            thumbs_under(&rt, scenes::c25::SCOPE_B),
        );
        assert!(a > 0 && b > 0, "both lists show thumbnails: {a}, {b}");
        let views = rt.state.c25.views.get();
        press(&mut rt, "t");
        assert_eq!(thumbs_under(&rt, scenes::c25::SCOPE), 0);
        assert_eq!(thumbs_under(&rt, scenes::c25::SCOPE_B), 0);
        assert_eq!(rt.state.c25.views.get(), views + 1, "one view run for both");
        let rendered = rt.state.c25.decoded;
        press(&mut rt, "t");
        for _ in 0..30 {
            rt.frame(DT);
        }
        assert_eq!(thumbs_under(&rt, scenes::c25::SCOPE), a);
        assert_eq!(thumbs_under(&rt, scenes::c25::SCOPE_B), b);
        assert_eq!(rt.state.c25.decoded, rendered, "nothing rendered again");
    }

    #[test]
    fn a_filtered_list_draws_each_rows_own_thumbnail() {
        let mut rt = scene("c25-filled-light", |_| {});
        press(&mut rt, "f");
        for _ in 0..60 {
            rt.frame(DT);
        }
        let d = &rt.state.c25;
        let shown = d.coll.visible_range();
        assert!(!shown.is_empty());
        for i in shown.filter(|&i| i < d.listing.len()) {
            let e = d.listing.entry(i);
            let key = e.thumbnail.expect("images only");
            // The fake worker shapes each image by its key: the image held for this row's key
            // is the one rendered for it.
            let State::Ready { width, height, .. } = d.store.state(key) else {
                panic!("{} has no thumbnail", e.name);
            };
            let px = width.max(height);
            let want = match key.get() % 4 {
                0 => (px, px),
                1 => (px, px * 3 / 4),
                2 => (px * 3 / 4, px),
                _ => (px, px / 2),
            };
            assert_eq!((width, height), want, "{}", e.name);
            assert_eq!(d.store.thumb(key).pixels().size(), (width, height));
        }
    }
}

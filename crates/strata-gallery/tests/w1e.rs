//! W1e: every drop zone C1e, C2, C6, C7 and C8 attach, and every drag source C9 and the viewers
//! attach, reaches a keyed binder (as X2's is) with the key its element carries, unique in the
//! window or inside the scope it names. Each test
//! mounts a page's own component state with a binder that records what it hears.

mod w1e {
    use std::cell::RefCell;
    use std::collections::HashSet;

    use rune::{Cx, El, ElementInfo, Env, Runtime, col};
    use strata_components::collection::element::row_key;
    use strata_components::contract::{Dnd, DragSource, ElementKey, EntrySource};
    use strata_components::location_bar::{LocationBar, crumb_key};
    use strata_components::preview_drawer::{Arrangement, HeaderInfo, PreviewDrawer};
    use strata_components::sidebar::{DEFAULT_KEY as SIDEBAR, PlacesSidebar, SidebarData};
    use strata_components::tabs::{
        Accepts, BrowserTabs, BrowserTabsState, Tab, TabId, TabsData, new_tab_key, tab_key,
    };
    use strata_components::viewers::image::ImageView;
    use strata_components::viewers::model::ModelView;
    use strata_components::viewers::route::RouteView;
    use strata_gallery::{Gallery, scenes};
    use strata_kit::icons::IconName;
    use strata_kit::look::Look;

    const FRAME: f64 = 1.0 / 60.0;

    /// A key a binder heard: the element's key and its scope.
    type Heard = (String, Option<String>);

    /// What a binder heard in one build of the view: zones, then sources, in order.
    #[derive(Default)]
    struct Build {
        zones: Vec<Heard>,
        sources: Vec<Heard>,
    }

    thread_local! {
        /// One entry per build of the view, the newest last.
        static BUILDS: RefCell<Vec<Build>> = const { RefCell::new(Vec::new()) };
    }

    /// Starts a new build: each test's view calls it first.
    fn begin_build() {
        BUILDS.with(|b| b.borrow_mut().push(Build::default()));
    }

    /// A binder that records each zone's and source's key and attaches nothing.
    fn recording<S: 'static>() -> Dnd<S> {
        let note = |source: bool, key: &ElementKey| {
            let heard = (key.key().to_owned(), key.scope().map(str::to_owned));
            BUILDS.with(|b| {
                let mut b = b.borrow_mut();
                let build = b.last_mut().expect("the view calls begin_build first");
                if source {
                    build.sources.push(heard);
                } else {
                    build.zones.push(heard);
                }
            });
        };
        Dnd::custom_keyed(
            move |el, key, _| {
                note(false, key);
                el
            },
            move |el, key, _| {
                note(true, key);
                el
            },
        )
    }

    /// The zones and sources heard since the last call, each once. No build may hear one key
    /// twice in one scope: two zones (or two sources) built under one key would be one target
    /// to a binder that finds them by key, whatever keys Rune leaves on the elements (a
    /// virtual row keeps its own).
    fn heard_all() -> (HashSet<Heard>, HashSet<Heard>) {
        let builds = BUILDS.with(|b| std::mem::take(&mut *b.borrow_mut()));
        assert!(!builds.is_empty(), "the view never built");
        let mut zones = HashSet::new();
        let mut sources = HashSet::new();
        for build in builds {
            for (list, all) in [(build.zones, &mut zones), (build.sources, &mut sources)] {
                let mut seen = HashSet::new();
                for h in list {
                    assert!(seen.insert(h.clone()), "{h:?} heard twice in one build");
                    all.insert(h);
                }
            }
        }
        (zones, sources)
    }

    /// The zones heard since the last call: see [`heard_all`].
    fn heard() -> HashSet<Heard> {
        let (zones, sources) = heard_all();
        assert!(sources.is_empty(), "no source expected: {sources:?}");
        zones
    }

    fn gallery(scene: &str) -> Gallery {
        let f = scenes::all_scenes()
            .into_iter()
            .find(|(n, _)| *n == scene)
            .map(|(_, f)| f)
            .unwrap_or_else(|| panic!("no scene {scene}"));
        let mut g = Gallery::default();
        f(&mut g);
        g
    }

    fn mount<S: 'static>(state: S, view: fn(&S, &mut Cx<S>) -> El) -> Runtime<S> {
        BUILDS.with(|b| b.borrow_mut().clear());
        let mut rt = Runtime::new(state, view, strata_gallery::SIZE, Env::new(false, 1.0));
        for i in 0..30 {
            rt.frame(f64::from(i) * FRAME);
        }
        rt
    }

    /// The keyed segments of `path` (`#n` stands for an unkeyed element).
    fn keyed(path: &str) -> Vec<&str> {
        path.split('/')
            .filter(|s| !s.is_empty() && !s.starts_with('#'))
            .collect()
    }

    /// Each heard key is on exactly one live element: in the window, or under the element
    /// keyed its scope when it has one. Rune renames a sibling that repeats a key
    /// `key#dup1`, `key#dup2` (and warns), so those count as the same key: two zones given one
    /// key show up as two elements, not one.
    fn check_heard_against_tree<S>(rt: &Runtime<S>, heard: &HashSet<Heard>) {
        let elements: Vec<ElementInfo> = rt.elements().into_iter().filter(|e| !e.exiting).collect();
        let same_key = |name: &str, key: &str| {
            name == key
                || name
                    .strip_prefix(key)
                    .is_some_and(|rest| rest.starts_with("#dup"))
        };
        for (key, scope) in heard {
            let on: Vec<&str> = elements
                .iter()
                .filter(|e| e.key.as_deref().is_some_and(|k| same_key(k, key)))
                .filter(|e| match scope {
                    None => true,
                    Some(sc) => {
                        let segs = keyed(&e.path);
                        let at = segs
                            .iter()
                            .rposition(|s| same_key(s, key))
                            .expect("its own segment");
                        segs[..at].contains(&sc.as_str())
                    }
                })
                .map(|e| e.path.as_str())
                .collect();
            assert_eq!(
                on.len(),
                1,
                "{key} (scope {scope:?}) should be on one element, is on {on:?}"
            );
        }
    }

    fn assert_has(heard: &HashSet<Heard>, key: &str, scope: Option<&str>) {
        let want = (key.to_owned(), scope.map(str::to_owned));
        assert!(heard.contains(&want), "{want:?} not heard among {heard:?}");
    }

    // ---- C1e: folder rows (scoped by the collection) and the background ----

    fn c1_view(g: &Gallery, _: &mut Cx<Gallery>) -> El {
        begin_build();
        let list = scenes::c1::collection().dnd(recording()).build(g);
        col().w_full().h_full().child(list)
    }

    #[test]
    fn a_collections_rows_carry_its_key_as_their_scope() {
        let g = gallery("c1-ten-rows-light");
        let scope = scenes::c1::SCOPE;
        // Every row of the ten is mounted: each folder row takes a zone of its own.
        let listing = &g.c1.listing;
        let folders: HashSet<Heard> = (0..listing.len())
            .filter(|&i| listing.entry(i).kind.is_folder_like())
            .map(|i| (row_key(listing.id_at(i)), Some(scope.to_owned())))
            .collect();
        assert!(folders.len() > 1, "the scene lists several folders");
        let rt = mount(g, c1_view);
        let heard = heard();
        check_heard_against_tree(&rt, &heard);
        assert_has(&heard, scope, None);
        let rows: HashSet<Heard> = heard
            .iter()
            .filter(|(k, _)| k.starts_with("row-"))
            .cloned()
            .collect();
        assert_eq!(
            rows, folders,
            "one zone per folder row, scoped by the collection"
        );
    }

    // ---- C2: each column's rows and background, and the blank strip ----

    fn c2_view(g: &Gallery, _: &mut Cx<Gallery>) -> El {
        begin_build();
        let strip = scenes::c2::columns_view().dnd(recording()).build(g);
        col().w_full().h_full().child(strip)
    }

    #[test]
    fn columns_key_the_blank_strip_and_scope_each_columns_rows() {
        let rt = mount(gallery("c2-three-columns-light"), c2_view);
        let heard = heard();
        check_heard_against_tree(&rt, &heard);
        assert_has(&heard, &format!("{}-blank", scenes::c2::KEY), None);
        let backgrounds: HashSet<&String> = heard
            .iter()
            .filter(|(k, s)| s.is_none() && k.as_str() != "c2-blank")
            .map(|(k, _)| k)
            .collect();
        assert!(backgrounds.len() >= 3, "a background per column: {heard:?}");
        let scopes: HashSet<&str> = heard.iter().filter_map(|(_, s)| s.as_deref()).collect();
        assert!(scopes.len() >= 2, "rows in several columns: {heard:?}");
        for s in &scopes {
            assert!(
                backgrounds.iter().any(|b| b.as_str() == *s),
                "{s} is a column's key"
            );
        }
    }

    // ---- C6: each ancestor crumb ----

    const LOC: &str = "w1e-loc";

    fn c6_view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
        begin_build();
        let mut layer = Vec::new();
        let bar = LocationBar::new(
            LOC,
            &g.look,
            &g.c6.trail,
            &g.c6.bar,
            |g: &mut Gallery| &mut g.c6.bar,
            |_: &mut Gallery, _, _| {},
        )
        .dnd(recording())
        .build(cx, &mut layer);
        col().w_full().h_full().child(bar).children(layer)
    }

    #[test]
    fn each_ancestor_crumb_is_keyed_for_the_binder() {
        let g = gallery("c6-deep-light");
        let ancestors = g.c6.trail.crumbs.len() - 1;
        let rt = mount(g, c6_view);
        let heard = heard();
        check_heard_against_tree(&rt, &heard);
        for i in 0..ancestors {
            assert_has(&heard, &crumb_key(LOC, i), None);
        }
        assert!(
            !heard.contains(&(crumb_key(LOC, ancestors), None)),
            "the current crumb takes no drop"
        );
    }

    // ---- C7: tabs, New tab and the strip ----

    const STRIP: &str = "w1e-tabs";

    struct Tabs {
        look: Look,
        state: BrowserTabsState,
    }

    fn tabs() -> [Tab<'static>; 3] {
        [
            Tab::new(TabId(1), "Home", IconName::Folder),
            Tab::new(TabId(2), "Trash", IconName::Trash).accepts(Accepts::NONE),
            Tab::new(TabId(3), "Music", IconName::Folder),
        ]
    }

    fn c7_view(h: &Tabs, cx: &mut Cx<Tabs>) -> El {
        begin_build();
        let tabs = tabs();
        let data = TabsData::new(&tabs, Some(TabId(1)));
        let strip = BrowserTabs::new(
            &data,
            &h.state,
            |h: &mut Tabs| &mut h.state,
            |h| &mut h.state.drag,
        )
        .dnd(recording())
        .opens_tabs(|_, offer| offer.local)
        .build(&h.look, cx);
        col().w_full().h_full().child(strip)
    }

    #[test]
    fn tabs_new_tab_and_the_strip_are_keyed_for_the_binder() {
        let h = Tabs {
            look: Look::fallback(false),
            state: BrowserTabsState::new(STRIP),
        };
        let rt = mount(h, c7_view);
        let heard = heard();
        check_heard_against_tree(&rt, &heard);
        assert_has(&heard, &tab_key(STRIP, TabId(3)), None);
        assert_has(&heard, &new_tab_key(STRIP), None);
        assert_has(&heard, STRIP, None);
        assert!(
            !heard.contains(&(tab_key(STRIP, TabId(2)), None)),
            "Trash builds no zone"
        );
    }

    // ---- C8: the sidebar, its rows and its pin gaps ----

    fn c8_view(g: &Gallery, _: &mut Cx<Gallery>) -> El {
        begin_build();
        let d = &g.c8;
        let data = SidebarData {
            places: &d.places,
            pins: &d.pins,
            devices: &d.devices,
            current: d.current.as_ref(),
        };
        let sidebar = PlacesSidebar::new(&d.sidebar, |g: &mut Gallery| &mut g.c8.sidebar, data)
            .dnd(recording())
            .build(&g.look);
        rune::row().w_full().h_full().child(sidebar)
    }

    #[test]
    fn the_sidebar_its_rows_and_its_gaps_are_keyed_for_the_binder() {
        let g = gallery("c8-full-light");
        let pins = g.c8.pins.len();
        let rows = accepting_rows(&g);
        let rt = mount(g, c8_view);
        let heard = heard();
        check_heard_against_tree(&rt, &heard);
        assert_has(&heard, SIDEBAR, None);
        for i in 0..=pins {
            assert_has(&heard, &format!("{SIDEBAR}-band-{i}"), None);
        }
        assert_eq!(
            heard_rows(&heard),
            rows,
            "every row that takes drops, by its key"
        );
    }

    #[test]
    fn every_device_row_that_takes_drops_is_keyed_for_the_binder() {
        let g = gallery("c8-devices-light");
        let rows = accepting_rows(&g);
        assert!(
            rows.iter().any(|(k, _)| k.contains("-device-")),
            "the scene has devices that take drops"
        );
        let rt = mount(g, c8_view);
        let heard = heard();
        check_heard_against_tree(&rt, &heard);
        assert_eq!(heard_rows(&heard), rows);
    }

    /// The rows the sidebar shows that take drops, as the binder should hear them: places and
    /// pins that accept drops (pins only while PINNED is open), and devices that accept drops,
    /// mounted and not releasing.
    fn accepting_rows(g: &Gallery) -> HashSet<Heard> {
        let d = &g.c8;
        let places = d
            .places
            .iter()
            .filter(|p| p.accepts_drops)
            .map(|p| p.item());
        let pins = d
            .pins
            .iter()
            .filter(|p| p.accepts_drops && !d.sidebar.pinned_collapsed)
            .map(|p| p.item());
        let devices = d
            .devices
            .iter()
            .flat_map(|g| &g.devices)
            .filter(|v| v.accepts_drops && v.mounted && !v.pending)
            .map(|v| v.item());
        places
            .chain(pins)
            .chain(devices)
            .map(|id| (id.key(SIDEBAR), None))
            .collect()
    }

    fn heard_rows(heard: &HashSet<Heard>) -> HashSet<Heard> {
        let row = |k: &str| {
            ["place", "pin", "device"]
                .iter()
                .any(|kind| k.starts_with(&format!("{SIDEBAR}-{kind}-")))
        };
        heard.iter().filter(|(k, _)| row(k)).cloned().collect()
    }

    // ---- C9 and the viewers: drag sources ----

    fn none<S: 'static>() -> DragSource<S> {
        DragSource::new(|_: &S| None)
    }

    fn c9_view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
        begin_build();
        let look = g.look();
        PreviewDrawer::new(&g.c9.drawer, |g: &mut Gallery| &mut g.c9.drawer)
            .arrangement(Arrangement::List)
            .header(HeaderInfo::new("notes.md").icon(look.icon(IconName::FileCode)))
            .placeholder("No preview for this selection")
            .drag_source(recording(), none())
            .build(col(), look, cx)
    }

    #[test]
    fn the_preview_headers_drag_source_is_keyed_for_the_binder() {
        let rt = mount(gallery("c9-list-open-light"), c9_view);
        let (zones, sources) = heard_all();
        assert!(zones.is_empty(), "{zones:?}");
        check_heard_against_tree(&rt, &sources);
        let want: HashSet<Heard> = [("preview-handle".to_owned(), None)].into();
        assert_eq!(sources, want);
    }

    /// Which viewer the C14b page shows: its drag source goes through the shared viewport.
    #[derive(Clone, Copy)]
    enum Viewer {
        Image,
        Model,
        Route,
    }

    thread_local! {
        static VIEWER: std::cell::Cell<Viewer> = const { std::cell::Cell::new(Viewer::Image) };
    }

    fn viewer_view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
        begin_build();
        let d = &g.c14b;
        let look = g.look();
        let viewer = match VIEWER.with(std::cell::Cell::get) {
            Viewer::Image => ImageView::new(&d.image, |g: &mut Gallery| &mut g.c14b.image)
                .key("w1e-image")
                .pixels(&d.photo)
                .name("harbour.png")
                .drag_source(recording(), none())
                .build(look, cx),
            Viewer::Model => ModelView::new(
                &d.model,
                |g: &mut Gallery| &mut g.c14b.model,
                |g: &mut Gallery| &mut g.c14b.model.viewport,
            )
            .key("w1e-model")
            .pixels(&d.render)
            .drag_source(recording(), none())
            .build(look, cx),
            Viewer::Route => RouteView::new(
                &d.route,
                |g: &mut Gallery| &mut g.c14b.route,
                |g: &mut Gallery| &mut g.c14b.route.viewport,
            )
            .key("w1e-route")
            .pixels(&d.route_px)
            .drag_source(recording(), none())
            .build(look, cx),
        };
        col().w_full().h_full().child(viewer)
    }

    fn viewer_source(viewer: Viewer, scene: &str, key: &str) {
        VIEWER.with(|v| v.set(viewer));
        let rt = mount(gallery(scene), viewer_view);
        let (zones, sources) = heard_all();
        assert!(zones.is_empty(), "{zones:?}");
        check_heard_against_tree(&rt, &sources);
        let want: HashSet<Heard> = [(key.to_owned(), None)].into();
        assert_eq!(sources, want, "{scene}");
    }

    #[test]
    fn each_viewers_drag_source_is_keyed_by_its_picture() {
        viewer_source(Viewer::Image, "c14b-image-light", "w1e-image-picture");
        viewer_source(Viewer::Model, "c14b-model-light", "w1e-model-picture");
        viewer_source(Viewer::Route, "c14b-route-light", "w1e-route-picture");
    }
}

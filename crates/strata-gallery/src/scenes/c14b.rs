//! The gallery page for C14b Viewers and archive preview: its scenes, demo state, view and key and
//! poll hooks. Scene names start with `c14b-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-3-views/C14b-viewers.md`.
//!
//! Each viewer sits in C9's preview drawer beside a fake listing, so the header's busy spinner
//! and the drawer's width are real. Renders, loads and unlocks are fake workers on the virtual
//! clock: `poll` hands each result over once its time comes, so goldens and scripts see the same
//! results every run.
//!
//! # Keys that stand in for the host
//!
//! Strata binds these, not the components; the page binds them so scripts can drive them:
//!
//! | Key | Stands in for |
//! | --- | --- |
//! | Ctrl+Alt+Space | The media play key: plays or pauses the route |
//! | Ctrl+Alt+Left, Ctrl+Alt+Right | The media seek keys: 5 s of route playback back or on |
//! | `u` | Settings → Units: metric or imperial |
//! | `n` | Selecting the next model file in the listing |
//! | `t` | Settings → Type to search: off makes `h j k l` move in the archive tree |
//!
//! Left at the archive's root, where Strata returns focus to the listing, shows that on the
//! page's status line (`c14b-status`) instead.

use std::collections::VecDeque;

use rune::{Color, Cx, El, Key, KeyEvent, Pixels, Tx, col, text};
use strata_components::contract::EntryId;
use strata_components::preview_drawer::{
    Arrangement, DrawerState, HeaderInfo, HostedPreview, PreviewDrawer,
};
use strata_components::viewers::archive::{
    ArchiveDir, ArchiveItem, ArchivePrompt, ArchiveState, ArchiveTree, ArchiveView, PromptState,
};
use strata_components::viewers::image::ImageView;
use strata_components::viewers::layout::Position;
use strata_components::viewers::model::{
    CameraView, Encoding, Mesh, ModelFile, ModelKind, ModelState, ModelView, Stage,
};
use strata_components::viewers::route::{
    RouteCommand, RouteData, RouteLabels, RouteSample, RouteState, RouteView, Seek, Units,
};
use strata_components::viewers::viewport::{Mode, ViewportState};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use crate::{Gallery, Scene};

/// What the drawer shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Show {
    /// Nothing yet: `poll` loads the model, so the page opened from the index shows it.
    #[default]
    None,
    Image,
    Model,
    Route,
    Archive,
    Locked,
}

/// A fake worker's result, due at a time on the clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Job {
    Stage(Stage),
    /// The render (or embedded picture) of this view lands.
    Image(CameraView),
    /// The render fails.
    Fail,
}

/// The fake model files the listing holds, in order: `n` steps through them.
const MODELS: [(&str, ModelKind); 4] = [
    ("bracket.stl", ModelKind::Mesh),
    ("gear.stl", ModelKind::Mesh),
    ("lid.3mf", ModelKind::Embedded),
    ("benchy.gcode", ModelKind::Embedded),
];

/// The render's size in pixels.
const RENDER: u32 = 384;
/// An embedded thumbnail's size in pixels (a slicer's).
const THUMBNAIL: [u32; 2] = [300, 200];
/// The photo's size in pixels.
const PHOTO: [u32; 2] = [960, 640];
/// The route picture and the profile space, as Strata's helper makes them.
const ROUTE_PX: [f32; 2] = [800.0, 600.0];
const PROFILE_PX: [f32; 2] = [800.0, 70.0];
/// The password the locked archive takes.
pub const PASSWORD: &str = "preview";
/// The first member id the page hands out: a range of its own, never reused (W1b).
const ARCHIVE_IDS: u64 = 8_000_000_000;

/// No busy label: what the drawer hosts for the views that never work in the background.
#[derive(Default)]
pub struct Idle;

impl HostedPreview for Idle {}

/// C14b's demo state, held in `Gallery::c14b`.
#[derive(Default)]
pub struct Demo {
    pub show: Show,
    pub drawer: DrawerState,
    pub idle: Idle,
    pub now: f64,
    pub image: ViewportState,
    pub photo: Pixels,
    pub model: ModelState,
    pub render: Pixels,
    /// Which of [`MODELS`] is open.
    pub file: usize,
    pub jobs: VecDeque<(f64, Job)>,
    /// The next view render fails.
    pub fail_next: bool,
    /// Views the model view asked to render.
    pub renders: Vec<CameraView>,
    pub route: RouteState,
    pub route_px: Pixels,
    pub archive: Option<ArchiveState>,
    pub prompt: PromptState,
    /// Strata's Type to search, which keeps `h j k l` out of the archive tree while on.
    pub type_to_search: bool,
    next_id: u64,
    /// The page's status line: what the host would do (Left at the archive root).
    pub status: String,
}

impl Demo {
    fn ids(&mut self) -> impl FnMut() -> EntryId + '_ {
        if self.next_id == 0 {
            self.next_id = ARCHIVE_IDS;
        }
        move || {
            self.next_id += 1;
            EntryId::new(self.next_id)
        }
    }

    /// Opens model file `file`, scheduling its first load from `now`; `image_at` is when its
    /// picture lands, or never.
    fn open_model(&mut self, file: usize, image_at: Option<f64>) {
        let (name, kind) = MODELS[file];
        self.show = Show::Model;
        self.file = file;
        let extension = name.rsplit('.').next().unwrap_or_default().to_owned();
        self.model.open(ModelFile {
            name: name.to_owned(),
            extension,
            position: Some(Position {
                index: file + 1,
                count: MODELS.len(),
                results: false,
            }),
            kind,
        });
        self.jobs.clear();
        let t = self.now;
        match kind {
            ModelKind::Mesh => {
                self.jobs.push_back((t + 0.1, Job::Stage(Stage::Reading)));
                self.jobs
                    .push_back((t + 0.3, Job::Stage(Stage::Rendering { triangles: 12_480 })));
                if let Some(at) = image_at {
                    self.jobs
                        .push_back((t + at - 0.1, Job::Stage(Stage::Finishing)));
                }
            }
            ModelKind::Embedded => {
                self.jobs.push_back((t + 0.1, Job::Stage(Stage::Thumbnail)));
            }
        }
        if let Some(at) = image_at {
            self.jobs
                .push_back((t + at, Job::Image(CameraView::FrontLeft)));
        }
    }

    /// The picture for `view` lands now.
    fn land(&mut self, view: CameraView, look: &Look) {
        let kind = MODELS[self.file].1;
        let (w, h, rgba) = match kind {
            ModelKind::Mesh => (RENDER, RENDER, render(view, look)),
            ModelKind::Embedded => (THUMBNAIL[0], THUMBNAIL[1], thumbnail()),
        };
        self.render.set(w, h, rgba);
        let mesh = (kind == ModelKind::Mesh).then_some(Mesh {
            view,
            encoding: Encoding::BinaryStl,
            triangles: 12_480,
        });
        let _ = self.model.image([w as f32, h as f32], mesh);
    }

    /// A camera button asked for `view`: schedule its render.
    fn render_view(&mut self, view: CameraView) {
        self.renders.push(view);
        let t = self.now;
        self.jobs.clear();
        self.jobs.push_back((t + 0.05, Job::Stage(Stage::Reading)));
        self.jobs
            .push_back((t + 0.1, Job::Stage(Stage::Rendering { triangles: 12_480 })));
        self.jobs.push_back((t + 0.3, Job::Stage(Stage::Finishing)));
        let done = if std::mem::take(&mut self.fail_next) {
            Job::Fail
        } else {
            Job::Image(view)
        };
        self.jobs.push_back((t + 0.4, done));
    }

    fn open_route(&mut self, data: RouteData) {
        self.show = Show::Route;
        self.route_px.set(
            ROUTE_PX[0] as u32,
            ROUTE_PX[1] as u32,
            route_picture(&data.samples),
        );
        self.route = RouteState::new(data, RouteLabels::default());
    }

    fn open_archive(&mut self, entered: bool) {
        self.show = Show::Archive;
        let tree = ArchiveTree::new(members(), self.ids());
        let mut archive = ArchiveState::new("c14b-archive", tree);
        if entered {
            archive.enter();
        }
        self.archive = Some(archive);
    }
}

fn open(g: &mut Gallery, dark: bool) -> &mut Demo {
    g.open("c14b", dark);
    g.c14b = Demo {
        type_to_search: true,
        ..Demo::default()
    };
    g.c14b.drawer.open_at_once();
    &mut g.c14b
}

fn image_scene(g: &mut Gallery, dark: bool) {
    let d = open(g, dark);
    d.show = Show::Image;
    d.photo.set(PHOTO[0], PHOTO[1], photo());
    d.image = ViewportState::new(Mode::Fit);
    let _ = d
        .image
        .set_content(Some([PHOTO[0] as f32, PHOTO[1] as f32]));
}

/// The first model, loaded at Front left.
fn model_loaded(g: &mut Gallery, dark: bool) {
    open(g, dark).open_model(0, None);
    let look = g.look.clone();
    let d = &mut g.c14b;
    d.jobs.clear();
    d.land(CameraView::FrontLeft, &look);
}

/// Back right renders over the Back left picture.
fn model_rendering(g: &mut Gallery, dark: bool) {
    model_loaded(g, dark);
    let look = g.look.clone();
    let d = &mut g.c14b;
    let view = d.model.step(1).expect("loaded");
    d.land(view, &look);
    d.model.step(1).expect("rendered");
    d.model.progress(Stage::Rendering { triangles: 12_480 });
}

fn model_first_load(g: &mut Gallery, dark: bool, image_at: Option<f64>) {
    open(g, dark).open_model(1, image_at);
}

fn embedded(g: &mut Gallery, dark: bool, file: usize, image_at: Option<f64>) {
    open(g, dark).open_model(file, image_at);
}

fn model_failed(g: &mut Gallery, dark: bool) {
    let d = open(g, dark);
    d.open_model(3, None);
    d.jobs.clear();
    d.model.open(ModelFile {
        name: "benchy.bgcode".into(),
        extension: "bgcode".into(),
        position: Some(Position {
            index: 4,
            count: 4,
            results: false,
        }),
        kind: ModelKind::Embedded,
    });
    d.model
        .load_failed("This G-code file has no usable embedded thumbnail");
}

fn gcode(g: &mut Gallery, dark: bool) {
    embedded(g, dark, 3, None);
    let look = g.look.clone();
    let d = &mut g.c14b;
    d.jobs.clear();
    d.land(CameraView::FrontLeft, &look);
}

fn view_error(g: &mut Gallery, dark: bool) {
    model_loaded(g, dark);
    g.c14b.fail_next = true;
}

/// A timed 12 km ridge walk, from 10 to 30 m, as a result of a search.
fn ridge() -> RouteData {
    let samples = loop_samples(240, 12_000.0, Some((10.0, 30.0)));
    RouteData {
        name: "ridge.gpx".into(),
        format: "GPX".into(),
        position: Some(Position {
            index: 3,
            count: 5,
            results: true,
        }),
        distance_m: 12_000.0,
        duration_s: Some(37_070),
        elevation: Some([10.0, 30.0]),
        samples,
        image_px: ROUTE_PX,
        profile_px: PROFILE_PX,
    }
}

fn route_scene(g: &mut Gallery, dark: bool, start: bool) {
    let d = open(g, dark);
    d.open_route(ridge());
    if !start {
        d.route.command(RouteCommand::Seek(Seek::By(5.0)));
    }
}

fn route_point(g: &mut Gallery, dark: bool) {
    let d = open(g, dark);
    d.open_route(RouteData {
        name: "summit.gpx".into(),
        format: "GPX".into(),
        position: Some(Position {
            index: 1,
            count: 5,
            results: false,
        }),
        samples: vec![RouteSample {
            position: [400.0, 300.0],
            distance_m: 0.0,
            elevation_y: None,
            starts_segment: true,
        }],
        image_px: ROUTE_PX,
        profile_px: PROFILE_PX,
        ..RouteData::default()
    });
}

/// The ridge stretched to 1,000 km, so the distance's tenths change faster than frames: what
/// playback's 10 Hz text commits are for (`viewers::route`).
fn route_long(g: &mut Gallery, dark: bool) {
    let d = open(g, dark);
    let mut data = ridge();
    data.name = "traverse.gpx".into();
    data.distance_m = 1_000_000.0;
    data.samples = loop_samples(240, 1_000_000.0, Some((10.0, 30.0)));
    d.open_route(data);
}

/// An untimed KML along an estuary, from 5 m below the sea to 20 m above.
fn route_low(g: &mut Gallery, dark: bool) {
    let d = open(g, dark);
    d.open_route(RouteData {
        name: "estuary.kml".into(),
        format: "KML".into(),
        position: Some(Position {
            index: 2,
            count: 5,
            results: false,
        }),
        distance_m: 3_400.0,
        duration_s: None,
        elevation: Some([-5.0, 20.0]),
        samples: loop_samples(120, 3_400.0, Some((-5.0, 20.0))),
        image_px: ROUTE_PX,
        profile_px: PROFILE_PX,
    });
}

fn archive(g: &mut Gallery, dark: bool, entered: bool) {
    open(g, dark).open_archive(entered);
}

fn locked(g: &mut Gallery, dark: bool) {
    let d = open(g, dark);
    d.show = Show::Locked;
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c14b", dark)`, then fills in `g.c14b`.
pub const SCENES: &[Scene] = &[
    ("c14b-image-light", |g| image_scene(g, false)),
    ("c14b-image-dark", |g| image_scene(g, true)),
    ("c14b-model-light", |g| model_loaded(g, false)),
    ("c14b-model-dark", |g| model_loaded(g, true)),
    ("c14b-model-rendering-light", |g| model_rendering(g, false)),
    ("c14b-model-rendering-dark", |g| model_rendering(g, true)),
    ("c14b-model-loading-light", |g| {
        model_first_load(g, false, None)
    }),
    ("c14b-model-loading-dark", |g| {
        model_first_load(g, true, None)
    }),
    ("c14b-model-failed-light", |g| model_failed(g, false)),
    ("c14b-model-failed-dark", |g| model_failed(g, true)),
    ("c14b-gcode-light", |g| gcode(g, false)),
    ("c14b-gcode-dark", |g| gcode(g, true)),
    ("c14b-route-light", |g| route_scene(g, false, false)),
    ("c14b-route-dark", |g| route_scene(g, true, false)),
    ("c14b-route-point-light", |g| route_point(g, false)),
    ("c14b-route-point-dark", |g| route_point(g, true)),
    ("c14b-archive-light", |g| archive(g, false, true)),
    ("c14b-archive-dark", |g| archive(g, true, true)),
    ("c14b-archive-locked-light", |g| locked(g, false)),
    ("c14b-archive-locked-dark", |g| locked(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens. Scripts use some.
pub const REVIEW: &[Scene] = &[
    // A first load whose picture lands at 0.8 s.
    ("c14b-model-first-light", |g| {
        model_first_load(g, false, Some(0.8))
    }),
    ("c14b-model-first-dark", |g| {
        model_first_load(g, true, Some(0.8))
    }),
    ("c14b-model-view-error-light", |g| view_error(g, false)),
    ("c14b-model-view-error-dark", |g| view_error(g, true)),
    // A 3MF with one embedded PNG, which lands at 2.5 s.
    ("c14b-model-embedded-light", |g| {
        embedded(g, false, 2, Some(2.5))
    }),
    ("c14b-model-embedded-dark", |g| {
        embedded(g, true, 2, Some(2.5))
    }),
    // A G-code file whose slicer thumbnail lands at 2.5 s.
    ("c14b-gcode-load-light", |g| {
        embedded(g, false, 3, Some(2.5))
    }),
    ("c14b-gcode-load-dark", |g| embedded(g, true, 3, Some(2.5))),
    ("c14b-route-start-light", |g| route_scene(g, false, true)),
    ("c14b-route-start-dark", |g| route_scene(g, true, true)),
    ("c14b-route-long-light", |g| route_long(g, false)),
    ("c14b-route-long-dark", |g| route_long(g, true)),
    ("c14b-route-low-light", |g| route_low(g, false)),
    ("c14b-route-low-dark", |g| route_low(g, true)),
    ("c14b-archive-passive-light", |g| archive(g, false, false)),
    ("c14b-archive-passive-dark", |g| archive(g, true, false)),
];

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let d = &g.c14b;
    let look = g.look();
    let k = &look.kit;
    let header = match d.show {
        Show::Image => HeaderInfo::new("harbour.png")
            .icon(look.icon(IconName::Documents))
            .details(["412 KB", "Yesterday", "PNG image"]),
        Show::Model => HeaderInfo::new(d.model.file().name.clone())
            .icon(look.icon(IconName::Documents))
            .details(["2.1 MB", "Today 10:12", "3D model"]),
        Show::Route => HeaderInfo::new(d.route.data().name.clone())
            .icon(look.icon(IconName::Documents))
            .details(["86 KB", "Last week", "GPS route"]),
        Show::Archive | Show::Locked => HeaderInfo::new("project.zip")
            .icon(look.icon(IconName::Documents))
            .details(["1.4 MB", "Today 08:30", "ZIP archive"]),
        Show::None => HeaderInfo::default(),
    };
    let drawer = PreviewDrawer::new(&d.drawer, |g: &mut Gallery| &mut g.c14b.drawer)
        .arrangement(Arrangement::List)
        .header(header);
    let drawer = match d.show {
        Show::Model => drawer.hosted(&d.model, |g: &mut Gallery| &mut g.c14b.model),
        _ => drawer.hosted(&d.idle, |g: &mut Gallery| &mut g.c14b.idle),
    };
    let content = match d.show {
        Show::None => col(),
        Show::Image => ImageView::new(&d.image, |g: &mut Gallery| &mut g.c14b.image)
            .key("c14b-image")
            .pixels(&d.photo)
            .name("harbour.png")
            .size_px(PHOTO)
            .build(look, cx),
        Show::Model => ModelView::new(
            &d.model,
            |g: &mut Gallery| &mut g.c14b.model,
            |g: &mut Gallery| &mut g.c14b.model.viewport,
        )
        .key("c14b-model")
        .pixels(&d.render)
        .on_camera(|g: &mut Gallery, view, _tx| g.c14b.render_view(view))
        .build(look, cx),
        Show::Route => RouteView::new(
            &d.route,
            |g: &mut Gallery| &mut g.c14b.route,
            |g: &mut Gallery| &mut g.c14b.route.viewport,
        )
        .key("c14b-route")
        .pixels(&d.route_px)
        .build(look, cx),
        Show::Archive => match &d.archive {
            Some(_) => archive_view(g).build(g),
            None => col(),
        },
        Show::Locked => ArchivePrompt::new(&d.prompt, |g: &mut Gallery| &mut g.c14b.prompt)
            .key("c14b-prompt")
            .on_unlock(unlock)
            .build(look),
    };
    let mut files = col()
        .key("files")
        .role("listbox")
        .label("Files")
        .focusable(true)
        .w_full()
        .h_full()
        .padding(look.space.md)
        .gap(look.space.xs)
        .children(listing(d).into_iter().map(|name| {
            text(name)
                .font(k.ty.ui)
                .color(k.c.text_muted)
                .h(look.row.list)
        }));
    if !d.status.is_empty() {
        files = files.child(
            text(d.status.clone())
                .key("c14b-status")
                .font(k.ty.small)
                .color(k.c.text_accent),
        );
    }
    drawer.content(content).build(files, look, cx)
}

fn archive_view(g: &Gallery) -> ArchiveView<Gallery> {
    ArchiveView::new(
        |g: &Gallery| g.c14b.archive.as_ref().expect("an archive"),
        |g: &mut Gallery| g.c14b.archive.as_mut().expect("an archive"),
        |g: &Gallery| g.look(),
    )
    .letters(!g.c14b.type_to_search)
    .on_leave(|g: &mut Gallery, _tx| {
        g.c14b.status = "Left at the root: focus returns to the listing".into();
    })
}

/// The fake listing's names, beside the drawer.
fn listing(d: &Demo) -> Vec<String> {
    let mut names: Vec<String> = match d.show {
        Show::Model => MODELS.iter().map(|(n, _)| (*n).to_owned()).collect(),
        Show::Route => ["estuary.kml", "notes.txt", "ridge.gpx", "summit.gpx"]
            .map(str::to_owned)
            .to_vec(),
        _ => ["harbour.png", "notes.md", "project.zip"]
            .map(str::to_owned)
            .to_vec(),
    };
    names.insert(0, "Documents".into());
    names
}

fn unlock(g: &mut Gallery, password: String, _tx: &mut Tx) {
    let d = &mut g.c14b;
    if password == PASSWORD {
        d.prompt = PromptState::default();
        d.open_archive(true);
    } else {
        d.prompt
            .set_error(Some("The password is incorrect.".into()));
    }
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index. The
/// keys stand in for the host's (module docs).
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let media = key.mods.ctrl && key.mods.alt;
    let d = &mut g.c14b;
    let command = match &key.key {
        Key::Text(t) if media && t == " " => Some(RouteCommand::PlayPause),
        Key::Command(' ') if media => Some(RouteCommand::PlayPause),
        Key::Left if media => Some(RouteCommand::Seek(Seek::By(-5.0))),
        Key::Right if media => Some(RouteCommand::Seek(Seek::By(5.0))),
        _ => None,
    };
    if let Some(command) = command {
        return d.show == Show::Route && d.route.command_tx(command, tx);
    }
    if key.mods.ctrl || key.mods.alt || key.mods.cmd {
        return false;
    }
    match &key.key {
        Key::Text(t) if t == "u" && d.show == Show::Route => {
            let units = match d.route.units() {
                Units::Metric => Units::Imperial,
                Units::Imperial => Units::Metric,
            };
            d.route.set_units(units);
            true
        }
        Key::Text(t) if t == "n" && d.show == Show::Model => {
            let next = (d.file + 1) % MODELS.len();
            // A big model: its first load takes 3 s.
            d.open_model(next, Some(3.0));
            true
        }
        Key::Text(t) if t == "t" && d.show == Show::Archive => {
            d.type_to_search = !d.type_to_search;
            true
        }
        _ => false,
    }
}

/// Runs at the start of every frame while this page is open, with the clock in seconds. Hands
/// over the fake workers' results that are due, and plays the route. True when state changed.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    g.c14b.now = t;
    if g.c14b.show == Show::None {
        g.c14b.drawer.open_at_once();
        g.c14b.open_model(0, Some(0.5));
        return true;
    }
    let mut changed = false;
    while let Some(&(due, job)) = g.c14b.jobs.front() {
        if due > t {
            break;
        }
        g.c14b.jobs.pop_front();
        changed = true;
        match job {
            Job::Stage(stage) => g.c14b.model.progress(stage),
            Job::Image(view) => {
                let look = g.look.clone();
                g.c14b.land(view, &look);
            }
            Job::Fail => g
                .c14b
                .model
                .view_failed("The sandboxed preview renderer failed"),
        }
    }
    if g.c14b.show == Show::Route {
        changed |= g.c14b.route.tick(t, tx);
    }
    changed
}

// Fake data.

/// The archive's members, in Strata's order: folders first, then files, each by name.
fn members() -> ArchiveDir {
    let file = |name: &str, size: u64, icon: &str| ArchiveItem::File {
        name: name.into(),
        size,
        icon: icon.into(),
    };
    let dir = |name: &str, children: Vec<ArchiveItem>| {
        ArchiveItem::Dir(ArchiveDir {
            name: name.into(),
            children,
        })
    };
    ArchiveDir {
        name: String::new(),
        children: vec![
            dir(
                "assets",
                vec![
                    dir(
                        "icons",
                        vec![file("app.png", 18_200, ""), file("logo.svg", 3_400, "")],
                    ),
                    file("styles.css", 6_120, ""),
                ],
            ),
            dir(
                "docs",
                vec![file("api.md", 22_800, ""), file("guide.md", 9_640, "")],
            ),
            dir("empty", vec![]),
            file("build.rs", 1_210, ""),
            file("Cargo.toml", 880, ""),
            file("LICENSE", 1_070, ""),
            file("README.md", 12_400, ""),
        ],
    }
}

/// `n` samples round a lopsided loop with a tail, `total` metres long, heights between `range`.
fn loop_samples(n: usize, total: f64, range: Option<(f64, f64)>) -> Vec<RouteSample> {
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            let a = t * std::f64::consts::TAU;
            let x = 400.0 + 280.0 * a.sin() + 40.0 * (3.0 * a).sin();
            let y = 300.0 - 200.0 * a.cos() + 30.0 * (2.0 * a).sin();
            let elevation_y = range.map(|_| {
                // A climb to the far side and back, with a dip.
                let h = 0.5 - 0.5 * a.cos() - 0.15 * (3.0 * a).sin().max(0.0);
                (66.0 - 62.0 * h.clamp(0.0, 1.0)) as f32
            });
            RouteSample {
                position: [x as f32, y as f32],
                distance_m: t * total,
                elevation_y,
                starts_segment: i == 0,
            }
        })
        .collect()
}

/// RGBA for a `w` × `h` image from `f(x, y)`.
fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            out.extend_from_slice(&f(x, y));
        }
    }
    out
}

/// `c` (linear light) darkened by `shade`, as opaque sRGB bytes, as a PNG holds them.
fn rgba(c: Color, shade: f32) -> [u8; 4] {
    let shaded = Color {
        r: c.r * shade,
        g: c.g * shade,
        b: c.b * shade,
        a: 1.0,
    };
    let [r, g, b, _] = shaded.premul_srgb(1.0);
    let u = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    [u(r), u(g), u(b), 255]
}

/// The route line in the accent, on a clear background, as the helper draws it.
fn route_picture(samples: &[RouteSample]) -> Vec<u8> {
    let [w, h] = [ROUTE_PX[0] as usize, ROUTE_PX[1] as usize];
    let mut out = vec![0u8; w * h * 4];
    let mut stamp = |x: f32, y: f32| {
        for dy in -2i32..=2 {
            for dx in -2i32..=2 {
                if dx * dx + dy * dy > 5 {
                    continue;
                }
                let (px, py) = (x as i32 + dx, y as i32 + dy);
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    let i = (py as usize * w + px as usize) * 4;
                    out[i..i + 4].copy_from_slice(&[64, 120, 230, 255]);
                }
            }
        }
    };
    for pair in samples.windows(2) {
        let (a, b) = (pair[0].position, pair[1].position);
        let steps = ((b[0] - a[0]).hypot(b[1] - a[1]) as usize).max(1);
        for s in 0..=steps {
            let t = s as f32 / steps as f32;
            stamp(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
        }
    }
    if let [only] = samples {
        stamp(only.position[0], only.position[1]);
    }
    out
}

/// A shaded box on the theme's surface, its lit faces turning with the camera: a stand-in for
/// Strata's software render.
fn render(view: CameraView, look: &Look) -> Vec<u8> {
    let k = &look.kit;
    let (bg, accent) = (k.c.bg_base, k.c.text_accent);
    let s = RENDER as f32;
    // Which face is lit depends on the view, so each view looks different.
    let lit = match view {
        CameraView::FrontLeft => [1.0, 0.75, 0.55],
        CameraView::BackLeft => [0.55, 1.0, 0.75],
        CameraView::BackRight => [0.75, 0.55, 1.0],
        CameraView::FrontRight => [1.0, 0.55, 0.75],
    };
    image(RENDER, RENDER, |x, y| {
        let (u, v) = (x as f32 / s - 0.5, y as f32 / s - 0.5);
        // An isometric cube: a top rhombus centred 0.12 up, and two side faces hanging 0.3
        // below its lower edges.
        let top = (u.abs() * 0.58 + (v + 0.12).abs()) < 0.2;
        let lower_edge = 0.08 - u.abs() * 0.58;
        let side = u.abs() < 0.345 && v >= lower_edge && v < lower_edge + 0.3;
        if top {
            rgba(accent, lit[0])
        } else if side && u < 0.0 {
            rgba(accent, lit[1] * 0.8)
        } else if side {
            rgba(accent, lit[2] * 0.65)
        } else {
            rgba(bg, 1.0)
        }
    })
}

/// A slicer's thumbnail: a boat on a clear plate.
fn thumbnail() -> Vec<u8> {
    let [w, h] = THUMBNAIL;
    image(w, h, |x, y| {
        let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
        let plate = v > 0.78 && v < 0.86 && u > 0.08 && u < 0.92;
        let hull = v > 0.55 && v < 0.78 && (u - 0.5).abs() < 0.32 - (v - 0.55) * 0.6;
        let cabin = v > 0.3 && v <= 0.55 && u > 0.38 && u < 0.62;
        if hull || cabin {
            [235, 130, 45, 255]
        } else if plate {
            [80, 84, 96, 255]
        } else {
            [0, 0, 0, 0]
        }
    })
}

/// A harbour at dusk with a clear window, which the checkerboard shows through.
fn photo() -> Vec<u8> {
    let [w, h] = PHOTO;
    image(w, h, |x, y| {
        let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
        let mix = |a: f32, b: f32, t: f32| (a + (b - a) * t).clamp(0.0, 255.0) as u8;
        let c = if fy < 0.6 {
            let t = fy / 0.6;
            [
                mix(250.0, 80.0, t),
                mix(170.0, 90.0, t),
                mix(100.0, 170.0, t),
            ]
        } else {
            let t = (fy - 0.6) / 0.4;
            [mix(30.0, 10.0, t), mix(80.0, 40.0, t), mix(120.0, 80.0, t)]
        };
        let (cx, cy) = (fx - 0.3, (fy - 0.35) * h as f32 / w as f32);
        let alpha = if cx * cx + cy * cy < 0.01 { 0 } else { 255 };
        [c[0], c[1], c[2], alpha]
    })
}

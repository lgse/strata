//! The model view's state: the file, the camera, the load and render stages, and the picture's
//! viewport. The host renders; this only holds what the view shows and decides which presses
//! count.

use std::f32::consts::{PI, TAU};

use crate::viewers::layout::Position;
use crate::viewers::viewport::{Mode, ViewportState, ZoomReport};

/// One of the four camera presets, named by where the camera stands: +X right, −Y front, +Z up
/// (Strata's `ModelView`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CameraView {
    /// Where every file starts.
    #[default]
    FrontLeft,
    FrontRight,
    BackRight,
    BackLeft,
}

impl CameraView {
    /// Strata's order: [`CameraView::step`] walks it backwards for Next.
    pub const ALL: [CameraView; 4] = [
        CameraView::FrontLeft,
        CameraView::FrontRight,
        CameraView::BackRight,
        CameraView::BackLeft,
    ];

    fn index(self) -> usize {
        match self {
            CameraView::FrontLeft => 0,
            CameraView::FrontRight => 1,
            CameraView::BackRight => 2,
            CameraView::BackLeft => 3,
        }
    }

    /// `step` presses of Next (negative: Previous). Next turns the camera clockwise seen from
    /// above: Front left, Back left, Back right, Front right, then Front left again.
    pub fn step(self, step: i32) -> CameraView {
        let i = (self.index() as i32 - step.rem_euclid(4)).rem_euclid(4);
        CameraView::ALL[i as usize]
    }

    /// From the model's centre toward the camera, as Strata's renderer and indicator share it.
    pub fn direction(self) -> [f32; 3] {
        match self {
            CameraView::FrontLeft => [-1.0, -1.0, 1.0],
            CameraView::FrontRight => [1.0, -1.0, 1.0],
            CameraView::BackRight => [1.0, 1.0, 1.0],
            CameraView::BackLeft => [-1.0, 1.0, 1.0],
        }
    }

    /// Where the indicator's dot sits, as a clockwise angle in radians from the right, in screen
    /// space (y down) seen from above: Front left is lower left.
    pub fn angle(self) -> f32 {
        let [x, y, _] = self.direction();
        (-y).atan2(x)
    }
}

/// What the file is: a mesh the host renders per camera view, or a picture saved in the file
/// (a 3MF's one embedded PNG, a FreeCAD thumbnail, a G-code slicer thumbnail), which once
/// loaded shows alone, with no camera controls, indicator or badges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ModelKind {
    #[default]
    Mesh,
    Embedded,
}

/// Where a load or a render stands, for the drawer's spinner (Strata's `ModelPreviewStage`, and
/// the "Loading preview…" it starts with).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    Loading,
    Reading,
    /// Reading an embedded picture.
    Thumbnail,
    Rendering {
        triangles: usize,
    },
    Finishing,
}

/// How the mesh is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Encoding {
    BinaryStl,
    AsciiStl,
    ThreeMf,
}

/// What a finished render says about the mesh (Strata's `ModelMesh`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Mesh {
    /// The view the picture was rendered from.
    pub view: CameraView,
    pub encoding: Encoding,
    pub triangles: usize,
}

/// The file the view shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelFile {
    /// The display name.
    pub name: String,
    /// The extension as the name has it ("stl"): the format badge shows it in upper case until
    /// the mesh's own format is known.
    pub extension: String,
    /// Its place among the model files around it, if the host counts one.
    pub position: Option<Position>,
    pub kind: ModelKind,
}

/// The words the model view shows and assistive technology reads. Strata passes translated
/// text; the defaults are Strata's English.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ModelLabels {
    pub picture: String,
    pub previous: String,
    pub next: String,
    pub camera: String,
    pub unavailable: String,
    pub view: fn(CameraView) -> String,
    pub stage: fn(Stage) -> String,
    pub encoding: fn(Encoding) -> String,
    pub triangles: fn(usize) -> String,
}

impl Default for ModelLabels {
    fn default() -> Self {
        ModelLabels {
            picture: "Model preview".into(),
            previous: "Previous model view".into(),
            next: "Next model view".into(),
            camera: "Camera position".into(),
            unavailable: "Preview unavailable".into(),
            view: |v| {
                match v {
                    CameraView::FrontLeft => "Front left",
                    CameraView::FrontRight => "Front right",
                    CameraView::BackRight => "Back right",
                    CameraView::BackLeft => "Back left",
                }
                .into()
            },
            stage: |s| match s {
                Stage::Loading => "Loading preview…".into(),
                Stage::Reading => "Reading model…".into(),
                Stage::Thumbnail => "Reading thumbnail…".into(),
                Stage::Rendering { triangles } => match triangles {
                    1 => "Rendering 1 triangle…".into(),
                    n => format!("Rendering {} triangles…", grouped(n)),
                },
                Stage::Finishing => "Finishing preview…".into(),
            },
            encoding: |e| {
                match e {
                    Encoding::BinaryStl => "Binary STL",
                    Encoding::AsciiStl => "ASCII STL",
                    Encoding::ThreeMf => "3MF",
                }
                .into()
            },
            triangles: |n| match n {
                1 => "1 triangle".into(),
                n => format!("{} triangles", grouped(n)),
            },
        }
    }
}

/// `12480` as "12,480".
pub(crate) fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The model view's state. The host keeps one per preview and drives it as its loads and
/// renders report: [`ModelState::open`] for a new file, then [`ModelState::progress`],
/// [`ModelState::image`], [`ModelState::view_failed`] or [`ModelState::load_failed`]. A camera
/// button calls [`ModelState::step`], which says which view to render next.
#[derive(Debug)]
pub struct ModelState {
    /// The picture's viewport (Fit for a mesh, ScaleDown for an embedded picture).
    pub viewport: ViewportState,
    file: ModelFile,
    labels: ModelLabels,
    /// The view asked for: what the indicator shows.
    camera: CameraView,
    /// The view the picture shows: a failed render goes back to it.
    shown: CameraView,
    /// The indicator's angle, unwrapped, so each turn takes the shorter way round.
    angle: f32,
    /// No camera button has turned the camera since the file opened: the indicator lands at its
    /// angle rather than sweeping there (Strata builds a fresh indicator per file).
    fresh: bool,
    /// Some while a first load or a view render runs.
    stage: Option<Stage>,
    /// The spinner's label for `stage`, kept so [`ModelState::busy`] can lend it.
    busy: Option<String>,
    has_image: bool,
    mesh: Option<Mesh>,
    /// A triangle count a render reported before its picture.
    triangles: Option<usize>,
    /// A view render or decode that failed after the first picture.
    error: Option<String>,
    /// A first load that failed.
    failed: Option<String>,
}

impl Default for ModelState {
    fn default() -> Self {
        ModelState::new(ModelLabels::default())
    }
}

impl ModelState {
    /// No file yet, with `labels`.
    pub fn new(labels: ModelLabels) -> ModelState {
        ModelState {
            viewport: ViewportState::new(Mode::Fit),
            file: ModelFile::default(),
            labels,
            camera: CameraView::FrontLeft,
            shown: CameraView::FrontLeft,
            angle: CameraView::FrontLeft.angle(),
            fresh: true,
            stage: None,
            busy: None,
            has_image: false,
            mesh: None,
            triangles: None,
            error: None,
            failed: None,
        }
    }

    /// A new file: back to Front left, and loading. The camera and the indicator start over at
    /// once: the indicator lands at Front left rather than sweeping from the last file's view
    /// ([`ModelState::fresh`]).
    pub fn open(&mut self, file: ModelFile) {
        let mode = match file.kind {
            ModelKind::Mesh => Mode::Fit,
            ModelKind::Embedded => Mode::ScaleDown,
        };
        let labels = std::mem::take(&mut self.labels);
        *self = ModelState::new(labels);
        self.viewport = ViewportState::new(mode);
        self.file = file;
        self.set_stage(Some(Stage::Loading));
    }

    /// The load or render reached `stage`. Ignored while nothing loads.
    pub fn progress(&mut self, stage: Stage) {
        if self.stage.is_none() {
            return;
        }
        if let Stage::Rendering { triangles } = stage {
            self.triangles = Some(triangles);
        }
        self.set_stage(Some(stage));
    }

    /// The picture arrived: the host has set its `Pixels`, `content_px` big. A mesh render
    /// says which view it shows and what the mesh is; an embedded picture passes `None`.
    /// Returns the viewport's report, if its level changed.
    pub fn image(&mut self, content_px: [f32; 2], mesh: Option<Mesh>) -> Option<ZoomReport> {
        self.has_image = true;
        if let Some(mesh) = mesh {
            self.triangles = Some(mesh.triangles);
            self.turn_to(mesh.view);
            self.shown = mesh.view;
            self.mesh = Some(mesh);
        }
        self.error = None;
        self.set_stage(None);
        self.viewport.set_content(Some(content_px))
    }

    /// A view render or a decode failed after the first picture: the picture stays, the
    /// indicator goes back to its view, and the error shows under the controls. A camera button
    /// tries again.
    pub fn view_failed(&mut self, message: impl Into<String>) {
        if !self.has_image {
            self.load_failed(message);
            return;
        }
        self.turn_to(self.shown);
        self.error = Some(message.into());
        self.set_stage(None);
    }

    /// The first load failed: "Preview unavailable" and `message`, with no camera controls.
    pub fn load_failed(&mut self, message: impl Into<String>) {
        self.failed = Some(message.into());
        self.set_stage(None);
    }

    /// A camera button: `step` is +1 for Next, −1 for Previous. Before the first picture, and
    /// while a render runs, a press does nothing (`None`). Otherwise the camera turns, a render
    /// starts, and this returns the view to render.
    pub fn step(&mut self, step: i32) -> Option<CameraView> {
        if !self.can_step() || self.stage.is_some() {
            return None;
        }
        let view = self.camera.step(step);
        self.fresh = false;
        self.turn_to(view);
        self.error = None;
        self.set_stage(Some(Stage::Loading));
        Some(view)
    }

    /// Whether the camera buttons are sensitive: once the first picture is there.
    pub fn can_step(&self) -> bool {
        self.has_image && self.failed.is_none()
    }

    fn turn_to(&mut self, view: CameraView) {
        let previous = self.camera;
        self.camera = view;
        let turn = (view.angle() - previous.angle() + PI).rem_euclid(TAU) - PI;
        self.angle += turn;
    }

    fn set_stage(&mut self, stage: Option<Stage>) {
        self.stage = stage;
        self.busy = stage.map(self.labels.stage);
    }

    pub fn file(&self) -> &ModelFile {
        &self.file
    }

    pub fn labels(&self) -> &ModelLabels {
        &self.labels
    }

    pub fn camera(&self) -> CameraView {
        self.camera
    }

    /// The indicator's unwrapped angle, in radians.
    pub fn angle(&self) -> f32 {
        self.angle
    }

    /// No camera button has turned the camera since the file opened: the indicator shows its
    /// angle at once, with no sweep.
    pub fn fresh(&self) -> bool {
        self.fresh
    }

    pub fn stage(&self) -> Option<Stage> {
        self.stage
    }

    pub fn has_image(&self) -> bool {
        self.has_image
    }

    /// A render runs over a picture already shown.
    pub fn rendering(&self) -> bool {
        self.has_image && self.stage.is_some()
    }

    pub fn mesh(&self) -> Option<Mesh> {
        self.mesh
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    /// Whether the picture shows alone, with nothing around it: an embedded picture, loaded.
    pub fn plain(&self) -> bool {
        self.file.kind == ModelKind::Embedded && self.has_image
    }

    /// The format badge: the mesh's format once known, else the extension in upper case.
    pub fn format_badge(&self) -> String {
        match self.mesh {
            Some(mesh) => (self.labels.encoding)(mesh.encoding),
            None => self.file.extension.to_uppercase(),
        }
    }

    /// The triangle badge, once a render reported a count.
    pub fn triangle_badge(&self) -> Option<String> {
        self.triangles.map(self.labels.triangles)
    }

    /// The spinner's label while something loads: the drawer's header shows it
    /// ([`HostedPreview::busy`](crate::preview_drawer::HostedPreview::busy)).
    pub fn busy(&self) -> Option<&str> {
        self.busy.as_deref()
    }
}

impl crate::preview_drawer::HostedPreview for ModelState {
    fn busy(&self) -> Option<&str> {
        ModelState::busy(self)
    }
}

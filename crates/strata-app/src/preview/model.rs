// SPDX-License-Identifier: MIT

use std::{cell::Cell, path::Path, rc::Rc};

use rune::{Cx, El, Pixels, Tx};
use strata_components::viewers::{
    model::{
        CameraView, Encoding, Mesh, ModelFile, ModelKind, ModelLabels, ModelState, ModelView, Stage,
    },
    viewport::ViewportState,
};
use strata_core::{
    i18n::{count, tr},
    model::FileEntry,
};
use strata_fs::services::{
    LoadHandle, MediaPreviewSize, ModelEncoding, ModelMesh, ModelPalette, ModelPreviewStage,
    ModelView as Camera, Preview, PreviewContent, PreviewEvent, PreviewRequest, PreviewRequestId,
};
use strata_kit::{look::Look, theme::Rgba8};

use super::{
    labels, load,
    viewer::{self, Viewer},
};
use crate::App;

/// Key of the model viewer inside the preview drawer.
pub(crate) const MODEL_KEY: &str = "preview-model";

thread_local! {
    static NEXT_VIEWER: Cell<u64> = const { Cell::new(1) };
}

pub(super) struct ModelViewer {
    pub(super) state: ModelState,
    pixels: Pixels,
    id: u64,
    entry: FileEntry,
    render: Option<LoadHandle>,
    next_request: u64,
}

impl ModelViewer {
    pub(super) fn stop(&mut self) {
        self.render = None;
    }

    fn shows(&self, entry: &FileEntry) -> bool {
        self.entry.location == entry.location
    }
}

pub(super) fn opening(app: &App, entry: &FileEntry) -> Option<Viewer> {
    if !strata_fs::services::is_model(entry) {
        return None;
    }
    let mut state = ModelState::new(model_labels());
    state.open(file(app, entry, ModelKind::Mesh));
    Some(Viewer::Model(Box::new(ModelViewer {
        state,
        pixels: Pixels::new(),
        id: NEXT_VIEWER.with(|next| next.replace(next.get().wrapping_add(1))),
        entry: entry.clone(),
        render: None,
        next_request: 1,
    })))
}

fn file(app: &App, entry: &FileEntry, kind: ModelKind) -> ModelFile {
    let extension = Path::new(&entry.native_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_owned();
    ModelFile {
        name: entry.display_name.clone(),
        extension,
        position: labels::listing_position(app, entry, strata_fs::services::is_model),
        kind,
    }
}

/// A stage of the provider's load, for the view's spinner.
pub(super) fn progress(app: &mut App, stage: ModelPreviewStage) -> bool {
    let Viewer::Model(model) = &mut app.preview.viewer else {
        return false;
    };
    model.state.progress(stage_of(stage));
    true
}

fn stage_of(stage: ModelPreviewStage) -> Stage {
    match stage {
        ModelPreviewStage::Reading => Stage::Reading,
        ModelPreviewStage::Thumbnail => Stage::Thumbnail,
        ModelPreviewStage::Rendering { triangles } => Stage::Rendering { triangles },
        ModelPreviewStage::Finishing => Stage::Finishing,
    }
}

pub(super) fn ready(
    app: &mut App,
    entry: FileEntry,
    png: Vec<u8>,
    mesh: Option<ModelMesh>,
) -> Viewer {
    let viewer = match std::mem::replace(&mut app.preview.viewer, Viewer::Empty) {
        Viewer::Model(model) if model.shows(&entry) => Viewer::Model(model),
        _ => opening(app, &entry).unwrap_or(Viewer::Loading),
    };
    if let Viewer::Model(model) = &viewer {
        decode(app, model.id, png, mesh);
    }
    viewer
}

pub(super) fn palette(look: &Look) -> ModelPalette {
    let rgb = |color| {
        let Rgba8 { r, g, b, .. } = Rgba8::from_color(color);
        (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
    };
    ModelPalette {
        accent: rgb(look.glow.color),
        surface: rgb(look.kit.c.surface_raised),
    }
}

/// A camera button turned the camera: the provider renders that view.
pub(super) fn render_view(app: &mut App, view: CameraView, _: &mut Tx) {
    let provider = app.preview.loader.provider();
    let later = app.preview.loader.later.clone();
    let palette = palette(&app.look);
    let Viewer::Model(model) = &mut app.preview.viewer else {
        return;
    };
    let edge = load::DEFAULT_MEDIA_EDGE;
    let serial = model.next_request;
    model.next_request = serial.wrapping_add(1);
    let id = model.id;
    let request = PreviewRequest {
        // Apart from the preview's own requests: the reply is routed by its closure.
        id: PreviewRequestId(u64::MAX / 4 + serial),
        entry: model.entry.clone(),
        text_byte_limit: load::TEXT_BYTE_LIMIT,
        render_document: false,
        pdf_page: 0,
        media_size: MediaPreviewSize::for_viewport(edge, edge, 1),
        model_palette: palette,
        model_view: camera(view),
        archive_password: None,
    };
    let emit = Rc::new(move |event: PreviewEvent| {
        later.then(move |app: &mut App, _: &mut Tx| rendered(app, id, event));
    });
    model.render = Some(provider.load(request, emit));
}

fn rendered(app: &mut App, id: u64, event: PreviewEvent) -> bool {
    let Some(model) = viewer_mut(app, id) else {
        return false;
    };
    match event {
        PreviewEvent::Progress { stage, .. } => {
            model.state.progress(stage_of(stage));
            return true;
        }
        PreviewEvent::Ready(Preview {
            content: PreviewContent::Model { png, mesh },
            ..
        }) => {
            model.render = None;
            decode(app, id, png, mesh);
            return false;
        }
        PreviewEvent::Failed { message, .. } => model.state.view_failed(tr(&message)),
        _ => model.state.view_failed(tr("Preview unavailable")),
    }
    model.render = None;
    true
}

/// Decodes `png` off the UI thread; the viewer `id` takes it if it is still shown.
fn decode(app: &App, id: u64, png: Vec<u8>, mesh: Option<ModelMesh>) {
    let later = app.preview.loader.later.clone();
    glib::MainContext::default().spawn_local(async move {
        let decoded = gio::spawn_blocking(move || {
            image::load_from_memory(&png)
                .map(|image| image.to_rgba8())
                .map_err(|error| error.to_string())
        })
        .await;
        let decoded = decoded.unwrap_or_else(|_| Err(tr("Preview unavailable")));
        later.then(move |app: &mut App, _: &mut Tx| show(app, id, decoded, mesh));
    });
}

fn show(
    app: &mut App,
    id: u64,
    decoded: Result<image::RgbaImage, String>,
    mesh: Option<ModelMesh>,
) -> bool {
    let embedded = mesh.is_none();
    let opened = embedded.then(|| {
        let entry = viewer_mut(app, id).map(|model| model.entry.clone());
        entry.map(|entry| file(app, &entry, ModelKind::Embedded))
    });
    let Some(model) = viewer_mut(app, id) else {
        return false;
    };
    let image = match decoded {
        Ok(image) => image,
        Err(message) => {
            model.state.view_failed(message);
            return true;
        }
    };
    if let Some(Some(file)) = opened {
        model.state.open(file);
    }
    let (width, height) = image.dimensions();
    model.pixels.set(width, height, image.into_raw());
    let size = [width as f32, height as f32];
    let _ = model.state.image(size, mesh.map(mesh_of));
    true
}

fn mesh_of(mesh: ModelMesh) -> Mesh {
    Mesh {
        view: camera_view(mesh.view),
        encoding: match mesh.encoding {
            ModelEncoding::BinaryStl => Encoding::BinaryStl,
            ModelEncoding::AsciiStl => Encoding::AsciiStl,
            ModelEncoding::ThreeMf => Encoding::ThreeMf,
        },
        triangles: mesh.triangles,
    }
}

fn camera(view: CameraView) -> Camera {
    match view {
        CameraView::FrontLeft => Camera::FrontLeft,
        CameraView::FrontRight => Camera::FrontRight,
        CameraView::BackRight => Camera::BackRight,
        CameraView::BackLeft => Camera::BackLeft,
    }
}

fn camera_view(view: Camera) -> CameraView {
    match view {
        Camera::FrontLeft => CameraView::FrontLeft,
        Camera::FrontRight => CameraView::FrontRight,
        Camera::BackRight => CameraView::BackRight,
        Camera::BackLeft => CameraView::BackLeft,
    }
}

fn viewer_mut(app: &mut App, id: u64) -> Option<&mut ModelViewer> {
    match &mut app.preview.viewer {
        Viewer::Model(model) if model.id == id => Some(model),
        _ => None,
    }
}

fn model_labels() -> ModelLabels {
    let mut labels = ModelLabels::default();
    labels.picture = tr("Model preview");
    labels.previous = tr("Previous model view");
    labels.next = tr("Next model view");
    labels.camera = tr("Camera position");
    labels.unavailable = tr("Preview unavailable");
    labels.view = |view| {
        tr(match view {
            CameraView::FrontLeft => "Front left",
            CameraView::FrontRight => "Front right",
            CameraView::BackRight => "Back right",
            CameraView::BackLeft => "Back left",
        })
    };
    labels.stage = |stage| match stage {
        Stage::Loading => tr("Loading preview…"),
        Stage::Reading => tr("Reading model…"),
        Stage::Thumbnail => tr("Reading thumbnail…"),
        Stage::Rendering { triangles } => count("rendering_triangles", triangles),
        Stage::Finishing => tr("Finishing preview…"),
    };
    labels.encoding = |encoding| match encoding {
        Encoding::BinaryStl => tr("Binary STL"),
        Encoding::AsciiStl => tr("ASCII STL"),
        Encoding::ThreeMf => "3MF".to_owned(),
    };
    labels.triangles = |triangles| count("triangles", triangles);
    labels.position = labels::position_caption;
    labels
}

pub(super) fn build_view(model: &ModelViewer, app: &App, cx: &mut Cx<App>) -> El {
    ModelView::new(&model.state, model_mut, viewport_mut)
        .key(MODEL_KEY)
        .pixels(&model.pixels)
        .on_camera(render_view)
        .drag_source(crate::dnd::binder(app), crate::dnd::preview_drag_source())
        .build(&app.look, cx)
}

fn model_mut(app: &mut App) -> &mut ModelState {
    let preview = &mut app.preview;
    match &mut preview.viewer {
        Viewer::Model(model) => &mut model.state,
        _ => viewer::spare("model", &mut preview.detached.model),
    }
}

fn viewport_mut(app: &mut App) -> &mut ViewportState {
    let preview = &mut app.preview;
    match &mut preview.viewer {
        Viewer::Model(model) => &mut model.state.viewport,
        _ => viewer::spare("model", &mut preview.detached.viewport),
    }
}

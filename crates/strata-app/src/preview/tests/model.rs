// SPDX-License-Identifier: MIT

use rune::Tx;
use strata_components::viewers::model::{CameraView, ModelState};
use strata_core::model::Location;
use strata_fs::services::{
    ModelEncoding, ModelMesh, ModelPreviewStage, ModelView as Camera, PreviewContent,
};

use super::{
    MODULE, Viewer, disable_auto_preview, focus_view,
    provider::{Recorded, install, listed_with, png},
    select, space,
};
use crate::{
    App,
    host::offscreen::Offscreen,
    test_support::{PATIENCE, hosted},
};

const STL: &str = "solid sample\nendsolid\n";

fn state(host: &Offscreen) -> Option<&ModelState> {
    match &host.app().preview.viewer {
        Viewer::Model(model) => Some(&model.state),
        _ => None,
    }
}

fn shows(host: &mut Offscreen, done: impl Fn(&ModelState) -> bool, what: &str) {
    let shown = host.run_until(PATIENCE, |host| state(host).is_some_and(&done));
    assert!(shown, "the model view: {what}; it shows {:?}", state(host));
}

fn mesh(view: Camera) -> PreviewContent {
    PreviewContent::Model {
        png: png(64, 64),
        mesh: Some(ModelMesh {
            view,
            encoding: ModelEncoding::AsciiStl,
            triangles: 1,
        }),
    }
}

/// Opens the preview on `name` in a folder holding `files`, with `provider` answering.
fn preview(
    host: &mut Offscreen,
    provider: &Recorded,
    files: &[(&str, &str)],
    name: &str,
) -> tempfile::TempDir {
    install(host, provider);
    let folder = listed_with(host, files);
    disable_auto_preview(host);
    select(host, &Location::local(folder.path().join(name)));
    focus_view(host);
    space(host);
    provider.wait_for(host, 0);
    folder
}

/// Turns the camera one step, as the Next button does.
fn next(host: &mut Offscreen) {
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        let view = match &mut app.preview.viewer {
            Viewer::Model(model) => model.state.step(1),
            _ => None,
        };
        if let Some(view) = view {
            super::super::model::render_view(app, view, tx);
        }
    });
    while host.turn() {}
}

#[test]
fn a_model_renders_each_camera_view_and_drops_a_render_for_a_viewer_that_has_gone() {
    hosted(
        MODULE,
        "model::a_model_renders_each_camera_view_and_drops_a_render_for_a_viewer_that_has_gone",
        |mut host| {
            let provider = Recorded::default();
            let files = [("sample.stl", STL), ("other.stl", STL)];
            let _folder = preview(&mut host, &provider, &files, "sample.stl");
            shows(
                &mut host,
                |s| s.busy().is_some() && !s.has_image(),
                "its frame, loading",
            );
            let place = state(&host).and_then(|s| s.file().position);
            assert_eq!(
                place.map(|p| (p.index, p.count)),
                Some((2, 2)),
                "2 of 2 models"
            );

            let rendering = ModelPreviewStage::Rendering { triangles: 1 };
            provider.progress(0, rendering);
            shows(
                &mut host,
                |s| s.busy() == Some("Rendering 1 triangle…"),
                "the stage",
            );
            provider.answer(0, mesh(Camera::FrontLeft));
            shows(&mut host, ModelState::has_image, "the picture");
            let badges = state(&host).map(|s| (s.format_badge(), s.triangle_badge()));
            let shown = Some(("ASCII STL".to_owned(), Some("1 triangle".to_owned())));
            assert_eq!(badges, shown, "the badges");

            next(&mut host);
            provider.wait_for(&mut host, 1);
            assert_eq!(provider.request(1).model_view, Camera::BackLeft, "Next");
            provider.answer(1, mesh(Camera::BackLeft));
            shows(
                &mut host,
                |s| s.camera() == CameraView::BackLeft && !s.rendering(),
                "Back left",
            );

            next(&mut host);
            provider.wait_for(&mut host, 2);
            provider.fail(2, "The model renderer failed");
            shows(
                &mut host,
                |s| s.error().is_some() && s.has_image(),
                "the error over the picture",
            );

            next(&mut host);
            provider.wait_for(&mut host, 3);
            host.runtime().update(|app: &mut App, _: &mut Tx| {
                let entry = app.preview.current.clone().expect("a previewed file");
                app.preview.viewer = super::super::model::opening(app, &entry).expect("a model");
            });
            provider.answer(3, mesh(Camera::BackRight));
            for _ in 0..20 {
                host.turn();
            }
            assert!(
                state(&host).is_some_and(|s| !s.has_image()),
                "a render for the old viewer"
            );
        },
    );
}

#[test]
fn a_picture_saved_in_a_model_file_shows_alone() {
    hosted(
        MODULE,
        "model::a_picture_saved_in_a_model_file_shows_alone",
        |mut host| {
            let provider = Recorded::default();
            let _folder = preview(&mut host, &provider, &[("sample.3mf", "")], "sample.3mf");
            let embedded = PreviewContent::Model {
                png: png(32, 32),
                mesh: None,
            };
            provider.answer(0, embedded);
            shows(&mut host, ModelState::plain, "the picture alone");
            assert!(
                state(&host).is_some_and(|s| s.mesh().is_none()),
                "no mesh, no badges"
            );
        },
    );
}

#[test]
fn a_load_that_fails_at_once_leaves_no_report_for_the_frame() {
    hosted(
        MODULE,
        "model::a_load_that_fails_at_once_leaves_no_report_for_the_frame",
        |mut host| {
            let provider = Recorded::default();
            install(&mut host, &provider);
            let folder = listed_with(&mut host, &[("oversized.stl", STL)]);
            disable_auto_preview(&mut host);
            select(
                &mut host,
                &Location::local(folder.path().join("oversized.stl")),
            );
            focus_view(&mut host);
            crate::test_support::send_key(
                &mut host,
                rune::KeyEvent {
                    key: rune::Key::Text(" ".to_owned()),
                    mods: rune::Mods::default(),
                },
            );
            while provider.count() == 0 {
                host.turn();
            }
            assert!(state(&host).is_some(), "the frame");
            provider.fail(0, "This model exceeds the 128 MiB preview limit.");
            while host.turn() {}
            assert!(matches!(host.app().preview.viewer, Viewer::Message { .. }));
        },
    );
}

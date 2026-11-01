// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{ModelEncoding, ModelMesh, ModelView};

pub(super) fn ready(provider: &Provider, index: usize, view: Option<ModelView>) {
    let pending = provider.0.borrow();
    let pending = &pending[index];
    let mut image = resvg::tiny_skia::Pixmap::new(16, 16).expect("image");
    image.fill(resvg::tiny_skia::Color::from_rgba8(100, 150, 200, 255));
    (pending.emit)(PreviewEvent::Ready(Preview {
        request_id: pending.request.id,
        entry: pending.request.entry.clone(),
        content_type: "model/stl".into(),
        content: PreviewContent::Model {
            png: image.encode_png().expect("PNG"),
            mesh: view.map(|view| ModelMesh {
                view,
                encoding: ModelEncoding::BinaryStl,
                triangles: 23,
            }),
        },
    }));
}

#[test]
fn initial_mesh_load_shows_details_before_the_image_arrives() {
    crate::test_support::gtk_test(
        "ui::preview::tests::model::initial_mesh_load_shows_details_before_the_image_arrives",
        || {
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            drawer.show(entry("mesh.stl"), None);
            {
                let model = drawer.state.model.borrow();
                let model = model.as_ref().expect("model UI before the first image");
                assert!(model.picture.paintable().is_none());
                assert_eq!(model.title.text(), "mesh.stl");
                assert_eq!(model.format.text(), "STL");
                assert!(!model.triangles.is_visible());
                assert!(
                    drawer.state.model_spinner.is_visible(),
                    "the header shows the first load"
                );
                assert!(!model.previous.is_sensitive());
                assert!(!model.next.is_sensitive());
                model.next.emit_clicked();
                assert_eq!(provider.0.borrow().len(), 1);
            }
            {
                let pending = provider.0.borrow();
                (pending[0].emit)(PreviewEvent::Progress {
                    request_id: pending[0].request.id,
                    stage: crate::services::ModelPreviewStage::Rendering { triangles: 23 },
                });
            }
            {
                let model = drawer.state.model.borrow();
                let model = model.as_ref().expect("model metadata during rendering");
                assert!(model.picture.paintable().is_none());
                assert_eq!(model.triangles.text(), "23 triangles");
                assert!(model.triangles.is_visible());
                assert!(drawer.state.model_spinner.is_visible());
            }
            ready(&provider, 0, Some(ModelView::FrontLeft));
            {
                let model = drawer.state.model.borrow();
                let model = model.as_ref().expect("loaded model UI");
                assert!(model.picture.paintable().is_some());
                assert_eq!(model.format.text(), "Binary STL");
                assert!(!drawer.state.model_spinner.is_visible());
                assert!(model.previous.is_sensitive());
                assert!(model.next.is_sensitive());
            }
            drawer.close();
        },
    );
}

#[test]
fn camera_buttons_cycle_views_keep_the_image_while_loading_and_reset_for_another_file() {
    crate::test_support::gtk_test(
        "ui::preview::tests::model::camera_buttons_cycle_views_keep_the_image_while_loading_and_reset_for_another_file",
        || {
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            drawer.show(entry("mesh.stl"), None);
            ready(&provider, 0, Some(ModelView::FrontLeft));
            for (index, view) in [
                ModelView::BackLeft,
                ModelView::BackRight,
                ModelView::FrontRight,
                ModelView::FrontLeft,
            ]
            .into_iter()
            .enumerate()
            {
                let (next, image) = {
                    let model = drawer.state.model.borrow();
                    let model = model.as_ref().expect("mesh controls");
                    (model.next.clone(), model.picture.paintable())
                };
                next.emit_clicked();
                assert_eq!(provider.0.borrow().len(), index + 2);
                assert_eq!(provider.0.borrow()[index + 1].request.model_view, view);
                assert!(
                    drawer.state.model_spinner.is_visible(),
                    "the header must show loading feedback"
                );
                {
                    let pending = provider.0.borrow();
                    let pending = &pending[index + 1];
                    (pending.emit)(PreviewEvent::Progress {
                        request_id: pending.request.id,
                        stage: crate::services::ModelPreviewStage::Rendering { triangles: 23 },
                    });
                }
                assert!(drawer.state.model_spinner.is_visible());
                assert_eq!(drawer.state.content_type.text(), "model/stl");
                next.emit_clicked();
                assert_eq!(
                    provider.0.borrow().len(),
                    index + 2,
                    "loading blocks another request"
                );
                assert_eq!(
                    drawer
                        .state
                        .model
                        .borrow()
                        .as_ref()
                        .expect("retained mesh")
                        .picture
                        .paintable(),
                    image
                );
                ready(&provider, index + 1, Some(view));
                assert_eq!(
                    drawer
                        .state
                        .model
                        .borrow()
                        .as_ref()
                        .expect("mesh controls")
                        .view(),
                    view
                );
                assert!(!drawer.state.model_spinner.is_visible());
            }
            let previous = drawer
                .state
                .model
                .borrow()
                .as_ref()
                .expect("mesh controls")
                .previous
                .clone();
            previous.emit_clicked();
            assert_eq!(
                provider
                    .0
                    .borrow()
                    .last()
                    .expect("camera request")
                    .request
                    .model_view,
                ModelView::FrontRight
            );
            assert!(drawer.state.model_spinner.is_visible());
            drawer.show(entry("another.stl"), None);
            assert!(
                drawer.state.model_spinner.is_visible(),
                "the new model has its own initial loading feedback"
            );
            assert_eq!(
                provider
                    .0
                    .borrow()
                    .last()
                    .expect("new file request")
                    .request
                    .model_view,
                ModelView::FrontLeft
            );
            ready(&provider, 5, Some(ModelView::FrontRight));
            assert!(
                drawer
                    .state
                    .model
                    .borrow()
                    .as_ref()
                    .expect("new model shell")
                    .picture
                    .paintable()
                    .is_none(),
                "stale render must not replace the new file"
            );
            ready(&provider, 6, Some(ModelView::FrontLeft));
            drawer.show(entry("thumbnail.3mf"), None);
            ready(&provider, 7, None);
            assert!(
                drawer.state.model.borrow().is_none(),
                "thumbnail has no camera controls"
            );
            drawer.close();
        },
    );
}

#[test]
fn camera_reload_failures_keep_the_last_view_and_allow_retry() {
    crate::test_support::gtk_test(
        "ui::preview::tests::model::camera_reload_failures_keep_the_last_view_and_allow_retry",
        || {
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            drawer.show(entry("mesh.stl"), None);
            ready(&provider, 0, Some(ModelView::FrontLeft));
            let image = drawer
                .state
                .model
                .borrow()
                .as_ref()
                .expect("mesh controls")
                .picture
                .paintable();
            let next = drawer
                .state
                .model
                .borrow()
                .as_ref()
                .expect("mesh controls")
                .next
                .clone();
            next.emit_clicked();
            let pending = provider.0.borrow();
            (pending[1].emit)(PreviewEvent::Failed {
                request_id: pending[1].request.id,
                entry: pending[1].request.entry.clone(),
                message: "This model is too complex to draw within the preview rendering limit. Try a lower-detail version.".into(),
            });
            drop(pending);
            assert_eq!(drawer.state.model_view.get(), ModelView::FrontLeft);
            assert!(drawer.state.model.borrow().is_some(), "last rendered view");
            assert!(!drawer.state.model_spinner.is_visible());
            next.emit_clicked();
            assert_eq!(provider.0.borrow().len(), 3);
            assert_eq!(
                provider.0.borrow()[2].request.model_view,
                ModelView::BackLeft
            );
            let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
            png.extend(16u32.to_be_bytes());
            png.extend(16u32.to_be_bytes());
            let pending = provider.0.borrow();
            (pending[2].emit)(PreviewEvent::Ready(Preview {
                request_id: pending[2].request.id,
                entry: pending[2].request.entry.clone(),
                content_type: "model/stl".into(),
                content: PreviewContent::Model {
                    png,
                    mesh: Some(ModelMesh {
                        view: ModelView::BackLeft,
                        encoding: ModelEncoding::BinaryStl,
                        triangles: 23,
                    }),
                },
            }));
            drop(pending);
            {
                let model = drawer.state.model.borrow();
                let model = model.as_ref().expect("last decodable view");
                assert_eq!(model.picture.paintable(), image);
                assert_eq!(model.view(), ModelView::FrontLeft);
                assert!(!drawer.state.model_spinner.is_visible());
            }
            assert_eq!(drawer.state.model_view.get(), ModelView::FrontLeft);
            next.emit_clicked();
            assert_eq!(provider.0.borrow().len(), 4);
            assert_eq!(
                provider.0.borrow()[3].request.model_view,
                ModelView::BackLeft
            );
            ready(&provider, 3, Some(ModelView::BackLeft));
            assert_eq!(drawer.state.model_view.get(), ModelView::BackLeft);
            drawer.close();
        },
    );
}

#[test]
fn camera_buttons_keep_the_keys_in_the_drawer_while_a_view_renders() {
    crate::test_support::gtk_test(
        "ui::preview::tests::model::camera_buttons_keep_the_keys_in_the_drawer_while_a_view_renders",
        || {
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            let window = gtk::Window::builder().child(&drawer.widget()).build();
            window.present();
            drawer.show(entry("mesh.stl"), None);
            ready(&provider, 0, Some(ModelView::FrontLeft));
            let next = drawer
                .state
                .model
                .borrow()
                .as_ref()
                .expect("mesh controls")
                .next
                .clone();
            assert!(next.grab_focus());
            while glib::MainContext::default().iteration(false) {}
            let owns_keys = || drawer.state.pane.has_css_class("preview-keyboard-owner");
            assert!(owns_keys());

            next.emit_clicked();
            while glib::MainContext::default().iteration(false) {}
            assert!(drawer.state.model_spinner.is_visible());
            assert!(
                next.has_focus(),
                "rendering must not drop the clicked control's focus"
            );
            assert!(owns_keys());

            ready(&provider, 1, Some(ModelView::BackLeft));
            while glib::MainContext::default().iteration(false) {}
            assert!(next.has_focus());
            assert!(owns_keys());
            window.destroy();
        },
    );
}

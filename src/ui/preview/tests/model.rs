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
fn camera_buttons_cycle_views_keep_the_image_while_loading_and_reset_for_another_file() {
    crate::test_support::gtk_test(
        "ui::preview::tests::model::camera_buttons_cycle_views_keep_the_image_while_loading_and_reset_for_another_file",
        || {
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            drawer.show(entry("mesh.stl"), None);
            ready(&provider, 0, Some(ModelView::FrontLeft));
            for (index, view) in [
                ModelView::FrontRight,
                ModelView::BackRight,
                ModelView::BackLeft,
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
                {
                    let model = drawer.state.model.borrow();
                    let model = model.as_ref().expect("rendering mesh");
                    assert!(
                        model.loading_overlay.is_visible(),
                        "the main view must show loading feedback"
                    );
                    assert_eq!(model.progress.text(), "Loading preview…");
                }
                {
                    let pending = provider.0.borrow();
                    (pending[index + 1].emit)(PreviewEvent::Progress {
                        request_id: pending[index + 1].request.id,
                        stage: crate::services::ModelPreviewStage::Rendering { triangles: 23 },
                    });
                }
                assert_eq!(
                    drawer
                        .state
                        .model
                        .borrow()
                        .as_ref()
                        .expect("rendering mesh")
                        .progress
                        .text(),
                    "Rendering 23 triangles…"
                );
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
                assert!(
                    !drawer
                        .state
                        .model
                        .borrow()
                        .as_ref()
                        .expect("rendered mesh")
                        .loading_overlay
                        .is_visible()
                );
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
                ModelView::BackLeft
            );
            drawer.show(entry("another.stl"), None);
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
            ready(&provider, 5, Some(ModelView::BackLeft));
            assert!(
                drawer.state.model.borrow().is_none(),
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
            assert!(
                !drawer
                    .state
                    .model
                    .borrow()
                    .as_ref()
                    .expect("last rendered view")
                    .loading_overlay
                    .is_visible()
            );
            assert!(next.is_sensitive());
            next.emit_clicked();
            assert_eq!(provider.0.borrow().len(), 3);
            assert_eq!(
                provider.0.borrow()[2].request.model_view,
                ModelView::FrontRight
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
                        view: ModelView::FrontRight,
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
                assert!(!model.loading_overlay.is_visible());
            }
            assert_eq!(drawer.state.model_view.get(), ModelView::FrontLeft);
            assert!(next.is_sensitive());
            next.emit_clicked();
            assert_eq!(provider.0.borrow().len(), 4);
            assert_eq!(
                provider.0.borrow()[3].request.model_view,
                ModelView::FrontRight
            );
            ready(&provider, 3, Some(ModelView::FrontRight));
            assert_eq!(drawer.state.model_view.get(), ModelView::FrontRight);
            drawer.close();
        },
    );
}

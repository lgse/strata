// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{MediaPreviewSize, ModelPalette, ModelView};

#[test]
fn model_metadata_accepts_only_the_requested_camera_or_a_supported_thumbnail() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("result.model");
    for format in [
        ModelFormat::Stl,
        ModelFormat::ThreeMf,
        ModelFormat::FreeCad,
        ModelFormat::Gcode,
    ] {
        for view in ModelView::ALL {
            let render = ModelRender {
                purpose: crate::services::ModelRenderPurpose::Preview,
                format,
                view,
                size: MediaPreviewSize::new(200, 200),
                palette: ModelPalette::default(),
            };
            let mesh = ModelMesh {
                view,
                encoding: if format == ModelFormat::Stl {
                    ModelEncoding::BinaryStl
                } else {
                    ModelEncoding::ThreeMf
                },
                triangles: 2,
            };
            for (value, expected) in [
                (
                    serde_json::to_vec(&Some(mesh)).expect("metadata"),
                    matches!(format, ModelFormat::Stl | ModelFormat::ThreeMf).then_some(Some(mesh)),
                ),
                (
                    b"null".to_vec(),
                    (format != ModelFormat::Stl).then_some(None),
                ),
                (
                    serde_json::to_vec(&ModelMesh {
                        view: view.step(1),
                        ..mesh
                    })
                    .expect("wrong view"),
                    None,
                ),
                (
                    serde_json::to_vec(&ModelMesh {
                        triangles: 0,
                        ..mesh
                    })
                    .expect("empty mesh"),
                    None,
                ),
                (
                    serde_json::to_vec(&ModelMesh {
                        triangles: crate::services::model_preview::MAX_MODEL_TRIANGLES + 1,
                        ..mesh
                    })
                    .expect("excess count"),
                    None,
                ),
                (
                    serde_json::to_vec(&ModelMesh {
                        encoding: if format == ModelFormat::Stl {
                            ModelEncoding::ThreeMf
                        } else {
                            ModelEncoding::AsciiStl
                        },
                        ..mesh
                    })
                    .expect("wrong format"),
                    None,
                ),
                (b"unknown".to_vec(), None),
                (vec![b' '; 257], None),
            ] {
                std::fs::write(&path, &value).expect("metadata");
                let result = read_mesh(render, &path);
                match expected {
                    Some(expected) => assert_eq!(result.expect("valid metadata"), expected),
                    None => assert!(result.is_err(), "{format:?} {view:?} {value:?}"),
                }
            }
        }
    }
}

#[test]
fn thumbnail_cancel_and_timeout_return_while_reaping_is_delayed() {
    use std::{
        process::Command,
        sync::{Arc, mpsc},
        thread,
        time::{Duration, Instant},
    };

    for cancel in [false, true] {
        let input = tempfile::tempdir().expect("model input");
        let output = Arc::new(super::super::PrivateOutput::create().expect("private output"));
        let output_path = output.path().to_owned();
        let source = input.path().join("model.stl");
        std::fs::write(&source, b"fixture").expect("model input");
        let hold =
            Arc::new(crate::services::file_activity::hold([source], || {}).expect("model hold"));
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "printf '{\"child-pid\": %s}\n' \"$$\" >&2; exec sleep 60",
        ]);
        let renderer = super::super::spawn_renderer(&mut command).expect("renderer");
        let cancellation = super::super::Cancellation::default();
        if cancel {
            cancellation.cancel();
        }
        let (release, delayed) = mpsc::channel();
        let (sent, returned) = mpsc::channel();
        let foreground = thread::spawn(move || {
            let result = wait_with_reaper(
                renderer,
                &cancellation,
                Duration::ZERO,
                &mut || {},
                (output, hold),
                move || {
                    let _released = delayed.recv();
                },
            );
            let _sent = sent.send(result);
        });
        let result = returned.recv_timeout(Duration::from_secs(2));
        let output_retained = output_path.exists();
        let input_retained = crate::services::file_activity::in_use_within(input.path());
        // Release even on a failed assertion, so the fixture cannot strand its reaper.
        let _released = release.send(());
        foreground.join().expect("foreground worker");
        let error = result
            .expect("render worker must return before the reaper is released")
            .expect_err("cancelled or timed out thumbnail");
        assert_eq!(
            error,
            if cancel {
                "Preview cancelled"
            } else {
                "The preview renderer timed out"
            }
        );
        assert!(
            output_retained,
            "private output belongs to the reaper until it finishes"
        );
        assert!(
            input_retained,
            "file operation hold belongs to the reaper until it finishes"
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while output_path.exists() || crate::services::file_activity::in_use_within(input.path()) {
            assert!(
                Instant::now() < deadline,
                "reaper must clean up the completed renderer"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}

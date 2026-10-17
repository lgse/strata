// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{MediaPreviewSize, ModelPalette, ModelView};

#[test]
fn model_metadata_accepts_only_the_requested_camera_or_a_supported_thumbnail() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("result.model");
    for format in [ModelFormat::Stl, ModelFormat::ThreeMf, ModelFormat::FreeCad] {
        for view in ModelView::ALL {
            let render = ModelRender {
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
                    (format != ModelFormat::FreeCad).then_some(Some(mesh)),
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

// SPDX-License-Identifier: MIT

use std::path::Path;

use strata_preview::services::ModelFormat;

use super::{Kind, of};

/// GTK's `thumbnail_kind` names its kinds by extension, ignoring case; a file it names none
/// for gets no thumbnail.
#[test]
fn a_file_gets_the_renderer_its_extension_names() {
    for (name, kind) in [
        ("photo.PNG", Some(Kind::Image)),
        ("scan.jxl", Some(Kind::Image)),
        ("paper.pdf", Some(Kind::Pdf)),
        ("part.stl", Some(Kind::RenderedModel)),
        ("print.3mf", Some(Kind::EmbeddedModel(ModelFormat::ThreeMf))),
        (
            "design.FCStd",
            Some(Kind::EmbeddedModel(ModelFormat::FreeCad)),
        ),
        ("job.bgcode", Some(Kind::EmbeddedModel(ModelFormat::Gcode))),
        ("tool.AppImage", Some(Kind::AppImage)),
        ("clip.webm", Some(Kind::Video)),
        ("song.flac", Some(Kind::Video)),
        ("notes.txt", None),
        ("no-extension", None),
    ] {
        assert_eq!(of(Path::new(name)), kind, "{name}");
    }
}

#[test]
fn only_plain_images_and_appimages_are_light() {
    assert!(!Kind::Image.heavy());
    assert!(!Kind::AppImage.heavy());
    for kind in [Kind::Pdf, Kind::Video, Kind::RenderedModel, Kind::RawImage] {
        assert!(kind.heavy(), "{kind:?}");
    }
}

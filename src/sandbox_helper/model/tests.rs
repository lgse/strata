// SPDX-License-Identifier: MIT

use super::*;
use std::io::Write;

fn render(input: &Path, value: &str) -> Result<Vec<u8>, String> {
    let format = ModelFormat::for_name(input.as_os_str()).expect("model format");
    render_reporting(input, &format!("{}:{value}", format.argument()), &|_| {})
}

#[test]
fn model_progress_tracks_parsing_rendering_and_encoding_and_stops_on_failure() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("progress.stl");
    fs::write(&path, "solid sample\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid").expect("STL");
    let stages = std::cell::RefCell::new(Vec::new());
    let png = render_reporting(&path, "stl:200x200:00ff00:101010", &|stage| {
        stages.borrow_mut().push(stage)
    })
    .expect("model render");
    assert!(Pixmap::decode_png(&png).is_ok());
    assert_eq!(
        *stages.borrow(),
        vec![
            ModelPreviewStage::Reading,
            ModelPreviewStage::Rendering { triangles: 1 },
            ModelPreviewStage::Finishing
        ]
    );
    stages.borrow_mut().clear();
    fs::write(&path, b"not an STL").expect("invalid model");
    assert!(
        render_reporting(&path, "stl:200x200:00ff00:101010", &|stage| stages
            .borrow_mut()
            .push(stage))
        .is_err()
    );
    assert_eq!(*stages.borrow(), vec![ModelPreviewStage::Reading]);
}

#[test]
fn large_binary_stl_is_accepted_and_over_limit_reports_triangles() {
    for (count, accepted) in [(1_620_000u32, true), (2_000_001, false)] {
        let mut bytes = vec![0; 84 + count as usize * 50];
        bytes[80..84].copy_from_slice(&count.to_le_bytes());
        let result = stl(&bytes);
        if accepted {
            assert_eq!(result.expect("81 MB STL").len(), count as usize);
        } else {
            assert!(
                result
                    .expect_err("triangle budget")
                    .contains("2 million triangle")
            );
        }
    }
}

#[test]
fn freecad_thumbnail_needs_no_geometry_reader_and_missing_thumbnail_is_unavailable() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("sample.FCStd");
    for include_thumbnail in [true, false] {
        let mut package = zip::ZipWriter::new(fs::File::create(&path).expect("FreeCAD package"));
        let options = zip::write::SimpleFileOptions::default();
        package
            .start_file("Document.xml", options)
            .expect("document part");
        package
            .write_all(b"geometry deliberately not parsed")
            .expect("document");
        if include_thumbnail {
            package
                .start_file("thumbnails/Thumbnail.png", options)
                .expect("thumbnail part");
            let mut thumbnail = Pixmap::new(24, 24).expect("pixmap");
            thumbnail.fill(Color::from_rgba8(255, 0, 0, 255));
            package
                .write_all(&thumbnail.encode_png().expect("PNG"))
                .expect("thumbnail");
        }
        package.finish().expect("finished package");
        let result = render(&path, "200x200:00ff00:101010");
        if include_thumbnail {
            let png = result.expect("FreeCAD thumbnail");
            assert_eq!(
                Pixmap::decode_png(&png)
                    .expect("decoded thumbnail")
                    .pixel(0, 0)
                    .expect("pixel")
                    .red(),
                255
            );
        } else {
            assert!(
                result
                    .expect_err("missing thumbnail")
                    .contains("no usable embedded thumbnail")
            );
        }
    }
}

#[test]
fn binary_stl_with_solid_header_is_not_misread_as_ascii() {
    let mut bytes = vec![0; 84 + 50];
    bytes[..5].copy_from_slice(b"solid");
    bytes[80..84].copy_from_slice(&1u32.to_le_bytes());
    let points = [[0f32, 0., 0.], [1., 0., 0.], [0., 1., 0.]];
    for (vertex, point) in points.iter().enumerate() {
        for (axis, value) in point.iter().enumerate() {
            let at = 84 + 12 + vertex * 12 + axis * 4;
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    assert_eq!(stl(&bytes).expect("binary STL"), vec![points]);
}

#[test]
fn ascii_stl_parses_faces_and_rejects_non_finite_coordinates() {
    let source = b"solid sample\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid";
    assert_eq!(stl(source).expect("ASCII STL").len(), 1);
    assert!(stl(b"solid sample\nvertex 0 0 0\nvertex NaN 0 0\nvertex 0 1 0\nendsolid").is_err());
}

#[test]
fn package_renders_build_items_and_component_transforms() {
    let model = br#"<model><resources><object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object><object id="2"><components><component objectid="1" transform="1 0 0 0 1 0 0 0 1 10 0 0"/></components></object></resources><build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 0 5 0"/></build></model>"#;
    let triangles = triangles_3mf(model).expect("3MF model");
    assert_eq!(
        triangles,
        vec![[[10., 5., 0.], [11., 5., 0.], [10., 6., 0.]]]
    );
}

#[test]
fn cyclic_components_are_bounded() {
    let model = br#"<model><resources><object id="1"><components><component objectid="1"/></components></object></resources><build><item objectid="1"/></build></model>"#;
    assert!(triangles_3mf(model).is_err());
}

#[test]
fn multipart_packages_show_one_thumbnail_or_explain_why_rendering_is_unavailable() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("assembly.3mf");
    for thumbnail_count in [0, 1, 2] {
        let mut package = zip::ZipWriter::new(fs::File::create(&path).expect("package"));
        let options = zip::write::SimpleFileOptions::default();
        for part in ["3D/Objects/part.model", "3D/3dmodel.model"] {
            package.start_file(part, options).expect("model part");
            package
                .write_all(b"must not parse partial geometry")
                .expect("model content");
        }
        for index in 0..thumbnail_count {
            package
                .start_file(format!("Metadata/{index}-thumbnail.png"), options)
                .expect("thumbnail part");
            let mut thumbnail = Pixmap::new(8, 8).expect("pixmap");
            thumbnail.fill(Color::from_rgba8(255, 0, 0, 255));
            package
                .write_all(&thumbnail.encode_png().expect("PNG"))
                .expect("thumbnail");
        }
        package.finish().expect("finished package");
        let result = render(&path, "200x200:00ff00:101010");
        if thumbnail_count == 1 {
            assert_eq!(
                Pixmap::decode_png(&result.expect("thumbnail"))
                    .expect("PNG")
                    .pixel(0, 0)
                    .expect("pixel")
                    .red(),
                255
            );
        } else {
            assert_eq!(
                result.expect_err("multipart rendering unavailable"),
                "Multipart model detected. Unable to render preview."
            );
        }
    }
}

#[test]
fn cross_part_components_report_multipart_even_when_the_referenced_part_is_missing() {
    let xml = br#"<model><resources><object id="1"><components><component objectid="1" path="/3D/Objects/part.model"/></components></object></resources><build><item objectid="1"/></build></model>"#;
    assert_eq!(
        triangles_3mf(xml).expect_err("cross-part component"),
        "Multipart model detected. Unable to render preview."
    );
}

#[test]
fn root_relationship_selects_a_nonstandard_model_part_and_named_thumbnail() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("related.3mf");
    let mut package = zip::ZipWriter::new(fs::File::create(&path).expect("package"));
    let options = zip::write::SimpleFileOptions::default();
    package
        .start_file("_rels/.rels", options)
        .expect("relationships");
    package.write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="model" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel" Target="/Models/root.model"/><Relationship Id="thumbnail" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail" Target="/Metadata/preview.png"/></Relationships>"#).expect("relationships");
    package
        .start_file("Models/root.model", options)
        .expect("root model");
    package.write_all(br#"<model><resources><object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources><build><item objectid="1"/></build></model>"#).expect("model");
    package.finish().expect("package complete");
    let rendered = render(&path, "200x200:00ff00:101010").expect("root selected from relationship");
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("package");
    let mut package = zip::ZipWriter::new_append(file).expect("append thumbnail");
    package
        .start_file("Metadata/preview.png", options)
        .expect("thumbnail");
    let mut thumbnail = Pixmap::new(8, 8).expect("pixmap");
    thumbnail.fill(Color::from_rgba8(255, 0, 0, 255));
    package
        .write_all(&thumbnail.encode_png().expect("PNG"))
        .expect("thumbnail");
    package.finish().expect("package complete");
    let embedded = render(&path, "200x200:00ff00:101010").expect("named thumbnail");
    assert_ne!(rendered, embedded);
    assert_eq!(
        Pixmap::decode_png(&embedded)
            .expect("PNG")
            .pixel(0, 0)
            .expect("pixel")
            .red(),
        255
    );
}

#[test]
fn single_thumbnail_is_preferred_and_multiple_or_invalid_thumbnails_render_the_model() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("sample.3mf");
    let file = fs::File::create(&path).expect("package");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("3D/3dmodel.model", options)
        .expect("model part");
    zip.write_all(br#"<model><resources><object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources><build><item objectid="1"/></build></model>"#).expect("model content");
    zip.start_file("Metadata/thumbnail.png", options)
        .expect("thumbnail part");
    let mut thumbnail = Pixmap::new(8, 8).expect("thumbnail pixmap");
    thumbnail.fill(Color::from_rgba8(255, 0, 0, 255));
    zip.write_all(&thumbnail.encode_png().expect("thumbnail PNG"))
        .expect("thumbnail content");
    zip.finish().expect("package complete");

    let embedded = render(&path, "200x200:00ff00:101010").expect("embedded thumbnail");
    assert_eq!(
        Pixmap::decode_png(&embedded)
            .expect("thumbnail")
            .pixel(0, 0)
            .expect("pixel")
            .red(),
        255
    );
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("package");
    let mut zip = zip::ZipWriter::new_append(file).expect("append to package");
    zip.start_file("Metadata/other-thumbnail.png", options)
        .expect("second thumbnail part");
    zip.write_all(&thumbnail.encode_png().expect("second PNG"))
        .expect("second thumbnail");
    zip.finish().expect("package complete");
    let multiple =
        render(&path, "200x200:00ff00:101010").expect("multiple thumbnails use geometry");
    assert_ne!(multiple, embedded);
    let mut package = zip::ZipArchive::new(fs::File::open(&path).expect("package")).expect("ZIP");
    let mut model = Vec::new();
    package
        .by_name("3D/3dmodel.model")
        .expect("model")
        .read_to_end(&mut model)
        .expect("model bytes");
    drop(package);
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).expect("package"));
    zip.start_file("3D/3dmodel.model", options)
        .expect("model part");
    zip.write_all(&model).expect("model");
    zip.start_file("Metadata/thumbnail.png", options)
        .expect("thumbnail part");
    zip.write_all(b"not an image").expect("invalid thumbnail");
    zip.finish().expect("package complete");
    let shaded = render(&path, "200x200:00ff00:101010").expect("rendered model");
    assert_eq!(multiple, shaded);
    assert_ne!(embedded, shaded);
    let recolored = render(&path, "200x200:0000ff:101010").expect("recolored model");
    assert_ne!(shaded, recolored);
}

#[test]
fn component_reference_and_expansion_budgets_reject_fanout() {
    let component = "<component objectid=\"1\"/>";
    let xml = format!(
        "<model><resources><object id=\"1\"><components>{}</components></object></resources><build><item objectid=\"1\"/></build></model>",
        component.repeat(MAX_MODEL_COMPONENT_REFERENCES + 1)
    );
    assert_eq!(
        triangles_3mf(xml.as_bytes()).expect_err("bounded failure"),
        "3MF component reference limit exceeded"
    );

    let xml = format!(
        "<model><resources><object id=\"1\"><components>{}</components></object></resources><build><item objectid=\"1\"/></build></model>",
        component.repeat(MAX_MODEL_COMPONENT_REFERENCES / 2)
    );
    assert_eq!(
        triangles_3mf(xml.as_bytes()).expect_err("bounded failure"),
        "3MF component expansion limit exceeded"
    );
}

#[test]
fn missing_geometry_references_are_rejected() {
    for xml in [
        "<model><build><item objectid=\"1\"/></build></model>",
        "<model><resources><object id=\"1\"><mesh><triangles><triangle v1=\"0\" v2=\"1\" v3=\"2\"/></triangles></mesh></object></resources><build><item objectid=\"1\"/></build></model>",
    ] {
        assert!(
            triangles_3mf(xml.as_bytes())
                .expect_err("bounded failure")
                .contains("missing")
        );
    }
}

#[test]
fn overlapping_faces_exhaust_the_raster_budget_without_finishing() {
    let faces = vec![[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]; 1000];
    let stages = std::cell::RefCell::new(Vec::new());
    let error = shade(&faces, 800, 800, [255; 3], [0; 3], &|stage| {
        stages.borrow_mut().push(stage)
    })
    .expect_err("bounded failure");
    assert!(error.contains("rendering limit"));
    assert!(!stages.borrow().contains(&ModelPreviewStage::Finishing));
}

// SPDX-License-Identifier: MIT

use super::*;
use docx_rs::FromXML;

const WORD: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

#[test]
fn namespace_aliases_preserve_document_content_and_formatting() {
    let fixture = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/documents/report.docx"),
    )
    .expect("fixture");
    let expected =
        super::super::to_html(&docx_rs::read_docx(&fixture).expect("canonical document")).html;
    for prefix in ["ns0", "word", ""] {
        let mut input = ZipArchive::new(Cursor::new(&fixture)).expect("fixture package");
        let mut output = ZipWriter::new(Cursor::new(Vec::new()));
        for index in 0..input.len() {
            let mut part = input.by_index(index).expect("part");
            if part.name() != "word/document.xml" {
                output.raw_copy_file(part).expect("copy part");
                continue;
            }
            let mut xml = String::new();
            part.read_to_string(&mut xml).expect("document XML");
            xml = xml
                .replace("xmlns:w=", "xmlns:attr=")
                .replace(" w:", " attr:");
            let declaration = if prefix.is_empty() {
                format!("xmlns=\"{WORD}\"")
            } else {
                format!("xmlns:{prefix}=\"{WORD}\"")
            };
            xml = xml.replace("<w:document ", &format!("<w:document {declaration} "));
            let qualified = if prefix.is_empty() {
                String::new()
            } else {
                format!("{prefix}:")
            };
            xml = xml
                .replace("<w:", &format!("<{qualified}"))
                .replace("</w:", &format!("</{qualified}"));
            output
                .start_file(part.name(), SimpleFileOptions::default())
                .expect("document part");
            output.write_all(xml.as_bytes()).expect("write XML");
        }
        let bytes = output.finish().expect("finish package").into_inner();
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("aliases.docx");
        std::fs::write(&path, bytes).expect("write document");
        let actual = super::super::read_document(&path).expect("aliased document");
        assert_eq!(actual.html, expected, "prefix {prefix:?}");
    }
}

#[test]
fn scoped_rebinding_cannot_turn_foreign_runs_into_word_text() {
    let xml = format!(
        r#"<w:document xmlns:w="{WORD}"><w:body><w:p><w:r><w:t>before</w:t><w:t xmlns:w="urn:foreign">hidden</w:t><x:t xmlns:x="{WORD}">after &amp; &lt;literal&gt;</x:t></w:r></w:p></w:body></w:document>"#
    );
    let normalized = normalize_xml(xml.as_bytes()).expect("normalized XML");
    let document = docx_rs::Document::from_xml(normalized.as_slice()).expect("document");
    let docx = docx_rs::Docx {
        document,
        ..docx_rs::Docx::new()
    };
    let html = super::super::to_html(&docx).html;
    assert!(html.contains("beforeafter &amp; &lt;literal&gt;"), "{html}");
    assert!(!html.contains("hidden"), "{html}");
}

#[test]
fn malformed_namespaces_and_dtds_are_rejected() {
    for xml in [
        "<unbound:p/>",
        "<p><r></p>",
        "<p>",
        "<!DOCTYPE p [<!ENTITY x 'text'>]><p>&x;</p>",
    ] {
        assert!(normalize_xml(xml.as_bytes()).is_err(), "{xml}");
    }
}

#[test]
fn package_budget_counts_all_xml_parts_and_preserves_binary_parts() {
    let mut output = ZipWriter::new(Cursor::new(Vec::new()));
    for name in ["first.xml", "second.xml"] {
        output
            .start_file(name, SimpleFileOptions::default())
            .expect("XML part");
        output.write_all(b"<p>text</p>").expect("XML content");
    }
    output
        .start_file("media/image.bin", SimpleFileOptions::default())
        .expect("binary part");
    output.write_all(&[0, 255, 1, 128]).expect("binary content");
    let bytes = output.finish().expect("package").into_inner();
    assert!(normalize_package_with_limit(&bytes, 21).is_err());
    let normalized = normalize_package_with_limit(&bytes, 22).expect("budget boundary");
    let mut package = ZipArchive::new(Cursor::new(normalized)).expect("normalized package");
    let mut binary = Vec::new();
    package
        .by_name("media/image.bin")
        .expect("binary part")
        .read_to_end(&mut binary)
        .expect("binary read");
    assert_eq!(binary, [0, 255, 1, 128]);
}

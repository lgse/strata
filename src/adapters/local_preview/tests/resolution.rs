// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::PrivateContext;

#[derive(Debug, PartialEq)]
enum Resolved<'a> {
    Text(&'a str),
    Image,
    Media,
    Pdf,
    Unsupported,
}

const BINARY_DATA: &[u8] = &[0, 159, 146, 150, 0, 255, 0, 1];
const PNG_HEADER: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01";

const PREVIEW_FAMILY_CASES: &[(&str, &[u8], Resolved<'static>)] = &[
    (
        "notes.png",
        b"just some notes\n",
        Resolved::Text("just some notes\n"),
    ),
    (
        "clip.mp4",
        b"not a movie\n",
        Resolved::Text("not a movie\n"),
    ),
    (
        "clip.h264",
        b"not a movie\n",
        Resolved::Text("not a movie\n"),
    ),
    ("empty.H264", b"", Resolved::Text("")),
    (
        "stream.h264",
        b"\x00\x00\x00\x01\x67\x64\x00\x0c\xac\xd9\x41\x41",
        Resolved::Media,
    ),
    (
        "STREAM.H264",
        b"\x00\x00\x01\x67\x64\x00\x0c\xac\xd9\x41\x41",
        Resolved::Media,
    ),
    (
        "app.ts",
        b"const x = 1;\n",
        Resolved::Text("const x = 1;\n"),
    ),
    (
        "backup.zip",
        b"not an archive\n",
        Resolved::Text("not an archive\n"),
    ),
    (
        "report.docx",
        b"not a document\n",
        Resolved::Text("not a document\n"),
    ),
    ("App.tsx", b"<div />\n", Resolved::Text("<div />\n")),
    ("README.md", PNG_HEADER, Resolved::Image),
    ("photo.txt", PNG_HEADER, Resolved::Image),
    (
        "paper.txt",
        b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\n",
        Resolved::Pdf,
    ),
    (
        "icon.svg",
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
        Resolved::Image,
    ),
    ("data.json", BINARY_DATA, Resolved::Unsupported),
    (
        "wide.txt",
        b"\xff\xfeh\x00i\x00\n\x00",
        Resolved::Text("hi\n"),
    ),
    (
        "DSCF0001.RAF",
        b"FUJIFILMCCD-RAW 0201FF383501\0",
        Resolved::Image,
    ),
    ("GOPR0001.GPR", &[0, 1, 2, 3, 255, 254], Resolved::Image),
];

fn write_fixture(directory: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = directory.path().join(name);
    fs::write(&path, bytes).expect("preview fixture");
    path
}

fn sandbox_forbidden(
    reason: &'static str,
) -> impl FnOnce(
    &Path,
    ParseOperation,
    i32,
    MediaPreviewBackend,
    &Cancellation,
) -> Result<crate::sandbox::ParseOutput, String>
+ Send
+ 'static {
    move |_, _, _, _, _| -> Result<crate::sandbox::ParseOutput, String> { panic!("{reason}") }
}

fn load_local_file(
    provider: &LocalPreviewProvider,
    context: &glib::MainContext,
    path: &Path,
    name: &str,
    reason: &'static str,
) -> Vec<PreviewEvent> {
    let request = preview_request(crate::model::Location::local(path), name);
    load_until_first_event(provider, context, request, sandbox_forbidden(reason))
}

fn only_event<'a>(events: &'a [PreviewEvent], name: &str) -> &'a PreviewEvent {
    assert_eq!(events.len(), 1, "{name}");
    &events[0]
}

fn ready_content<'a>(event: &'a PreviewEvent, name: &str) -> &'a PreviewContent {
    let PreviewEvent::Ready(preview) = event else {
        panic!("{name} preview failed: {event:?}");
    };
    &preview.content
}

fn is_expected_family_operation(name: &str, operation: &ParseOperation) -> bool {
    if name.ends_with(".RAF") || name.ends_with(".GPR") {
        matches!(operation, ParseOperation::PreviewRaw)
    } else {
        matches!(
            operation,
            ParseOperation::PreviewImage | ParseOperation::PreviewPdf(_)
        )
    }
}

fn resolve<'a>(content: &'a PreviewContent, path: &Path, name: &str) -> Resolved<'a> {
    match content {
        PreviewContent::Text { content, .. } => Resolved::Text(content),
        PreviewContent::Rasterized { .. } => Resolved::Image,
        PreviewContent::SandboxedMedia { media } => {
            assert_eq!(media.path, path);
            Resolved::Media
        }
        PreviewContent::Pdf { .. } => Resolved::Pdf,
        PreviewContent::Unsupported => Resolved::Unsupported,
        other => panic!("{name} resolved to {other:?}"),
    }
}

#[test]
fn uncertain_file_names_resolve_their_preview_from_the_content() {
    with_default_main_context(|provider, context| {
        let directory = tempfile::tempdir().expect("preview fixture directory");
        let activity = crate::services::file_activity::begin(
            crate::services::file_activity::OperationScope::default(),
        );
        for (name, bytes, fenced, expected) in [
            (
                "some notes",
                &b"hello\nworld\n"[..],
                false,
                Ok(PreviewContent::Text {
                    content: "hello\nworld\n".to_owned(),
                    truncated: false,
                }),
            ),
            (
                "some data",
                BINARY_DATA,
                false,
                Ok(PreviewContent::Unsupported),
            ),
            (
                "fenced data",
                BINARY_DATA,
                true,
                Err(crate::services::file_activity::HELD_FOR_OPERATION),
            ),
        ] {
            let path = write_fixture(&directory, name, bytes);
            let _fence = fenced.then(|| activity.fence([path.clone()]));
            let events = load_local_file(
                provider,
                context,
                &path,
                name,
                "text and unsupported files must not reach the sandbox",
            );

            let outcome = match only_event(&events, name) {
                PreviewEvent::Ready(preview) => Ok(preview.content.clone()),
                PreviewEvent::Failed { message, .. } => Err(message.as_str()),
                _ => panic!("{name} preview neither loaded nor failed"),
            };
            assert_eq!(outcome, expected, "{name}");
        }
    });
}

#[test]
fn text_subclassed_names_resolve_their_preview_from_the_type_hierarchy() {
    with_default_main_context(|provider, context| {
        let directory = tempfile::tempdir().expect("preview fixture directory");
        for (name, contents) in [
            ("config.yaml", "key: value\n"),
            ("config.toml", "[section]\nkey = 1\n"),
        ] {
            let path = write_fixture(&directory, name, contents.as_bytes());
            let events = load_local_file(
                provider,
                context,
                &path,
                name,
                "text files must not reach the sandbox",
            );

            assert_eq!(
                ready_content(only_event(&events, name), name),
                &PreviewContent::Text {
                    content: contents.to_owned(),
                    truncated: false,
                },
                "{name}"
            );
        }
    });
}

#[test]
fn preview_families_follow_file_bytes_not_names() {
    with_default_main_context(|provider, context| {
        let directory = tempfile::tempdir().expect("preview fixture directory");
        for (name, bytes, expected) in PREVIEW_FAMILY_CASES {
            let name = *name;
            let path = write_fixture(&directory, name, bytes);
            let request = PreviewRequest {
                render_document: true,
                ..preview_request(crate::model::Location::local(&path), name)
            };
            let events =
                load_until_first_event(provider, context, request, move |_, operation, _, _, _| {
                    assert!(
                        is_expected_family_operation(name, &operation),
                        "{name} reached the sandbox as {operation:?}"
                    );
                    Ok(parse_output(Vec::new(), 0, 0))
                });

            let content = ready_content(only_event(&events, name), name);
            assert_eq!(resolve(content, &path, name), *expected, "{name}");
        }
    });
}

#[test]
fn text_reads_refuse_a_file_an_operation_fences() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("notes.txt");
    fs::write(&path, "notes").expect("text file");
    let file = gio::File::for_path(&path);
    let read = || PrivateContext::new().block_on(read_text(&file, Some(&path), 64));
    let activity = crate::services::file_activity::begin(
        crate::services::file_activity::OperationScope::default(),
    );

    let fence = activity.fence([directory.path().to_owned()]);
    let refused = read().expect_err("a fenced file is not read");
    drop(fence);

    assert_eq!(
        refused.message(),
        crate::services::file_activity::HELD_FOR_OPERATION
    );
    assert_eq!(read().expect("read once the fence lifts").0, "notes");
}

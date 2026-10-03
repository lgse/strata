// SPDX-License-Identifier: MIT

use super::acceptance::{request, wait_until};
use super::*;
use crate::services::image_conversion::ImageKind;

fn button(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    if let Some(button) = widget.downcast_ref::<gtk::Button>()
        && button.label().as_deref() == Some(label)
    {
        return Some(button.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = button(&current, label) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

#[test]
fn active_filter_matches_actual_format_and_png_aliases_not_labels() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::image_conversion::active_filter_matches_actual_format_and_png_aliases_not_labels",
        || {
            crate::ui::prepare_portal_ui();
            let root = tempfile::tempdir().expect("fixture");
            let mut req = request(root.path().to_owned());
            req.filters = vec![
                FileFilter::new("Misleading JPEG label").glob("*.PNG"),
                FileFilter::new("PNG MIME").mimetype("image/png"),
                FileFilter::new("Images").mimetype("image/*"),
                FileFilter::new("JPEG aliases").glob("*.jpeg"),
                FileFilter::new("SVG only").glob("*.svg"),
                FileFilter::new("All files").glob("*"),
                FileFilter::new("Mixed").glob("*.png").glob("*.jpg"),
                FileFilter::new("Case pattern").glob("*.[pP][nN][gG]"),
            ];
            let state =
                build_chooser(req, Arc::new(AtomicBool::new(false)), |_| {}).expect("chooser");
            let path = root.path().join("photo.png");
            for (index, jpeg, png) in [
                (0, None, Some("PNG")),
                (1, None, Some("png")),
                (2, Some("jpg"), Some("png")),
                (3, Some("jpeg"), None),
                (4, None, None),
                (5, Some("jpg"), Some("png")),
                (6, Some("jpg"), Some("png")),
                (7, None, Some("png")),
            ] {
                state
                    .filter_dropdown
                    .as_ref()
                    .expect("filter")
                    .selected
                    .set(index);
                for (kind, expected) in [(ImageKind::Jpeg, jpeg), (ImageKind::Png, png)] {
                    assert_eq!(
                        state
                            .image_target(&path, kind)
                            .as_deref()
                            .and_then(Path::extension)
                            .and_then(|ext| ext.to_str()),
                        expected,
                        "filter {index}: {kind:?}"
                    );
                }
            }
            state.cancel();
        },
    );
}

#[test]
fn remote_image_names_are_repaired_or_checked_by_the_original_filename_filter() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::image_conversion::remote_image_names_are_repaired_or_checked_by_the_original_filename_filter",
        || {
            crate::ui::prepare_portal_ui();
            let mut bmp = std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(1, 1)
                .write_to(&mut bmp, image::ImageFormat::Bmp)
                .expect("BMP fixture");
            let text = b"Bitmap notes".as_slice();
            let jpeg = b"\xff\xd8\xff...".as_slice();
            for (bytes, name, pattern, expected) in [
                (text, "notes.txt", Some("*.txt"), Some("notes.txt")),
                (text, "README", Some("*"), Some("README")),
                (text, "README", None, Some("README")),
                (
                    bmp.get_ref().as_slice(),
                    "README",
                    Some("*"),
                    Some("README.bmp"),
                ),
                (
                    jpeg,
                    "attachment.pdf",
                    Some("*.pdf"),
                    Some("attachment.pdf"),
                ),
                (jpeg, "photo.bin", Some("*"), Some("photo.jpg")),
                (jpeg, "photo.bin", None, Some("photo.jpg")),
                (jpeg, "photo.bin", Some("*.txt"), None),
            ] {
                let root = tempfile::tempdir().expect("fixture");
                let path = root.path().join(name);
                std::fs::write(&path, bytes).expect("downloaded bytes");
                let result = Rc::new(RefCell::new(None));
                let received = result.clone();
                let mut req = request(root.path().to_owned());
                if let Some(pattern) = pattern {
                    req.filters = vec![FileFilter::new("Test filter").glob(pattern)];
                }
                let state = build_chooser(req, Arc::new(AtomicBool::new(false)), move |value| {
                    received.replace(Some(value));
                })
                .expect("chooser");
                state.complete_download(path.clone());
                assert!(
                    visible_modal_layer(&state.window).is_none(),
                    "{name}: {pattern:?}"
                );
                assert!(
                    !state.download_in_progress(),
                    "must not decode {name}: {pattern:?}"
                );
                if let Some(expected) = expected {
                    let selected = result
                        .borrow_mut()
                        .take()
                        .expect("result")
                        .expect("accepted");
                    assert_eq!(selected.uris().len(), 1);
                    let output = gio::File::for_uri(&selected.uris()[0].to_string())
                        .path()
                        .expect("local output");
                    assert_eq!(output, root.path().join(expected));
                    assert_eq!(std::fs::read(output).expect("returned bytes"), bytes);
                } else {
                    assert!(result.borrow().is_none());
                    assert!(state.completion.borrow().is_some());
                    assert!(state.error.is_visible());
                    assert_eq!(std::fs::read(path).expect("original bytes"), bytes);
                    state.cancel();
                }
            }
        },
    );
}

#[test]
fn already_png_bytes_are_renamed_without_conversion_and_html_is_rejected() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::image_conversion::already_png_bytes_are_renamed_without_conversion_and_html_is_rejected",
        || {
            crate::ui::prepare_portal_ui();
            let root = tempfile::tempdir().expect("fixture");
            let path = root.path().join("image.jpg");
            image::DynamicImage::new_rgba8(2, 3)
                .save_with_format(&path, image::ImageFormat::Png)
                .expect("PNG");
            let original = std::fs::read(&path).expect("original");
            let result = Rc::new(RefCell::new(None));
            let received = result.clone();
            let mut req = request(root.path().to_owned());
            req.filters = vec![FileFilter::new("PNG").glob("*.png")];
            let state = build_chooser(req, Arc::new(AtomicBool::new(false)), move |value| {
                received.replace(Some(value));
            })
            .expect("chooser");
            let invalid = root.path().join("error.png");
            std::fs::write(&invalid, b"<html>not an image</html>").expect("HTML");
            state.complete_download(invalid);
            assert!(state.error.is_visible());
            assert!(result.borrow().is_none());
            state.complete_download(path);
            let selected = result
                .borrow_mut()
                .take()
                .expect("result")
                .expect("accepted");
            let output = gio::File::for_uri(&selected.uris()[0].to_string())
                .path()
                .expect("local output");
            assert_eq!(output.extension().expect("extension"), "png");
            assert_eq!(std::fs::read(output).expect("PNG bytes"), original);
            assert!(!state.download_in_progress());
        },
    );
}

#[test]
fn declining_conversion_keeps_original_and_changed_filter_invalidates_confirmation() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::image_conversion::declining_conversion_keeps_original_and_changed_filter_invalidates_confirmation",
        || {
            crate::ui::prepare_portal_ui();
            let root = tempfile::tempdir().expect("fixture");
            let path = root.path().join("source.jpg");
            std::fs::write(&path, "original").expect("file");
            let result = Rc::new(RefCell::new(None));
            let received = result.clone();
            let mut req = request(root.path().to_owned());
            req.filters = vec![
                FileFilter::new("PNG").glob("*.png"),
                FileFilter::new("SVG").glob("*.svg"),
            ];
            let state = build_chooser(req, Arc::new(AtomicBool::new(false)), move |value| {
                received.replace(Some(value));
            })
            .expect("chooser");
            state.confirm_image_conversion(
                path.clone(),
                path.with_extension("png"),
                ImageKind::Jpeg,
            );
            let modal = visible_modal_layer(&state.window).expect("conversion prompt");
            button(&modal, "Cancel").expect("Cancel").emit_clicked();
            wait_until(|| visible_modal_layer(&state.window).is_none());
            assert!(result.borrow().is_none());
            assert_eq!(
                std::fs::read_to_string(&path).expect("original retained"),
                "original"
            );
            state.confirm_image_conversion(
                path.clone(),
                path.with_extension("png"),
                ImageKind::Jpeg,
            );
            let modal = visible_modal_layer(&state.window).expect("conversion prompt");
            state
                .filter_dropdown
                .as_ref()
                .expect("filter")
                .selected
                .set(1);
            button(&modal, "Convert to PNG")
                .expect("convert")
                .emit_clicked();
            assert!(!state.download_in_progress());
            assert!(result.borrow().is_none());
            assert!(state.error.text().contains("filter changed"));
            state.cancel();
        },
    );
}

#[test]
fn cancelled_image_worker_cannot_complete_a_replacement_and_failures_allow_retry() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::image_conversion::cancelled_image_worker_cannot_complete_a_replacement_and_failures_allow_retry",
        || {
            crate::ui::prepare_portal_ui();
            let root = tempfile::tempdir().expect("fixture");
            let state = build_chooser(
                request(root.path().to_owned()),
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .expect("chooser");
            let (send, recv) = std::sync::mpsc::channel();
            state.image_job(
                "Converting…",
                move |cancelled| {
                    recv.recv().expect("release");
                    assert!(cancelled.is_cancelled());
                    Ok(())
                },
                |_, ()| panic!("cancelled worker completed"),
            );
            assert!(state.cancel_download());
            let done = Rc::new(Cell::new(false));
            let finished = done.clone();
            state.image_job("Converting…", |_| Ok(()), move |_, ()| finished.set(true));
            send.send(()).expect("release worker");
            wait_until(|| done.get());
            state.image_job(
                "Converting…",
                |_| Err::<(), _>("Could not convert".into()),
                |_, ()| panic!("failed conversion completed"),
            );
            wait_until(|| state.error.is_visible());
            assert!(state.completion.borrow().is_some());
            assert!(state.accept_button.is_sensitive());
            assert!(!state.download_in_progress());
            state.cancel();
        },
    );
}

#[test]
fn typed_local_name_opens_the_file_without_repairing_its_extension() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::image_conversion::typed_local_name_opens_the_file_without_repairing_its_extension",
        || {
            crate::ui::prepare_portal_ui();
            let root = tempfile::tempdir().expect("fixture");
            let typed = root.path().join("photo.jfif");
            let neighbour = root.path().join("photo.jpg");
            image::DynamicImage::new_rgb8(1, 1)
                .save_with_format(&typed, image::ImageFormat::Jpeg)
                .expect("JPEG fixture");
            std::fs::write(&neighbour, "unrelated").expect("neighbour fixture");
            let jpeg = std::fs::read(&typed).expect("JPEG bytes");
            let result = Rc::new(RefCell::new(None));
            let received = result.clone();
            let state = build_chooser(
                request(root.path().to_owned()),
                Arc::new(AtomicBool::new(false)),
                move |value| {
                    received.replace(Some(value));
                },
            )
            .expect("chooser");
            wait_until(|| {
                state
                    .view
                    .browser()
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
            });
            let filename = state.filename.as_ref().expect("open name field");
            filename.set_text("photo.jfif");
            filename.emit_activate();
            wait_until(|| result.borrow().is_some());
            let selected = result
                .borrow_mut()
                .take()
                .expect("result")
                .expect("accepted");
            let output = gio::File::for_uri(&selected.uris()[0].to_string())
                .path()
                .expect("local output");
            assert_eq!(output, typed);
            assert_eq!(std::fs::read(&typed).expect("typed bytes"), jpeg);
            assert_eq!(
                std::fs::read_to_string(&neighbour).expect("neighbour retained"),
                "unrelated"
            );
            assert!(!state.download_in_progress());
        },
    );
}

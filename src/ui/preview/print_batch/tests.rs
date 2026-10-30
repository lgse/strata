// SPDX-License-Identifier: MIT

use super::*;
use std::cell::Cell;
use std::os::unix::fs::symlink;
use std::time::{Duration, Instant};

#[test]
fn folder_walk_is_recursive_ordered_and_does_not_follow_folder_links() {
    let root = tempfile::tempdir().expect("create folder test directory");
    let folder = root.path().join("folder");
    let nested = folder.join("nested");
    fs::create_dir_all(&nested).expect("create nested folder");
    fs::write(folder.join("b.txt"), "b").expect("write second file");
    fs::write(folder.join("a.txt"), "a").expect("write first file");
    fs::write(nested.join("c.txt"), "c").expect("write nested file");
    symlink(&folder, nested.join("cycle")).expect("create folder cycle");
    symlink(folder.join("a.txt"), folder.join("duplicate.txt"))
        .expect("create duplicate file link");

    let collected = collect_paths(
        &[folder.clone(), folder.join("a.txt")],
        &AtomicBool::new(false),
    );
    assert_eq!(
        collected.paths,
        vec![
            folder.join("a.txt"),
            folder.join("b.txt"),
            nested.join("c.txt")
        ]
    );
    assert_eq!(collected.skipped.len(), 1);
    assert!(collected.skipped[0].name.ends_with("cycle"));
}

#[test]
fn unreadable_or_missing_entries_are_skipped_without_losing_valid_files() {
    let root = tempfile::tempdir().expect("create missing-file test directory");
    let valid = root.path().join("valid.txt");
    fs::write(&valid, "valid").expect("write valid file");
    let missing = root.path().join("missing.txt");
    let collected = collect_paths(&[missing, valid.clone()], &AtomicBool::new(false));
    assert_eq!(collected.paths, vec![valid]);
    assert_eq!(collected.skipped.len(), 1);
}

#[test]
fn text_pages_are_paginated_and_staged_under_the_budget() {
    let root = tempfile::tempdir().expect("create text-page test directory");
    let text = (0..180).map(|n| format!("Line {n}\n")).collect::<String>();
    let (pages, bytes) = render_text_pages(
        &text,
        root.path(),
        MAX_STAGED_BYTES,
        &AtomicBool::new(false),
    )
    .expect("render text pages");
    assert!(pages.len() > 1);
    assert!(bytes > 0);
    assert!(pages.iter().all(|page| page.exists()));
    assert!(render_text_pages(&text, root.path(), 1, &AtomicBool::new(false)).is_err());
    assert!(
        render_text_pages(&text, root.path(), MAX_STAGED_BYTES, &AtomicBool::new(true)).is_err()
    );
}

#[test]
fn combined_job_exports_pages_in_selection_order() {
    crate::test_support::gtk_test(
        "ui::preview::print_batch::tests::combined_job_exports_pages_in_selection_order",
        || {
            let staging = tempfile::tempdir().expect("create print staging directory");
            let output = tempfile::tempdir().expect("create PDF output directory");
            let mut pages = Vec::new();
            for (index, (red, blue)) in [(1.0, 0.0), (0.0, 1.0)].into_iter().enumerate() {
                let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 100, 100)
                    .expect("create source image");
                let cr = cairo::Context::new(&surface).expect("create source context");
                cr.set_source_rgb(red, 0.0, blue);
                cr.paint().expect("paint source page");
                let path = staging.path().join(format!("page-{index}.png"));
                surface
                    .write_to_png(&mut fs::File::create(&path).expect("create source PNG"))
                    .expect("write source PNG");
                pages.push(path);
            }
            let prepared = Prepared {
                _temporary: staging,
                pages,
                files: 2,
                skipped: Vec::new(),
                bytes: 0,
            };
            let path = output.path().join("combined.pdf");
            let operation = prepared_operation(prepared);
            operation.set_export_filename(path.to_str().expect("UTF-8 PDF output path"));
            let done = Rc::new(Cell::new(false));
            operation.connect_done({
                let done = done.clone();
                move |_, _| done.set(true)
            });
            operation
                .run(gtk::PrintOperationAction::Export, None::<&gtk::Window>)
                .expect("export combined print job");
            let deadline = Instant::now() + Duration::from_secs(5);
            while !done.get() && Instant::now() < deadline {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(done.get(), "combined print job did not finish");
            let uri = gio::File::for_path(&path).uri();
            let document = poppler::Document::from_file(&uri, None).expect("open exported PDF");
            assert_eq!(document.n_pages(), 2);
            for (index, red_expected) in [(0, true), (1, false)] {
                let page = document.page(index).expect("find exported page");
                let (width, height) = page.size();
                let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 100, 100)
                    .expect("create rendered page image");
                let cr = cairo::Context::new(&surface).expect("create render context");
                cr.scale(100.0 / width, 100.0 / height);
                page.render(&cr);
                drop(cr);
                surface.flush();
                let stride = surface.stride() as usize;
                let data = surface.data().expect("read rendered page pixels");
                let offset = 50 * stride + 50 * 4;
                let blue = data[offset];
                let red = data[offset + 2];
                assert_eq!(red > blue, red_expected, "page {index} color order");
            }
        },
    );
}

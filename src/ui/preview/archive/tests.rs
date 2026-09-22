// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::{ArchiveBrowser, child_summary, directory_at};
use crate::services::{
    ArchiveDirectory, ArchiveFileEntry, ArchiveNode, ArchivePreviewTree, archive_preview_tree,
};

fn tree() -> ArchivePreviewTree {
    archive_preview_tree(vec![
        ArchiveFileEntry {
            name: "src/lib.rs".to_owned(),
            directory: false,
            size: 10,
        },
        ArchiveFileEntry {
            name: "src/main.rs".to_owned(),
            directory: false,
            size: 20,
        },
        ArchiveFileEntry {
            name: "src".to_owned(),
            directory: true,
            size: 0,
        },
        ArchiveFileEntry {
            name: "README.md".to_owned(),
            directory: false,
            size: 30,
        },
    ])
}

fn src_index(root: &ArchiveDirectory) -> usize {
    root.children
        .iter()
        .position(|node| matches!(node, ArchiveNode::Directory(dir) if dir.name == "src"))
        .expect("src directory")
}

#[test]
fn root_resolves_with_an_empty_path() {
    let tree = tree();
    assert_eq!(directory_at(&tree.root, &[]).name, "");
}

#[test]
fn path_resolves_to_the_selected_directory() {
    let tree = tree();
    let index = src_index(&tree.root);
    let directory = directory_at(&tree.root, &[index]);
    assert_eq!(directory.name, "src");
    assert_eq!(child_summary(directory), (2, 0));
}

#[test]
fn file_index_stops_resolution_at_the_parent() {
    let tree = tree();
    let directory = directory_at(&tree.root, &[src_index(&tree.root), 0]);
    assert_eq!(directory.name, "src");
}

#[test]
fn out_of_range_index_stops_resolution() {
    let tree = tree();
    assert_eq!(directory_at(&tree.root, &[99]).name, "");
}

#[test]
fn child_summary_counts_direct_children() {
    let tree = tree();
    assert_eq!(child_summary(&tree.root), (1, 1));
}

fn dir_index(directory: &ArchiveDirectory, name: &str) -> usize {
    directory
        .children
        .iter()
        .position(|node| matches!(node, ArchiveNode::Directory(dir) if dir.name == name))
        .unwrap_or_else(|| panic!("{name} directory"))
}

fn crumb_button(browser: &ArchiveBrowser, offset: usize) -> gtk::Button {
    let mut child = browser
        .crumbs
        .first_child()
        .expect("a breadcrumb button exists");
    for _ in 0..offset {
        child = child.next_sibling().expect("the next breadcrumb");
    }
    child
        .downcast::<gtk::Button>()
        .expect("a breadcrumb button")
}

fn navigate_calls() -> std::rc::Rc<std::cell::RefCell<Vec<usize>>> {
    std::rc::Rc::new(std::cell::RefCell::new(Vec::new()))
}

#[test]
fn root_crumb_jumps_back_to_the_root() {
    crate::test_support::gtk_test(
        "ui::preview::archive::tests::root_crumb_jumps_back_to_the_root",
        || {
            let jumps = navigate_calls();
            let mut browser = ArchiveBrowser::new(
                tree(),
                std::rc::Rc::new({
                    let jumps = jumps.clone();
                    move |depth| jumps.borrow_mut().push(depth)
                }),
            );
            browser.open_child(src_index(&browser.tree.root));
            crumb_button(&browser, 1).emit_clicked();
            assert_eq!(jumps.borrow().as_slice(), &[0]);
        },
    );
}

#[test]
fn intermediate_crumb_jumps_to_its_directory() {
    crate::test_support::gtk_test(
        "ui::preview::archive::tests::intermediate_crumb_jumps_to_its_directory",
        || {
            let tree = archive_preview_tree(vec![ArchiveFileEntry {
                name: "src/mod/x.rs".to_owned(),
                directory: false,
                size: 1,
            }]);
            let jumps = navigate_calls();
            let mut browser = ArchiveBrowser::new(
                tree,
                std::rc::Rc::new({
                    let jumps = jumps.clone();
                    move |depth| jumps.borrow_mut().push(depth)
                }),
            );
            let src = dir_index(&browser.tree.root, "src");
            browser.open_child(src);
            let src_directory = match &browser.tree.root.children[src] {
                ArchiveNode::Directory(dir) => dir,
                _ => unreachable!("src is a directory"),
            };
            browser.open_child(dir_index(src_directory, "mod"));
            crumb_button(&browser, 2).emit_clicked();
            assert_eq!(jumps.borrow().as_slice(), &[1]);
        },
    );
}

fn realized_row_count(browser: &ArchiveBrowser) -> usize {
    fn count(widget: &gtk::Widget) -> usize {
        let mut total = usize::from(widget.has_css_class("preview-archive-row"));
        let mut child = widget.first_child();
        while let Some(next) = child {
            total += count(&next);
            child = next.next_sibling();
        }
        total
    }
    count(browser.root().upcast_ref())
}

fn flat_tree(count: usize) -> ArchivePreviewTree {
    ArchivePreviewTree {
        root: ArchiveDirectory {
            name: String::new(),
            children: (0..count)
                .map(|index| ArchiveNode::File {
                    name: format!("file-{index:05}.txt"),
                    size: u64::try_from(index).unwrap_or(u64::MAX),
                })
                .collect(),
        },
        file_count: count,
    }
}

#[test]
fn large_directories_model_every_child_without_realizing_rows() {
    crate::test_support::gtk_test(
        "ui::preview::archive::tests::large_directories_model_every_child_without_realizing_rows",
        || {
            let browser = ArchiveBrowser::new(flat_tree(2000), std::rc::Rc::new(|_| {}));
            assert_eq!(browser.model.n_items(), 2000);
            let realized = realized_row_count(&browser);
            assert!(
                realized < 100,
                "expected only visible rows realized, found {realized}"
            );
        },
    );
}

#[test]
fn navigating_republishes_the_model_and_summary() {
    crate::test_support::gtk_test(
        "ui::preview::archive::tests::navigating_republishes_the_model_and_summary",
        || {
            let tree = archive_preview_tree(vec![
                ArchiveFileEntry {
                    name: "docs/a.txt".to_owned(),
                    directory: false,
                    size: 1,
                },
                ArchiveFileEntry {
                    name: "docs/b.txt".to_owned(),
                    directory: false,
                    size: 2,
                },
                ArchiveFileEntry {
                    name: "empty".to_owned(),
                    directory: true,
                    size: 0,
                },
                ArchiveFileEntry {
                    name: "top.txt".to_owned(),
                    directory: false,
                    size: 3,
                },
            ]);
            let mut browser = ArchiveBrowser::new(tree, std::rc::Rc::new(|_| {}));
            assert_eq!(browser.model.n_items(), 3);
            assert_eq!(browser.count.text(), "1 files, 2 folders");
            assert!(!browser.empty.is_visible());

            browser.open_child(dir_index(&browser.tree.root, "docs"));
            assert_eq!(browser.model.n_items(), 2);
            assert_eq!(browser.count.text(), "2 files");

            browser.navigate_to(0);
            browser.open_child(dir_index(&browser.tree.root, "empty"));
            assert_eq!(browser.model.n_items(), 0);
            assert_eq!(browser.count.text(), "Empty folder");
            assert!(browser.empty.is_visible());

            browser.navigate_to(0);
            assert_eq!(browser.model.n_items(), 3);
            assert!(!browser.empty.is_visible());
        },
    );
}

#[test]
#[ignore = "requires a mapped GTK window; run this test alone"]
fn mapped_window_binds_visible_archive_rows() {
    const CHILD: &str = "STRATA_ARCHIVE_MAPPED_ROWS_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status =
            std::process::Command::new(std::env::current_exe().expect("test executable exists"))
                .args([
                    "--exact",
                    "ui::preview::archive::tests::mapped_window_binds_visible_archive_rows",
                    "--ignored",
                ])
                .env(CHILD, "1")
                .status()
                .expect("isolated mapped archive-rows test should start");
        assert!(status.success());
        return;
    }
    if gtk::init().is_err() {
        return;
    }
    let tree = archive_preview_tree(vec![
        ArchiveFileEntry {
            name: "b.txt".to_owned(),
            directory: false,
            size: 2,
        },
        ArchiveFileEntry {
            name: "docs".to_owned(),
            directory: true,
            size: 0,
        },
        ArchiveFileEntry {
            name: "a.txt".to_owned(),
            directory: false,
            size: 1,
        },
    ]);
    let browser = ArchiveBrowser::new(tree, std::rc::Rc::new(|_| {}));
    let window = gtk::Window::builder().child(browser.root()).build();
    window.set_default_size(400, 600);
    window.present();
    for _ in 0..200 {
        while gtk::glib::MainContext::default().iteration(false) {}
    }
    let mut names = Vec::new();
    let mut child = browser.list().first_child();
    while let Some(row) = child {
        if let Some(name_label) = row
            .first_child()
            .as_ref()
            .and_then(gtk::Widget::first_child)
            .and_then(|icon| icon.next_sibling())
            .and_downcast::<gtk::Label>()
        {
            names.push(name_label.text().to_string());
        }
        child = row.next_sibling();
    }
    window.destroy();
    assert_eq!(names.first().map(String::as_str), Some("docs/"));
    assert!(names.contains(&"a.txt".to_owned()));
    assert!(names.len() <= 3);
}

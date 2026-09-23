// SPDX-License-Identifier: MIT

use super::{
    ArchiveDirectory, ArchiveFileEntry, ArchiveNode, MediaPreviewSize, PreviewContent,
    archive_preview_format, archive_preview_tree, content_family, has_plain_text_extension,
    is_extensionless_dotfile, is_image_path, is_media_path,
    is_non_executable_extensionless_dotfile, normalize_preview_text,
};

#[test]
fn preview_text_normalizes_nul_before_any_gtk_view() {
    assert_eq!(normalize_preview_text("before\0after"), "before�after");
    assert!(matches!(
        normalize_preview_text("ordinary text"),
        std::borrow::Cow::Borrowed(_)
    ));
}

#[test]
fn media_viewport_sizes_follow_display_scale_without_exceeding_the_pixel_budget() {
    assert_eq!(
        MediaPreviewSize::for_viewport(520, 800, 1),
        MediaPreviewSize::new(520, 800)
    );
    assert_eq!(
        MediaPreviewSize::for_viewport(520, 800, 2),
        MediaPreviewSize::new(1040, 1280)
    );
    assert_eq!(
        MediaPreviewSize::for_viewport(i32::MAX, i32::MAX, 2),
        MediaPreviewSize::new(1280, 1280)
    );
}

#[test]
fn recognizes_image_paths_for_metadata_probes() {
    assert!(is_image_path(std::path::Path::new("photo.PNG")));
    assert!(!is_image_path(std::path::Path::new("notes.txt")));
    assert!(is_media_path(std::path::Path::new("movie.mp4")));
    assert!(is_media_path(std::path::Path::new("song.flac")));
    assert!(!is_media_path(std::path::Path::new("photo.png")));
}

#[test]
fn recognizes_configuration_files_as_plain_text() {
    assert!(has_plain_text_extension(std::ffi::OsStr::new(
        "settings.conf"
    )));
    assert!(has_plain_text_extension(std::ffi::OsStr::new(
        "SETTINGS.INI"
    )));
    assert!(!has_plain_text_extension(std::ffi::OsStr::new(
        "archive.zip"
    )));
}

#[test]
fn recognizes_extensionless_dotfiles() {
    assert!(is_extensionless_dotfile(std::ffi::OsStr::new(".steampath")));
    assert!(!is_extensionless_dotfile(std::ffi::OsStr::new("steampath")));
    assert!(!is_extensionless_dotfile(std::ffi::OsStr::new(
        ".settings.toml"
    )));
}

#[test]
fn recognizes_non_executable_extensionless_dotfiles() {
    let name = std::ffi::OsStr::new(".steamid");

    assert!(is_non_executable_extensionless_dotfile(
        name,
        Some(0o100644)
    ));
    assert!(!is_non_executable_extensionless_dotfile(
        name,
        Some(0o100755)
    ));
    assert!(!is_non_executable_extensionless_dotfile(name, None));
}

#[test]
fn classifies_common_preview_content_types() {
    assert_eq!(content_family("image/png"), PreviewContent::Image);
    assert_eq!(content_family("image/gif"), PreviewContent::Media);
    assert_eq!(content_family("video/mp4"), PreviewContent::Media);
    assert!(matches!(
        content_family("application/pdf"),
        PreviewContent::Pdf { .. }
    ));
    assert!(matches!(
        content_family("text/x-rust"),
        PreviewContent::Text { .. }
    ));
    assert!(matches!(
        content_family("application/problem+json"),
        PreviewContent::Text { .. }
    ));
    assert_eq!(
        content_family("application/octet-stream"),
        PreviewContent::Unsupported
    );
}

#[test]
fn archive_preview_format_detects_previewable_archives() {
    use super::super::operations::ArchiveFormat;
    use std::ffi::OsStr;

    assert_eq!(
        archive_preview_format(OsStr::new("a.zip")),
        Some(ArchiveFormat::Zip)
    );
    assert_eq!(
        archive_preview_format(OsStr::new("a.7z")),
        Some(ArchiveFormat::SevenZ)
    );
    assert_eq!(
        archive_preview_format(OsStr::new("a.tar.gz")),
        Some(ArchiveFormat::TarGz)
    );
    assert_eq!(
        archive_preview_format(OsStr::new("a.tgz")),
        Some(ArchiveFormat::TarGz)
    );
    assert_eq!(
        archive_preview_format(OsStr::new("a.tar")),
        Some(ArchiveFormat::Tar)
    );
    assert_eq!(archive_preview_format(OsStr::new("a.rar")), None);
    assert_eq!(archive_preview_format(OsStr::new("a.gz")), None);
    assert_eq!(archive_preview_format(OsStr::new("a.txt")), None);
}

#[test]
fn archive_preview_tree_builds_directories_and_files() {
    let tree = archive_preview_tree(vec![
        ArchiveFileEntry {
            name: "dir/file.txt".to_owned(),
            directory: false,
            size: 10,
        },
        ArchiveFileEntry {
            name: "dir".to_owned(),
            directory: true,
            size: 0,
        },
        ArchiveFileEntry {
            name: "readme.md".to_owned(),
            directory: false,
            size: 20,
        },
    ]);
    assert_eq!(tree.root.name, "");
    let (files, dirs) = dir_stats(&tree.root);
    assert_eq!(files, 1);
    assert_eq!(dirs, 1);
    assert_eq!(tree.file_count, 2);
}

fn dir_stats(directory: &ArchiveDirectory) -> (usize, usize) {
    directory
        .children
        .iter()
        .fold((0, 0), |(files, folders), node| match node {
            ArchiveNode::Directory(_) => (files, folders + 1),
            ArchiveNode::File { .. } => (files + 1, folders),
        })
}

#[test]
fn archive_preview_tree_preserves_literal_segments() {
    let tree = archive_preview_tree(vec![
        ArchiveFileEntry {
            name: "assets//images/icon.png".to_owned(),
            directory: false,
            size: 5,
        },
        ArchiveFileEntry {
            name: "./dir/../leaf.txt".to_owned(),
            directory: false,
            size: 6,
        },
    ]);
    let mut names: Vec<_> = tree.root.children.iter().map(node_name).collect();
    names.sort();
    assert_eq!(names, vec![".", "assets"]);
    let dot = tree
        .root
        .children
        .iter()
        .find_map(|node| match node {
            ArchiveNode::Directory(d) if d.name == "." => Some(d),
            _ => None,
        })
        .expect("root contains a literal '.' directory");
    let dir = dot
        .children
        .iter()
        .find_map(|node| match node {
            ArchiveNode::Directory(d) if d.name == "dir" => Some(d),
            _ => None,
        })
        .expect("'.' contains 'dir'");
    let dotdot = dir
        .children
        .iter()
        .find_map(|node| match node {
            ArchiveNode::Directory(d) if d.name == ".." => Some(d),
            _ => None,
        })
        .expect("'dir' contains a literal '..' directory");
    assert_eq!(dotdot.children.len(), 1);
    assert_eq!(node_name(&dotdot.children[0]), "leaf.txt");
    assert_eq!(tree.file_count, 2);
}

#[test]
fn archive_preview_tree_keeps_colliding_file_and_directory() {
    let tree = archive_preview_tree(vec![
        ArchiveFileEntry {
            name: "x".to_owned(),
            directory: true,
            size: 0,
        },
        ArchiveFileEntry {
            name: "x".to_owned(),
            directory: false,
            size: 100,
        },
    ]);
    assert_eq!(tree.root.children.len(), 2);
    assert!(matches!(&tree.root.children[0], ArchiveNode::Directory(_)));
    assert!(matches!(
        &tree.root.children[1],
        ArchiveNode::File { name, size: 100 } if name == "x"
    ));
    assert_eq!(tree.file_count, 1);
}

#[test]
fn archive_preview_tree_orders_children_directories_first_then_alphabetically() {
    let tree = archive_preview_tree(vec![
        ArchiveFileEntry {
            name: "B/file.txt".to_owned(),
            directory: false,
            size: 1,
        },
        ArchiveFileEntry {
            name: "a/file.txt".to_owned(),
            directory: false,
            size: 2,
        },
        ArchiveFileEntry {
            name: "A".to_owned(),
            directory: true,
            size: 0,
        },
        ArchiveFileEntry {
            name: "B".to_owned(),
            directory: true,
            size: 0,
        },
        ArchiveFileEntry {
            name: "b.txt".to_owned(),
            directory: false,
            size: 3,
        },
        ArchiveFileEntry {
            name: "A.txt".to_owned(),
            directory: false,
            size: 4,
        },
    ]);
    let names: Vec<_> = tree.root.children.iter().map(node_name).collect();
    assert_eq!(names, vec!["A", "a", "B", "A.txt", "b.txt"]);
}

fn node_name(node: &ArchiveNode) -> &str {
    match node {
        ArchiveNode::Directory(dir) => &dir.name,
        ArchiveNode::File { name, .. } => name,
    }
}

#[test]
fn archive_preview_tree_handles_moderately_deep_paths() {
    let depth = 1_000;
    let name = (0..depth).map(|_| "a/").collect::<String>() + "file.txt";
    let tree = archive_preview_tree(vec![ArchiveFileEntry {
        name,
        directory: false,
        size: 7,
    }]);
    assert_eq!(tree.file_count, 1);
    let mut level = &tree.root;
    for _ in 0..depth {
        assert_eq!(level.children.len(), 1);
        let ArchiveNode::Directory(next) = &level.children[0] else {
            panic!("expected a single directory level");
        };
        assert_eq!(next.name, "a");
        level = next;
    }
    assert_eq!(level.children.len(), 1);
    assert!(matches!(
        &level.children[0],
        ArchiveNode::File { name, size: 7 } if name == "file.txt"
    ));
}

#[test]
fn archive_preview_tree_survives_hostile_nesting_depths() {
    let depth = 100_000;
    let entries = vec![
        ArchiveFileEntry {
            name: (0..depth).map(|_| "a/").collect::<String>() + "file.txt",
            directory: false,
            size: 1,
        },
        ArchiveFileEntry {
            name: (0..depth).map(|_| "b/").collect::<String>() + "other.txt",
            directory: false,
            size: 2,
        },
        ArchiveFileEntry {
            name: "shallow.txt".to_owned(),
            directory: false,
            size: 3,
        },
    ];
    let tree = archive_preview_tree(entries);
    assert_eq!(tree.file_count, 3);
    assert_eq!(tree.root.children.len(), 3);
    assert_eq!(node_name(&tree.root.children[0]), "a");
    assert_eq!(node_name(&tree.root.children[1]), "b");
    assert_eq!(node_name(&tree.root.children[2]), "shallow.txt");
    for (root, leaf) in [("a", "file.txt"), ("b", "other.txt")] {
        let mut level = tree
            .root
            .children
            .iter()
            .find(|node| node_name(node) == root)
            .and_then(|node| match node {
                ArchiveNode::Directory(directory) => Some(directory),
                ArchiveNode::File { .. } => None,
            })
            .expect("deep chain root");
        for _ in 1..depth {
            let ArchiveNode::Directory(next) = level
                .children
                .iter()
                .find(|node| node_name(node) == root)
                .expect("deep chain level")
            else {
                panic!("expected a directory level");
            };
            level = next;
        }
        assert_eq!(level.children.len(), 1);
        assert_eq!(node_name(&level.children[0]), leaf);
    }
}

#[test]
fn archive_preview_tree_keeps_literal_separators_and_collisions_at_depth() {
    let deep = (0..500).map(|_| "nest/").collect::<String>();
    let tree = archive_preview_tree(vec![
        ArchiveFileEntry {
            name: format!("{deep}mixed\\seps/./down/../leaf.txt"),
            directory: false,
            size: 1,
        },
        ArchiveFileEntry {
            name: format!("{deep}mixed\\seps/./down/../leaf.txt"),
            directory: false,
            size: 1,
        },
        ArchiveFileEntry {
            name: format!("{deep}collision"),
            directory: false,
            size: 2,
        },
        ArchiveFileEntry {
            name: format!("{deep}collision/inner.txt"),
            directory: false,
            size: 3,
        },
        ArchiveFileEntry {
            name: format!("{deep}{}long-component", "x".repeat(500)),
            directory: false,
            size: 4,
        },
    ]);
    assert_eq!(tree.file_count, 5);
    let mut level = &tree.root;
    for _ in 0..500 {
        let ArchiveNode::Directory(next) = level
            .children
            .iter()
            .find(|node| node_name(node) == "nest")
            .expect("nest level")
        else {
            panic!("expected a nest directory");
        };
        level = next;
    }
    let mut names: Vec<_> = level.children.iter().map(node_name).collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "collision",
            "collision",
            "mixed\\seps",
            &("x".repeat(500) + "long-component")
        ]
    );
    let mut level = level
        .children
        .iter()
        .find_map(|node| match node {
            ArchiveNode::Directory(d) if d.name == "mixed\\seps" => Some(d),
            _ => None,
        })
        .expect("literal backslash directory");
    for segment in [".", "down", ".."] {
        level = level
            .children
            .iter()
            .find_map(|node| match node {
                ArchiveNode::Directory(d) if d.name == *segment => Some(d),
                _ => None,
            })
            .unwrap_or_else(|| panic!("literal {segment} directory"));
    }
    assert_eq!(level.children.len(), 2);
    assert!(
        level
            .children
            .iter()
            .all(|node| matches!(node, ArchiveNode::File { name, .. } if name == "leaf.txt"))
    );
}

#[test]
fn archive_preview_tree_indexes_wide_sibling_lists() {
    let mut entries = Vec::new();
    for index in 0..5000 {
        entries.push(ArchiveFileEntry {
            name: format!("file-{index:05}.txt"),
            directory: false,
            size: index as u64,
        });
    }
    entries.push(ArchiveFileEntry {
        name: "shared".to_owned(),
        directory: false,
        size: 1,
    });
    entries.push(ArchiveFileEntry {
        name: "shared".to_owned(),
        directory: true,
        size: 0,
    });
    entries.push(ArchiveFileEntry {
        name: "shared".to_owned(),
        directory: false,
        size: 1,
    });
    entries.push(ArchiveFileEntry {
        name: "nested/deep.txt".to_owned(),
        directory: false,
        size: 2,
    });
    let tree = archive_preview_tree(entries);
    assert_eq!(tree.file_count, 5003);
    let names: Vec<_> = tree
        .root
        .children
        .iter()
        .map(|node| match node {
            ArchiveNode::Directory(directory) => (true, directory.name.clone()),
            ArchiveNode::File { name, .. } => (false, name.clone()),
        })
        .collect();
    assert_eq!(names.len(), 5004);
    assert_eq!(
        &names[..2],
        &[(true, "nested".to_owned()), (true, "shared".to_owned()),]
    );
    assert_eq!(names[2], (false, "file-00000.txt".to_owned()));
    assert_eq!(names[5001], (false, "file-04999.txt".to_owned()));
    assert_eq!(
        &names[5002..],
        &[(false, "shared".to_owned()), (false, "shared".to_owned()),]
    );
    let files: Vec<_> = names[2..].to_vec();
    let mut sorted = files.clone();
    sorted.sort_by(|left, right| {
        left.1
            .to_ascii_lowercase()
            .cmp(&right.1.to_ascii_lowercase())
            .then_with(|| left.1.cmp(&right.1))
    });
    assert_eq!(files, sorted);
    let nested = tree
        .root
        .children
        .iter()
        .find_map(|node| match node {
            ArchiveNode::Directory(directory) if directory.name == "nested" => Some(directory),
            _ => None,
        })
        .expect("nested directory");
    assert_eq!(nested.children.len(), 1);
}

fn identity_entries() -> Vec<ArchiveFileEntry> {
    [
        "../../etc/passwd",
        "./normal.txt",
        "dir/../hidden.txt",
        "a\\b.txt",
        "x",
        "x/child.txt",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| ArchiveFileEntry {
        name: name.to_owned(),
        directory: false,
        size: index as u64,
    })
    .collect()
}

fn find_dir<'a>(directory: &'a ArchiveDirectory, name: &str) -> &'a ArchiveDirectory {
    directory
        .children
        .iter()
        .find_map(|node| match node {
            ArchiveNode::Directory(child) if child.name == name => Some(child),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected directory {name:?}"))
}

#[test]
fn archive_preview_tree_preserves_literal_member_identities() {
    let tree = archive_preview_tree(identity_entries());
    assert_eq!(tree.file_count, 6);
    assert!(
        tree.root
            .children
            .iter()
            .any(|node| matches!(node, ArchiveNode::File { name, .. } if name == "a\\b.txt"))
    );
    let dot = find_dir(&tree.root, ".");
    assert!(
        dot.children
            .iter()
            .any(|node| matches!(node, ArchiveNode::File { name, .. } if name == "normal.txt"))
    );
    let parent = find_dir(find_dir(&tree.root, "dir"), "..");
    assert!(
        parent
            .children
            .iter()
            .any(|node| matches!(node, ArchiveNode::File { name, .. } if name == "hidden.txt"))
    );
    let grandparent = find_dir(find_dir(&tree.root, ".."), "..");
    let etc = find_dir(grandparent, "etc");
    assert!(
        etc.children
            .iter()
            .any(|node| matches!(node, ArchiveNode::File { name, .. } if name == "passwd"))
    );
    let files: Vec<_> = tree
        .root
        .children
        .iter()
        .filter(|node| matches!(node, ArchiveNode::File { name, .. } if name == "x"))
        .collect();
    assert_eq!(files.len(), 1);
    find_dir(&tree.root, "x");
}

#[test]
fn archive_preview_tree_is_independent_of_member_order() {
    let forward = identity_entries();
    let mut backward = identity_entries();
    backward.reverse();
    assert_eq!(
        archive_preview_tree(forward),
        archive_preview_tree(backward)
    );
    let dupe = || ArchiveFileEntry {
        name: "duplicate.txt".to_owned(),
        directory: false,
        size: 1,
    };
    let tree = archive_preview_tree(vec![dupe(), dupe()]);
    assert_eq!(tree.file_count, 2);
    assert_eq!(tree.root.children.len(), 2);
}

fn password_request(password: Option<&str>) -> super::PreviewRequest {
    use crate::model::{EntryKind, FileEntry, Location, MetadataValue};

    super::PreviewRequest {
        id: super::PreviewRequestId(1),
        entry: FileEntry {
            location: Location::local(std::path::Path::new("/tmp/secret.zip")),
            thumbnail_path: None,
            native_name: "secret.zip".into(),
            display_name: "secret.zip".into(),
            kind: EntryKind::File,
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Unknown,
            mode: MetadataValue::Unknown,
            recent_unix_seconds: MetadataValue::Unknown,
            image_dimensions: MetadataValue::Unknown,
            child_count: MetadataValue::Unknown,
            duration_seconds: MetadataValue::Unknown,
            is_hidden: false,
        },
        text_byte_limit: 1024,
        render_document: false,
        pdf_page: 0,
        media_size: MediaPreviewSize::new(640, 800),
        archive_password: password.map(|password| super::SecretString::new(password.to_owned())),
    }
}

#[test]
fn preview_request_debug_redacts_the_archive_password() {
    let rendered = format!(
        "{:?}",
        password_request(Some("strata-security-repro-password"))
    );
    assert!(
        !rendered.contains("strata-security-repro-password"),
        "Debug must never expose the password, got {rendered:?}"
    );
    assert!(
        rendered.contains("Some([REDACTED])"),
        "a present password must render redacted-but-present, got {rendered:?}"
    );
    let rendered = format!("{:?}", password_request(None));
    assert!(
        rendered.contains("archive_password: None"),
        "an absent password must stay distinguishable, got {rendered:?}"
    );
}

#[test]
fn secret_string_exposes_its_value_explicitly() {
    let secret = super::SecretString::new("s3cret".to_owned());
    assert_eq!(secret.expose(), "s3cret");
    assert_eq!(format!("{secret:?}"), "[REDACTED]");
}

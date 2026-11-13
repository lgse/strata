// SPDX-License-Identifier: MIT

//! What the delete confirmation says of its items, ported from the GTK interface: their
//! counts, how far a deletion reaches, and their sizes. Each function names the GTK function
//! it copies; a change to one on `main` is a change to its copy here (`A7.md`, "Ported from
//! GTK: watch merges from `main`").

use std::time::Duration;

use strata_core::{
    i18n::{count, file_size, tr},
    model::{EntryKind, FileEntry, Location, MetadataValue},
};
use strata_fs::adapters::{
    directory_summary::{DirectorySummary, summarize_directory},
    gio_file_for_location, mount_watch, query_file_entry, trash_support,
};

/// `item_count_label` in `src/ui/browser/entry.rs`.
pub(super) fn item_count_label(n: usize) -> String {
    count("items", n)
}

/// `entry_kind_summary` in `src/ui/browser/entry.rs`.
pub(super) fn entry_kind_summary(entries: &[FileEntry]) -> String {
    let directories = entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Directory)
        .count();
    let files = entries.iter().filter(|entry| entry.is_file()).count();
    match (files, directories) {
        (files, 0) if files == entries.len() => count("files", files),
        (0, directories) if directories == entries.len() => count("folders", directories),
        _ => item_count_label(entries.len()),
    }
}

/// How far a deletion that bypasses Trash reaches (`DeletionReach` in
/// `src/ui/browser/trash.rs`): a network or cloud provider may keep its own copy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Reach {
    Local,
    Remote,
}

/// `deletion_reach` in `src/ui/browser/trash.rs`: a mixed selection takes the stronger,
/// local, warning.
pub(super) fn deletion_reach(entries: &[FileEntry]) -> Reach {
    let mounts = mount_watch::latest();
    let remote = |location: &Location| match location.native_path() {
        Some(path) => mounts.is_remote_share(path),
        None => {
            !location.is_administrator()
                && !location
                    .uri_value()
                    .and_then(glib::Uri::parse_scheme)
                    .is_some_and(|scheme| scheme.eq_ignore_ascii_case("trash"))
        }
    };
    if !entries.is_empty() && entries.iter().all(|entry| remote(&entry.location)) {
        Reach::Remote
    } else {
        Reach::Local
    }
}

/// `reach_description` in `src/ui/browser/trash.rs`.
pub(super) fn reach_description(reach: Reach, single: bool) -> String {
    tr(match (reach, single) {
        (Reach::Local, true) => {
            "This item will be permanently deleted. This action cannot be undone."
        }
        (Reach::Local, false) => {
            "These items will be permanently deleted. This action cannot be undone."
        }
        (Reach::Remote, true) => {
            "This item will be deleted without using Trash. Strata can't undo this; whether it can be recovered depends on the storage provider."
        }
        (Reach::Remote, false) => {
            "These items will be deleted without using Trash. Strata can't undo this; whether they can be recovered depends on the storage provider."
        }
    })
}

/// `trash_unavailable_reason` in `src/ui/browser/trash/delete_dialog.rs`: the mount a failed
/// trash attempt recorded, when the first item is on one.
pub(super) fn trash_unavailable_reason(entries: &[FileEntry]) -> Option<String> {
    let first = &entries.first()?.location;
    if first.is_administrator() {
        return Some(tr("Trash isn't available when browsing as administrator."));
    }
    let parent = first.parent()?;
    let (mount, _) = trash_support::unsupported_reason_for(parent.native_path()?)?;
    Some(
        rust_i18n::t!(
            "Trash isn't available on %{mount_point} (%{fs_type}).",
            mount_point = mount.mount_point.display(),
            fs_type = mount.fs_type
        )
        .into_owned(),
    )
}

/// `delete_confirmation_file_size` in `src/ui/browser/trash/delete_dialog.rs`.
pub(super) fn size_text(size: &MetadataValue<u64>) -> String {
    match size {
        MetadataValue::Known(size) => file_size(*size),
        MetadataValue::Unknown | MetadataValue::Unavailable => "—".to_owned(),
    }
}

/// `delete_entry_metadata_text` in `src/ui/browser/trash/delete_dialog.rs`: a file link's
/// size waits for its target's.
pub(super) fn metadata_text(entry: &FileEntry) -> String {
    if entry.kind == EntryKind::DirectorySymbolicLink {
        tr("Folder link")
    } else if entry.is_directory() {
        tr("Folder")
    } else if entry.kind == EntryKind::FileSymbolicLink {
        size_text(&MetadataValue::Unknown)
    } else {
        size_text(&entry.size)
    }
}

/// `permanent_delete_subtitle` in `src/ui/browser/trash/delete_dialog.rs`.
pub(super) fn permanent_delete_subtitle(reach: Reach, summary: &DirectorySummary) -> String {
    let template = match (reach, summary.truncated()) {
        (Reach::Local, true) => "At least %{items} · at least %{size} will be permanently deleted",
        (Reach::Local, false) => "%{items} · %{size} will be permanently deleted",
        (Reach::Remote, true) => "At least %{items} · at least %{size} will be deleted",
        (Reach::Remote, false) => "%{items} · %{size} will be deleted",
    };
    rust_i18n::t!(
        template,
        items = item_count_label(summary.item_count),
        size = file_size(summary.total_size)
    )
    .into_owned()
}

/// `delete_entry_size` in `src/ui/browser/entry.rs`: a file link counts its target's size.
async fn delete_entry_size(entry: &FileEntry) -> MetadataValue<u64> {
    if entry.kind == EntryKind::FileSymbolicLink {
        query_file_entry(entry.location.clone())
            .await
            .map_or(MetadataValue::Unavailable, |link| link.size)
    } else {
        entry.size.clone()
    }
}

/// `aggregate_directory_summary` in `src/ui/browser/entry.rs`: every item and what each
/// folder holds, with each file link's size as it is found (`on_link`).
pub(super) async fn aggregate_directory_summary(
    entries: &[FileEntry],
    mut on_link: impl FnMut(usize, &MetadataValue<u64>),
) -> DirectorySummary {
    let mut total = DirectorySummary::default();
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 && index % 256 == 0 {
            glib::timeout_future(Duration::ZERO).await;
        }
        if entry.kind == EntryKind::Directory {
            let directory = gio_file_for_location(&entry.location);
            match summarize_directory(&directory).await {
                Ok(summary) => total.include(summary),
                Err(_) => total.issues.unreadable = true,
            }
        } else {
            total.item_count = total.item_count.saturating_add(1);
            total.visible_file_count = total.visible_file_count.saturating_add(1);
            let size = delete_entry_size(entry).await;
            if entry.kind == EntryKind::FileSymbolicLink {
                on_link(index, &size);
            }
            match size {
                MetadataValue::Known(size) => {
                    total.total_size = total.total_size.saturating_add(size);
                }
                MetadataValue::Unknown | MetadataValue::Unavailable => {
                    if !entry.is_symbolic_link() || entry.kind == EntryKind::FileSymbolicLink {
                        total.issues.unreadable = true;
                    }
                }
            }
        }
    }
    total
}

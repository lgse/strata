// SPDX-License-Identifier: MIT

//! Archive member tree preview and encrypted-archive password prompt (`A13`), hosted inside
//! [`super::Preview`] over `strata_components::viewers::archive`.

use rune::{El, KeyEvent, Tx};
use strata_components::viewers::archive::{
    ArchiveDir, ArchiveItem, ArchivePrompt, ArchiveState, ArchiveTree, ArchiveView, PromptState,
};
use strata_core::model::FileEntry;
use strata_desktop::keys::{Active, bindings::ESCAPE_ANY_SHIFT};
use strata_fs::services::{
    ArchiveDirectory, ArchiveNode, ArchivePreviewTree, SecretString, archive_preview_format,
};
use strata_kit::look::Look;

use super::{keys, labels, load};
use crate::{
    App,
    commands::{self, Situation},
    shell,
    slots::Ids,
};

/// Collection scope key of the archive member tree inside the preview drawer.
pub(crate) const ARCHIVE_KEY: &str = "preview-archive";
/// Root key of the encrypted-archive password prompt inside the preview drawer.
pub(crate) const PROMPT_KEY: &str = "preview-archive-prompt";
/// Text input key of the encrypted-archive password prompt.
pub(crate) const PROMPT_FIELD_KEY: &str = "preview-archive-prompt-field";

/// Whether `entry` is a local previewable archive (`is_previewable_archive` in
/// `src/ui/window/keyboard/items/preview_keys.rs`).
pub(super) fn is_previewable_archive(entry: &FileEntry) -> bool {
    !entry.is_directory()
        && entry.location.native_path().is_some()
        && archive_preview_format(&entry.native_name).is_some()
}

/// Builds a passive [`ArchiveState`] for `tree`, allocating unique [`EntryId`]s from `ids`.
pub(super) fn build_state(tree: &ArchivePreviewTree, ids: &Ids) -> ArchiveState {
    let dir = convert_directory(&tree.root, true);
    let built = ArchiveTree::new(dir, || ids.fresh());
    ArchiveState::new(ARCHIVE_KEY, built).with_labels(labels::archive())
}

fn convert_directory(directory: &ArchiveDirectory, root: bool) -> ArchiveDir {
    let children = directory
        .children
        .iter()
        .map(|node| match node {
            ArchiveNode::Directory(sub) => ArchiveItem::Dir(convert_directory(sub, false)),
            ArchiveNode::File { name, size } => ArchiveItem::File {
                name: name.clone(),
                size: *size,
                icon: crate::entry_icon::icon_for_name(name).to_owned(),
            },
        })
        .collect();
    ArchiveDir {
        name: if root {
            String::new()
        } else {
            directory.name.clone()
        },
        children,
    }
}

pub(super) fn build_archive_view(
    app: &App,
    get: fn(&App) -> &ArchiveState,
    get_mut: fn(&mut App) -> &mut ArchiveState,
) -> El {
    archive_view(app, get, get_mut).build(app)
}

pub(super) fn tick_archive(
    app: &mut App,
    get: fn(&App) -> &ArchiveState,
    get_mut: fn(&mut App) -> &mut ArchiveState,
    tx: &mut Tx,
) -> bool {
    archive_view(app, get, get_mut).tick(app, tx)
}

fn archive_view(
    app: &App,
    get: fn(&App) -> &ArchiveState,
    get_mut: fn(&mut App) -> &mut ArchiveState,
) -> ArchiveView<App> {
    let letters = !shell::live_keys(app).type_to_search_active();
    ArchiveView::new(get, get_mut, |app: &App| &app.look)
        .letters(letters)
        .on_leave(|app: &mut App, _| app.preview.drawer.focus_files())
}

pub(super) fn build_prompt_view(
    prompt: &PromptState,
    look: &Look,
    access: fn(&mut App) -> &mut PromptState,
) -> El {
    ArchivePrompt::new(prompt, access)
        .key(PROMPT_KEY)
        .labels(labels::archive_prompt())
        .on_field_key(|app: &mut App, ev: &KeyEvent, tx: &mut Tx| {
            let at = Situation::of(app, tx);
            let press = commands::read(app, ev, &at, tx);
            let returns = shell::preferences(app).tenxer_mode && press.is(keys::TENXER_RETURN);
            if press.is(ESCAPE_ANY_SHIFT) || returns {
                return commands::on_key(app, ev, tx);
            }
            false
        })
        .on_unlock(|app: &mut App, password, _| {
            let Some(entry) = app.preview.current.clone() else {
                return;
            };
            app.preview.loader.focus_archive_on_ready = true;
            load::start_with_password(app, entry, SecretString::new(password));
        })
        .build(look)
}

#[cfg(test)]
mod tests;

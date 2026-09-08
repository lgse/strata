// SPDX-License-Identifier: MIT

//! Bounded GIO Trash deletion, independent of browser widgets.

use gio::prelude::*;

pub(crate) struct EmptyTrashOutcome {
    pub(crate) deleted: usize,
    pub(crate) failed: usize,
    /// Capped at 8 messages regardless of `failed`.
    pub(crate) errors: Vec<String>,
}

/// Deletes one batch at a time, independently of any prior measurement's budget or truncation.
pub(crate) async fn empty_trash(
    root: &gio::File,
    mut on_progress: impl FnMut(usize),
) -> Result<EmptyTrashOutcome, glib::Error> {
    let enumerator = root
        .enumerate_children_future(
            "standard::name,standard::display-name",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?;
    let mut outcome = EmptyTrashOutcome {
        deleted: 0,
        failed: 0,
        errors: Vec::new(),
    };
    loop {
        let children = enumerator
            .next_files_future(64, glib::Priority::DEFAULT)
            .await?;
        if children.is_empty() {
            break;
        }
        for info in children {
            let file = root.child(info.name());
            match file.delete_future(glib::Priority::DEFAULT).await {
                Ok(_) => outcome.deleted += 1,
                Err(error) => {
                    outcome.failed += 1;
                    if outcome.errors.len() < 8 {
                        outcome
                            .errors
                            .push(format!("{}: {error}", info.display_name()));
                    }
                }
            }
        }
        on_progress(outcome.deleted + outcome.failed);
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests;

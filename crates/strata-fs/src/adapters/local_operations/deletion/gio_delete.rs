// SPDX-License-Identifier: MIT

use std::{future::Future, pin::Pin, rc::Rc};

use gio::prelude::*;

use super::super::{await_cancellable, leftovers::Deadline, next_children};
use crate::adapters::{gio_close, trash_support::TrashError};

pub(in crate::adapters::local_operations) fn permanently_delete(
    file: gio::File,
    directory: bool,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        delete_tree(file, directory, cancellable, None)
            .await
            .map_err(|error| error.error)
    })
}

pub(in crate::adapters::local_operations) async fn permanently_delete_counted(
    file: gio::File,
    directory: bool,
    cancellable: gio::Cancellable,
    deadline: Deadline,
) -> Result<(), TrashError> {
    delete_tree(file, directory, cancellable, Some(deadline)).await
}

fn delete_tree(
    file: gio::File,
    directory: bool,
    cancellable: gio::Cancellable,
    deadline: Option<Deadline>,
) -> Pin<Box<dyn Future<Output = Result<(), TrashError>>>> {
    Box::pin(async move {
        if directory {
            delete_children(&file, &cancellable, deadline.as_ref()).await?;
        }
        delete_file(&file, &cancellable)
            .await
            .map_err(TrashError::permanent)?;
        if let Some(deadline) = deadline {
            deadline.note_removed();
        }
        Ok(())
    })
}

async fn delete_file(file: &gio::File, cancellable: &gio::Cancellable) -> Result<(), glib::Error> {
    await_cancellable(file, cancellable, |file, cancellable, result| {
        file.delete_async(glib::Priority::DEFAULT, Some(cancellable), move |output| {
            result.resolve(output)
        });
    })
    .await
}

async fn delete_children(
    file: &gio::File,
    cancellable: &gio::Cancellable,
    deadline: Option<&Deadline>,
) -> Result<(), TrashError> {
    let enumerator = gio_close::enumerate_children(
        file,
        "standard::name,standard::type",
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(cancellable),
    )
    .await
    .map_err(TrashError::permanent)?;
    loop {
        let children = next_children(&enumerator, cancellable)
            .await
            .map_err(TrashError::permanent)?;
        if children.is_empty() {
            return Ok(());
        }
        for child in children {
            let name = child.name();
            delete_tree(
                file.child(&name),
                child.file_type() == gio::FileType::Directory,
                cancellable.clone(),
                deadline.cloned(),
            )
            .await
            .map_err(|error| error.below(name))?;
        }
    }
}

pub(in crate::adapters::local_operations) fn permanently_delete_keeping(
    file: gio::File,
    kept: Rc<Vec<gio::File>>,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        if kept.iter().any(|kept| kept.equal(&file)) {
            return Ok(());
        }
        if !kept.iter().any(|kept| kept.has_prefix(&file)) {
            return permanently_delete(file, true, cancellable).await;
        }
        let enumerator = gio_close::enumerate_children(
            &file,
            "standard::name,standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(&cancellable),
        )
        .await?;
        loop {
            let children = next_children(&enumerator, &cancellable).await?;
            if children.is_empty() {
                return Ok(());
            }
            for child in children {
                delete_child_keeping(&file, child, &kept, &cancellable).await?;
            }
        }
    })
}

async fn delete_child_keeping(
    file: &gio::File,
    child: gio::FileInfo,
    kept: &Rc<Vec<gio::File>>,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    let child_file = file.child(child.name());
    if child.file_type() == gio::FileType::Directory {
        return permanently_delete_keeping(child_file, kept.clone(), cancellable.clone()).await;
    }
    if kept.iter().any(|kept| kept.equal(&child_file)) {
        return Ok(());
    }
    permanently_delete(child_file, false, cancellable.clone()).await
}

// SPDX-License-Identifier: MIT

use std::{future::Future, pin::Pin, rc::Rc};

use gio::prelude::*;

use super::super::{await_cancellable, next_children};
use crate::adapters::gio_close;

pub(in crate::adapters::local_operations) fn permanently_delete(
    file: gio::File,
    directory: bool,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        if directory {
            delete_children(&file, &cancellable).await?;
        }
        delete_file(&file, &cancellable).await
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
) -> Result<(), glib::Error> {
    let enumerator = gio_close::enumerate_children(
        file,
        "standard::name,standard::type",
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(cancellable),
    )
    .await?;
    loop {
        let children = await_cancellable(
            &*enumerator,
            cancellable,
            |enumerator, cancellable, result| {
                enumerator.next_files_async(
                    64,
                    glib::Priority::DEFAULT,
                    Some(cancellable),
                    move |output| result.resolve(output),
                );
            },
        )
        .await?;
        if children.is_empty() {
            return Ok(());
        }
        for child in children {
            permanently_delete(
                file.child(child.name()),
                child.file_type() == gio::FileType::Directory,
                cancellable.clone(),
            )
            .await?;
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

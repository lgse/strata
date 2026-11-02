// SPDX-License-Identifier: MIT

use super::*;

pub(in crate::adapters::local_operations) fn classify_merge<'a>(
    (source, target): (&'a gio::File, &'a gio::File),
    plan: &'a mut MergePlan,
    cancellable: &'a gio::Cancellable,
    (policy, folder): (&'a NamePolicy, TargetFolder),
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>> + 'a>> {
    Box::pin(async move {
        let enumerator;
        let mut listed = match native_children(source, cancellable).await? {
            Some(children) => NamedChildren::listed(children, folder.names),
            None => {
                enumerator = enumerate_children(source, cancellable).await?;
                NamedChildren::new(&enumerator, folder.names)
            }
        };
        loop {
            let children = listed.next(cancellable).await?;
            if children.is_empty() {
                return Ok(());
            }
            // Left out, so nothing at its name is replaced.
            let kept = children
                .into_iter()
                .filter(|(_, kind, _)| *kind != gio::FileType::SymbolicLink || folder.names.links);
            for (name, kind, planned) in kept {
                let child = (
                    source.child(name),
                    kind == gio::FileType::Directory,
                    target.child(planned.target),
                );
                classify_merge_child(child, plan, cancellable, (policy, folder)).await?;
            }
        }
    })
}

async fn enumerate_children(
    source: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<gio::FileEnumerator, glib::Error> {
    await_cancellable(source, cancellable, |source, cancellable, result| {
        source.enumerate_children_async(
            "standard::name,standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await
}

async fn native_children(
    folder: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<Option<Vec<(OsString, gio::FileType)>>, glib::Error> {
    let Some(path) = folder.path().filter(|_| folder.is_native()) else {
        return Ok(None);
    };
    let cancellable = cancellable.clone();
    run_blocking(move || {
        let mut children = Vec::new();
        let listed = list_folder(&path, &mut |chunk| {
            children.extend(chunk.into_iter().map(|(name, kind)| (name, gio_kind(kind))));
            if cancellable.is_cancelled() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        if cancellable.is_cancelled() {
            return Err(cancelled_local_operation());
        }
        listed.map_err(trash::errno_error)?;
        Ok(Some(children))
    })
    .await
}

fn gio_kind(kind: rustix::fs::FileType) -> gio::FileType {
    match kind {
        rustix::fs::FileType::Directory => gio::FileType::Directory,
        rustix::fs::FileType::Symlink => gio::FileType::SymbolicLink,
        rustix::fs::FileType::RegularFile => gio::FileType::Regular,
        _ => gio::FileType::Special,
    }
}

async fn classify_merge_child(
    (child_source, source_is_directory, child_target): (gio::File, bool, gio::File),
    plan: &mut MergePlan,
    cancellable: &gio::Cancellable,
    (policy, folder): (&NamePolicy, TargetFolder),
) -> Result<(), glib::Error> {
    if cancellable.is_cancelled() {
        return Err(cancelled_local_operation());
    }
    let target_info = merge_target_info(&child_target, cancellable).await?;
    match target_info.as_ref().map(gio::FileInfo::file_type) {
        None => plan.created.extend(location_for_file(&child_target)),
        Some(gio::FileType::Directory) if source_is_directory => {
            let folders = (&child_source, &child_target, target_info);
            classify_subfolder(folders, plan, cancellable, (policy, folder)).await?;
        }
        Some(kind) if (kind == gio::FileType::Directory) != source_is_directory => {
            return Err(glib::Error::new(
                gio::IOErrorEnum::NotSupported,
                &crate::i18n::tr("A file and a folder cannot safely replace one another"),
            ));
        }
        Some(_) => plan.overwritten.extend(location_for_file(&child_target)),
    }
    Ok(())
}

/// `None` where nothing has the name.
async fn merge_target_info(
    target: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<Option<gio::FileInfo>, glib::Error> {
    let queried = await_cancellable(target, cancellable, |target, cancellable, result| {
        target.query_info_async(
            "standard::type,unix::device",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await;
    match queried {
        Ok(info) => Ok(Some(info)),
        Err(error) if error.matches(gio::IOErrorEnum::NotFound) => Ok(None),
        Err(error) => Err(error),
    }
}

async fn classify_subfolder(
    (child_source, child_target, target_info): (&gio::File, &gio::File, Option<gio::FileInfo>),
    plan: &mut MergePlan,
    cancellable: &gio::Cancellable,
    (policy, folder): (&NamePolicy, TargetFolder),
) -> Result<(), glib::Error> {
    let folders = (child_source, child_target);
    let source_device = if child_source.is_native() && child_target.is_native() {
        merged_folder_device(child_source, cancellable).await?
    } else {
        None
    };
    let devices = (
        source_device,
        target_info.and_then(|info| device(child_target, &info)),
    );
    let folder = existing_folder_names(policy, folder, folders, devices, cancellable).await?;
    classify_merge(folders, plan, cancellable, (policy, folder)).await
}

pub(super) async fn merged_folder_device(
    folder: &gio::File,
    cancellable: &gio::Cancellable,
) -> Result<Option<u64>, glib::Error> {
    let info = await_cancellable(folder, cancellable, |folder, cancellable, result| {
        folder.query_info_async(
            "standard::type,unix::device",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await?;
    if info.file_type() != gio::FileType::Directory {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            &crate::i18n::tr("Only folders can be merged"),
        ));
    }
    Ok(device(folder, &info))
}

// SPDX-License-Identifier: MIT

use super::*;

pub(in crate::adapters::local_operations) fn copy_failure_on_fat32(
    error: &glib::Error,
    fat32_destination: bool,
) -> (Option<String>, bool) {
    if was_cancelled(error) {
        return (None, false);
    }
    if !fat32_destination {
        return (Some(crate::services::gio_error_message(error)), false);
    }
    let description = if error.message().contains("File too large") {
        crate::i18n::tr(
            "A file is too large for this FAT32 drive (maximum file size: 4 GiB). Use an exFAT or another large-file-capable drive instead.",
        )
    } else {
        crate::services::gio_error_message(error)
    };
    let details = rust_i18n::t!(
        "Earlier completed copies may still be writing; wait for safe eject before unplugging. Details: %{error}",
        error = crate::services::gio_error_detail(error)
    );
    (
        Some(crate::i18n::join_sentences(&description, &details)),
        true,
    )
}

/// Size probes never fail with NoSpace on their own, so the code marks this refusal.
pub(in crate::adapters::local_operations) fn fat32_file_too_large(name: &str) -> glib::Error {
    glib::Error::new(
        gio::IOErrorEnum::NoSpace,
        &rust_i18n::t!(
            "%{name} is too large for a FAT32 drive (maximum file size: 4 GiB). Use an exFAT or another large-file-capable drive instead.",
            name = name
        ),
    )
}

#[derive(Clone)]
pub(in crate::adapters::local_operations) struct TransferEstimate {
    pub(in crate::adapters::local_operations) bytes: Option<u64>,
    pub(in crate::adapters::local_operations) files: usize,
    pub(in crate::adapters::local_operations) kind: gio::FileType,
    pub(in crate::adapters::local_operations) filesystem: Option<glib::GString>,
}

pub(in crate::adapters::local_operations) fn transfer_size(
    file: gio::File,
    cancellable: gio::Cancellable,
    max_file_size: Option<u64>,
) -> Pin<Box<dyn Future<Output = Result<TransferEstimate, glib::Error>>>> {
    Box::pin(async move {
        let info = await_cancellable(&file, &cancellable, |file, cancellable, result| {
            file.query_info_async(
                "standard::type,standard::size,id::filesystem",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await?;
        let filesystem = info.attribute_string(gio::FILE_ATTRIBUTE_ID_FILESYSTEM);
        if info.file_type() != gio::FileType::Directory {
            return file_estimate(&file, &info, max_file_size, filesystem);
        }
        folder_estimate(&file, (&cancellable, max_file_size), filesystem).await
    })
}

fn file_estimate(
    file: &gio::File,
    info: &gio::FileInfo,
    max_file_size: Option<u64>,
    filesystem: Option<glib::GString>,
) -> Result<TransferEstimate, glib::Error> {
    let size = info
        .has_attribute(gio::FILE_ATTRIBUTE_STANDARD_SIZE)
        .then(|| info.size().max(0) as u64);
    if info.file_type() == gio::FileType::Regular && exceeds(max_file_size, size) {
        return Err(fat32_file_too_large(
            &transfer_source_name(file)
                .unwrap_or_default()
                .to_string_lossy(),
        ));
    }
    Ok(TransferEstimate {
        bytes: size,
        files: 1,
        kind: info.file_type(),
        filesystem,
    })
}

fn exceeds(limit: Option<u64>, size: Option<u64>) -> bool {
    matches!((limit, size), (Some(limit), Some(size)) if size > limit)
}

async fn folder_estimate(
    file: &gio::File,
    (cancellable, max_file_size): (&gio::Cancellable, Option<u64>),
    filesystem: Option<glib::GString>,
) -> Result<TransferEstimate, glib::Error> {
    let enumerator = gio_close::enumerate_children(
        file,
        "standard::name",
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(cancellable),
    )
    .await?;
    let mut estimate = TransferEstimate {
        bytes: Some(0_u64),
        files: 0_usize,
        kind: gio::FileType::Directory,
        filesystem,
    };
    loop {
        let children = next_children(&enumerator, cancellable).await?;
        if children.is_empty() {
            return Ok(estimate);
        }
        for chunk in children.chunks(MAX_LOCAL_IO_WORKERS) {
            add_children(&mut estimate, file, chunk, (cancellable, max_file_size)).await?;
        }
    }
}

async fn add_children(
    estimate: &mut TransferEstimate,
    file: &gio::File,
    chunk: &[gio::FileInfo],
    (cancellable, max_file_size): (&gio::Cancellable, Option<u64>),
) -> Result<(), glib::Error> {
    let context = glib::MainContext::default();
    let tasks = chunk
        .iter()
        .map(|child| {
            context.spawn_local(transfer_size(
                file.child(child.name()),
                cancellable.clone(),
                max_file_size,
            ))
        })
        .collect::<Vec<_>>();
    for task in tasks {
        let child_size = task
            .await
            .map_err(|_| io_error("Transfer-size worker failed"))??;
        estimate.bytes = estimate
            .bytes
            .and_then(|total| child_size.bytes.and_then(|size| total.checked_add(size)));
        estimate.files = estimate.files.saturating_add(child_size.files);
    }
    Ok(())
}

#[derive(Debug)]
pub(in crate::adapters::local_operations) struct TransferSizes {
    pub(in crate::adapters::local_operations) sizes: Vec<Option<u64>>,
    pub(in crate::adapters::local_operations) total: Option<u64>,
    pub(in crate::adapters::local_operations) files: Vec<Option<usize>>,
    pub(in crate::adapters::local_operations) total_files: Option<usize>,
    pub(in crate::adapters::local_operations) kinds: Vec<Option<gio::FileType>>,
    pub(in crate::adapters::local_operations) filesystems: Vec<Option<glib::GString>>,
}

pub(in crate::adapters::local_operations) async fn transfer_sizes(
    files: &[gio::File],
    cancellable: &gio::Cancellable,
    max_file_size: Option<u64>,
) -> Result<TransferSizes, glib::Error> {
    let mut sizes = Vec::with_capacity(files.len());
    let mut counts = Vec::with_capacity(files.len());
    let mut total = Some(0_u64);
    let mut total_files = Some(0_usize);
    let mut kinds = Vec::with_capacity(files.len());
    let mut filesystems = Vec::with_capacity(files.len());
    for chunk in files.chunks(MAX_LOCAL_IO_WORKERS) {
        let context = glib::MainContext::default();
        let tasks = chunk
            .iter()
            .map(|file| {
                context.spawn_local(transfer_size(
                    file.clone(),
                    cancellable.clone(),
                    max_file_size,
                ))
            })
            .collect::<Vec<_>>();
        for task in tasks {
            let size = match task.await {
                Ok(Ok(size)) => Some(size),
                Ok(Err(error))
                    if was_cancelled(&error) || error.matches(gio::IOErrorEnum::NoSpace) =>
                {
                    return Err(error);
                }
                Ok(Err(_)) | Err(_) => None,
            };
            total = total.and_then(|total| {
                size.as_ref()
                    .and_then(|size| size.bytes.and_then(|bytes| total.checked_add(bytes)))
            });
            total_files = total_files
                .and_then(|total| size.as_ref().map(|size| total.saturating_add(size.files)));
            counts.push(size.as_ref().map(|size| size.files));
            sizes.push(size.as_ref().and_then(|size| size.bytes));
            kinds.push(size.as_ref().map(|size| size.kind));
            filesystems.push(size.and_then(|size| size.filesystem));
        }
    }
    Ok(TransferSizes {
        sizes,
        total,
        files: counts,
        total_files,
        kinds,
        filesystems,
    })
}

// SPDX-License-Identifier: MIT

use super::*;

/// How a GIO copy of a tree writes each item.
#[derive(Clone)]
pub(in crate::adapters::local_operations) struct GioCopy {
    pub(in crate::adapters::local_operations) into: CopyInto,
    pub(in crate::adapters::local_operations) cancellable: gio::Cancellable,
    pub(in crate::adapters::local_operations) record: CopyRecord,
    pub(in crate::adapters::local_operations) progress: Option<Rc<TransferProgressTracker>>,
    pub(in crate::adapters::local_operations) names: GioNames,
}

pub(in crate::adapters::local_operations) fn copy_tree(
    source: gio::File,
    target: gio::File,
    copy: GioCopy,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        let info = await_cancellable(&source, &copy.cancellable, |source, cancellable, result| {
            source.query_info_async(
                "standard::type,standard::size,unix::device",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await?;
        if info.file_type() == gio::FileType::Directory {
            copy_folder(&source, &target, &info, copy).await
        } else {
            copy_file(source, target, &info, copy).await
        }
    })
}

async fn copy_folder(
    source: &gio::File,
    target: &gio::File,
    info: &gio::FileInfo,
    copy: GioCopy,
) -> Result<(), glib::Error> {
    let existing = copy.into.existing_folder(target, &copy.cancellable).await;
    if existing.is_none() {
        let created_root = match &copy.record {
            CopyRecord::Root(root) => Some(root.clone()),
            CopyRecord::Nothing => None,
        };
        make_copy_folder(target, &copy.cancellable, &created_root).await?;
    }
    let source_folder = (source, device(source, info));
    let folder = copy
        .names
        .of(source_folder, target, existing, &copy.cancellable)
        .await?;
    let record = match copy.record {
        CopyRecord::Root(_) => CopyRecord::Nothing,
        record => record,
    };
    let enumerator = gio_close::enumerate_children(
        source,
        "standard::name,standard::type",
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(&copy.cancellable),
    )
    .await?;
    let mut children = FolderCopy {
        source,
        target,
        links: folder.names.links,
        copy: GioCopy {
            record,
            names: copy.names.within(folder),
            ..copy
        },
        renamed: Vec::new(),
    };
    let mut listed = NamedChildren::new(&enumerator, folder.names);
    let copied = children.copy_all(&mut listed).await;
    if let Some(progress) = &children.copy.progress {
        progress.report(TransferReport {
            renamed: children.renamed,
            ..TransferReport::default()
        });
    }
    copied
}

/// Copies the children of one folder, noting those saved under another name.
struct FolderCopy<'a> {
    source: &'a gio::File,
    target: &'a gio::File,
    links: bool,
    copy: GioCopy,
    renamed: Vec<RenamedItem>,
}

impl FolderCopy<'_> {
    async fn copy_all(&mut self, listed: &mut NamedChildren<'_>) -> Result<(), glib::Error> {
        loop {
            let children = listed.next(&self.copy.cancellable).await?;
            if children.is_empty() {
                return Ok(());
            }
            for (name, kind, planned) in children {
                self.copy_child(&name, kind, &planned).await?;
            }
        }
    }

    fn leaves_links_out(&self) -> bool {
        !self.links || self.copy.names.left.refused.get()
    }

    async fn copy_child(
        &mut self,
        name: &OsStr,
        kind: gio::FileType,
        planned: &Planned,
    ) -> Result<(), glib::Error> {
        let (source, target) = (self.source, self.target);
        let link = kind == gio::FileType::SymbolicLink;
        if link && self.leaves_links_out() {
            self.copy
                .names
                .left
                .add(source.child(name), target.child(name));
            return Ok(());
        }
        let copied = copy_tree(
            source.child(name),
            target.child(&planned.target),
            self.copy.clone(),
        )
        .await;
        if link && copied.as_ref().is_err_and(|error| unheld(error).is_some()) {
            self.copy.names.left.refused.set(true);
            self.copy
                .names
                .left
                .add(source.child(name), target.child(name));
            return Ok(());
        }
        self.note_renamed(name, planned, copied.is_ok()).await;
        copied
    }

    async fn note_renamed(&mut self, name: &OsStr, planned: &Planned, copied: bool) {
        let saved_as = self.target.child(&planned.target);
        let Some(cause) = planned.cause else {
            return;
        };
        if !copied && !query_exists(&saved_as, &self.copy.cancellable).await {
            return;
        }
        self.renamed
            .extend(renamed_item(&self.target.child(name), &saved_as, cause));
    }
}

async fn copy_file(
    source: gio::File,
    target: gio::File,
    info: &gio::FileInfo,
    copy: GioCopy,
) -> Result<(), glib::Error> {
    let source_size = info
        .has_attribute(gio::FILE_ATTRIBUTE_STANDARD_SIZE)
        .then(|| info.size().max(0) as u64);
    let flags = copy.into.file_flags() | gio::FileCopyFlags::NOFOLLOW_SYMLINKS;
    let track_bytes = source_size.is_none_or(|size| size >= BYTE_PROGRESS_MIN_FILE_SIZE);
    let file_progress = copy.progress.as_ref().map(|progress| {
        progress.begin_file(
            transfer_source_name(&source)
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            track_bytes,
        )
    });
    let reporter = file_progress
        .as_ref()
        .and_then(FileTransferProgress::reporter);
    let link = (info.file_type() == gio::FileType::SymbolicLink).then(|| source.clone());
    let result = copy_file_with_progress(source, target, flags, copy.cancellable, reporter)
        .await
        .map_err(|error| refused_link(link, error));
    if result.is_ok()
        && let Some(file_progress) = file_progress
    {
        file_progress.finish(source_size);
    }
    result
}

/// SMB and MTP backends refuse links.
fn refused_link(link: Option<gio::File>, error: glib::Error) -> glib::Error {
    let Some(link) = link.filter(|_| error.matches(gio::IOErrorEnum::NotSupported)) else {
        return error;
    };
    let message = rust_i18n::t!(
        "Could not copy %{name}: %{error}",
        name = transfer_source_name(&link)
            .unwrap_or_default()
            .to_string_lossy(),
        error = crate::services::gio_error_detail(&error)
    );
    glib::Error::new(Unheld::Symlink, &message)
}

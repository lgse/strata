// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CopyInto {
    New,
    Existing,
}

impl CopyInto {
    pub(super) fn file_flags(self) -> gio::FileCopyFlags {
        let overwrite = if self == Self::Existing {
            gio::FileCopyFlags::OVERWRITE
        } else {
            gio::FileCopyFlags::NONE
        };
        gio::FileCopyFlags::ALL_METADATA | overwrite
    }

    pub(super) async fn existing_folder(
        self,
        target: &gio::File,
        cancellable: &gio::Cancellable,
    ) -> Option<ExistingFolder> {
        if self == Self::New {
            return None;
        }
        let info = await_cancellable(target, cancellable, |target, cancellable, result| {
            target.query_info_async(
                "standard::type,unix::device",
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |output| result.resolve(output),
            );
        })
        .await
        .ok()?;
        Some(ExistingFolder {
            device: device(target, &info),
        })
    }
}

#[derive(Clone, Copy)]
pub(super) struct ExistingFolder {
    pub(super) device: Option<u64>,
}

pub(super) fn device(file: &gio::File, info: &gio::FileInfo) -> Option<u64> {
    let device = (file.is_native() && info.has_attribute(gio::FILE_ATTRIBUTE_UNIX_DEVICE))
        .then(|| u64::from(info.attribute_uint32(gio::FILE_ATTRIBUTE_UNIX_DEVICE)));
    #[cfg(any(test, feature = "test-support"))]
    let device = fs_quirks::gio_device(file, device);
    device
}

#[derive(Clone, Copy)]
pub(super) struct TargetFolder {
    pub(super) names: FolderNames,
    pub(super) devices: (Option<u64>, Option<u64>),
}

#[derive(Clone, Default)]
pub(super) struct GioNames {
    pub(super) policy: NamePolicy,
    pub(super) parent: Option<TargetFolder>,
    pub(super) left: LeftOut,
}

impl GioNames {
    pub(super) fn new(policy: NamePolicy, left: LeftOut) -> Self {
        Self {
            policy,
            parent: None,
            left,
        }
    }

    pub(super) async fn of(
        &self,
        (source, source_device): (&gio::File, Option<u64>),
        target: &gio::File,
        existing: Option<ExistingFolder>,
        cancellable: &gio::Cancellable,
    ) -> Result<TargetFolder, glib::Error> {
        let devices = (source_device, existing.and_then(|existing| existing.device));
        match (self.parent, existing) {
            (Some(parent), None) => Ok(parent),
            (Some(parent), Some(_)) => {
                let folders = (source, target);
                existing_folder_names(&self.policy, parent, folders, devices, cancellable).await
            }
            (None, _) => Ok(TargetFolder {
                names: self.policy.folder(source, target, cancellable).await?,
                devices,
            }),
        }
    }

    pub(super) fn within(&self, folder: TargetFolder) -> Self {
        Self {
            policy: self.policy.clone(),
            parent: Some(folder),
            left: self.left.clone(),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct LeftOut {
    pub(super) refused: Rc<Cell<bool>>,
    pub(super) links: Rc<RefCell<Vec<(gio::File, gio::File)>>>,
}

impl LeftOut {
    pub(super) fn add(&self, source: gio::File, target: gio::File) {
        self.links.borrow_mut().push((source, target));
    }

    pub(super) fn sources(&self) -> Vec<gio::File> {
        self.links
            .borrow()
            .iter()
            .map(|(source, _)| source.clone())
            .collect()
    }

    pub(super) fn report(&self, progress: Option<&TransferProgressTracker>) {
        let Some(progress) = progress else {
            return;
        };
        let skipped = self
            .links
            .borrow()
            .iter()
            .filter_map(|(_, target)| {
                Some(SkippedItem {
                    location: location_for_file(target)?,
                    kind: SpecialKind::Symlink,
                })
            })
            .collect::<Vec<_>>();
        if !skipped.is_empty() {
            progress.report(TransferReport {
                skipped,
                ..TransferReport::default()
            });
        }
    }

    pub(super) fn keep_in(&self, sources: &copy_engine::Manifest, source: &Path) {
        let Some(parent) = source.parent() else {
            return;
        };
        for (link, _) in self.links.borrow().iter() {
            if let Some(path) = link.path()
                && let Ok(path) = path.strip_prefix(parent)
            {
                sources.keep(path, KeptCause::Skipped(SpecialKind::Symlink));
            }
        }
    }
}

/// Looked up again only where another filesystem is mounted at either, as a bind mount of the
/// parent shares its rules.
pub(super) async fn existing_folder_names(
    policy: &NamePolicy,
    parent: TargetFolder,
    (source, target): (&gio::File, &gio::File),
    devices: (Option<u64>, Option<u64>),
    cancellable: &gio::Cancellable,
) -> Result<TargetFolder, glib::Error> {
    if target.is_native() && (devices.1.is_none() || devices != parent.devices) {
        Ok(TargetFolder {
            names: policy.folder(source, target, cancellable).await?,
            devices,
        })
    } else {
        Ok(parent)
    }
}

pub(super) async fn query_exists(file: &gio::File, cancellable: &gio::Cancellable) -> bool {
    await_cancellable(file, cancellable, |file, cancellable, result| {
        file.query_info_async(
            gio::FILE_ATTRIBUTE_STANDARD_TYPE,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
            Some(cancellable),
            move |output| result.resolve(output),
        );
    })
    .await
    .is_ok()
}

pub(super) async fn next_children(
    enumerator: &gio::FileEnumerator,
    cancellable: &gio::Cancellable,
) -> Result<Vec<gio::FileInfo>, glib::Error> {
    await_cancellable(
        enumerator,
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
    .await
}

pub(super) fn renamed_item(
    requested: &gio::File,
    saved_as: &gio::File,
    cause: RenameCause,
) -> Option<RenamedItem> {
    Some(RenamedItem {
        requested: location_for_file(requested)?,
        saved_as: location_for_file(saved_as)?,
        cause,
    })
}

/// Children whose names are fixed come once the folder is listed, numbered past every sibling.
/// Where the mount takes two names for one, the whole folder is listed first.
pub(super) struct NamedChildren<'a> {
    pub(super) children: Children<'a>,
    pub(super) names: FolderNames,
    pub(super) kept: Vec<OsString>,
    pub(super) held: Vec<(OsString, gio::FileType)>,
    pub(super) listed: bool,
}

pub(super) enum Children<'a> {
    Enumerated(&'a gio::FileEnumerator),
    Listed(Vec<(OsString, gio::FileType)>),
}

impl<'a> NamedChildren<'a> {
    pub(super) fn new(enumerator: &'a gio::FileEnumerator, names: FolderNames) -> Self {
        Self::of(Children::Enumerated(enumerator), names)
    }

    pub(super) fn listed(children: Vec<(OsString, gio::FileType)>, names: FolderNames) -> Self {
        Self::of(Children::Listed(children), names)
    }

    pub(super) fn of(children: Children<'a>, names: FolderNames) -> Self {
        Self {
            children,
            names,
            kept: Vec::new(),
            held: Vec::new(),
            listed: false,
        }
    }

    pub(super) async fn batch(
        &mut self,
        cancellable: &gio::Cancellable,
    ) -> Result<Vec<(OsString, gio::FileType)>, glib::Error> {
        match &mut self.children {
            Children::Enumerated(enumerator) => Ok(next_children(enumerator, cancellable)
                .await?
                .iter()
                .map(|child| (child.name().into_os_string(), child.file_type()))
                .collect()),
            Children::Listed(children) => Ok(std::mem::take(children)),
        }
    }

    pub(super) async fn next(
        &mut self,
        cancellable: &gio::Cancellable,
    ) -> Result<Vec<(OsString, gio::FileType, Planned)>, glib::Error> {
        let kept = |name: OsString| Planned {
            target: name,
            cause: None,
        };
        while !self.listed {
            let batch = self.batch(cancellable).await?;
            self.listed = batch.is_empty();
            let children = batch.into_iter();
            if !self.names.fixing {
                return Ok(children
                    .map(|(name, kind)| (name.clone(), kind, kept(name)))
                    .collect());
            }
            if self.names.rules.folds {
                self.held.extend(children);
                continue;
            }
            let mut ready = Vec::new();
            for (name, kind) in children {
                let is_directory = kind == gio::FileType::Directory;
                if fix_name(&name, &self.names.rules, is_directory)
                    .cause
                    .is_some()
                {
                    self.held.push((name, kind));
                } else {
                    self.kept.push(name.clone());
                    ready.push((name.clone(), kind, kept(name)));
                }
            }
            if !ready.is_empty() {
                return Ok(ready);
            }
        }
        let held = std::mem::take(&mut self.held);
        if held.is_empty() {
            return Ok(Vec::new());
        }
        let entries = self
            .kept
            .iter()
            .map(|name| PlanEntry {
                name,
                is_directory: false,
                fixing: false,
                repeatable: false,
            })
            .chain(held.iter().map(|(name, kind)| PlanEntry {
                name,
                is_directory: *kind == gio::FileType::Directory,
                fixing: true,
                repeatable: false,
            }))
            .collect::<Vec<_>>();
        let planned = plan_names(&entries, &self.names.rules, Key::Folded)
            .map_err(placement_error)?
            .into_iter()
            .skip(self.kept.len());
        Ok(held
            .into_iter()
            .zip(planned)
            .map(|((name, kind), planned)| (name, kind, planned))
            .collect())
    }
}

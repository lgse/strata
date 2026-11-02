// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy)]
pub(in crate::adapters::local_operations) enum Removal {
    Trash,
    Trashed(Option<ItemIdentity>),
    Without(Displacement),
}

#[derive(Clone, Copy)]
pub(in crate::adapters::local_operations) enum ReplaceStrategy {
    PlaceSource,
    Staged(Removal),
}

pub(in crate::adapters::local_operations) struct ReplacePlan {
    pub(in crate::adapters::local_operations) source_parent: Arc<OwnedFd>,
    pub(in crate::adapters::local_operations) source_name: OsString,
    pub(in crate::adapters::local_operations) source: EntryStat,
    pub(in crate::adapters::local_operations) target_parent: Arc<OwnedFd>,
    pub(in crate::adapters::local_operations) target_name: OsString,
    pub(in crate::adapters::local_operations) target: EntryStat,
    pub(in crate::adapters::local_operations) target_pin: Pinned,
    pub(in crate::adapters::local_operations) strategy: ReplaceStrategy,
}

impl ReplacePlan {
    pub(in crate::adapters::local_operations) fn source_entry(&self) -> Entry<'_> {
        Entry::new(&*self.source_parent, &self.source_name)
    }

    pub(in crate::adapters::local_operations) fn target_entry(&self) -> Entry<'_> {
        Entry::new(&*self.target_parent, &self.target_name)
    }

    pub(in crate::adapters::local_operations) fn target_display(&self) -> String {
        self.target_name.to_string_lossy().into_owned()
    }

    pub(in crate::adapters::local_operations) fn directory(&self) -> bool {
        self.source.is_directory()
    }

    pub(in crate::adapters::local_operations) fn copy_stays_outside_source(&self) -> bool {
        !self.directory()
            || placement::contains(fs(), self.source.identity, self.target_parent.as_fd())
                == Some(false)
    }
}

pub(in crate::adapters::local_operations) fn split_local_path(
    path: &Path,
) -> Result<(PathBuf, OsString), glib::Error> {
    path.parent()
        .zip(path.file_name())
        .map(|(parent, name)| (parent.to_path_buf(), name.to_os_string()))
        .ok_or_else(|| translated_io_error("Invalid local filesystem target"))
}

/// `None` means both names are one item.
pub(in crate::adapters::local_operations) fn plan_replace(
    source_path: &Path,
    target_path: &Path,
    move_source: bool,
) -> Result<Option<ReplacePlan>, glib::Error> {
    let inspected = Inspected::of(source_path, target_path)?;
    if inspected.source.is_directory() != inspected.target.is_directory() {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            &crate::i18n::tr("A file and a folder cannot safely replace one another"),
        ));
    }
    let target_directory = fs().stat_fd(inspected.target_parent.as_fd()).ok();
    if inspected.same_item(target_directory) {
        return Ok(None);
    }
    let strategy = inspected.strategy(move_source, target_path, target_directory)?;
    Ok(Some(inspected.into_plan(strategy)))
}

/// Both items of a replace, stated through their pinned parents.
struct Inspected {
    source_parent: OwnedFd,
    source_name: OsString,
    source: EntryStat,
    target_parent: OwnedFd,
    target_name: OsString,
    target: EntryStat,
}

fn inspect(parent: &OwnedFd, name: &OsStr, path: &Path) -> Result<EntryStat, glib::Error> {
    fs().stat(Entry::new(parent, name), false).map_err(|error| {
        io_error(rust_i18n::t!(
            "Could not inspect %{path}: %{error}",
            path = path.display(),
            error = os_error_detail(error)
        ))
    })
}

impl Inspected {
    fn of(source_path: &Path, target_path: &Path) -> Result<Self, glib::Error> {
        let (source_parent_path, source_name) = split_local_path(source_path)?;
        let (target_parent_path, target_name) = split_local_path(target_path)?;
        let source_parent = open_local_parent_directory(&source_parent_path).map_err(io_error)?;
        let target_parent = open_local_parent_directory(&target_parent_path).map_err(io_error)?;
        let source = inspect(&source_parent, &source_name, source_path)?;
        let target = inspect(&target_parent, &target_name, target_path)?;
        Ok(Self {
            source_parent,
            source_name,
            source,
            target_parent,
            target_name,
            target,
        })
    }

    fn source_entry(&self) -> Entry<'_> {
        Entry::new(&self.source_parent, &self.source_name)
    }

    fn target_entry(&self) -> Entry<'_> {
        Entry::new(&self.target_parent, &self.target_name)
    }

    fn same_item(&self, target_directory: Option<EntryStat>) -> bool {
        let Some(target_directory) = target_directory else {
            return false;
        };
        let source_directory = fs()
            .stat_fd(self.source_parent.as_fd())
            .ok()
            .map(|dir| dir.identity);
        if source_directory != Some(target_directory.identity)
            || self.source.identity != self.target.identity
        {
            return false;
        }
        // A hard link elsewhere in the folder is a separate name and is replaced.
        self.source_name == self.target_name
            || placement::alias_of(fs(), self.source_entry(), self.target_entry()).is_some()
    }

    fn renames_within(&self, target_directory: Option<EntryStat>) -> bool {
        if fs().mount_key(self.source_parent.as_fd()) != fs().mount_key(self.target_parent.as_fd())
        {
            return false;
        }
        target_directory.is_some_and(|dir| dir.identity.device == self.source.identity.device)
    }

    fn strategy(
        &self,
        move_source: bool,
        target_path: &Path,
        target_directory: Option<EntryStat>,
    ) -> Result<ReplaceStrategy, glib::Error> {
        if move_source && self.renames_within(target_directory) {
            return Ok(ReplaceStrategy::PlaceSource);
        }
        if fs().can_trash(target_path) {
            return Ok(ReplaceStrategy::Staged(Removal::Trash));
        }
        let displacement = displacement_without_trash(
            &self.target_parent,
            self.source.is_directory(),
            &self.target_name.to_string_lossy(),
        )?;
        Ok(ReplaceStrategy::Staged(Removal::Without(displacement)))
    }

    fn into_plan(self, strategy: ReplaceStrategy) -> ReplacePlan {
        let target_pin = Pinned::at(
            fs(),
            self.target_entry(),
            &self.target,
            known_mount(fs(), self.target_parent.as_fd()).1.identity,
        );
        ReplacePlan {
            source_parent: Arc::new(self.source_parent),
            source_name: self.source_name,
            source: self.source,
            target_parent: Arc::new(self.target_parent),
            target_name: self.target_name,
            target: self.target,
            target_pin,
            strategy,
        }
    }
}

pub(in crate::adapters::local_operations) fn ensure_unchanged(
    entry: Entry<'_>,
    recorded: &EntryStat,
    pin: &Pinned,
) -> Result<EntryStat, glib::Error> {
    let unchanged = |stat: &EntryStat| {
        pin.unchanged(
            fs(),
            mount_profile::identity_at(fs(), entry.dir),
            recorded,
            stat,
        )
    };
    match fs().stat(entry, false) {
        Ok(stat) if unchanged(&stat) => Ok(stat),
        Ok(_) | Err(rustix::io::Errno::NOENT) => Err(translated_io_error(
            "The target changed while the operation was in progress",
        )),
        Err(error) => Err(os_io_error(error)),
    }
}

pub(in crate::adapters::local_operations) struct DisplacedOriginal {
    pub(in crate::adapters::local_operations) parent: OffMain<Arc<OwnedFd>>,
    pub(in crate::adapters::local_operations) originals: placement::Originals,
    pub(in crate::adapters::local_operations) name: OsString,
    pub(in crate::adapters::local_operations) identity: Recorded,
}

impl DisplacedOriginal {
    pub(in crate::adapters::local_operations) fn aside(
        parent: Arc<OwnedFd>,
        aside: placement::SetAside,
    ) -> Self {
        Self {
            parent: OffMain::new(parent),
            originals: aside.originals,
            name: aside.name,
            identity: Recorded {
                identity: aside.identity,
                attributes: aside.attributes,
            },
        }
    }

    pub(in crate::adapters::local_operations) fn kept_in(&self) -> &OsStr {
        &self.originals.name
    }
}

pub(in crate::adapters::local_operations) async fn remove_displaced_original(
    original: DisplacedOriginal,
    deadline: &Deadline,
) -> Result<(), glib::Error> {
    let DisplacedOriginal {
        parent,
        originals,
        name,
        identity,
    } = original;
    let roots = vec![PinnedEntry {
        parent: originals.handle(),
        name,
        identity: Some(identity),
    }];
    let deadline = deadline.clone();
    parallel_delete_local_with(roots, gio::Cancellable::new(), move |roots, cancelled| {
        let workers = local_delete_worker_count(&roots);
        parallel_delete_local_blocking_with_workers(roots, cancelled, workers, &deadline)?;
        originals
            .remove(fs(), parent.as_fd(), &deadline)
            .map_err(|refused| originals.refusal(fs(), parent.as_fd(), refused))
    })
    .await
}

pub(in crate::adapters::local_operations) enum SourcePlaced {
    Trashed(Option<ItemIdentity>),
    SetAside(placement::SetAside),
    CopyAfterTrash(Option<ItemIdentity>),
    /// The original is back at its name, and the source must be copied.
    PutBack,
}

pub(in crate::adapters::local_operations) fn place_source(
    plan: &ReplacePlan,
    target_path: &Path,
    cancellable: &gio::Cancellable,
) -> Result<SourcePlaced, glib::Error> {
    let target = plan.target_entry();
    let current = ensure_unchanged(target, &plan.target, &plan.target_pin)?;
    match trash::trash_verified(target, target_path, &current, cancellable) {
        Ok(trashed) => return place_after_trash(plan, trashed),
        Err(error) if is_trash_unsupported_failure(false, &error) => {}
        Err(error) => return Err(error.error),
    }
    place_beside_original(plan, &current)
}

fn place_after_trash(
    plan: &ReplacePlan,
    trashed: Option<ItemIdentity>,
) -> Result<SourcePlaced, glib::Error> {
    let expected = Some(plan.source.identity);
    match placement::place(fs(), plan.source_entry(), plan.target_entry(), expected) {
        Ok(_) => Ok(SourcePlaced::Trashed(trashed)),
        Err(
            error @ (PlaceError::Exists
            | PlaceError::Changed
            | PlaceError::Unconfirmed
            | PlaceError::IntoItself),
        ) => Err(original_in_trash(&crate::services::error_detail(
            placement_reason(error),
        ))),
        Err(_) if !plan.copy_stays_outside_source() => Err(original_in_trash(
            &crate::services::error_detail(placement_reason(PlaceError::IntoItself)),
        )),
        Err(_) => Ok(SourcePlaced::CopyAfterTrash(trashed)),
    }
}

fn place_beside_original(
    plan: &ReplacePlan,
    current: &EntryStat,
) -> Result<SourcePlaced, glib::Error> {
    let (target, source) = (plan.target_entry(), plan.source_entry());
    let name = plan.target_display();
    let aside = placement::set_aside(fs(), target, current.identity, current.kept_attributes())
        .map_err(|error| set_aside_failure(error, &name))?;
    let error = match placement::place(fs(), source, target, Some(plan.source.identity)) {
        Ok(_) => return Ok(SourcePlaced::SetAside(aside)),
        Err(error) => error,
    };
    if placement::put_back(fs(), plan.target_parent.as_fd(), &aside, &plan.target_name).is_err() {
        return Err(io_error(original_kept_aside(
            &name,
            &aside.kept(),
            &crate::services::error_detail(placement_reason(error)),
        )));
    }
    match error {
        PlaceError::CrossDevice | PlaceError::Unsupported if plan.copy_stays_outside_source() => {
            Ok(SourcePlaced::PutBack)
        }
        error => Err(placement_failure(
            error,
            &plan.source_name.to_string_lossy(),
            could_not_replace,
        )),
    }
}

// SPDX-License-Identifier: MIT

use super::*;

pub(super) struct Followed<T> {
    pub(super) value: Result<T, glib::Error>,
    pub(super) reported: Reported,
    pub(super) left: Left,
    pub(super) manifest: Option<Manifest>,
}

#[derive(Default)]
pub(super) struct Left {
    pub(super) renamed: Vec<(PathBuf, PathBuf, RenameCause)>,
    pub(super) skipped: Vec<(PathBuf, SpecialKind)>,
    pub(super) item: Option<(OsString, RenameCause)>,
}

pub(super) fn rebase(path: &mut PathBuf, from: &Path, to: &Path) {
    if let Ok(rest) = path.strip_prefix(from) {
        *path = if rest.as_os_str().is_empty() {
            to.to_path_buf()
        } else {
            to.join(rest)
        };
    }
}

pub(super) fn report(progress: Option<&TransferProgressTracker>, root: &Path, left: Left) {
    if let Some(progress) = progress {
        progress.report(left_report(root, left));
    }
}

pub(super) fn left_report(root: &Path, left: Left) -> TransferReport {
    let location = |path: PathBuf| Location::local(root.join(path));
    let item = left.item.map(|(requested, cause)| RenamedItem {
        requested: Location::local(root.with_file_name(requested)),
        saved_as: Location::local(root.to_path_buf()),
        cause,
    });
    TransferReport {
        renamed: item
            .into_iter()
            .chain(
                left.renamed
                    .into_iter()
                    .map(|(requested, saved_as, cause)| RenamedItem {
                        requested: location(requested),
                        saved_as: location(saved_as),
                        cause,
                    }),
            )
            .collect(),
        skipped: left
            .skipped
            .into_iter()
            .map(|(path, kind)| SkippedItem {
                location: location(path),
                kind,
            })
            .collect(),
        ..TransferReport::default()
    }
}

pub(super) enum Item {
    Path(PathBuf),
    Pinned(Arc<OwnedFd>, OsString),
}

pub(super) async fn following<T: Send + 'static>(
    item: Item,
    options: Options,
    progress: Option<Rc<TransferProgressTracker>>,
    cancellable: &gio::Cancellable,
    work: impl FnOnce(&Arc<Copy>, &Source) -> Result<T, Failure> + Send + 'static,
) -> Result<Followed<T>, glib::Error> {
    let shared = Arc::new(SharedProgress::default());
    let follower = progress.map(|progress| {
        let name = match &item {
            Item::Path(path) => path.file_name(),
            Item::Pinned(_, name) => Some(name.as_os_str()),
        };
        if let Some(name) = name {
            progress.set_current_file(name.to_string_lossy().into_owned());
        }
        progress.follow(&shared)
    });
    let counters = shared.clone();
    let copied = run_cancellable(cancellable, move |cancelled| {
        let (parent, name) = match item {
            Item::Path(path) => match split(&path) {
                Ok(split) => split,
                Err(error) => return (Err(error), Left::default(), None),
            },
            Item::Pinned(parent, name) => (parent, name),
        };
        let source = Source {
            parent,
            name,
            recorded_in: options.manifest.then(manifest::Folder::root),
        };
        let copy = Copy::new(
            fs(),
            source.name.clone(),
            (cancelled, counters, true),
            (options, Deadline::default()),
        );
        let value = work(&copy, &source).map_err(|failure| failure.into_error(&source.name));
        let manifest = source.recorded_in.map(|root| {
            let mut manifest = manifest::of(source.parent, root);
            for copy in std::mem::take(&mut *lock(&copy.copies)) {
                manifest.copied_to(copy);
            }
            manifest
        });
        (value, std::mem::take(&mut *lock(&copy.left)), manifest)
    })
    .await;
    let reported = follower.map_or_else(|| shared.reported(), |follower| follower.stop());
    let (value, left, manifest) =
        copied.map_err(|_| translated_io_error("The copy stopped unexpectedly"))?;
    Ok(Followed {
        value,
        reported,
        left,
        manifest,
    })
}

pub(in crate::adapters::local_operations) async fn copy_from_stage(
    staged: &StagedSibling,
    name: OsString,
    durable: bool,
    cancellable: &gio::Cancellable,
) -> Result<Ownership, glib::Error> {
    let source = Source {
        parent: staged.parent.clone(),
        name: staged.name.clone(),
        recorded_in: None,
    };
    let identity = staged.identity;
    let copied = run_cancellable(cancellable, move |cancelled| {
        let options = Options {
            durable,
            ..Options::default()
        };
        let copy = Copy::new(
            fs(),
            name.clone(),
            (cancelled, Arc::default(), false),
            (options, Deadline::default()),
        );
        let parent = source.parent.clone();
        from_stage(&copy, &source, identity, &parent, &name)
            .map_err(|failure| failure.into_error(&name))
    })
    .await;
    copied.map_err(|_| translated_io_error("The copy stopped unexpectedly"))?
}

pub(in crate::adapters::local_operations) fn unlock_stage(staged: &StagedSibling) {
    if staged.restrictive.is_empty() {
        return;
    }
    let fs = fs();
    let Ok(root) = fs.open_dir(Entry::new(&*staged.parent, &staged.name)) else {
        return;
    };
    if fs
        .stat_fd(root.as_fd())
        .is_ok_and(|stat| stat.identity == staged.identity)
    {
        let mount = Mount::new(fs, root.as_fd());
        restrictive::loosen(fs, root.as_fd(), &staged.restrictive, &mount);
    }
}

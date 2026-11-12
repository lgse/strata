// SPDX-License-Identifier: MIT

use super::*;

fn vouched(expected: &ItemIdentity, path: &Path) -> Option<bool> {
    let current = identity_at_path(path)?;
    (expected.proof(&current) != Proof::Different).then(|| expected.unchanged(&current))
}

async fn trashed_as(expected: ItemIdentity, path: PathBuf) -> Option<bool> {
    gio::spawn_blocking(move || vouched(&expected, &path))
        .await
        .ok()
        .flatten()
}

#[derive(Clone)]
pub(in crate::adapters::local_operations) struct RestoreEntry {
    pub(in crate::adapters::local_operations) source: Location,
    pub(in crate::adapters::local_operations) display_name: String,
    pub(in crate::adapters::local_operations) original_target: Option<Location>,
    pub(in crate::adapters::local_operations) trash_info: Option<PathBuf>,
    pub(in crate::adapters::local_operations) confirmed_destination: Option<PathBuf>,
    pub(in crate::adapters::local_operations) physical_path: Option<PathBuf>,
}

type Found = Result<RestoreEntry, glib::Error>;

pub(in crate::adapters::local_operations) async fn trashed_entries_for_originals(
    original_locations: &[Location],
    cancellable: &gio::Cancellable,
    originals: &HashMap<Location, ItemIdentity>,
) -> Result<Vec<Result<RestoreEntry, (Location, glib::Error)>>, glib::Error> {
    if cancellable.is_cancelled() {
        return Err(cancelled_local_operation());
    }
    let requested = original_locations
        .iter()
        .filter_map(|location| location.native_path().map(Path::to_path_buf))
        .collect::<HashSet<_>>();
    // GVfs can miss an item re-trashed under the same basename after a restore, so prefer the
    // authoritative freedesktop.org metadata before consulting trash:///.
    let context = RestoreContext::current().await;
    let mut fallback = metadata_entries(&context, &requested, cancellable, originals).await?;
    if cancellable.is_cancelled() {
        return Err(cancelled_local_operation());
    }
    let all_requested = requested.len() == original_locations.len();
    if fallback.len() == requested.len() && all_requested {
        return Ok(in_request_order(original_locations, |path| {
            fallback.remove(path)
        }));
    }
    let lookup = GvfsLookup {
        requested: &requested,
        fallback: &fallback,
        originals,
        context: &context,
    };
    let mut newest = lookup.newest(cancellable).await?;
    let missing = requested
        .iter()
        .any(|path| !fallback.contains_key(path) && !newest.contains_key(path));
    if missing || !all_requested {
        return Err(trashed_items_unavailable());
    }
    Ok(in_request_order(original_locations, |path| {
        fallback
            .remove(path)
            .or_else(|| newest.remove(path).map(|newest| chosen(path, newest)))
    }))
}

fn in_request_order(
    original_locations: &[Location],
    mut found: impl FnMut(&Path) -> Option<Found>,
) -> Vec<Result<RestoreEntry, (Location, glib::Error)>> {
    original_locations
        .iter()
        .filter_map(|location| {
            let found = found(location.native_path()?)?;
            Some(found.map_err(|error| (location.clone(), error)))
        })
        .collect()
}

async fn metadata_entries(
    context: &RestoreContext,
    requested: &HashSet<PathBuf>,
    cancellable: &gio::Cancellable,
    originals: &HashMap<Location, ItemIdentity>,
) -> Result<HashMap<PathBuf, Found>, glib::Error> {
    let (context, requested) = (context.clone(), requested.clone());
    let (cancellable, originals) = (cancellable.clone(), originals.clone());
    gio::spawn_blocking(move || trash_entries(&context, &requested, &cancellable, &originals))
        .await
        .map_err(|_| glib::Error::new(gio::IOErrorEnum::Failed, "Trash lookup task failed"))
}

/// Looks through trash:/// for what the Trash metadata did not find.
struct GvfsLookup<'a> {
    requested: &'a HashSet<PathBuf>,
    fallback: &'a HashMap<PathBuf, Found>,
    originals: &'a HashMap<Location, ItemIdentity>,
    context: &'a RestoreContext,
}

impl GvfsLookup<'_> {
    async fn newest(
        &self,
        cancellable: &gio::Cancellable,
    ) -> Result<HashMap<PathBuf, Newest>, glib::Error> {
        let trash = gio::File::for_uri("trash:///");
        let enumerated = await_cancellable(&trash, cancellable, |trash, cancellable, result| {
            trash.enumerate_children_async(
                "standard::name,standard::display-name,standard::target-uri,trash::orig-path,trash::deletion-date",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
                Some(cancellable),
                move |value| result.resolve(value),
            );
        })
        .await;
        let enumerator = match enumerated {
            Err(error) if error.matches(gio::IOErrorEnum::NotSupported) => {
                return Err(trashed_items_unavailable());
            }
            enumerator => enumerator?,
        };
        let mut newest = HashMap::new();
        loop {
            if cancellable.is_cancelled() {
                return Err(cancelled_local_operation());
            }
            let infos = next_children(&enumerator, cancellable).await?;
            if infos.is_empty() {
                return Ok(newest);
            }
            for info in infos {
                self.consider(&trash, &info, &mut newest).await;
            }
        }
    }

    async fn consider(
        &self,
        trash: &gio::File,
        info: &gio::FileInfo,
        newest: &mut HashMap<PathBuf, Newest>,
    ) {
        let Some(original_path) = info.attribute_byte_string("trash::orig-path") else {
            return;
        };
        let original_path = PathBuf::from(original_path.as_str());
        if !self.requested.contains(&original_path) || self.fallback.contains_key(&original_path) {
            return;
        }
        let deletion_date = info
            .attribute_string("trash::deletion-date")
            .map(|value| value.to_string())
            .unwrap_or_default();
        let Some(location) = location_for_file(&trash.child(info.name())) else {
            return;
        };
        let original = Location::local(&original_path);
        let recorded = self.originals.get(&original);
        let Some(vouched) = self.vouch(recorded, &location, &original).await else {
            return;
        };
        let entry = gvfs_trash_entry(location, info, &original_path);
        let candidate = Candidate {
            rank: (vouched, deletion_date),
            recorded: recorded.is_some(),
            entry,
        };
        keep_newest(newest, original_path, candidate);
    }

    async fn vouch(
        &self,
        recorded: Option<&ItemIdentity>,
        location: &Location,
        original: &Location,
    ) -> Option<bool> {
        let Some(&expected) = recorded else {
            return Some(true);
        };
        match plan_restore_for_location(location, Some(original), None, None, self.context).await {
            Ok(plan) => trashed_as(expected, plan.source_path).await,
            Err(_) => None,
        }
    }
}

struct Newest {
    rank: (bool, String),
    entry: RestoreEntry,
    tied: bool,
}

struct Candidate {
    rank: (bool, String),
    recorded: bool,
    entry: RestoreEntry,
}

fn keep_newest(newest: &mut HashMap<PathBuf, Newest>, original: PathBuf, candidate: Candidate) {
    let Candidate {
        rank,
        recorded,
        entry,
    } = candidate;
    if let Some(kept) = newest.get_mut(&original)
        && kept.rank >= rank
    {
        kept.tied |= recorded && rank.0 && kept.rank == rank;
        return;
    }
    newest.insert(
        original,
        Newest {
            rank,
            entry,
            tied: false,
        },
    );
}

fn chosen(original: &Path, newest: Newest) -> Found {
    if !newest.tied {
        return Ok(newest.entry);
    }
    let name = Location::local(original).display_name();
    Err(glib::Error::new(
        gio::IOErrorEnum::Failed,
        &rust_i18n::t!(
            "More than one item in Trash could be “%{name}”, so they were left there",
            name = name
        ),
    ))
}

fn trashed_items_unavailable() -> glib::Error {
    glib::Error::new(
        gio::IOErrorEnum::NotFound,
        &crate::i18n::tr("One or more recently trashed items are no longer available"),
    )
}

pub(in crate::adapters::local_operations) fn gvfs_trash_entry(
    source: Location,
    info: &gio::FileInfo,
    original_path: &Path,
) -> RestoreEntry {
    RestoreEntry {
        physical_path: trash_thumbnail_path(&source, info),
        source,
        display_name: info.display_name().to_string(),
        original_target: Some(Location::local(original_path)),
        trash_info: None,
        confirmed_destination: None,
    }
}

fn trash_entries(
    context: &RestoreContext,
    requested: &HashSet<PathBuf>,
    cancellable: &gio::Cancellable,
    originals: &HashMap<Location, ItemIdentity>,
) -> HashMap<PathBuf, Found> {
    let mut roots = requested
        .iter()
        .flat_map(|path| candidate_trash_roots(context, Some(path)))
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    trash_entries_at(&roots, requested, cancellable, originals)
}

pub(in crate::adapters::local_operations) fn trash_entries_at(
    roots: &[PathBuf],
    requested: &HashSet<PathBuf>,
    cancellable: &gio::Cancellable,
    originals: &HashMap<Location, ItemIdentity>,
) -> HashMap<PathBuf, Found> {
    let scan = MetadataScan {
        requested,
        originals,
    };
    let mut newest = HashMap::new();
    for trash_root in roots {
        scan.newest_in(trash_root, cancellable, &mut newest);
    }
    newest
        .into_iter()
        .map(|(path, newest)| {
            let entry = chosen(&path, newest);
            (path, entry)
        })
        .collect()
}

/// Reads the freedesktop.org Trash metadata for the requested originals.
struct MetadataScan<'a> {
    requested: &'a HashSet<PathBuf>,
    originals: &'a HashMap<Location, ItemIdentity>,
}

impl MetadataScan<'_> {
    fn newest_in(
        &self,
        trash_root: &Path,
        cancellable: &gio::Cancellable,
        newest: &mut HashMap<PathBuf, Newest>,
    ) {
        let info_root = trash_root.join("info");
        let files_root = trash_root.join("files");
        let Ok(infos) = std::fs::read_dir(info_root) else {
            return;
        };
        for info in infos.flatten() {
            if cancellable.is_cancelled() {
                break;
            }
            if let Some((original_path, candidate)) =
                self.candidate(trash_root, &files_root, info.path())
            {
                keep_newest(newest, original_path, candidate);
            }
        }
    }

    fn candidate(
        &self,
        trash_root: &Path,
        files_root: &Path,
        info_path: PathBuf,
    ) -> Option<(PathBuf, Candidate)> {
        let name = info_path.file_name()?;
        let file_name = name.as_bytes().strip_suffix(b".trashinfo")?;
        let contents = std::fs::read_to_string(&info_path).ok()?;
        let encoded_path = contents.lines().find_map(|line| line.strip_prefix("Path="));
        let deletion_date = contents
            .lines()
            .find_map(|line| line.strip_prefix("DeletionDate="))
            .unwrap_or_default();
        let original_path = encoded_path
            .and_then(decode_trashinfo_path)
            .and_then(|path| absolute_original(trash_root, path))?;
        if !self.requested.contains(&original_path) {
            return None;
        }
        let source_path = files_root.join(OsString::from_vec(file_name.to_vec()));
        let recorded = self.originals.get(&Location::local(&original_path));
        let vouched = match recorded {
            Some(expected) => vouched(expected, &source_path),
            None => std::fs::symlink_metadata(&source_path)
                .is_ok()
                .then_some(true),
        }?;
        let candidate = Candidate {
            rank: (vouched, deletion_date.to_owned()),
            recorded: recorded.is_some(),
            entry: trashinfo_entry(source_path, &original_path, info_path),
        };
        Some((original_path, candidate))
    }
}

fn trashinfo_entry(source_path: PathBuf, original_path: &Path, info_path: PathBuf) -> RestoreEntry {
    RestoreEntry {
        source: Location::local(&source_path),
        display_name: source_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| crate::i18n::tr("Trashed item")),
        original_target: Some(Location::local(original_path)),
        trash_info: Some(info_path),
        confirmed_destination: None,
        physical_path: Some(source_path.clone()),
    }
}

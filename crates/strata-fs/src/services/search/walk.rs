// SPDX-License-Identifier: MIT
//! The recursive index walk: a fair, budgeted scheduler over directory slices.

use std::{
    collections::{BinaryHeap, HashMap, HashSet, VecDeque},
    fs::FileType,
    iter::Peekable,
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
    time::Instant,
};

use super::{
    GENERATED_TREE_GLOBS, PUBLISH_INTERVAL, SearchCoverage, SearchExclusions, SearchItem,
    SharedIndex, TraversalBudget, file_activity, native_kind, unique_roots,
};
use crate::model::{EntryKind, MetadataValue};

struct DirectoryTask {
    path: PathBuf,
    root: PathBuf,
    depth: usize,
    probe_only: bool,
    skipped_entries: usize,
    batch_size: usize,
    virtual_work: usize,
}

struct ScheduledDirectory {
    task: DirectoryTask,
    sequence: u64,
}

pub(super) fn build_root_overrides(root: &Path) -> ignore::overrides::Override {
    let mut overrides = ignore::overrides::OverrideBuilder::new(root);
    for generated_tree in GENERATED_TREE_GLOBS {
        overrides
            .add(generated_tree)
            .expect("valid generated-tree prune glob");
    }
    overrides.build().expect("valid generated-tree prune globs")
}

fn directory_walker(
    task: &DirectoryTask,
    overrides: &ignore::overrides::Override,
    boundaries: Arc<HashSet<PathBuf>>,
    show_hidden: bool,
) -> ignore::Walk {
    let mut builder = ignore::WalkBuilder::new(&task.path);
    builder
        .follow_links(false)
        .standard_filters(true)
        // `standard_filters` resets hidden-file filtering.
        .hidden(!show_hidden)
        .require_git(false)
        .overrides(overrides.clone())
        .max_depth(Some(1))
        // Nested mounts are walked separately, never through both roots.
        .filter_entry(move |entry| {
            entry.depth() == 0
                || !(boundaries.contains(entry.path()) || is_kernel_filesystem(entry.path()))
        });
    builder.build()
}

// Avoid filling a root search with kernel interfaces; explicit roots bypass this filter.
fn is_kernel_filesystem(path: &Path) -> bool {
    ["/proc", "/sys", "/dev"]
        .iter()
        .any(|mount| path == Path::new(mount))
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum PathAdmission {
    Unique,
    Duplicate,
    EntryLimit,
}

pub(super) fn admit_path(
    indexed_paths: &mut HashSet<PathBuf>,
    path: &Path,
    max_entries: usize,
) -> PathAdmission {
    if indexed_paths.contains(path) {
        PathAdmission::Duplicate
    } else if indexed_paths.len() >= max_entries {
        PathAdmission::EntryLimit
    } else {
        indexed_paths.insert(path.to_path_buf());
        PathAdmission::Unique
    }
}

/// Why a walk stopped before its queue drained.
enum Halt {
    /// The index no longer wants this walk; nothing more is published.
    Cancelled,
    /// A budget ran out; what was indexed so far is published.
    Limit,
}

type Branch = BinaryHeap<ScheduledDirectory>;

/// An open directory listing and the hold that keeps its folder from changing under it.
struct Listing {
    walker: Peekable<ignore::Walk>,
    hold: file_activity::Hold,
}

/// What one slice of a directory listed.
#[derive(Default)]
struct Slice {
    seen_entries: usize,
    processed_entries: usize,
    work: usize,
    children: Vec<PathBuf>,
}

fn entry_mode(
    entry: &ignore::DirEntry,
    file_type: Option<FileType>,
    is_directory: bool,
) -> MetadataValue<u32> {
    use std::os::unix::fs::MetadataExt;

    if is_directory {
        return MetadataValue::Unknown;
    }
    // Executability follows the target, not the walker's symlink mode.
    let metadata = if file_type.is_some_and(|kind| kind.is_symlink()) {
        std::fs::metadata(entry.path()).ok()
    } else {
        entry.metadata().ok()
    };
    metadata
        .map(|metadata| MetadataValue::Known(metadata.mode()))
        .unwrap_or(MetadataValue::Unknown)
}

struct Walk<'a> {
    index: &'a SharedIndex,
    show_hidden: bool,
    budget: TraversalBudget,
    exclusions: SearchExclusions,
    directory_batch_limit: usize,
    indexed_entries: usize,
    indexed_paths: HashSet<PathBuf>,
    pending_items: Vec<SearchItem>,
    coverage: SearchCoverage,
    last_publish: Instant,
    walk_start: Instant,
    boundaries: Arc<HashSet<PathBuf>>,
    root_overrides: HashMap<PathBuf, ignore::overrides::Override>,
    real_roots: HashMap<PathBuf, PathBuf>,
    pending_branches: VecDeque<Branch>,
    pending_directory_count: usize,
    next_sequence: u64,
}

impl<'a> Walk<'a> {
    fn new(
        index: &'a SharedIndex,
        show_hidden: bool,
        budget: TraversalBudget,
        exclusions: SearchExclusions,
    ) -> Self {
        Self {
            index,
            show_hidden,
            budget: TraversalBudget {
                initial_directory_batch: budget.initial_directory_batch.max(1),
                max_pending_directories: budget.max_pending_directories.max(1),
                ..budget
            },
            exclusions,
            directory_batch_limit: (budget.max_entries / 100).clamp(1, 64),
            indexed_entries: 0,
            indexed_paths: HashSet::new(),
            pending_items: Vec::with_capacity(256),
            coverage: SearchCoverage::default(),
            last_publish: Instant::now(),
            walk_start: Instant::now(),
            boundaries: Arc::default(),
            root_overrides: HashMap::new(),
            real_roots: HashMap::new(),
            pending_branches: VecDeque::new(),
            pending_directory_count: 0,
            next_sequence: 0,
        }
    }

    fn take_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = sequence.wrapping_add(1);
        sequence
    }

    fn schedule_roots(&mut self, roots: Vec<PathBuf>) {
        let roots = unique_roots(roots);
        self.boundaries = Arc::new(roots.iter().cloned().collect());
        self.root_overrides = roots
            .iter()
            .map(|root| (root.clone(), build_root_overrides(root)))
            .collect();
        for root in roots {
            self.schedule_root(root);
        }
    }

    fn schedule_root(&mut self, root: PathBuf) {
        let name = root.file_name().unwrap_or_default().to_string_lossy();
        if self.exclusions.is_excluded(&root, &name, true) {
            return;
        }
        if self.pending_directory_count >= self.budget.max_pending_directories {
            self.coverage.directory_limit = true;
            return;
        }
        let task = DirectoryTask {
            path: root.clone(),
            root,
            depth: 0,
            probe_only: self.budget.max_depth == 0,
            skipped_entries: 0,
            batch_size: self.budget.initial_directory_batch,
            virtual_work: 0,
        };
        let mut branch = BinaryHeap::new();
        branch.push(ScheduledDirectory {
            task,
            sequence: self.take_sequence(),
        });
        self.pending_branches.push_back(branch);
        self.pending_directory_count += 1;
    }

    fn run(&mut self) -> Result<(), Halt> {
        while let Some(mut branch) = self.pending_branches.pop_front() {
            let Some(scheduled) = branch.pop() else {
                continue;
            };
            self.pending_directory_count = self.pending_directory_count.saturating_sub(1);
            self.visit(branch, scheduled.task)?;
        }
        Ok(())
    }

    fn real_path(&mut self, directory: &DirectoryTask) -> PathBuf {
        let real_root = self
            .real_roots
            .entry(directory.root.clone())
            .or_insert_with(|| {
                directory
                    .root
                    .canonicalize()
                    .unwrap_or_else(|_| directory.root.clone())
            });
        match directory.path.strip_prefix(&directory.root) {
            Ok(relative) if !relative.as_os_str().is_empty() => real_root.join(relative),
            _ => real_root.clone(),
        }
    }

    fn requeue(&mut self, branch: Branch) {
        if !branch.is_empty() {
            self.pending_branches.push_back(branch);
        }
    }

    fn visit(&mut self, mut branch: Branch, mut directory: DirectoryTask) -> Result<(), Halt> {
        if self.index.indexing_cancelled() {
            return Err(Halt::Cancelled);
        }
        let real = self.real_path(&directory);
        let Ok(hold) = file_activity::hold([&directory.path, &real], || {}) else {
            self.coverage.unreadable = true;
            self.requeue(branch);
            return Ok(());
        };
        let overrides = self
            .root_overrides
            .get(&directory.root)
            .expect("root overrides");
        let walker = directory_walker(
            &directory,
            overrides,
            self.boundaries.clone(),
            self.show_hidden,
        )
        .peekable();
        let mut listing = Listing { walker, hold };
        let mut slice = Slice::default();
        let exhausted = self.walk_slice(&directory, &mut listing, &mut slice)?;
        drop(listing);
        // Within a branch, depth puts the continuation ahead of this slice's
        // children, but earlier slices' children retain their lower work keys.
        let resume_work = directory.virtual_work.saturating_add(slice.work);
        let children = self.schedule_children(&directory, slice.children, resume_work);
        let mut new_branches = Vec::new();
        if directory.depth == 0 {
            new_branches.extend(children.into_iter().map(|child| BinaryHeap::from([child])));
        } else {
            branch.extend(children);
        }
        if !exhausted {
            directory.skipped_entries = directory
                .skipped_entries
                .saturating_add(slice.processed_entries);
            directory.batch_size = directory.batch_size.saturating_mul(2);
            directory.virtual_work = resume_work;
            branch.push(ScheduledDirectory {
                task: directory,
                sequence: self.take_sequence(),
            });
            self.pending_directory_count += 1;
        }
        self.requeue(branch);
        self.pending_branches.extend(new_branches);
        Ok(())
    }

    /// Schedules the children a slice found after that slice's work.
    fn schedule_children(
        &mut self,
        directory: &DirectoryTask,
        children: Vec<PathBuf>,
        resume_work: usize,
    ) -> Vec<ScheduledDirectory> {
        let entry_depth = directory.depth.saturating_add(1);
        children
            .into_iter()
            .map(|path| ScheduledDirectory {
                task: DirectoryTask {
                    path,
                    root: directory.root.clone(),
                    depth: entry_depth,
                    probe_only: entry_depth >= self.budget.max_depth,
                    skipped_entries: 0,
                    batch_size: self.budget.initial_directory_batch,
                    virtual_work: resume_work,
                },
                sequence: self.take_sequence(),
            })
            .collect()
    }

    /// Lists up to one batch of `directory`; `Ok(true)` once it has no entries left.
    fn walk_slice(
        &mut self,
        directory: &DirectoryTask,
        listing: &mut Listing,
        slice: &mut Slice,
    ) -> Result<bool, Halt> {
        let Listing { walker, hold } = listing;
        loop {
            if self.index.indexing_cancelled() {
                return Err(Halt::Cancelled);
            }
            if hold.stopped() {
                self.coverage.unreadable = true;
                return Ok(true);
            }
            if self.walk_start.elapsed() >= self.budget.time_budget {
                self.coverage.time_limit = true;
                return Err(Halt::Limit);
            }
            if slice.processed_entries >= directory.batch_size.max(self.directory_batch_limit) {
                // An empty continuation would outrank children and waste their next turn.
                return Ok(walker.peek().is_none());
            }
            let Some(result) = walker.next() else {
                return Ok(true);
            };
            if let Some(entry) = self.countable_entry(directory, result, slice) {
                self.index_entry(directory, entry, slice)?;
            }
        }
    }

    /// The next entry past those earlier slices already processed.
    fn countable_entry(
        &mut self,
        directory: &DirectoryTask,
        result: Result<ignore::DirEntry, ignore::Error>,
        slice: &mut Slice,
    ) -> Option<ignore::DirEntry> {
        let Ok(entry) = result else {
            self.coverage.unreadable = true;
            return None;
        };
        if entry.depth() == 0 {
            self.coverage.unreadable |= entry.error().is_some();
            return None;
        }
        slice.seen_entries += 1;
        if slice.seen_entries <= directory.skipped_entries {
            return None;
        }
        slice.processed_entries += 1;
        self.coverage.unreadable |= entry.error().is_some();
        Some(entry)
    }

    fn index_entry(
        &mut self,
        directory: &DirectoryTask,
        entry: ignore::DirEntry,
        slice: &mut Slice,
    ) -> Result<(), Halt> {
        let file_type = entry.file_type();
        let is_directory = file_type.is_some_and(|kind| kind.is_dir());
        let name = entry.file_name().to_string_lossy();
        if self
            .exclusions
            .is_excluded(entry.path(), &name, is_directory)
        {
            return Ok(());
        }
        // Structural entries are cheap within a branch so nested documents progress
        // before dense runs of regular files consume the shared entry budget.
        slice.work = slice.work.saturating_add(if is_directory { 1 } else { 8 });
        if directory.probe_only {
            self.coverage.depth_limit = true;
            return Ok(());
        }
        let mode = entry_mode(&entry, file_type, is_directory);
        let path = entry.into_path();
        let kind = file_type.map_or(EntryKind::Other, |kind| native_kind(kind, &path));
        match admit_path(&mut self.indexed_paths, &path, self.budget.max_entries) {
            PathAdmission::Duplicate => return Ok(()),
            PathAdmission::EntryLimit => {
                self.coverage.entry_limit = true;
                return Err(Halt::Limit);
            }
            PathAdmission::Unique => {}
        }
        self.pending_items.push(SearchItem::with_metadata(
            path.clone(),
            &directory.root,
            is_directory,
            kind,
            mode,
        ));
        self.indexed_entries += 1;
        if is_directory {
            self.discover_child(path, slice);
        }
        self.publish_pending();
        Ok(())
    }

    fn discover_child(&mut self, path: PathBuf, slice: &mut Slice) {
        // Keep one queue slot available for this slice's continuation. A child that
        // cannot be admitted is omitted, while already queued work still completes.
        if self.pending_directory_count < self.budget.max_pending_directories.saturating_sub(1) {
            self.pending_directory_count += 1;
            slice.children.push(path);
        } else {
            self.coverage.directory_limit = true;
        }
    }

    fn publish_pending(&mut self) {
        if self.pending_items.len() >= 256 {
            append_index_items(self.index, &mut self.pending_items, true, self.coverage);
        }
        if self.last_publish.elapsed() >= PUBLISH_INTERVAL {
            append_index_items(self.index, &mut self.pending_items, true, self.coverage);
            self.index.broadcast_change();
            self.last_publish = Instant::now();
        }
    }

    fn finish(mut self) {
        append_index_items(self.index, &mut self.pending_items, false, self.coverage);
        let elapsed_ms = self.walk_start.elapsed().as_millis() as u64;
        if self.coverage.is_partial() {
            tracing::warn!(
                entries = self.indexed_entries,
                elapsed_ms,
                coverage = ?self.coverage,
                "search index partial"
            );
        } else {
            tracing::info!(
                entries = self.indexed_entries,
                elapsed_ms,
                "search index built"
            );
        }
        self.index.broadcast_change();
    }
}

pub(super) fn build_index(
    index: &SharedIndex,
    roots: Vec<PathBuf>,
    show_hidden: bool,
    budget: TraversalBudget,
    exclusions: SearchExclusions,
) {
    let mut walk = Walk::new(index, show_hidden, budget, exclusions);
    walk.schedule_roots(roots);
    if let Err(Halt::Cancelled) = walk.run() {
        return;
    }
    walk.finish();
}

pub(super) fn append_index_items(
    index: &SharedIndex,
    items: &mut Vec<SearchItem>,
    indexing: bool,
    coverage: SearchCoverage,
) {
    let mut state = index
        .state
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if index.cancel_initial_indexer.load(Ordering::Acquire) {
        items.clear();
        return;
    }
    state.items.append(items);
    state.indexing = indexing;
    state.coverage = coverage;
}

impl PartialEq for ScheduledDirectory {
    fn eq(&self, other: &Self) -> bool {
        self.task.virtual_work == other.task.virtual_work
            && self.task.depth == other.task.depth
            && self.sequence == other.sequence
    }
}

impl Eq for ScheduledDirectory {}

impl PartialOrd for ScheduledDirectory {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledDirectory {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .task
            .virtual_work
            .cmp(&self.task.virtual_work)
            .then_with(|| other.task.depth.cmp(&self.task.depth))
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

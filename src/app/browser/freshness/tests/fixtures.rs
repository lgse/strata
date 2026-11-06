// SPDX-License-Identifier: MIT

use super::*;

pub(super) const NFS: u64 = 40;
pub(super) const DISK: u64 = 7;

pub(super) fn name_hash(name: &str) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    hasher.finish()
}

pub(super) fn entry(folder: &Location, name: &str) -> FileEntry {
    FileEntry {
        location: folder
            .child(std::ffi::OsStr::new(name))
            .expect("a child location"),
        native_name: OsString::from(name),
        thumbnail_path: None,
        display_name: name.to_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        recent_uri: None,
    }
}

pub(super) fn stamp(mount: u64, inode: u64, outside_writers: bool) -> DirectoryStamp {
    DirectoryStamp {
        modified: (100, 5),
        changed: Some((100, 5)),
        identity: Some(ItemIdentity {
            evidence: Evidence {
                device: mount,
                inode,
                kind: rustix::fs::FileType::Directory,
                birth: Some((1, 0)),
                handle: None,
                attributes: None,
            },
            traits: IdentityTraits {
                numbers: InodeNumbers::Kept,
                folder_births: true,
                made_up_births: false,
            },
        }),
        mount: Some(MountToken::Native(mount)),
        outside_writers,
        coarse_window: None,
        daemon_cache: None,
        read_at: UNIX_EPOCH + Duration::from_secs(1_000),
    }
}

pub(super) fn network(inode: u64) -> DirectoryStamp {
    stamp(NFS, inode, true)
}

pub(super) fn local(inode: u64) -> DirectoryStamp {
    stamp(DISK, inode, false)
}

pub(super) struct Clock {
    browser: Rc<Browser>,
    start: Instant,
    asleep: Cell<Duration>,
}

impl Clock {
    fn new(browser: &Rc<Browser>) -> Self {
        let clock = Self {
            browser: browser.clone(),
            start: Instant::now(),
            asleep: Cell::new(Duration::ZERO),
        };
        clock.at(0);
        clock
    }

    pub(super) fn at(&self, seconds: u64) {
        let awake = Duration::from_secs(seconds);
        self.browser
            .freshness
            .clock
            .set(Some((self.start + awake, awake + self.asleep.get())));
    }

    pub(super) fn sleep(&self, seconds: u64) {
        self.asleep
            .set(self.asleep.get() + Duration::from_secs(seconds));
    }

    pub(super) fn tick_at(&self, seconds: u64) {
        self.at(seconds);
        self.browser.freshness_tick();
    }

    pub(super) fn armed(&self) -> Option<Duration> {
        self.browser
            .freshness
            .armed_for
            .get()
            .map(|due| due.saturating_duration_since(self.start))
    }
}

pub(super) fn browser_on(source: &Rc<Folders>) -> (Rc<Browser>, Clock) {
    let browser = Browser::new(source.clone());
    let clock = Clock::new(&browser);
    (browser, clock)
}

pub(super) fn set_presence(browser: &Rc<Browser>, presence: Presence) {
    browser.set_freshness(FreshnessInput {
        presence,
        ..browser.freshness_input()
    });
}

/// Holds freshness work back while the returned flag is set, like an open inline edit.
pub(super) fn hold_freshness(browser: &Browser, holding: bool) -> Rc<Cell<bool>> {
    let editing = Rc::new(Cell::new(holding));
    let held = editing.clone();
    browser.set_freshness_hold(move || held.get());
    editing
}

/// An operation writing a file into `folder`.
pub(super) fn writing_into(folder: &Location) -> file_activity::Activity {
    let folder = folder.native_path().expect("a native folder").to_path_buf();
    file_activity::begin(OperationScope {
        roots: vec![folder.join("listed")],
        folders: vec![folder],
    })
}

pub(super) type Events = Rc<RefCell<Vec<BrowserEvent>>>;

pub(super) fn recorded(browser: &Browser) -> Events {
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    events
}

pub(super) fn count(
    events: &RefCell<Vec<BrowserEvent>>,
    wanted: impl Fn(&BrowserEvent) -> bool,
) -> usize {
    events.borrow().iter().filter(|event| wanted(event)).count()
}

pub(super) fn names_at(browser: &Browser, depth: usize) -> Vec<String> {
    browser.state.borrow().columns[depth]
        .entries
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect()
}

pub(super) fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// Runs every row and fails once with every row that went wrong.
pub(super) fn assert_rows<Row>(
    rows: impl IntoIterator<Item = Row>,
    check: impl FnMut(Row) -> Option<String>,
) {
    let failures: Vec<String> = rows.into_iter().filter_map(check).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

pub(super) fn mismatch<T: PartialEq + std::fmt::Debug>(
    row: &str,
    got: T,
    expected: T,
) -> Option<String> {
    (got != expected).then(|| format!("{row}: got {got:?}, expected {expected:?}"))
}

// SPDX-License-Identifier: MIT

use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use super::{Source, Wake, Watcher};
use crate::adapters::volume::{
    MountKey, MountTable, forget_test_mount_capabilities, learn_mount_capability,
    mount_capabilities, mount_capabilities::forget_mount_capabilities,
};

const ROOT_ONLY: &str = "1 0 8:1 / / rw - ext4 /dev/root rw\n";
const WITH_SHARE: &str = "1 0 8:1 / / rw - ext4 /dev/root rw\n\
                          900 1 0:90 / /mnt/share rw - nfs4 server:/export rw\n";

struct Fake {
    text: Arc<Mutex<&'static str>>,
    reads: Arc<AtomicUsize>,
    wakes: mpsc::Receiver<Option<Wake>>,
    waker: mpsc::Sender<Option<Wake>>,
}

struct Control {
    text: Arc<Mutex<&'static str>>,
    reads: Arc<AtomicUsize>,
    wake: mpsc::Sender<Option<Wake>>,
}

fn fake(text: &'static str) -> (Fake, Control) {
    let text = Arc::new(Mutex::new(text));
    let reads = Arc::new(AtomicUsize::new(0));
    let (waker, wakes) = mpsc::channel();
    let control = Control {
        text: text.clone(),
        reads: reads.clone(),
        wake: waker.clone(),
    };
    (
        Fake {
            text,
            reads,
            wakes,
            waker,
        },
        control,
    )
}

impl Source for Fake {
    fn read(&mut self) -> Vec<u8> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.text.lock().expect("fake table").as_bytes().to_vec()
    }

    fn wait(&mut self) -> Option<Wake> {
        self.wakes.recv().ok().flatten()
    }

    fn settle(&mut self) {}

    fn waker(&self) -> Box<dyn Fn() + Send + Sync> {
        let waker = self.waker.clone();
        Box::new(move || {
            let _ = waker.send(Some(Wake::Asked));
        })
    }
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn shares(table: &MountTable) -> bool {
    table.is_network_path(Path::new("/mnt/share/folder"))
}

fn fresh_within_deadline(watcher: &Arc<Watcher>) -> MountTable {
    let (sender, received) = mpsc::channel();
    let asking = watcher.clone();
    std::thread::spawn(move || {
        let _ = sender.send(futures_lite::future::block_on(asking.fresh()));
    });
    received
        .recv_timeout(Duration::from_secs(10))
        .expect("a fresh table")
}

#[test]
fn a_table_is_published_only_when_it_changes() {
    static PUBLISHED: AtomicUsize = AtomicUsize::new(0);
    fn published() {
        PUBLISHED.fetch_add(1, Ordering::SeqCst);
    }
    let (source, control) = fake(ROOT_ONLY);
    let watcher = Watcher::start(source, published);
    assert!(!shares(&watcher.latest()));

    *control.text.lock().expect("fake table") = WITH_SHARE;
    control.wake.send(Some(Wake::Changed)).expect("watcher");
    wait_until("the change is published", || {
        PUBLISHED.load(Ordering::SeqCst) == 1
    });
    assert!(shares(&watcher.latest()));

    control.wake.send(Some(Wake::Changed)).expect("watcher");
    wait_until("the unchanged table is read", || {
        control.reads.load(Ordering::SeqCst) == 3
    });
    assert!(shares(&fresh_within_deadline(&watcher)));
    assert_eq!(PUBLISHED.load(Ordering::SeqCst), 1);
}

#[test]
fn a_publish_forgets_only_what_was_learned_of_real_mounts() {
    let real = MountKey::Device(u64::MAX - 9);
    let scoped = MountKey::Test(u64::MAX - 9);
    for key in [real, scoped] {
        learn_mount_capability(key, |capabilities| capabilities.hard_links = Some(false));
    }

    forget_mount_capabilities();

    assert_eq!(mount_capabilities(real).hard_links, None);
    assert_eq!(mount_capabilities(scoped).hard_links, Some(false));
    forget_test_mount_capabilities(u64::MAX - 9);
}

#[test]
fn a_fresh_table_is_read_after_it_is_asked_for() {
    let (source, control) = fake(ROOT_ONLY);
    let watcher = Watcher::start(source, || {});
    *control.text.lock().expect("fake table") = WITH_SHARE;
    assert!(!shares(&watcher.latest()));

    assert!(shares(&fresh_within_deadline(&watcher)));
    assert!(shares(&watcher.latest()));
}

#[test]
fn a_watcher_that_stopped_answers_with_its_last_table() {
    let (source, control) = fake(WITH_SHARE);
    let watcher = Watcher::start(source, || {});
    control.wake.send(None).expect("watcher");
    *control.text.lock().expect("fake table") = ROOT_ONLY;

    assert!(shares(&fresh_within_deadline(&watcher)));
    assert!(shares(&fresh_within_deadline(&watcher)));
}

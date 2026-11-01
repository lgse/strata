// SPDX-License-Identifier: MIT

use std::{
    future::Future,
    pin::{Pin, pin},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Wake, Waker},
    thread::ThreadId,
    time::{Duration, Instant},
};

use super::*;

#[derive(Default)]
pub(crate) struct Probe {
    state: Mutex<ProbeState>,
    changed: Condvar,
}

#[derive(Default)]
struct ProbeState {
    holding: bool,
    answering: bool,
    closed_on: Option<ThreadId>,
    finalized: bool,
}

impl Probe {
    pub(crate) fn hold(&self) {
        self.state.lock().expect("probe").holding = true;
    }

    pub(crate) fn release(&self) {
        self.state.lock().expect("probe").holding = false;
        self.changed.notify_all();
    }

    fn answer(&self) {
        let mut state = self.state.lock().expect("probe");
        state.answering = true;
        while state.holding {
            state = self.changed.wait(state).expect("probe");
        }
    }

    fn close(&self) {
        self.state.lock().expect("probe").closed_on = Some(std::thread::current().id());
    }

    fn finalize(&self) {
        self.state.lock().expect("probe").finalized = true;
    }

    pub(crate) fn finalized(&self) -> bool {
        self.state.lock().expect("probe").finalized
    }

    pub(crate) fn answering(&self) -> bool {
        self.state.lock().expect("probe").answering
    }

    pub(crate) fn closed_on(&self) -> Option<ThreadId> {
        self.state.lock().expect("probe").closed_on
    }
}

mod probed {
    use std::{
        path::{Path, PathBuf},
        sync::{Arc, Mutex, OnceLock},
    };

    use gtk::{
        gio::{self, prelude::*, subclass::prelude::*},
        glib,
    };

    use super::Probe;

    #[derive(Default)]
    pub struct ListingImp {
        pub(super) probe: OnceLock<Arc<Probe>>,
        listed: Mutex<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ListingImp {
        const NAME: &'static str = "StrataTestProbedListing";
        type Type = Listing;
        type ParentType = gio::FileEnumerator;
    }

    impl ObjectImpl for ListingImp {}

    impl Drop for ListingImp {
        fn drop(&mut self) {
            if let Some(probe) = self.probe.get() {
                probe.finalize();
            }
        }
    }

    impl FileEnumeratorImpl for ListingImp {
        fn next_file(
            &self,
            _: Option<&gio::Cancellable>,
        ) -> Result<Option<gio::FileInfo>, glib::Error> {
            self.probe.get().expect("probe").answer();
            if std::mem::replace(&mut *self.listed.lock().expect("listing"), true) {
                return Ok(None);
            }
            let info = gio::FileInfo::new();
            info.set_name("entry");
            info.set_file_type(gio::FileType::Regular);
            Ok(Some(info))
        }

        fn close(&self, _: Option<&gio::Cancellable>) -> (bool, Option<glib::Error>) {
            self.probe.get().expect("probe").close();
            (true, None)
        }
    }

    #[derive(Default)]
    pub struct ReadingImp {
        pub(super) probe: OnceLock<Arc<Probe>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ReadingImp {
        const NAME: &'static str = "StrataTestProbedReading";
        type Type = Reading;
        type ParentType = gio::FileInputStream;
    }

    impl ObjectImpl for ReadingImp {}

    impl Drop for ReadingImp {
        fn drop(&mut self) {
            if let Some(probe) = self.probe.get() {
                probe.finalize();
            }
        }
    }

    impl InputStreamImpl for ReadingImp {
        fn read(&self, _: &mut [u8], _: Option<&gio::Cancellable>) -> Result<usize, glib::Error> {
            self.probe.get().expect("probe").answer();
            Ok(0)
        }

        fn close(&self, _: Option<&gio::Cancellable>) -> Result<(), glib::Error> {
            self.probe.get().expect("probe").close();
            Ok(())
        }
    }

    #[derive(Default)]
    pub struct FolderImp {
        pub(super) probe: OnceLock<Arc<Probe>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FolderImp {
        const NAME: &'static str = "StrataTestProbedFolder";
        type Type = Folder;
        type Interfaces = (gio::File,);
    }

    impl ObjectImpl for FolderImp {}

    impl FileImpl for FolderImp {
        fn dup(&self) -> gio::File {
            self.obj().clone().upcast()
        }

        fn hash(&self) -> u32 {
            0
        }

        fn equal(&self, other: &gio::File) -> bool {
            self.obj().upcast_ref::<gio::File>() == other
        }

        fn is_native(&self) -> bool {
            true
        }

        fn uri(&self) -> String {
            "file:///probed".to_owned()
        }

        fn parse_name(&self) -> String {
            "/probed".to_owned()
        }

        fn basename(&self) -> Option<PathBuf> {
            Some(PathBuf::from("probed"))
        }

        fn path(&self) -> Option<PathBuf> {
            None
        }

        fn parent(&self) -> Option<gio::File> {
            None
        }

        fn has_prefix(&self, _: &gio::File) -> bool {
            self.probe.get().is_none()
        }

        fn resolve_relative_path(&self, _: impl AsRef<Path>) -> gio::File {
            glib::Object::new::<Folder>().upcast()
        }

        fn query_info(
            &self,
            _: &str,
            _: gio::FileQueryInfoFlags,
            _: Option<&gio::Cancellable>,
        ) -> Result<gio::FileInfo, glib::Error> {
            let info = gio::FileInfo::new();
            info.set_name("probed");
            info.set_file_type(if self.probe.get().is_some() {
                gio::FileType::Directory
            } else {
                gio::FileType::Regular
            });
            info.set_size(0);
            Ok(info)
        }

        fn enumerate_children(
            &self,
            _: &str,
            _: gio::FileQueryInfoFlags,
            _: Option<&gio::Cancellable>,
        ) -> Result<gio::FileEnumerator, glib::Error> {
            let probe = self.probe.get().expect("probe");
            probe.answer();
            let listing: Listing = glib::Object::new();
            let _ = listing.imp().probe.set(probe.clone());
            Ok(listing.upcast())
        }

        fn delete(&self, _: Option<&gio::Cancellable>) -> Result<(), glib::Error> {
            Ok(())
        }
    }

    #[derive(Default)]
    pub struct WritingImp {
        pub(super) probe: OnceLock<Arc<Probe>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WritingImp {
        const NAME: &'static str = "StrataTestProbedWriting";
        type Type = Writing;
        type ParentType = gio::FileOutputStream;
    }

    impl ObjectImpl for WritingImp {}

    impl Drop for WritingImp {
        fn drop(&mut self) {
            if let Some(probe) = self.probe.get() {
                probe.finalize();
            }
        }
    }

    impl OutputStreamImpl for WritingImp {
        fn write(&self, buffer: &[u8], _: Option<&gio::Cancellable>) -> Result<usize, glib::Error> {
            Ok(buffer.len())
        }

        fn close(&self, _: Option<&gio::Cancellable>) -> Result<(), glib::Error> {
            self.probe.get().expect("probe").close();
            Ok(())
        }
    }

    #[expect(
        unsafe_code,
        reason = "gio-rs offers no subclassing of GFileOutputStream, which GFile's create returns"
    )]
    // SAFETY: GFileOutputStream's instance and class structs extend GOutputStream's, and the
    // subclass overrides only GOutputStream's virtual methods, as gio-rs does for that parent.
    unsafe impl IsSubclassable<WritingImp> for gio::FileOutputStream {}

    #[expect(
        unsafe_code,
        reason = "gio-rs offers no subclassing of GFileInputStream, which GFile's read returns"
    )]
    // SAFETY: GFileInputStream's instance and class structs extend GInputStream's, and the
    // subclass overrides only GInputStream's virtual methods, as gio-rs does for that parent.
    unsafe impl IsSubclassable<ReadingImp> for gio::FileInputStream {}

    #[derive(Default)]
    pub struct ItemImp {
        pub(super) probe: OnceLock<Arc<Probe>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ItemImp {
        const NAME: &'static str = "StrataTestProbedItem";
        type Type = Item;
        type Interfaces = (gio::File,);
    }

    impl ObjectImpl for ItemImp {}

    impl FileImpl for ItemImp {
        fn is_native(&self) -> bool {
            true
        }

        fn create(
            &self,
            _: gio::FileCreateFlags,
            _: Option<&gio::Cancellable>,
        ) -> Result<gio::FileOutputStream, glib::Error> {
            let probe = self.probe.get().expect("probe");
            probe.answer();
            let writing: Writing = glib::Object::new();
            let _ = writing.imp().probe.set(probe.clone());
            Ok(writing.upcast())
        }

        fn read_fn(
            &self,
            _: Option<&gio::Cancellable>,
        ) -> Result<gio::FileInputStream, glib::Error> {
            let probe = self.probe.get().expect("probe");
            probe.answer();
            let reading: Reading = glib::Object::new();
            let _ = reading.imp().probe.set(probe.clone());
            Ok(reading.upcast())
        }
    }

    glib::wrapper! {
        pub struct Listing(ObjectSubclass<ListingImp>) @extends gio::FileEnumerator;
    }

    glib::wrapper! {
        pub struct Writing(ObjectSubclass<WritingImp>)
            @extends gio::FileOutputStream, gio::OutputStream, @implements gio::Seekable;
    }

    glib::wrapper! {
        pub struct Item(ObjectSubclass<ItemImp>) @implements gio::File;
    }

    glib::wrapper! {
        pub struct Folder(ObjectSubclass<FolderImp>) @implements gio::File;
    }

    glib::wrapper! {
        pub struct Reading(ObjectSubclass<ReadingImp>)
            @extends gio::FileInputStream, gio::InputStream, @implements gio::Seekable;
    }
}
use probed::{Folder, Item, Listing, Reading};

pub(crate) fn probed_folder(probe: &Arc<Probe>) -> gio::File {
    use gtk::glib::subclass::prelude::ObjectSubclassIsExt;
    let folder: Folder = glib::Object::new();
    let _ = folder.imp().probe.set(probe.clone());
    folder.upcast()
}

pub(crate) fn probed_file(probe: &Arc<Probe>) -> gio::File {
    use gtk::glib::subclass::prelude::ObjectSubclassIsExt;
    let file: Item = glib::Object::new();
    let _ = file.imp().probe.set(probe.clone());
    file.upcast()
}

struct Answered(AtomicBool);

impl Wake for Answered {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

pub(crate) fn closed_on_the_ui_thread_once_answered_and_let_go<F: Future>(
    open: impl FnOnce(gio::File) -> F,
) -> bool {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("the test owns the main context");
    let probe = Arc::new(Probe::default());
    {
        let answered = Arc::new(Answered(AtomicBool::new(false)));
        let waker = Waker::from(answered.clone());
        let mut open = pin!(open(probed_file(&probe)));
        assert!(
            open.as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending(),
            "the open is under way"
        );
        pump_until(&context, "the open answered", || {
            answered.0.load(Ordering::SeqCst)
        });
    }
    pump_until(&context, "the stream is finalized", || probe.finalized());
    probe.closed_on() == Some(std::thread::current().id())
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Listing,
    Reading,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LetGo {
    AfterTheCall,
    DuringTheCall,
}

fn call(kind: Kind, probe: &Arc<Probe>) -> Pin<Box<dyn Future<Output = ()>>> {
    use gtk::glib::subclass::prelude::ObjectSubclassIsExt;
    match kind {
        Kind::Listing => {
            let listing: Listing = glib::Object::new();
            let _ = listing.imp().probe.set(probe.clone());
            let listing = Closing::new(listing.upcast::<gio::FileEnumerator>());
            Box::pin(async move {
                let _ = listing.next_files_future(8, glib::Priority::DEFAULT).await;
            })
        }
        Kind::Reading => {
            let reading: Reading = glib::Object::new();
            let _ = reading.imp().probe.set(probe.clone());
            let reading = Closing::new(reading.upcast::<gio::InputStream>());
            Box::pin(async move {
                let _ = reading.read_bytes_future(8, glib::Priority::DEFAULT).await;
            })
        }
    }
}

pub(crate) fn pump_until(context: &glib::MainContext, what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        if !context.iteration(false) {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

#[test]
fn what_the_ui_thread_lets_go_of_is_closed_on_a_worker() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("the test owns the main context");
    let ui = std::thread::current().id();
    for kind in [Kind::Listing, Kind::Reading] {
        for let_go in [LetGo::AfterTheCall, LetGo::DuringTheCall] {
            let probe = Arc::new(Probe::default());
            if let_go == LetGo::DuringTheCall {
                probe.hold();
            }
            let task = context.spawn_local(call(kind, &probe));
            if let_go == LetGo::DuringTheCall {
                pump_until(&context, "the call is under way", || probe.answering());
                task.abort();
                probe.release();
            }
            drop(task);
            pump_until(&context, "the object is finalized", || probe.finalized());
            assert_ne!(probe.closed_on(), Some(ui), "{kind:?} let go {let_go:?}");
        }
    }
}

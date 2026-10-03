// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::VecDeque,
    ffi::OsString,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use futures_channel::oneshot;

use super::*;
use crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT;

pub(crate) type Reply = Result<Vec<Vec<Child>>, Unreadable>;

#[derive(Default)]
pub(crate) struct Controlled {
    requests: RefCell<Vec<(PathBuf, oneshot::Sender<Reply>)>>,
}

impl Controlled {
    pub(crate) fn requested(&self) -> Vec<PathBuf> {
        self.requests
            .borrow()
            .iter()
            .map(|(path, _)| path.clone())
            .collect()
    }

    pub(crate) fn reply(&self, index: usize, reply: Reply) -> bool {
        let (_, sender) = self.requests.borrow_mut().remove(index);
        sender.send(reply).is_ok()
    }

    pub(crate) fn abandoned(&self, index: usize) -> bool {
        self.requests.borrow()[index].1.is_canceled()
    }
}

struct Served(RefCell<VecDeque<Vec<Child>>>);

impl Enumeration for Served {
    fn next_batch(&self) -> LocalFuture<Result<Vec<Child>, Unreadable>> {
        let batch = self.0.borrow_mut().pop_front().unwrap_or_default();
        Box::pin(async move { Ok(batch) })
    }
}

impl FolderSource for Controlled {
    fn open(&self, directory: &Path) -> LocalFuture<Result<Box<dyn Enumeration>, Unreadable>> {
        let (sender, receiver) = oneshot::channel();
        self.requests
            .borrow_mut()
            .push((directory.to_path_buf(), sender));
        Box::pin(async move {
            let batches = receiver.await.map_err(|_| Unreadable)??;
            Ok(Box::new(Served(RefCell::new(batches.into()))) as Box<dyn Enumeration>)
        })
    }
}

pub(crate) fn folder(name: &str) -> Child {
    Child {
        name: name.into(),
        folder: true,
    }
}

fn file(name: &str) -> Child {
    Child {
        name: name.into(),
        folder: false,
    }
}

fn no_listing(_include_hidden: bool) -> Vec<OsString> {
    Vec::new()
}

fn context<'a>(
    current: Option<&'a Path>,
    listing: &'a dyn Fn(bool) -> Vec<OsString>,
) -> Context<'a> {
    Context {
        current,
        home: Path::new("/home/fixture"),
        show_hidden: false,
        listing,
    }
}

type Delivered = Rc<RefCell<Vec<Step>>>;

fn recorder() -> (Delivered, impl Fn() -> Box<dyn FnOnce(Step)>) {
    let delivered: Delivered = Rc::default();
    let sink = delivered.clone();
    (delivered, move || {
        let sink = sink.clone();
        Box::new(move |step| sink.borrow_mut().push(step))
    })
}

fn settle(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "completion did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn idle() {
    for _ in 0..20 {
        glib::MainContext::default().iteration(false);
    }
}

fn complete(text: &str, index: usize, count: usize) -> Step {
    Step::Complete {
        text: text.into(),
        index,
        count,
    }
}

#[test]
fn scope_follows_slashes_home_and_the_open_folder() {
    let home = Path::new("/home/fixture");
    let current = Some(Path::new("/work"));
    let folder = |directory: &str, stem: &str, prefix: &str| {
        Some(Scope::Folder {
            directory: PathBuf::from(directory),
            stem: stem.into(),
            prefix: prefix.into(),
        })
    };
    for (text, expected) in [
        (
            "Doc",
            Some(Scope::Listing {
                prefix: "Doc".into(),
            }),
        ),
        (
            "",
            Some(Scope::Listing {
                prefix: String::new(),
            }),
        ),
        ("~", folder("/home/fixture", "~/", "")),
        ("~/", folder("/home/fixture/", "~/", "")),
        ("~/Pro/co", folder("/home/fixture/Pro/", "~/Pro/", "co")),
        ("/usr/lo", folder("/usr/", "/usr/", "lo")),
        ("/", folder("/", "/", "")),
        ("src/ma", folder("/work/src/", "src/", "ma")),
        ("../", folder("/work/../", "../", "")),
        ("~other", None),
        ("~other/Documents", None),
    ] {
        assert_eq!(scope(text, current, home), expected, "{text:?}");
    }
    assert_eq!(
        scope("src/ma", None, home),
        None,
        "a relative path needs a native open folder"
    );
}

#[test]
fn uri_input_is_never_completed_even_with_slashes_or_credentials() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("main context");
    let source = Rc::new(Controlled::default());
    let completion = GoCompletion::new(source.clone());
    let listing = |_| vec![OsString::from("smb:")];
    let (delivered, deliver) = recorder();
    for text in [
        "sftp://user:secret@host.invalid/srv/",
        "smb://host.invalid/share/Do",
        "smb:",
        "dav:host/share",
        "//host.invalid/share/",
        "\\\\host\\share",
        "user@host.invalid:/srv/",
    ] {
        assert!(looks_like_uri(text), "{text:?}");
        assert_eq!(scope(text, Some(Path::new("/work")), Path::new("/h")), None);
        assert_eq!(
            completion.step(
                text,
                false,
                &context(Some(Path::new("/work")), &listing),
                deliver()
            ),
            Step::Hint(Hint::Uri),
            "{text:?}"
        );
    }
    idle();
    assert!(source.requested().is_empty(), "nothing was enumerated");
    assert!(delivered.borrow().is_empty());
}

#[test]
fn listing_completion_cycles_matching_folders_both_ways() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("main context");
    let completion = GoCompletion::new(Rc::new(Controlled::default()));
    let listing = |include_hidden: bool| {
        let hidden = include_hidden.then_some(".dotfiles");
        ["docs", "Downloads", "Documents", "music"]
            .into_iter()
            .chain(hidden)
            .map(OsString::from)
            .collect()
    };
    let context = context(Some(Path::new("/work")), &listing);
    let never = || -> Box<dyn FnOnce(Step)> { Box::new(|_| panic!("listing is synchronous")) };

    assert_eq!(
        completion.step("do", false, &context, never()),
        complete("docs/", 0, 3)
    );
    assert_eq!(
        completion.step("docs/", false, &context, never()),
        complete("Documents/", 1, 3)
    );
    assert_eq!(
        completion.step("Documents/", false, &context, never()),
        complete("Downloads/", 2, 3)
    );
    assert_eq!(
        completion.step("Downloads/", false, &context, never()),
        complete("docs/", 0, 3),
        "Tab wraps"
    );
    assert_eq!(
        completion.step("docs/", true, &context, never()),
        complete("Downloads/", 2, 3),
        "Shift+Tab wraps back"
    );

    completion.invalidate();
    assert_eq!(
        completion.step("DO", true, &context, never()),
        complete("Downloads/", 2, 3),
        "Shift+Tab starts at the last match"
    );
    completion.invalidate();
    assert_eq!(
        completion.step("zz", false, &context, never()),
        Step::Hint(Hint::NoMatch)
    );
    completion.invalidate();
    assert_eq!(
        completion.step(".d", false, &context, never()),
        complete(".dotfiles/", 0, 1),
        "a dot prefix asks the listing for its hidden folders"
    );
    completion.invalidate();
    assert_eq!(
        completion.step("~other", false, &context, never()),
        Step::Hint(Hint::OtherHome)
    );
}

#[test]
fn real_folder_completion_cycles_folders_forward_and_backward() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("main context");
    let root = tempfile::tempdir().expect("fixture");
    for name in ["alpha", "Alder", "beta", ".alpine"] {
        std::fs::create_dir(root.path().join(name)).expect("folder");
    }
    std::fs::write(root.path().join("almanac.txt"), b"file").expect("file");
    std::os::unix::fs::symlink(root.path().join("beta"), root.path().join("alias"))
        .expect("folder link");
    let completion = GoCompletion::new(Rc::new(GioFolders));
    let stem = format!("{}/", root.path().display());
    let (delivered, deliver) = recorder();

    let typed = format!("{stem}al");
    assert_eq!(
        completion.step(&typed, false, &context(None, &no_listing), deliver()),
        Step::Pending
    );
    settle(|| !delivered.borrow().is_empty());
    assert_eq!(
        delivered.borrow_mut().remove(0),
        complete(&format!("{stem}Alder/"), 0, 3)
    );
    let shown = format!("{stem}Alder/");
    let never = || -> Box<dyn FnOnce(Step)> { Box::new(|_| panic!("cycling is synchronous")) };
    let context = context(None, &no_listing);
    assert_eq!(
        completion.step(&shown, false, &context, never()),
        complete(&format!("{stem}alias/"), 1, 3),
        "a link to a folder is a folder"
    );
    assert_eq!(
        completion.step(&format!("{stem}alias/"), false, &context, never()),
        complete(&format!("{stem}alpha/"), 2, 3)
    );
    assert_eq!(
        completion.step(&format!("{stem}alpha/"), true, &context, never()),
        complete(&format!("{stem}alias/"), 1, 3)
    );

    completion.invalidate();
    assert_eq!(
        completion.step(&format!("{stem}.al"), false, &context, deliver()),
        Step::Pending
    );
    settle(|| !delivered.borrow().is_empty());
    assert_eq!(
        delivered.borrow_mut().remove(0),
        complete(&format!("{stem}.alpine/"), 0, 1),
        "a dot prefix reaches hidden folders"
    );

    completion.invalidate();
    completion.step(&format!("{stem}alm"), false, &context, deliver());
    settle(|| !delivered.borrow().is_empty());
    assert_eq!(
        delivered.borrow_mut().remove(0),
        Step::Hint(Hint::NoMatch),
        "files are not completed"
    );

    completion.step(&format!("{stem}missing/"), false, &context, deliver());
    settle(|| !delivered.borrow().is_empty());
    assert_eq!(
        delivered.borrow_mut().remove(0),
        Step::Hint(Hint::Unreadable)
    );
}

#[test]
fn pending_enumeration_is_cancelled_by_edits_and_never_answers_late() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("main context");
    let source = Rc::new(Controlled::default());
    let completion = GoCompletion::new(source.clone());
    let (delivered, deliver) = recorder();
    let context = context(Some(Path::new("/work")), &no_listing);

    assert_eq!(
        completion.step("/slow/pro", false, &context, deliver()),
        Step::Pending
    );
    assert!(completion.is_pending());
    assert_eq!(
        completion.step("/slow/pro", false, &context, deliver()),
        Step::Pending,
        "a repeated Tab waits for the same enumeration"
    );
    idle();
    assert_eq!(source.requested(), vec![PathBuf::from("/slow/")]);

    completion.invalidate();
    assert!(!completion.is_pending());
    settle(|| source.abandoned(0));
    assert!(!source.reply(0, Ok(vec![vec![folder("projects")]])));
    idle();
    assert!(delivered.borrow().is_empty(), "a stale answer is dropped");

    assert_eq!(
        completion.step("~/pro", true, &context, deliver()),
        Step::Pending
    );
    idle();
    assert_eq!(source.requested(), vec![PathBuf::from("/home/fixture/")]);
    assert!(source.reply(
        0,
        Ok(vec![
            vec![folder("projects"), file("proposal.txt")],
            vec![folder("Programs")],
        ])
    ));
    settle(|| !delivered.borrow().is_empty());
    assert_eq!(*delivered.borrow(), vec![complete("~/projects/", 1, 2)]);
    assert!(!completion.is_pending());
}

#[test]
fn a_dropped_completion_releases_its_receiver_when_the_answer_arrives() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("main context");
    let source = Rc::new(Controlled::default());
    let completion = GoCompletion::new(source.clone());
    let view = Rc::new(());
    let weak_view = Rc::downgrade(&view);
    let held = view.clone();
    completion.step("/closing/", false, &context(None, &no_listing), move |_| {
        drop(held);
        panic!("a closed window receives nothing");
    });
    drop(view);
    drop(completion);
    idle();
    source.reply(0, Ok(vec![vec![folder("late")]]));
    settle(|| weak_view.upgrade().is_none());
}

#[test]
fn failures_and_limits_keep_the_text_with_a_hint() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock().expect("main context");
    let source = Rc::new(Controlled::default());
    let completion = GoCompletion::new(source.clone());
    let (delivered, deliver) = recorder();
    let context = context(None, &no_listing);
    let files = |count: usize| -> Vec<Child> {
        (0..count).map(|index| file(&format!("f{index}"))).collect()
    };
    let folders = |count: usize| {
        (0..count)
            .map(|index| folder(&format!("d{index:04}")))
            .collect::<Vec<_>>()
    };

    for (reply, expected) in [
        (Err(Unreadable), Step::Hint(Hint::Unreadable)),
        (
            Ok(vec![files(MAX_SCANNED_ENTRIES), vec![folder("d")]]),
            Step::Hint(Hint::TooMany),
        ),
        (
            Ok(vec![folders(MAX_MATCHING_FOLDERS + 1)]),
            Step::Hint(Hint::TooMany),
        ),
        (
            Ok(vec![files(MAX_SCANNED_ENTRIES - 1), vec![folder("d")]]),
            complete("/big/d/", 0, 1),
        ),
        (
            Ok(vec![folders(MAX_MATCHING_FOLDERS)]),
            complete("/big/d0000/", 0, MAX_MATCHING_FOLDERS),
        ),
    ] {
        completion.invalidate();
        delivered.borrow_mut().clear();
        assert_eq!(
            completion.step("/big/d", false, &context, deliver()),
            Step::Pending
        );
        idle();
        assert!(source.reply(0, reply));
        settle(|| !delivered.borrow().is_empty());
        assert_eq!(*delivered.borrow(), vec![expected]);
    }

    let crowded = |_| {
        (0..=MAX_MATCHING_FOLDERS)
            .map(|index| OsString::from(format!("d{index}")))
            .collect()
    };
    completion.invalidate();
    assert_eq!(
        completion.step(
            "d",
            false,
            &Context {
                listing: &crowded,
                ..context
            },
            deliver()
        ),
        Step::Hint(Hint::TooMany),
        "the match limit also bounds the listing"
    );
}

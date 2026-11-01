// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use futures_channel::oneshot;
use futures_lite::future::{block_on, poll_once};
use gtk::{gio, glib};

use super::{
    Class, Job, LaneKey, Lanes, Progress, Responsiveness, Run, STALLED_AFTER, Target, Unanswered,
};
use crate::{
    adapters::volume::{MountTable, mount_watch},
    test_support::Gate,
};

const MOUNTS: &str = "\
1 0 8:1 / / rw - ext4 /dev/root rw
2 1 8:2 / /home rw - ext4 /dev/home rw
42 1 0:90 / /mnt/nas rw - nfs4 nas:/export rw
43 1 8:17 / /mnt/usb rw - vfat /dev/sdb1 rw
";

struct Bench {
    lanes: Lanes,
    mounts: MountTable,
    generation: u64,
    start: Instant,
}

impl Bench {
    fn new() -> Self {
        let mut bench = Self {
            lanes: Lanes::default(),
            mounts: MountTable::parse(b""),
            generation: 0,
            start: Instant::now(),
        };
        bench.follow(MOUNTS);
        bench
    }

    fn follow(&mut self, mounts: &str) {
        self.mounts = MountTable::parse(mounts);
        self.generation += 1;
        self.lanes.follow(self.generation, &self.mounts);
    }

    fn at(&self, seconds: f64) -> Instant {
        self.start + Duration::from_secs_f64(seconds)
    }

    fn target<'a>(&self, path: &'a str) -> Target<'a> {
        self.lanes.target(Path::new(path), &self.mounts)
    }

    fn ask(&mut self, path: &str, class: Class, seconds: f64) -> Result<u64, Unanswered> {
        let now = self.at(seconds);
        let target = self.target(path);
        let probe = self.lanes.verdict(&target, class, now, None)?;
        Ok(self.lanes.admit(probe.unwrap_or(target.key), now))
    }

    fn call(&mut self, path: &str, seconds: f64) -> u64 {
        self.ask(path, Class::Interactive, seconds)
            .expect("an answering mount")
    }

    fn answer(&mut self, ticket: u64, seconds: f64) {
        self.beat(ticket, seconds);
        self.lanes.finish(ticket, self.at(seconds));
    }

    fn fail(&mut self, ticket: u64, seconds: f64) {
        self.lanes.finish(ticket, self.at(seconds));
    }

    fn beat(&mut self, ticket: u64, seconds: f64) {
        self.lanes.beat(ticket, self.at(seconds));
    }

    fn answers(&mut self, path: &str, seconds: f64) -> bool {
        self.ask(path, Class::Interactive, seconds)
            .map(|ticket| self.answer(ticket, seconds))
            .is_ok()
    }

    fn learn(&mut self, ticket: u64, path: &str, id: u64, seconds: f64) {
        let now = self.at(seconds);
        self.lanes.learn(ticket, Path::new(path), id, now);
    }

    fn reach(&mut self, ticket: u64, path: &str, seconds: f64) -> bool {
        let lexical = self.target(path).lexical;
        let now = self.at(seconds);
        self.lanes.enter(ticket, Path::new(path), lexical, now)
    }

    fn reached(&mut self, ticket: u64, answered: bool, seconds: f64) {
        self.lanes.leave(ticket, answered, self.at(seconds));
    }

    fn state(&self, path: &str, seconds: f64) -> Responsiveness {
        self.lanes
            .responsiveness(&self.target(path), self.at(seconds))
    }
}

#[test]
fn a_mount_that_makes_no_progress_answers_new_requests_at_once() {
    let mut bench = Bench::new();
    let stuck = bench.call("/mnt/nas", 0.0);
    bench.learn(stuck, "/mnt/nas", 42, 0.0);
    let waiting = bench.call("/mnt/nas", 1.9);

    assert_eq!(
        bench.ask("/mnt/nas", Class::Interactive, 2.0),
        Err(Unanswered::TimedOut)
    );
    assert_eq!(
        bench.ask("/mnt/nas/never/listed", Class::Interactive, 2.0),
        Err(Unanswered::TimedOut)
    );
    assert!(bench.answers("/mnt/usb/photos", 2.0));
    assert!(bench.answers("/home/me", 2.0));
    assert_eq!(bench.state("/mnt/nas", 1.9), Responsiveness::Answering);
    assert_eq!(bench.state("/mnt/nas", 2.0), Responsiveness::Stalled);
    assert_eq!(bench.state("/mnt/nas", 10.0), Responsiveness::NotResponding);

    bench.answer(stuck, 10.5);
    assert_eq!(bench.state("/mnt/nas", 10.5), Responsiveness::Stalled);
    bench.answer(waiting, 10.6);
    assert_eq!(bench.state("/mnt/nas", 10.6), Responsiveness::Answering);
    assert!(bench.answers("/mnt/nas", 10.7));
}

#[test]
fn only_a_stalled_request_s_own_progress_clears_its_stall() {
    for (row, progresses) in [
        ("answered", Bench::answer as fn(&mut Bench, u64, f64)),
        ("listed a chunk", Bench::beat),
    ] {
        let mut bench = Bench::new();
        let stuck = bench.call("/mnt/nas", 0.0);
        bench.learn(stuck, "/mnt/nas", 42, 0.0);
        let cached = bench.call("/mnt/nas", 1.0);
        progresses(&mut bench, cached, 1.5);

        assert_eq!(
            bench.state("/mnt/nas", 2.5),
            Responsiveness::Stalled,
            "{row}"
        );
        for path in ["/mnt/nas", "/mnt/nas/x"] {
            assert_eq!(
                bench.ask(path, Class::Interactive, 3.0),
                Err(Unanswered::TimedOut),
                "{row}: {path}"
            );
        }
    }
}

#[test]
fn a_request_that_ends_without_progress_leaves_its_mount_stalled_until_one_answers() {
    let mut bench = Bench::new();
    let stuck = bench.call("/mnt/nas", 0.0);
    bench.learn(stuck, "/mnt/nas", 42, 0.0);
    bench.fail(stuck, 3.0);
    assert_eq!(bench.state("/mnt/nas", 3.1), Responsiveness::Stalled);

    let probe = bench.call("/mnt/nas/a", 3.2);
    assert_eq!(
        bench.ask("/mnt/nas/b", Class::Interactive, 3.3),
        Err(Unanswered::TimedOut)
    );
    assert_eq!(
        bench.ask("/mnt/nas", Class::Background, 3.3),
        Err(Unanswered::Skipped)
    );
    bench.fail(probe, 6.2);
    assert_eq!(bench.state("/mnt/nas", 10.0), Responsiveness::NotResponding);

    let probe = bench.call("/mnt/nas", 10.1);
    bench.answer(probe, 10.2);
    assert_eq!(bench.state("/mnt/nas", 10.2), Responsiveness::Answering);
    assert!(bench.answers("/mnt/nas/b", 10.3));

    let quick = bench.call("/mnt/usb", 11.0);
    bench.fail(quick, 11.1);
    assert!(
        bench.answers("/mnt/usb", 14.0),
        "an error that came at once"
    );
}

#[test]
fn a_mount_that_keeps_making_progress_keeps_answering() {
    let mut bench = Bench::new();
    let listing = bench.call("/mnt/nas/big", 0.0);
    bench.learn(listing, "/mnt/nas/big", 42, 0.0);
    for second in 1..=6 {
        let now = f64::from(second);
        bench.beat(listing, now);
        assert!(bench.answers("/mnt/nas/other", now + 0.9), "{second} s");
        assert_eq!(
            bench.state("/mnt/nas/big", now + 0.9),
            Responsiveness::Answering
        );
    }
}

#[test]
fn a_path_whose_mount_is_unknown_trips_only_itself_and_what_lies_below() {
    let mut bench = Bench::new();
    let home = bench.call("/home/me", 0.0);
    bench.learn(home, "/home/me", 2, 0.0);
    bench.answer(home, 0.0);
    let _through_symlink = bench.call("/home/me/nas/x", 0.0);

    for (path, answers) in [
        ("/home/me/nas/x", false),
        ("/home/me/nas/x/deeper", false),
        ("/home/me/nas/y", true),
        ("/home/me/docs", true),
        ("/home/me", true),
    ] {
        assert_eq!(bench.answers(path, 2.5), answers, "{path}");
    }
}

#[test]
fn a_call_that_learns_its_mount_trips_that_mount() {
    let mut bench = Bench::new();
    let through_symlink = bench.call("/home/me/nas", 0.0);
    bench.learn(through_symlink, "/home/me/nas", 42, 0.5);

    assert_eq!(bench.state("/home/me/nas", 2.4), Responsiveness::Answering);
    for (path, answers) in [
        ("/home/me/nas", false),
        ("/mnt/nas/docs", false),
        ("/home/me/docs", true),
    ] {
        assert_eq!(bench.answers(path, 2.5), answers, "{path}");
    }
}

#[test]
fn a_call_that_follows_a_link_trips_only_the_link() {
    let mut bench = Bench::new();
    let load = bench.call("/home/me", 0.0);
    bench.learn(load, "/home/me", 2, 0.0);
    assert!(bench.reach(load, "/home/me/nas", 0.5));

    for (path, answers) in [
        ("/home/me/nas", false),
        ("/home/me/nas/x", false),
        ("/home/me/docs", true),
        ("/home/me", true),
    ] {
        assert_eq!(bench.answers(path, 3.0), answers, "{path}");
    }

    bench.reached(load, false, 3.5);
    assert_eq!(bench.state("/home/me/nas", 3.6), Responsiveness::Stalled);
    assert_eq!(bench.state("/home/me", 5.4), Responsiveness::Answering);
    let other = bench.call("/home", 3.6);
    assert!(bench.reach(other, "/home/me/nas", 3.7));
    assert!(!bench.reach(load, "/home/me/nas", 3.8));
    bench.reached(other, true, 3.9);
    assert_eq!(bench.state("/home/me/nas", 3.9), Responsiveness::Answering);
}

#[test]
fn background_jobs_run_one_at_a_time_and_only_where_nothing_waits() {
    let mut bench = Bench::new();
    let interactive = bench.call("/mnt/usb", 0.0);
    assert_eq!(
        bench.ask("/mnt/usb", Class::Background, 0.1),
        Err(Unanswered::Skipped)
    );
    bench.answer(interactive, 0.2);

    let check = bench
        .ask("/mnt/usb", Class::Background, 0.3)
        .expect("an idle mount");
    assert_eq!(
        bench.ask("/mnt/usb", Class::Background, 0.4),
        Err(Unanswered::Skipped)
    );
    assert!(bench.answers("/mnt/usb", 0.5));
    bench.answer(check, 0.6);
    assert!(bench.ask("/mnt/usb", Class::Background, 0.7).is_ok());
}

#[test]
fn a_mount_table_change_forgets_only_mounts_it_moved() {
    let mut bench = Bench::new();
    let stuck = bench.call("/mnt/nas", 0.0);
    bench.learn(stuck, "/mnt/nas", 42, 0.0);
    let _unlearned = bench.call("/mnt/usb/a", 0.0);
    let root = bench.call("/mnt/new/x", 0.0);
    bench.learn(root, "/mnt/new/x", 1, 0.0);
    bench.answer(root, 0.0);
    assert!(!bench.answers("/mnt/nas", 3.0));
    assert!(!bench.answers("/mnt/usb/a", 3.0));

    let stick_swapped = "\
        1 0 8:1 / / rw - ext4 /dev/root rw\n2 1 8:2 / /home rw - ext4 /dev/home rw\n\
        51 1 8:33 / /mnt/usb rw - vfat /dev/sdc1 rw\n";
    bench.follow(stick_swapped);
    let after = bench.call("/mnt/nas", 3.1);
    bench.fail(stuck, 3.2);
    assert!(bench.answers("/mnt/usb/a", 3.1), "a stick put in its place");
    assert_eq!(bench.state("/mnt/nas", 6.0), Responsiveness::Stalled);
    bench.answer(after, 6.0);

    bench.follow("1 0 8:1 / / rw - ext4 /dev/root rw\n50 1 0:95 / /mnt/new rw - nfs4 other:/ rw\n");
    let arrived = bench.call("/mnt/new/x", 7.0);
    let (reply, mut answer) = oneshot::channel();
    let running = Run::new(bench.lanes.serial(), reply);
    bench.lanes.jobs.insert(
        (PathBuf::from("/mnt/new/x"), "check".to_owned()),
        Job {
            ticket: arrived,
            running,
            next: None,
        },
    );
    let stale = MountTable::parse(stick_swapped);
    bench
        .lanes
        .follow(bench.generation - 1, &stale)
        .into_iter()
        .for_each(Job::abandon);
    assert!(
        matches!(answer.try_recv(), Ok(None)),
        "a table older than the one followed"
    );
    assert!(bench.answers("/srv/data", 9.5), "the root mount");
    assert!(!bench.answers("/mnt/new/x", 9.5));
}

#[test]
fn a_gvfs_backend_that_stops_answering_trips_only_its_authority() {
    let mut bench = Bench::new();
    let ask = |bench: &mut Bench, authority: &str, seconds: f64| {
        let now = bench.at(seconds);
        let target = Target {
            key: LaneKey::Remote(authority.to_owned()),
            path: None,
            lexical: None,
        };
        let probe = bench
            .lanes
            .verdict(&target, Class::Interactive, now, None)?;
        Ok::<_, Unanswered>(bench.lanes.admit(probe.unwrap_or(target.key), now))
    };
    let stuck = ask(&mut bench, "smb://@nas:-1", 0.0).expect("an answering backend");
    let waiting = ask(&mut bench, "smb://@nas:-1", 1.9).expect("an answering backend");

    assert_eq!(
        ask(&mut bench, "smb://@nas:-1", 2.0),
        Err(Unanswered::TimedOut)
    );
    assert!(ask(&mut bench, "sftp://@server:-1", 2.0).is_ok());
    assert!(bench.answers("/mnt/nas", 2.0));

    bench.fail(stuck, 2.5);
    bench.fail(waiting, 2.5);
    let probe = ask(&mut bench, "smb://@nas:-1", 2.6).expect("the call that finds out");
    assert_eq!(
        ask(&mut bench, "smb://@nas:-1", 2.7),
        Err(Unanswered::TimedOut)
    );
    bench.answer(probe, 2.8);
    assert!(ask(&mut bench, "smb://@nas:-1", 2.9).is_ok());
}

#[test]
fn the_stall_watch_runs_while_a_request_may_still_stop_answering() {
    let mut bench = Bench::new();
    let mut seen = Vec::new();
    let mut watch = |bench: &mut Bench, row, seconds| {
        seen.push((row, bench.lanes.watch(bench.at(seconds))));
    };
    let listing = bench.call("/mnt/nas/big", 0.0);
    let stuck = bench.call("/mnt/usb", 0.5);
    bench.beat(listing, 1.0);
    watch(&mut bench, "both answering", 2.0);
    watch(&mut bench, "one went quiet", 2.5);
    bench.answer(listing, 2.8);
    watch(&mut bench, "only the quiet one left", 3.0);
    bench.beat(stuck, 4.0);
    watch(&mut bench, "quiet again", 6.0);
    bench.answer(stuck, 6.5);
    watch(&mut bench, "all answered", 8.5);

    assert_eq!(
        seen,
        [
            ("both answering", false),
            ("one went quiet", true),
            ("only the quiet one left", true),
            ("quiet again", true),
            ("all answered", false),
        ]
    );
}

#[test]
fn a_gvfs_call_dropped_while_it_waits_leaves_its_backend_answering() {
    let quiet = STALLED_AFTER + Duration::from_millis(200);
    let mut failures = Vec::new();
    for (row, authority, answers_after, given_up_after, expected) in [
        (
            "dropped while it waits",
            "sftp://@strata-dropped-call.invalid:-1",
            quiet * 2,
            quiet,
            (Responsiveness::Answering, true),
        ),
        (
            "ended without an answer",
            "sftp://@strata-unanswered-call.invalid:-1",
            quiet,
            quiet * 2,
            (Responsiveness::Stalled, false),
        ),
    ] {
        glib::MainContext::new().block_on(async {
            let call = async {
                glib::timeout_future(answers_after).await;
                Err::<(), _>(glib::Error::new(gio::IOErrorEnum::TimedOut, "no answer"))
            };
            let laned = super::run_remote(authority.to_owned(), Class::Interactive, call);
            drop(glib::future_with_timeout(given_up_after, laned).await);
        });
        let got = (
            super::remote_responsiveness(authority.to_owned()),
            super::admit_remote(authority.to_owned(), Class::Background).is_ok(),
        );
        if got != expected {
            failures.push(format!("{row}: got {got:?}, expected {expected:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

type Checked<'a> = Pin<Box<dyn Future<Output = Result<Arc<u32>, Unanswered>> + 'a>>;

fn checked(path: &Path, answer: u32, work: impl FnOnce(&Progress) + Send + 'static) -> Checked<'_> {
    let mut checked = Box::pin(super::run_joined(
        path,
        "check".to_owned(),
        move |progress, _| {
            work(progress);
            answer
        },
    ));
    assert!(block_on(poll_once(checked.as_mut())).is_none());
    checked
}

fn within<T>(answer: impl Future<Output = T>) -> Option<T> {
    glib::MainContext::new()
        .block_on(glib::future_with_timeout(Duration::from_secs(5), answer))
        .ok()
}

#[test]
fn a_request_after_a_publish_removed_its_mount_runs_its_own_work() {
    crate::test_support::gtk_test(
        "adapters::volume::mount_lanes::tests::a_request_after_a_publish_removed_its_mount_runs_its_own_work",
        || {
            let root = tempfile::tempdir().expect("fixture");
            let nas = root.path().join("nas");
            let table = |mounted: bool| {
                let mut lines = "1 0 8:1 / / rw - ext4 /dev/root rw\n".to_owned();
                if mounted {
                    lines += &format!("42 1 0:90 / {} rw - nfs4 nas:/export rw\n", nas.display());
                }
                MountTable::parse(lines)
            };
            let generation = Cell::new(u64::MAX / 2);
            let publish = |mounted: bool| {
                generation.set(generation.get() + 1);
                mount_watch::publish_for_isolated_test(Some(table(mounted)));
                drop(super::lanes_following(generation.get(), &table(mounted)));
            };
            for (row, reads) in [("joined", false), ("queued behind", true)] {
                publish(true);
                let gate = Gate::default();
                let mut stuck = gate.racer();
                let first = checked(&nas, 7, move |progress| {
                    if reads {
                        progress.begin_reading();
                    }
                    stuck();
                });
                let deadline = Instant::now() + Duration::from_secs(10);
                while !gate.reached() {
                    assert!(Instant::now() < deadline, "{row}: the check starts");
                    std::thread::sleep(Duration::from_millis(5));
                }
                let behind = checked(&nas, 8, |_| {});

                publish(false);
                let again = within(super::run_joined(&nas, "check".to_owned(), |_, _| 9u32));
                let waited = [within(first), within(behind)];
                drop(gate);

                assert_eq!(again, Some(Ok(Arc::new(9))), "{row}");
                assert_eq!(
                    waited,
                    [
                        Some(Err(Unanswered::TimedOut)),
                        Some(Err(Unanswered::TimedOut))
                    ],
                    "{row}"
                );
            }
        },
    );
}

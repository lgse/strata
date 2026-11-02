// SPDX-License-Identifier: MIT

use std::io::ErrorKind;

use super::*;
use crate::adapters::{FsOp, FsQuirks, Quirks, mount_lanes};

type Checks = Rc<RefCell<HashMap<Location, Result<(), LocationValidationError>>>>;
type StampAnswers = Rc<RefCell<Vec<(Location, StampAnswer)>>>;
type Outcome = Option<Result<usize, String>>;

const TIMED_OUT: &str = "The operation timed out";

fn timed_out() -> Outcome {
    Some(Err(TIMED_OUT.to_owned()))
}

fn gated_stamps(root: &Path, gate: &Gate) -> FsQuirks {
    FsQuirks::install(
        root,
        Quirks {
            before: vec![(FsOp::FolderStamp, "*", 0, gate.racer())],
            ..Quirks::default()
        },
    )
}

fn wait_until_stalled(since: Instant) {
    drain_until(|| since.elapsed() > mount_lanes::STALLED_AFTER + Duration::from_millis(200));
}

/// Admits a request on each remote backend and leaves it unanswered until it counts as stalled.
fn stall_backends(locations: &[Location]) -> Result<Vec<mount_lanes::Admitted>, String> {
    let stuck = locations
        .iter()
        .map(|location| {
            mount_lanes::admit_remote(
                mount_lanes::authority(location.uri_value().unwrap_or_default()),
                mount_lanes::Class::Interactive,
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|refusal| format!("{refusal:?}"))?;
    wait_until_stalled(Instant::now());
    Ok(stuck)
}

fn validate_into(checked: &Checks, location: &Location) -> LoadHandle {
    let checked = checked.clone();
    let key = location.clone();
    LocalFileSource.validate_location_async(
        location.clone(),
        Rc::new(move |result| {
            checked.borrow_mut().insert(key.clone(), result);
        }),
    )
}

fn check_into(answers: &StampAnswers, folder: &Path) -> LoadHandle {
    let answers = answers.clone();
    LocalFileSource.check_stamps(
        vec![Location::local(folder)],
        Rc::new(move |location, answer| answers.borrow_mut().push((location, answer))),
    )
}

fn answer_kinds(answers: &StampAnswers, folder: &Path) -> Vec<&'static str> {
    answers
        .borrow()
        .iter()
        .filter(|(location, _)| *location == Location::local(folder))
        .map(|(_, answer)| match answer {
            StampAnswer::Stamp(_) => "stamp",
            StampAnswer::Unresponsive => "unresponsive",
            _ => "other",
        })
        .collect()
}

fn both_outcomes(events: &Events) -> (Outcome, Outcome) {
    (outcome_of(events, 1), outcome_of(events, 2))
}

fn both_finished(events: &Events) -> bool {
    outcome_of(events, 1).is_some() && outcome_of(events, 2).is_some()
}

#[test]
fn repeated_loads_of_a_stuck_folder_share_one_call_and_then_time_out() -> Result<(), Box<dyn Error>>
{
    let _serial = serial();
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("file"), b"content")?;
    let gate = Gate::default();
    let scope = gated_stamps(root.path(), &gate);
    let events = Events::default();

    let first = load_into(&events, root.path(), 1);
    drain_until(|| gate.reached());
    let second = load_into(&events, root.path(), 2);
    drain_until(|| {
        mount_lanes::responsiveness(root.path()) == mount_lanes::Responsiveness::Stalled
    });
    assert_eq!(scope.count(FsOp::FolderStamp), 1);
    let third = load_into(&events, root.path(), 3);
    drain_until(|| outcome_of(&events, 3).is_some());
    assert_eq!(outcome_of(&events, 3), timed_out());
    assert_eq!(both_outcomes(&events), (None, None));

    gate.release();
    drain_until(|| both_finished(&events));
    assert_eq!(both_outcomes(&events), (Some(Ok(1)), Some(Ok(1))));
    assert_eq!(
        (
            scope.count(FsOp::FolderStamp),
            mount_lanes::responsiveness(root.path())
        ),
        (1, mount_lanes::Responsiveness::Answering)
    );
    drop((first, second, third));
    Ok(())
}

#[test]
fn a_slow_listing_that_keeps_answering_never_stalls_its_mount() -> Result<(), Box<dyn Error>> {
    let _serial = serial();
    let root = tempfile::tempdir()?;
    for name in ["a", "b", "c", "d", "e"] {
        fs::write(root.path().join(name), b"content")?;
    }
    let slow_chunk = || -> Box<dyn FnMut() + Send> {
        Box::new(|| std::thread::sleep(Duration::from_millis(700)))
    };
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            listing_chunk: Some(1),
            before: (0..5)
                .map(|_| (FsOp::ListChunks, "*", 0, slow_chunk()))
                .collect(),
            ..Quirks::default()
        },
    );
    let events = Events::default();

    let started = Instant::now();
    let first = load_into(&events, root.path(), 1);
    drain_until(|| started.elapsed() >= Duration::from_millis(2500));
    let second = load_into(&events, root.path(), 2);
    drain_until(|| both_finished(&events));

    assert_eq!(both_outcomes(&events), (Some(Ok(5)), Some(Ok(5))));
    drop((first, second));
    Ok(())
}

#[test]
fn a_load_asked_for_while_the_folder_is_being_read_lists_it_afresh() -> Result<(), Box<dyn Error>> {
    let _serial = serial();
    for names in [&["a", "b"][..], &[]] {
        let root = tempfile::tempdir()?;
        for name in names {
            fs::write(root.path().join(name), b"content")?;
        }
        let gate = Gate::default();
        let scope = FsQuirks::install(
            root.path(),
            Quirks {
                listing_chunk: Some(1),
                before: vec![(FsOp::ListChunks, "*", 0, gate.racer())],
                ..Quirks::default()
            },
        );
        let events = Events::default();

        let first = load_into(&events, root.path(), 1);
        drain_until(|| gate.reached());
        fs::write(root.path().join("c"), b"content")?;
        let second = load_into(&events, root.path(), 2);
        for _ in 0..50 {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(scope.count(FsOp::FolderStamp), 1, "one call at a time");

        gate.release();
        drain_until(|| both_finished(&events));
        assert_eq!(
            both_outcomes(&events),
            (Some(Ok(names.len())), Some(Ok(names.len() + 1))),
            "{names:?}"
        );
        assert_eq!(scope.count(FsOp::FolderStamp), 2);
        drop((first, second));
    }
    Ok(())
}

#[test]
fn a_link_whose_target_stops_answering_trips_only_that_link() -> Result<(), Box<dyn Error>> {
    let _serial = serial();
    let root = tempfile::tempdir()?;
    let sibling = root.path().join("sibling");
    fs::create_dir(root.path().join("target"))?;
    fs::create_dir(&sibling)?;
    fs::write(sibling.join("file"), b"content")?;
    std::os::unix::fs::symlink(root.path().join("target"), root.path().join("nas"))?;
    let gate = Gate::default();
    let _scope = FsQuirks::install(
        root.path(),
        Quirks {
            before: vec![(FsOp::FolderStamp, "nas", 0, gate.racer())],
            ..Quirks::default()
        },
    );
    let events = Events::default();

    let warm = load_into(&events, &sibling, 1);
    drain_until(|| outcome_of(&events, 1).is_some());
    let parked = load_into(&events, root.path(), 2);
    drain_until(|| gate.reached());
    wait_until_stalled(Instant::now());
    let again = load_into(&events, &sibling, 3);
    drain_until(|| outcome_of(&events, 3).is_some());

    assert_eq!(
        (
            outcome_of(&events, 3),
            mount_lanes::responsiveness(&sibling),
            mount_lanes::responsiveness(&root.path().join("nas"))
        ),
        (
            Some(Ok(1)),
            mount_lanes::Responsiveness::Answering,
            mount_lanes::Responsiveness::Stalled
        )
    );
    gate.release();
    drain_until(|| outcome_of(&events, 2).is_some());
    assert_eq!(outcome_of(&events, 2), Some(Ok(3)));
    drop((warm, parked, again));
    Ok(())
}

#[test]
fn a_load_or_folder_check_on_a_gvfs_backend_that_stopped_answering_is_refused_at_once()
-> Result<(), Box<dyn Error>> {
    let _serial = serial();
    let stopped = [
        Location::uri("sftp://strata-stopped-backend.invalid/share"),
        Location::uri("trash://strata-stopped-trash.invalid/"),
    ];
    let answering = Location::uri("sftp://strata-answering-backend.invalid/share");
    let stuck = stall_backends(&stopped)?;
    let events = Events::default();
    let checked = Checks::default();
    let load = |location: &Location, id| {
        enumerate_into(
            &events,
            DirectoryRequest {
                id: RequestId(id),
                ..directory_request(location.clone())
            },
        )
    };

    let refused = [load(&stopped[0], 1), load(&stopped[1], 3)];
    let elsewhere = load(&answering, 2);
    let checks =
        [&stopped[0], &stopped[1], &answering].map(|location| validate_into(&checked, location));
    drain_until(|| {
        (1..=3).all(|id| outcome_of(&events, id).is_some()) && checked.borrow().len() == 3
    });

    assert_eq!(
        (
            [outcome_of(&events, 1), outcome_of(&events, 3)],
            stopped
                .each_ref()
                .map(|location| LocalFileSource.responsiveness(location)),
            LocalFileSource.responsiveness(&answering),
        ),
        (
            [timed_out(), timed_out()],
            [Responsiveness::Stalled; 2],
            Responsiveness::Answering
        )
    );
    assert_ne!(outcome_of(&events, 2), timed_out());
    let unavailable = Err(LocationValidationError::Unavailable(
        crate::services::io_error_detail(&ErrorKind::TimedOut.into()),
    ));
    let checked = checked.take();
    assert_eq!(
        stopped.each_ref().map(|location| checked.get(location)),
        [Some(&unavailable); 2]
    );
    assert_ne!(checked.get(&answering), Some(&unavailable));
    drop((stuck, refused, elsewhere, checks));
    Ok(())
}

#[test]
fn a_hung_mount_holds_one_worker_and_answers_unresponsive() -> Result<(), Box<dyn Error>> {
    let _serial = serial();
    let root = tempfile::tempdir()?;
    let (hung, healthy) = (root.path().join("hung"), root.path().join("healthy"));
    fs::create_dir(&hung)?;
    fs::create_dir(&healthy)?;
    let gate = Gate::default();
    let scope = gated_stamps(&hung, &gate);
    let answers = StampAnswers::default();
    let relisted = Events::default();

    let first = check_into(&answers, &hung);
    drain_until(|| gate.reached());
    let repeats = [check_into(&answers, &hung), check_into(&answers, &hung)];
    let elsewhere = check_into(&answers, &healthy);
    let refused_relist = enumerate_into(&relisted, background_request(&hung));
    drain_until(|| answers.borrow().len() == 3 && outcome_of(&relisted, 1).is_some());
    let while_hung = (
        answer_kinds(&answers, &hung),
        answer_kinds(&answers, &healthy),
        outcome_of(&relisted, 1),
    );
    gate.release();
    drain_until(|| answers.borrow().len() == 4);

    assert_eq!(
        while_hung,
        (vec!["unresponsive"; 2], vec!["stamp"], timed_out())
    );
    assert_eq!(
        (
            answer_kinds(&answers, &hung),
            scope.count(FsOp::FolderStamp)
        ),
        (vec!["unresponsive", "unresponsive", "stamp"], 1)
    );
    drop((first, repeats, elsewhere, refused_relist));
    Ok(())
}

#[test]
fn a_check_answer_can_relist_while_the_check_reads_another_folder_of_its_mount()
-> Result<(), Box<dyn Error>> {
    let _serial = serial();
    let root = tempfile::tempdir()?;
    let (changed, slow) = (root.path().join("changed"), root.path().join("slow"));
    fs::create_dir(&changed)?;
    fs::create_dir(&slow)?;
    fs::write(changed.join("added elsewhere"), b"x")?;
    let gate = Gate::default();
    let _scope = gated_stamps(&slow, &gate);
    let relisted = Events::default();
    let relists: Rc<RefCell<Vec<LoadHandle>>> = Rc::default();
    let (events, handles, folder) = (relisted.clone(), relists.clone(), changed.clone());
    let _check = LocalFileSource.check_stamps(
        vec![Location::local(&changed), Location::local(&slow)],
        Rc::new(move |location, _| {
            if location == Location::local(&folder) {
                handles
                    .borrow_mut()
                    .push(enumerate_into(&events, background_request(&folder)));
            }
        }),
    );

    drain_until(|| gate.reached());
    let context = glib::MainContext::default();
    let settle = Instant::now() + Duration::from_millis(200);
    while outcome_of(&relisted, 1).is_none() && Instant::now() < settle {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
    gate.release();
    drain_until(|| outcome_of(&relisted, 1).is_some());

    assert_eq!(outcome_of(&relisted, 1), Some(Ok(1)));
    Ok(())
}

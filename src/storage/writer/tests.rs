// SPDX-License-Identifier: MIT

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use super::{Contents, Policy, Writer};

#[test]
fn jobs_run_one_at_a_time_in_order_and_survive_a_panicking_job() {
    let writer = Writer::start("strata-settings-test");
    let ran = Arc::new(Mutex::new(Vec::new()));
    let held = writer.hold();
    let answers: Vec<_> = (0..3)
        .map(|index| {
            let ran = ran.clone();
            writer.run(move || {
                assert_ne!(index, 1, "an injected failure");
                ran.lock()
                    .expect("ran")
                    .push((index, std::thread::current().id()));
                index
            })
        })
        .collect();
    assert!(!writer.flush(Duration::from_millis(50)));
    assert!(ran.lock().expect("ran").is_empty());
    drop(held);
    assert!(writer.flush(Duration::from_secs(10)));
    let ran = ran.lock().expect("ran").clone();
    assert_eq!(
        ran.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
        [0, 2]
    );
    assert!(
        ran.iter()
            .all(|(_, thread)| *thread != std::thread::current().id())
    );
    let answers: Vec<_> = answers
        .into_iter()
        .map(|mut answer| answer.try_recv().ok().flatten())
        .collect();
    assert_eq!(answers, [Some(0), None, Some(2)]);
}

#[test]
fn a_waiting_save_takes_the_newest_contents_for_its_path() {
    let writer = Writer::start("strata-settings-test");
    let directory = tempfile::tempdir().expect("fixture");
    let (settings, state, gone) = (
        directory.path().join("config/settings.toml"),
        directory.path().join("state.json"),
        directory.path().join("tabs.toml"),
    );
    std::fs::write(&gone, b"old tabs").expect("saved tabs");
    let produced = Arc::new(Mutex::new(Vec::new()));
    let held = writer.hold();
    let answers: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|contents| {
            let produced = produced.clone();
            writer.save(
                settings.clone(),
                Contents::Produced(Box::new(move || {
                    produced.lock().expect("produced").push(contents);
                    Ok(contents.as_bytes().to_vec())
                })),
                Policy::Config,
                None,
            )
        })
        .collect();
    drop(writer.save(
        state.clone(),
        Contents::Bytes(b"{}".to_vec()),
        Policy::State,
        None,
    ));
    drop(writer.save(gone.clone(), Contents::Removed, Policy::State, None));
    drop(held);
    assert!(writer.flush(Duration::from_secs(10)));
    assert_eq!(*produced.lock().expect("produced"), ["second"]);
    assert_eq!(std::fs::read(&settings).expect("settings"), b"second");
    assert_eq!(std::fs::read(&state).expect("state"), b"{}");
    assert!(!gone.exists());
    for mut answer in answers {
        assert!(matches!(answer.try_recv(), Ok(Some(Ok(())))));
    }
}

#[test]
fn only_files_another_process_wrote_are_read_again() {
    let writer = Writer::start("strata-settings-test");
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("folder-views.toml");
    let read = |writer: &Writer| {
        writer
            .reread(&path, |contents| Ok(contents.to_owned()))
            .expect("re-read")
    };
    drop(writer.save(
        path.clone(),
        Contents::Bytes(b"ours".to_vec()),
        Policy::State,
        None,
    ));
    assert!(writer.flush(Duration::from_secs(10)));
    assert_eq!(read(writer), None);
    std::fs::write(&path, b"theirs, longer").expect("another process saves");
    assert_eq!(read(writer).as_deref(), Some("theirs, longer"));
    assert_eq!(read(writer), None);
}

// SPDX-License-Identifier: MIT

//! The session host as a fuzz host: which replays run there, what a command takes to send
//! them there, and how their streams and artifacts say so.

use std::collections::BTreeSet;

use super::{
    command::planned,
    command_tests::parse,
    host::Host,
    observe_tests::stream,
    replay::Stream,
    report::{Examined, Run, hosts_line},
    rules::Ignored,
};
use crate::{DisplayKind, target::Target};

/// The tree's alphabet that names the session host.
const SESSION: &str = "session-pointer";

pub(crate) fn a_rune_replay_runs_on_the_session_host_where_its_alphabet_or_the_command_says() {
    // Only a Rune replay leaves the display the environment names; GTK is the reference on
    // Xvfb, as in every parity run.
    let environment = DisplayKind::from_environment().expect("the runner's display");
    assert_eq!(Host::Session.display(Target::Rune), Ok(DisplayKind::Wayland));
    assert_eq!(Host::Session.display(Target::Gtk), Ok(environment));
    assert_eq!(Host::Offscreen.display(Target::Rune), Ok(environment));

    let hosts = |line: &str| -> BTreeSet<Host> {
        let options = parse(line).unwrap_or_else(|error| panic!("{line}: {error}"));
        let plan = planned(&options).unwrap_or_else(|error| panic!("{line}: {error}"));
        assert!(!plan.jobs.is_empty(), "{line}");
        plan.jobs.iter().map(|job| job.host).collect()
    };
    let session = format!("run --alphabet {SESSION} --seeds 1..3 --target gtk,rune");
    assert_eq!(hosts(&session), BTreeSet::from([Host::Session]));
    assert_eq!(
        hosts(&format!("{session} --host offscreen")),
        BTreeSet::from([Host::Offscreen])
    );
    let calibration = "run --alphabet calibration --seeds 1..3 --target gtk,rune";
    assert_eq!(hosts(calibration), BTreeSet::from([Host::Offscreen]));
    assert_eq!(
        hosts(&format!("{calibration} --host session")),
        BTreeSet::from([Host::Session])
    );
    // A corpus replayed area by area takes each area's host.
    let replayed = hosts("replay --target gtk,rune");
    assert!(replayed.contains(&Host::Offscreen), "{replayed:?}");

    for (line, expected) in [
        (format!("{calibration} --host window"), "offscreen or session"),
        (
            "run --alphabet calibration --seeds 1 --target gtk --host session".to_owned(),
            "the Rune target's host",
        ),
    ] {
        let error = parse(&line).expect_err(expected);
        assert!(error.contains(expected), "{error:?} should say {expected:?}");
    }
}

pub(crate) fn a_session_host_run_names_its_host_and_keeps_its_own_folder() {
    let options = parse(&format!(
        "run --alphabet {SESSION} --seeds 1 --target gtk,rune"
    ))
    .expect("a run takes both targets");
    let sequence = super::generate::generate(
        &super::alphabet::load(SESSION).unwrap_or_else(|error| panic!("{error}")),
        1,
        2,
    );
    let examined = Examined::new(SESSION, &sequence, &options.targets, Host::Session);
    assert_eq!(examined.targets, "gtk-rune-session");
    let mut run = Run::new(&options, &Ignored::default());
    run.hosts([(SESSION, Host::Session)]);
    run.judge(&examined);
    let summary = run.summary();
    assert!(
        summary
            .iter()
            .any(|line| *line == format!("FUZZ artifacts: target/qa-fuzz/{SESSION}/gtk-rune-session/")),
        "{summary:?}"
    );
    assert!(
        summary
            .iter()
            .any(|line| line.starts_with("FUZZ hosts: rune on the session host")
                && line.ends_with(&format!("for {SESSION}; gtk on Xvfb"))),
        "{summary:?}"
    );
    // A GTK calibration of the alphabet replays nothing on a host of Rune's: its folder and
    // its summary are the offscreen host's.
    let gtk = parse(&format!("run --alphabet {SESSION} --seeds 1 --target gtk"))
        .expect("a calibration takes GTK alone");
    assert_eq!(
        Examined::new(SESSION, &sequence, &gtk.targets, Host::Session).targets,
        "gtk"
    );
    assert_eq!(hosts_line(&BTreeSet::from([SESSION]), false), None);
    assert_eq!(hosts_line(&BTreeSet::new(), true), None);

    // A stream says the display it ran on, and one kept before it said ran on Xvfb.
    let mut rune = stream(&["archive"]);
    rune.header.target = "rune".to_owned();
    rune.header.display = "wayland".to_owned();
    let read = Stream::parse(&rune.text()).expect("the stream reads back");
    assert_eq!(read.header.display, "wayland");
    let older = rune.text().replacen(r#""display":"wayland","#, "", 1);
    assert_ne!(older, rune.text());
    let read = Stream::parse(&older).expect("an older stream reads");
    assert_eq!(read.header.display, "x11");
    // Two displays of one sequence still answer one another: that is the survey.
    assert_eq!(read.header.mismatch(&rune.header), None);
}
